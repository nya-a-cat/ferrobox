#!/usr/bin/env python3
"""Extract a verified tar archive into a new directory with strict limits."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import tarfile


def fail(message: str) -> None:
    raise ValueError(message)


def normalized_name(member: tarfile.TarInfo) -> str:
    name = member.name
    if not name or "\x00" in name or "\\" in name:
        fail(f"unsafe archive member name: {name!r}")
    if len(name.encode("utf-8")) > 4096:
        fail("archive member name exceeds 4096 bytes")
    if any(ord(character) < 32 or ord(character) == 127 for character in name):
        fail(f"archive member name contains a control character: {name!r}")
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts:
        fail(f"archive member escapes the destination: {name!r}")
    normalized = str(path)
    if normalized in {"", "."}:
        return "."
    return normalized.removeprefix("./")


def inspect_members(
    archive: tarfile.TarFile,
    max_members: int,
    max_total_bytes: int,
    max_file_bytes: int,
) -> tuple[list[tarfile.TarInfo], dict[str, int]]:
    members = archive.getmembers()
    if len(members) > max_members:
        fail(f"archive has {len(members)} members; limit is {max_members}")

    names: set[str] = set()
    total_bytes = 0
    regular_files = 0
    directories = 0
    symbolic_links = 0
    hard_links = 0

    for member in members:
        name = normalized_name(member)
        if name in names:
            fail(f"archive repeats member path: {name!r}")
        names.add(name)

        if member.isdev() or member.type not in {
            tarfile.REGTYPE,
            tarfile.AREGTYPE,
            tarfile.DIRTYPE,
            tarfile.SYMTYPE,
            tarfile.LNKTYPE,
        }:
            fail(f"archive member has unsupported type: {name!r}")
        if member.size < 0 or member.size > max_file_bytes:
            fail(f"archive member size is outside the limit: {name!r}")
        if member.isfile():
            regular_files += 1
            total_bytes += member.size
            if total_bytes > max_total_bytes:
                fail(f"archive expands beyond {max_total_bytes} bytes")
        elif member.isdir():
            directories += 1
        elif member.issym():
            symbolic_links += 1
        elif member.islnk():
            hard_links += 1

        if member.issym() or member.islnk():
            link = member.linkname
            if not link or "\x00" in link or "\\" in link:
                fail(f"archive link has an unsafe target: {name!r}")
            if len(link.encode("utf-8")) > 4096:
                fail(f"archive link target exceeds 4096 bytes: {name!r}")

    return members, {
        "member_count": len(members),
        "regular_file_count": regular_files,
        "directory_count": directories,
        "symbolic_link_count": symbolic_links,
        "hard_link_count": hard_links,
        "logical_file_bytes": total_bytes,
    }


def extract(arguments: argparse.Namespace) -> dict[str, int | str]:
    archive_path = arguments.archive.resolve(strict=True)
    if not archive_path.is_file() or archive_path.is_symlink():
        fail("archive must be a regular, non-symlink file")

    destination = arguments.destination.absolute()
    if destination.exists() or destination.is_symlink():
        fail("destination must not already exist")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.partial-{os.getpid()}")
    if temporary.exists() or temporary.is_symlink():
        fail("temporary extraction directory already exists")
    temporary.mkdir(mode=0o700)

    try:
        if not hasattr(tarfile, "data_filter"):
            fail("Python tar extraction filters are unavailable")
        with tarfile.open(archive_path, mode="r:*") as archive:
            archive.errorlevel = 2
            members, metrics = inspect_members(
                archive,
                arguments.max_members,
                arguments.max_total_bytes,
                arguments.max_file_bytes,
            )

            def filter_member(
                member: tarfile.TarInfo, target: str
            ) -> tarfile.TarInfo:
                tarfile.data_filter(member, target)
                return member

            archive.extractall(
                temporary,
                members=members,
                numeric_owner=True,
                filter=filter_member,
            )
        temporary.replace(destination)
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise

    return {
        "schema_version": 1,
        "archive": str(archive_path),
        "destination": str(destination),
        **metrics,
    }


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("archive", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--max-members", type=int, default=200_000)
    parser.add_argument("--max-total-bytes", type=int, default=8 * 1024**3)
    parser.add_argument("--max-file-bytes", type=int, default=4 * 1024**3)
    parser.add_argument("--evidence", type=Path)
    arguments = parser.parse_args()
    if min(
        arguments.max_members,
        arguments.max_total_bytes,
        arguments.max_file_bytes,
    ) <= 0:
        parser.error("all extraction limits must be positive")
    return arguments


def main() -> None:
    arguments = parse_arguments()
    result = extract(arguments)
    payload = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if arguments.evidence:
        if arguments.evidence.exists() or arguments.evidence.is_symlink():
            fail("evidence path must not already exist")
        arguments.evidence.parent.mkdir(parents=True, exist_ok=True)
        arguments.evidence.write_text(payload, encoding="utf-8")
    print(payload, end="")


if __name__ == "__main__":
    main()
