#!/usr/bin/env python3
"""Enable and benchmark fs-verity on immutable runtime template assets."""

from __future__ import annotations

import argparse
import errno
import hashlib
import json
import math
import os
import re
import subprocess
import tempfile
import time
from pathlib import Path


DIGEST_RE = re.compile(r"(?:sha256:)?([0-9a-f]{64})")


def command_output(argv: list[str]) -> str:
    completed = subprocess.run(
        argv,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return completed.stdout.strip()


def parse_digest(output: str) -> str:
    matches = DIGEST_RE.findall(output)
    if len(matches) != 1:
        raise RuntimeError("fsverity output did not contain exactly one SHA-256 digest")
    return matches[0]


def timed_digest(argv: list[str]) -> tuple[str, int]:
    started = time.perf_counter_ns()
    output = command_output(argv)
    elapsed_us = (time.perf_counter_ns() - started) // 1_000
    return parse_digest(output), elapsed_us


def nearest_rank(samples: list[int], percentile: int) -> int:
    ordered = sorted(samples)
    rank = max(1, math.ceil(len(ordered) * percentile / 100))
    return ordered[rank - 1]


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def require_write_rejected(path: Path) -> int:
    try:
        descriptor = os.open(path, os.O_WRONLY)
    except OSError as error:
        if error.errno not in {errno.EACCES, errno.EPERM, errno.EROFS}:
            raise
        return error.errno
    os.close(descriptor)
    raise RuntimeError("fs-verity file unexpectedly opened for writing")


def verify_reflink_clone(
    path: Path, expected_sha256: str, fsverity: Path
) -> dict[str, object]:
    with tempfile.TemporaryDirectory(dir=path.parent) as temporary:
        clone = Path(temporary) / "clone"
        subprocess.run(
            ["cp", "--reflink=always", "--", str(path), str(clone)],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        clone_sha256 = file_sha256(clone)
        measured = subprocess.run(
            [str(fsverity), "measure", str(clone)],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        if measured.returncode == 0:
            raise RuntimeError("reflink clone unexpectedly retained fs-verity metadata")
        if clone_sha256 != expected_sha256:
            raise RuntimeError("reflink clone changed traditional SHA-256")
        return {
            "sha256": clone_sha256,
            "fsverity_metadata_preserved": False,
            "measure_return_code": measured.returncode,
        }


def inspect_artifact(
    name: str,
    path: Path,
    expected_sha256: str,
    fsverity: Path,
    samples: int,
) -> dict[str, object]:
    if not path.is_file() or not path.is_absolute():
        raise RuntimeError(f"{name} must be an absolute regular file")
    if not re.fullmatch(r"[0-9a-f]{64}", expected_sha256):
        raise RuntimeError(f"{name} expected SHA-256 is invalid")
    actual_sha256 = file_sha256(path)
    if actual_sha256 != expected_sha256:
        raise RuntimeError(f"{name} traditional SHA-256 did not match the catalog")

    filesystem = command_output(
        ["findmnt", "--noheadings", "--output", "FSTYPE", "--target", str(path)]
    )
    if filesystem != "btrfs":
        raise RuntimeError(f"{name} is stored on {filesystem}, expected btrfs")

    offline_digest, offline_digest_us = timed_digest(
        [str(fsverity), "digest", str(path), "--compact"]
    )
    started = time.perf_counter_ns()
    subprocess.run(
        [str(fsverity), "enable", str(path)],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    enable_us = (time.perf_counter_ns() - started) // 1_000

    measure_us: list[int] = []
    for _ in range(samples):
        measured_digest, elapsed_us = timed_digest(
            [str(fsverity), "measure", str(path)]
        )
        if measured_digest != offline_digest:
            raise RuntimeError(f"{name} measured digest changed")
        measure_us.append(elapsed_us)

    write_errno = require_write_rejected(path)
    artifact: dict[str, object] = {
        "name": name,
        "path": str(path),
        "filesystem": filesystem,
        "size_bytes": path.stat().st_size,
        "traditional_sha256": actual_sha256,
        "traditional_sha256_verified": True,
        "fsverity_digest": f"sha256:{offline_digest}",
        "fsverity_differs_from_traditional_sha256": offline_digest != expected_sha256,
        "offline_digest_us": offline_digest_us,
        "enable_us": enable_us,
        "measure_us": measure_us,
        "measure_p50_us": nearest_rank(measure_us, 50),
        "measure_p95_us": nearest_rank(measure_us, 95),
        "measure_max_us": max(measure_us),
        "measurements_match_offline_digest": True,
        "write_rejected_errno": write_errno,
    }
    if name == "rootfs":
        artifact["reflink_clone"] = verify_reflink_clone(
            path, expected_sha256, fsverity
        )
    return artifact


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--fsverity", type=Path, required=True)
    parser.add_argument("--kernel", type=Path, required=True)
    parser.add_argument("--kernel-sha256", required=True)
    parser.add_argument("--rootfs", type=Path, required=True)
    parser.add_argument("--rootfs-sha256", required=True)
    parser.add_argument("--source-manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=31)
    values = parser.parse_args()
    if not values.fsverity.is_file() or not values.fsverity.is_absolute():
        parser.error("--fsverity must be an absolute regular file")
    if (
        not values.source_manifest.is_file()
        or not values.source_manifest.is_absolute()
    ):
        parser.error("--source-manifest must be an absolute regular file")
    if values.samples < 5 or values.samples > 1000:
        parser.error("--samples must be between 5 and 1000")
    if not values.output.is_absolute():
        parser.error("--output must be absolute")
    return values


def main() -> None:
    arguments = parse_args()
    artifacts = [
        inspect_artifact(
            "kernel",
            arguments.kernel,
            arguments.kernel_sha256,
            arguments.fsverity,
            arguments.samples,
        ),
        inspect_artifact(
            "rootfs",
            arguments.rootfs,
            arguments.rootfs_sha256,
            arguments.fsverity,
            arguments.samples,
        ),
    ]
    for artifact in artifacts:
        if artifact["measure_p95_us"] > 100_000:
            raise RuntimeError("fs-verity constant-time measurement exceeded 100 ms P95")
        if not artifact["fsverity_differs_from_traditional_sha256"]:
            raise RuntimeError("fs-verity digest unexpectedly matched whole-file SHA-256")

    evidence = {
        "schema_version": 1,
        "contract_version": "ferrobox-fsverity-evidence-v1",
        "github": {
            "repository": os.environ.get("GITHUB_REPOSITORY", "unknown"),
            "commit": os.environ.get("GITHUB_SHA", "unknown"),
            "run_id": os.environ.get("GITHUB_RUN_ID", "unknown"),
        },
        "kernel_release": command_output(["uname", "-r"]),
        "tool_version": command_output([str(arguments.fsverity), "--version"]),
        "tool_source_manifest_sha256": file_sha256(arguments.source_manifest),
        "measurement_samples_per_artifact": arguments.samples,
        "artifacts": artifacts,
        "checks": [
            "official-signed-tool-source",
            "catalog-traditional-digest-match",
            "btrfs-fs-verity-enable",
            "offline-digest-equals-kernel-measurement",
            "constant-time-measurement-latency",
            "verity-files-reject-writes",
            "traditional-and-verity-digests-distinct",
            "rootfs-reflink-preserves-bytes",
            "rootfs-reflink-drops-verity-metadata",
        ],
    }
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(
        json.dumps(evidence, indent=2) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
