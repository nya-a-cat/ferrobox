#!/usr/bin/env python3
"""Build a deterministic local Go module proxy entry from generated sources."""

from __future__ import annotations

import argparse
import json
import stat
import zipfile
from pathlib import Path, PurePosixPath


EXCLUDED_ROOTS = {
    ".gitignore",
    ".openapi-generator",
    ".openapi-generator-ignore",
    ".travis.yml",
    "api",
    "git_push.sh",
}


def fail(message: str) -> None:
    raise SystemExit(message)


def included_files(source: Path) -> list[Path]:
    files = []
    for path in source.rglob("*"):
        relative = path.relative_to(source)
        if relative.parts[0] in EXCLUDED_ROOTS:
            continue
        if path.is_symlink():
            fail(f"Go module source contains a symlink: {relative}")
        if path.is_file():
            files.append(path)
    return sorted(files, key=lambda path: path.relative_to(source).as_posix())


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--module", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    if not arguments.version.startswith("v"):
        fail("Go module version must begin with v")
    go_mod = arguments.source / "go.mod"
    if not go_mod.is_file():
        fail(f"missing generated go.mod: {go_mod}")
    go_mod_bytes = go_mod.read_bytes()
    expected_module_line = f"module {arguments.module}\n".encode()
    if not go_mod_bytes.startswith(expected_module_line):
        fail("generated Go module identity does not match the package contract")

    version_root = arguments.output / arguments.module / "@v"
    if version_root.exists():
        fail(f"Go proxy version root already exists: {version_root}")
    version_root.mkdir(parents=True)
    archive = version_root / f"{arguments.version}.zip"
    prefix = PurePosixPath(f"{arguments.module}@{arguments.version}")
    with zipfile.ZipFile(
        archive,
        mode="x",
        compression=zipfile.ZIP_DEFLATED,
        compresslevel=9,
    ) as bundle:
        for source_file in included_files(arguments.source):
            relative = PurePosixPath(source_file.relative_to(arguments.source).as_posix())
            member = zipfile.ZipInfo(str(prefix / relative), (1980, 1, 1, 0, 0, 0))
            member.compress_type = zipfile.ZIP_DEFLATED
            member.create_system = 3
            member.external_attr = (stat.S_IFREG | 0o644) << 16
            bundle.writestr(member, source_file.read_bytes())

    (version_root / f"{arguments.version}.mod").write_bytes(go_mod_bytes)
    (version_root / f"{arguments.version}.info").write_text(
        json.dumps(
            {"Version": arguments.version, "Time": "1970-01-01T00:00:00Z"},
            separators=(",", ":"),
        )
        + "\n",
        encoding="utf-8",
    )
    (version_root / "list").write_text(arguments.version + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "module": arguments.module,
                "version": arguments.version,
                "archive": str(archive),
                "file_count": len(included_files(arguments.source)),
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
