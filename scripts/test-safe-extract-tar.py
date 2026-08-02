#!/usr/bin/env python3
"""Adversarial checks for safe-extract-tar.py."""

from __future__ import annotations

import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile


def add_bytes(archive: tarfile.TarFile, name: str, value: bytes) -> None:
    member = tarfile.TarInfo(name)
    member.size = len(value)
    member.mode = 0o644
    archive.addfile(member, io.BytesIO(value))


def run(extractor: Path, archive: Path, destination: Path, *extra: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(extractor), str(archive), str(destination), *extra],
        check=False,
        capture_output=True,
        text=True,
    )


def require_rejected(result: subprocess.CompletedProcess[str], name: str) -> None:
    if result.returncode == 0:
        raise AssertionError(f"unsafe fixture was accepted: {name}")


def main() -> None:
    extractor = Path(__file__).with_name("safe-extract-tar.py").resolve()
    checks: list[str] = []
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)

        safe_archive = root / "safe.tar"
        with tarfile.open(safe_archive, "w") as archive:
            add_bytes(archive, "workspace/value.txt", b"ferrobox\n")
        safe_destination = root / "safe"
        safe = run(extractor, safe_archive, safe_destination)
        if safe.returncode != 0:
            raise AssertionError(safe.stderr)
        if (safe_destination / "workspace/value.txt").read_bytes() != b"ferrobox\n":
            raise AssertionError("safe fixture contents changed")
        checks.append("safe-file")

        traversal_archive = root / "traversal.tar"
        with tarfile.open(traversal_archive, "w") as archive:
            add_bytes(archive, "../escape", b"escape")
        require_rejected(
            run(extractor, traversal_archive, root / "traversal"),
            "path-traversal",
        )
        if (root / "escape").exists():
            raise AssertionError("path traversal wrote outside the destination")
        checks.append("path-traversal-rejected")

        absolute_archive = root / "absolute.tar"
        with tarfile.open(absolute_archive, "w") as archive:
            add_bytes(archive, "/absolute", b"escape")
        require_rejected(
            run(extractor, absolute_archive, root / "absolute"),
            "absolute-path",
        )
        checks.append("absolute-path-rejected")

        link_archive = root / "link.tar"
        with tarfile.open(link_archive, "w") as archive:
            link = tarfile.TarInfo("outside")
            link.type = tarfile.SYMTYPE
            link.linkname = "../../outside"
            archive.addfile(link)
        require_rejected(run(extractor, link_archive, root / "link"), "outside-link")
        checks.append("outside-link-rejected")

        device_archive = root / "device.tar"
        with tarfile.open(device_archive, "w") as archive:
            device = tarfile.TarInfo("device")
            device.type = tarfile.CHRTYPE
            device.devmajor = 1
            device.devminor = 3
            archive.addfile(device)
        require_rejected(run(extractor, device_archive, root / "device"), "device")
        checks.append("special-file-rejected")

        duplicate_archive = root / "duplicate.tar"
        with tarfile.open(duplicate_archive, "w") as archive:
            add_bytes(archive, "same", b"first")
            add_bytes(archive, "same", b"second")
        require_rejected(
            run(extractor, duplicate_archive, root / "duplicate"),
            "duplicate-path",
        )
        checks.append("duplicate-path-rejected")

        limit_archive = root / "limit.tar"
        with tarfile.open(limit_archive, "w") as archive:
            add_bytes(archive, "one", b"1")
            add_bytes(archive, "two", b"2")
        require_rejected(
            run(
                extractor,
                limit_archive,
                root / "limit",
                "--max-members",
                "1",
            ),
            "member-limit",
        )
        checks.append("member-limit-enforced")

    print(json.dumps({"schema_version": 1, "checks": checks}, indent=2))


if __name__ == "__main__":
    main()
