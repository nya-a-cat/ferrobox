#!/usr/bin/env python3
"""Converge the GitHub Linux and Apple Silicon host capability evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Any


LINUX_CHECKS = (
    "rust-toolchain",
    "native-build-tools",
    "action-pin-policy",
    "dependency-lock",
    "workspace-tests",
    "workspace-build",
    "static-guest-build",
    "process-api-lifecycle",
    "cli-lifecycle",
)
MACOS_CHECKS = (
    "rust-toolchain",
    "native-tools",
    "action-pin-policy",
    "dependency-lock",
    "host-tests",
    "host-build",
    "process-api-lifecycle",
    "cli-lifecycle",
)
COMMON_CHECKS = (
    "rust-toolchain",
    "action-pin-policy",
    "dependency-lock",
    "process-api-lifecycle",
    "cli-lifecycle",
)
PLATFORMS = {
    "linux-aarch64": {
        "runner_arch": "ARM64",
        "runner_label": "ubuntu-24.04-arm",
        "runner_os": "Linux",
        "kernel": "Linux",
        "machine": "aarch64",
        "rust_host": "aarch64-unknown-linux-gnu",
        "static_guest_target": "aarch64-unknown-linux-musl",
        "elf_machine": 183,
        "checks": LINUX_CHECKS,
    },
    "linux-x86_64": {
        "runner_arch": "X64",
        "runner_label": "ubuntu-24.04",
        "runner_os": "Linux",
        "kernel": "Linux",
        "machine": "x86_64",
        "rust_host": "x86_64-unknown-linux-gnu",
        "static_guest_target": "x86_64-unknown-linux-musl",
        "elf_machine": 62,
        "checks": LINUX_CHECKS,
    },
    "macos-aarch64": {
        "runner_arch": "ARM64",
        "runner_label": "macos-15",
        "runner_os": "macOS",
        "kernel": "Darwin",
        "machine": "arm64",
        "rust_host": "aarch64-apple-darwin",
        "static_guest_target": None,
        "elf_machine": None,
        "checks": MACOS_CHECKS,
    },
}
SENSITIVE_KEYS = {"authorization", "credential", "secret", "token"}
TOP_LEVEL_FIELDS = {
    "contract",
    "cpu",
    "guest_artifact",
    "kvm",
    "platform_id",
    "runner",
    "schema_version",
    "source",
    "system",
    "toolchain",
    "verification",
}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"invalid host capability evidence {path}: {error}")


def reject_sensitive_keys(value: Any, location: str, errors: list[str]) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key.lower() in SENSITIVE_KEYS:
                errors.append(f"sensitive key at {location}.{key}")
            reject_sensitive_keys(child, f"{location}.{key}", errors)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            reject_sensitive_keys(child, f"{location}[{index}]", errors)


def validate_evidence(
    evidence: Any, path: Path, errors: list[str]
) -> tuple[str | None, dict[str, Any] | None]:
    if not isinstance(evidence, dict) or set(evidence) != TOP_LEVEL_FIELDS:
        errors.append(f"unexpected top-level fields in {path}")
        return None, None
    platform_id = evidence.get("platform_id")
    expected = PLATFORMS.get(platform_id)
    if expected is None:
        errors.append(f"unexpected platform identity {platform_id!r}")
        return None, None
    if evidence.get("schema_version") != 1:
        errors.append(f"{platform_id} schema version drift")
    if evidence.get("contract") != "ferrobox-host-capability-v1":
        errors.append(f"{platform_id} contract drift")

    source = evidence.get("source", {})
    if not isinstance(source, dict) or not re.fullmatch(
        r"[0-9a-f]{40}", str(source.get("commit", ""))
    ):
        errors.append(f"{platform_id} source identity drift")
    runner = evidence.get("runner", {})
    if (
        runner.get("label") != expected["runner_label"]
        or runner.get("arch") != expected["runner_arch"]
        or runner.get("os") != expected["runner_os"]
        or runner.get("environment") != "github-hosted"
    ):
        errors.append(f"{platform_id} runner identity drift")
    system = evidence.get("system", {})
    if (
        system.get("kernel") != expected["kernel"]
        or system.get("machine") != expected["machine"]
    ):
        errors.append(f"{platform_id} kernel identity drift")
    cpu = evidence.get("cpu", {})
    if cpu.get("architecture") != expected["machine"]:
        errors.append(f"{platform_id} CPU architecture drift")
    toolchain = evidence.get("toolchain", {})
    if (
        toolchain.get("rust_host") != expected["rust_host"]
        or toolchain.get("static_guest_target") != expected["static_guest_target"]
    ):
        errors.append(f"{platform_id} Rust target drift")
    guest = evidence.get("guest_artifact")
    if expected["static_guest_target"] is None:
        if guest is not None:
            errors.append(f"{platform_id} unexpectedly retained a Linux guest")
    else:
        if not isinstance(guest, dict):
            errors.append(f"{platform_id} static guest is missing")
        elif (
            guest.get("elf_machine") != expected["elf_machine"]
            or guest.get("has_interpreter") is not False
            or guest.get("valid_elf64_little_endian") is not True
            or guest.get("executable") is not True
            or not re.fullmatch(r"[0-9a-f]{64}", str(guest.get("sha256", "")))
        ):
            errors.append(f"{platform_id} static guest identity drift")
    kvm = evidence.get("kvm", {})
    if kvm.get("path") != "/dev/kvm" or kvm.get("firecracker_exercised") is not False:
        errors.append(f"{platform_id} KVM observation drift")
    for key in (
        "exists",
        "character_device",
        "readable",
        "writable",
        "openable_read_write",
    ):
        if not isinstance(kvm.get(key), bool):
            errors.append(f"{platform_id} KVM field {key} is not boolean")
    verification = evidence.get("verification", {})
    check_outcomes = verification.get("checks", {})
    if (
        verification.get("backend") != "process"
        or verification.get("isolation") != "none"
        or verification.get("unsafe_process_opt_in") is not True
        or verification.get("complete") is not True
        or verification.get("errors") != []
        or not isinstance(check_outcomes, dict)
        or set(check_outcomes) != set(expected["checks"])
        or set(check_outcomes.values()) != {"success"}
    ):
        errors.append(f"{platform_id} shared smoke contract failed")
    reject_sensitive_keys(evidence, platform_id, errors)
    return platform_id, expected


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    errors: list[str] = []
    paths = sorted(arguments.evidence_root.rglob("host-capability.json"))
    if len(paths) != len(PLATFORMS):
        errors.append(
            f"expected {len(PLATFORMS)} host capability documents, found {len(paths)}"
        )

    retained: dict[str, dict[str, Any]] = {}
    source_identity: dict[str, Any] | None = None
    for path in paths:
        evidence = read_json(path)
        platform_id, _ = validate_evidence(evidence, path, errors)
        if platform_id is None:
            continue
        if platform_id in retained:
            errors.append(f"duplicate host capability document for {platform_id}")
            continue
        source = evidence["source"]
        common_source = {
            "repository": source.get("repository"),
            "commit": source.get("commit"),
            "ref": source.get("ref"),
            "run_id": source.get("run_id"),
            "run_attempt": source.get("run_attempt"),
            "workflow": source.get("workflow"),
        }
        if source_identity is None:
            source_identity = common_source
        elif common_source != source_identity:
            errors.append(f"{platform_id} source identity differs from the matrix")
        retained[platform_id] = {
            "platform_id": platform_id,
            "evidence_sha256": sha256(path),
            "runner": evidence["runner"],
            "system": evidence["system"],
            "cpu": evidence["cpu"],
            "toolchain": evidence["toolchain"],
            "guest_artifact": evidence["guest_artifact"],
            "kvm": evidence["kvm"],
            "checks": evidence["verification"]["checks"],
        }

    missing = sorted(set(PLATFORMS) - set(retained))
    if missing:
        errors.append(f"missing platform evidence: {', '.join(missing)}")

    matrix = {
        "schema_version": 2,
        "contract": "ferrobox-host-architecture-matrix-v2",
        "source": source_identity,
        "shared_smoke": {
            "backend": "process",
            "isolation": "none",
            "common_checks": list(COMMON_CHECKS),
            "platform_count": len(retained),
        },
        "platforms": [retained[name] for name in sorted(retained)],
        "verification": {"complete": not errors, "errors": errors},
    }
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(
        json.dumps(matrix, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(json.dumps(matrix, indent=2, sort_keys=True))
    if errors:
        raise SystemExit("; ".join(errors))


if __name__ == "__main__":
    main()
