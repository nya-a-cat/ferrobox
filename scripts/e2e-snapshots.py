#!/usr/bin/env python3
"""Hosted-KVM acceptance for live snapshots, clones, restore, and rollback."""

from __future__ import annotations

import base64
import json
import os
from pathlib import Path
import stat
import subprocess
import time
from typing import Any
from urllib import error, parse, request


BASE_URL = os.environ.get("FERROBOX_API_URL", "http://127.0.0.1:18081")
RUNTIME_ROOT = Path(os.environ["FERROBOX_RUNTIME_ROOT"])
CHROOT_BASE = Path(os.environ["FERROBOX_CHROOT_BASE"])
AUDIT_LOG = Path(os.environ["FERROBOX_AUDIT_LOG"])
FAILURE_FILE = Path(os.environ["FERROBOX_TEST_CLONE_FAILURE_FILE"])
MANIFEST_EVIDENCE = Path(os.environ["FERROBOX_SNAPSHOT_MANIFEST_EVIDENCE"])
TTL_SECONDS = 600

sandboxes: dict[str, str] = {}
snapshots: dict[str, str] = {}
checks: list[str] = []


def api(
    method: str,
    path: str,
    *,
    token: str | None = None,
    payload: dict[str, Any] | None = None,
    expected: int | tuple[int, ...] = 200,
) -> Any:
    headers = {"Accept": "application/json"}
    data = None
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    if payload is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(payload).encode()
    call = request.Request(BASE_URL + path, data=data, headers=headers, method=method)
    try:
        with request.urlopen(call, timeout=90) as response:
            status = response.status
            raw = response.read()
    except error.HTTPError as failure:
        status = failure.code
        raw = failure.read()
    allowed = (expected,) if isinstance(expected, int) else expected
    parsed = json.loads(raw) if raw else None
    if status not in allowed:
        raise AssertionError(
            f"{method} {path}: expected {allowed}, received {status}: {parsed}"
        )
    return parsed


def create_sandbox() -> tuple[str, str]:
    created = api(
        "POST",
        "/v1/sandboxes",
        payload={
            "template": "python",
            "cpu_count": 1,
            "memory_mb": 512,
            "timeout_seconds": TTL_SECONDS,
            "network": {"internet_access": False},
        },
        expected=201,
    )
    sandbox_id = created["sandbox_id"]
    token = created["token"]
    sandboxes[sandbox_id] = token
    assert created["state"] == "running"
    return sandbox_id, token


def delete_sandbox(sandbox_id: str) -> None:
    token = sandboxes.pop(sandbox_id)
    api("DELETE", f"/v1/sandboxes/{sandbox_id}", token=token, expected=204)


def create_snapshot(sandbox_id: str, token: str, name: str) -> dict[str, Any]:
    created = api(
        "POST",
        f"/v1/sandboxes/{sandbox_id}/snapshots",
        token=token,
        payload={"name": name},
        expected=201,
    )
    snapshots[created["snapshot_id"]] = created["token"]
    return created


def delete_snapshot(snapshot_id: str) -> None:
    token = snapshots.pop(snapshot_id)
    api("DELETE", f"/v1/snapshots/{snapshot_id}", token=token, expected=204)


def write_file(sandbox_id: str, token: str, path: str, value: bytes) -> None:
    response = api(
        "PUT",
        f"/v1/sandboxes/{sandbox_id}/files",
        token=token,
        payload={
            "path": path,
            "content_base64": base64.b64encode(value).decode(),
            "overwrite": True,
        },
    )
    assert response["bytes_written"] == len(value)


def read_file(
    sandbox_id: str,
    token: str,
    path: str,
    *,
    expected: int | tuple[int, ...] = 200,
) -> bytes | None:
    query = parse.urlencode({"path": path})
    response = api(
        "GET",
        f"/v1/sandboxes/{sandbox_id}/files?{query}",
        token=token,
        expected=expected,
    )
    if response is None or "content_base64" not in response:
        return None
    return base64.b64decode(response["content_base64"])


def restore(snapshot_id: str, snapshot_token: str) -> tuple[str, str]:
    restored = api(
        "POST",
        f"/v1/snapshots/{snapshot_id}/restore",
        token=snapshot_token,
        payload={"timeout_seconds": TTL_SECONDS},
        expected=201,
    )
    sandbox_id = restored["sandbox_id"]
    token = restored["token"]
    sandboxes[sandbox_id] = token
    return sandbox_id, token


def process_ids() -> tuple[int, ...]:
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if (entry / "comm").read_text().strip() == "firecracker":
                found.append(int(entry.name))
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            pass
    return tuple(sorted(found))


def jail_roots() -> tuple[str, ...]:
    if not CHROOT_BASE.exists():
        return ()
    return tuple(sorted(str(path) for path in CHROOT_BASE.rglob("root") if path.is_dir()))


def cgroup_paths() -> tuple[str, ...]:
    root = Path("/sys/fs/cgroup/ferrobox")
    if not root.exists():
        return ()
    return tuple(sorted(str(path) for path in root.rglob("*") if path.is_dir()))


def network_namespaces() -> str:
    return subprocess.run(
        ["ip", "netns", "list"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def resource_sample() -> tuple[Any, ...]:
    return process_ids(), jail_roots(), cgroup_paths(), network_namespaces()


def wait_for_file(sandbox_id: str, token: str, path: str, value: bytes) -> None:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        observed = read_file(sandbox_id, token, path, expected=(200, 404))
        if observed == value:
            return
        time.sleep(0.25)
    raise AssertionError(f"restored active process did not write {path}")


def check_artifact(snapshot: dict[str, Any]) -> None:
    root = RUNTIME_ROOT / "snapshots" / snapshot["snapshot_id"]
    assert stat.S_IMODE(root.stat().st_mode) == 0o700
    for name in ("manifest.json", "vmstate", "memory", "rootfs.ext4"):
        assert stat.S_IMODE((root / name).stat().st_mode) == 0o444
    token_path = root / "restore-token"
    assert stat.S_IMODE(token_path.stat().st_mode) == 0o400
    restore_token = token_path.read_text()
    manifest_text = (root / "manifest.json").read_text()
    manifest = json.loads(manifest_text)
    assert restore_token not in manifest_text
    assert manifest["schema_version"] == 1
    assert manifest["snapshot_id"] == snapshot["snapshot_id"]
    assert manifest["digest_sha256"] == snapshot["digest_sha256"]
    assert set(manifest["artifacts"]) == {"vmstate", "memory", "rootfs.ext4"}
    assert not list((RUNTIME_ROOT / "snapshots").glob(".*.partial"))


def run() -> None:
    initial_resources = resource_sample()
    source_id, source_token = create_sandbox()
    api(
        "POST",
        f"/v1/sandboxes/{source_id}/snapshots",
        token=source_token,
        payload={"name": " padded"},
        expected=400,
    )
    checks.append("name-validation")

    write_file(source_id, source_token, "/home/sandbox/state.txt", b"captured")
    child_code = (
        "import pathlib,time; payload={'value': 41}; time.sleep(12); "
        "pathlib.Path('/home/sandbox/process.txt').write_text(str(payload['value']+1))"
    )
    launcher = (
        "import subprocess,sys; "
        f"subprocess.Popen([sys.executable,'-c',{child_code!r}],"
        "stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,"
        "stderr=subprocess.DEVNULL,start_new_session=True)"
    )
    launched = api(
        "POST",
        f"/v1/sandboxes/{source_id}/commands",
        token=source_token,
        payload={"argv": ["python3", "-c", launcher]},
    )
    assert launched["termination"] == {"kind": "exited", "exit_code": 0}

    primary = create_snapshot(source_id, source_token, "running-checkpoint")
    primary_id = primary["snapshot_id"]
    primary_token = primary["token"]
    assert primary["source_state"] == "running"
    assert len(primary["digest_sha256"]) == 64
    check_artifact(primary)
    MANIFEST_EVIDENCE.write_text(
        (RUNTIME_ROOT / "snapshots" / primary_id / "manifest.json").read_text()
    )
    assert api(
        "GET", f"/v1/sandboxes/{source_id}", token=source_token
    )["state"] == "running"
    assert read_file(source_id, source_token, "/home/sandbox/state.txt") == b"captured"
    checks.extend(["running-capture", "artifact-schema"])

    secondary = create_snapshot(source_id, source_token, "pagination-checkpoint")
    first_page = api(
        "GET",
        f"/v1/sandboxes/{source_id}/snapshots?limit=1",
        token=source_token,
    )
    assert [item["snapshot_id"] for item in first_page["snapshots"]] == [primary_id]
    assert first_page["next_cursor"] == primary_id
    second_page = api(
        "GET",
        f"/v1/sandboxes/{source_id}/snapshots?limit=1&cursor={primary_id}",
        token=source_token,
    )
    assert [item["snapshot_id"] for item in second_page["snapshots"]] == [
        secondary["snapshot_id"]
    ]
    assert second_page["next_cursor"] is None
    delete_snapshot(secondary["snapshot_id"])
    checks.append("stable-pagination")

    api("GET", f"/v1/snapshots/{primary_id}", token=source_token, expected=401)
    api("GET", f"/v1/sandboxes/{source_id}", token=primary_token, expected=401)
    verification = api(
        "POST", f"/v1/snapshots/{primary_id}/verify", token=primary_token
    )
    assert verification == {
        "snapshot_id": primary_id,
        "valid": True,
        "checked_artifacts": 3,
        "failure": None,
    }
    checks.extend(["token-separation", "integrity-verification"])

    write_file(source_id, source_token, "/home/sandbox/state.txt", b"source-after")
    assert read_file(source_id, source_token, "/home/sandbox/state.txt") == b"source-after"
    restored_id, restored_token = restore(primary_id, primary_token)
    assert restored_id != source_id
    assert restored_token not in {source_token, primary_token}
    assert read_file(restored_id, restored_token, "/home/sandbox/state.txt") == b"captured"
    wait_for_file(restored_id, restored_token, "/home/sandbox/process.txt", b"42")
    assert read_file(source_id, source_token, "/home/sandbox/state.txt") == b"source-after"
    checks.extend(["live-process-memory", "source-divergence", "independent-restore"])

    cloned = api(
        "POST",
        f"/v1/snapshots/{primary_id}/clones",
        token=primary_token,
        payload={"count": 2, "timeout_seconds": TTL_SECONDS},
        expected=201,
    )["sandboxes"]
    clone_ids = [item["sandbox_id"] for item in cloned]
    clone_tokens = [item["token"] for item in cloned]
    assert len(set(clone_ids + [source_id, restored_id])) == 4
    assert len(set(clone_tokens + [source_token, restored_token, primary_token])) == 5
    for item in cloned:
        sandboxes[item["sandbox_id"]] = item["token"]
        assert read_file(item["sandbox_id"], item["token"], "/home/sandbox/state.txt") == b"captured"
    write_file(clone_ids[0], clone_tokens[0], "/home/sandbox/state.txt", b"clone-a")
    assert read_file(clone_ids[1], clone_tokens[1], "/home/sandbox/state.txt") == b"captured"
    assert read_file(restored_id, restored_token, "/home/sandbox/state.txt") == b"captured"
    checks.append("clone-isolation")

    rolled_back = api(
        "POST",
        f"/v1/sandboxes/{source_id}/rollback/{primary_id}",
        token=source_token,
    )
    assert rolled_back["sandbox_id"] == source_id
    assert rolled_back["state"] == "running"
    assert read_file(source_id, source_token, "/home/sandbox/state.txt") == b"captured"
    checks.append("atomic-same-id-rollback")

    delete_sandbox(source_id)
    api("GET", f"/v1/sandboxes/{source_id}", token=source_token, expected=404)
    verification = api(
        "POST", f"/v1/snapshots/{primary_id}/verify", token=primary_token
    )
    assert verification["valid"] is True
    post_delete_id, post_delete_token = restore(primary_id, primary_token)
    assert read_file(post_delete_id, post_delete_token, "/home/sandbox/state.txt") == b"captured"
    checks.append("source-independent-lifetime")

    paused_id, paused_token = create_sandbox()
    api("POST", f"/v1/sandboxes/{paused_id}/pause", token=paused_token, expected=204)
    paused_snapshot = create_snapshot(paused_id, paused_token, "paused-checkpoint")
    assert paused_snapshot["source_state"] == "paused"
    assert api("GET", f"/v1/sandboxes/{paused_id}", token=paused_token)["state"] == "paused"
    api("POST", f"/v1/sandboxes/{paused_id}/resume", token=paused_token, expected=204)
    delete_sandbox(paused_id)
    assert api(
        "POST",
        f"/v1/snapshots/{paused_snapshot['snapshot_id']}/verify",
        token=paused_snapshot["token"],
    )["valid"] is True
    delete_snapshot(paused_snapshot["snapshot_id"])
    checks.append("paused-source-preservation")

    before_failure = resource_sample()
    FAILURE_FILE.write_text("1\n")
    try:
        api(
            "POST",
            f"/v1/snapshots/{primary_id}/clones",
            token=primary_token,
            payload={"count": 2, "timeout_seconds": TTL_SECONDS},
            expected=500,
        )
    finally:
        FAILURE_FILE.unlink(missing_ok=True)
    time.sleep(1)
    assert resource_sample() == before_failure
    assert api(
        "POST", f"/v1/snapshots/{primary_id}/verify", token=primary_token
    )["valid"] is True
    checks.append("partial-clone-cleanup")

    for sandbox_id in [restored_id, *clone_ids, post_delete_id]:
        delete_sandbox(sandbox_id)

    before_corruption = resource_sample()
    memory_path = RUNTIME_ROOT / "snapshots" / primary_id / "memory"
    with memory_path.open("r+b") as memory:
        memory.seek(4096)
        original = memory.read(1)
        assert original
        memory.seek(4096)
        memory.write(bytes([original[0] ^ 0xFF]))
        memory.flush()
        os.fsync(memory.fileno())
    invalid = api(
        "POST", f"/v1/snapshots/{primary_id}/verify", token=primary_token
    )
    assert invalid["valid"] is False
    assert "digest mismatch" in invalid["failure"]
    api(
        "POST",
        f"/v1/snapshots/{primary_id}/restore",
        token=primary_token,
        payload={"timeout_seconds": TTL_SECONDS},
        expected=503,
    )
    assert resource_sample() == before_corruption
    checks.append("corruption-fails-closed")
    delete_snapshot(primary_id)

    assert not sandboxes
    assert not snapshots
    assert not list((RUNTIME_ROOT / "snapshots").glob(".*.partial"))
    assert not [path for path in (RUNTIME_ROOT / "snapshots").iterdir()]
    audit = AUDIT_LOG.read_text()
    for secret in [source_token, primary_token, restored_token, *clone_tokens]:
        assert secret not in audit
    for operation in (
        "snapshot_create",
        "snapshot_restore",
        "snapshot_clone",
        "snapshot_rollback",
        "snapshot_delete",
    ):
        assert f'"operation":"{operation}"' in audit
    assert resource_sample() == initial_resources
    checks.extend(["artifact-cleanup", "audit-redaction"])
    print(json.dumps({"schema_version": 1, "checks": checks}, indent=2))


if __name__ == "__main__":
    try:
        run()
    finally:
        FAILURE_FILE.unlink(missing_ok=True)
        for sandbox_id, token in list(sandboxes.items()):
            try:
                api("DELETE", f"/v1/sandboxes/{sandbox_id}", token=token, expected=(204, 404))
            except Exception:
                pass
        for snapshot_id, token in list(snapshots.items()):
            try:
                api("DELETE", f"/v1/snapshots/{snapshot_id}", token=token, expected=(204, 404))
            except Exception:
                pass
