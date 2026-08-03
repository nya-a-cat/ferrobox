#!/usr/bin/env python3
"""Inspect exact npm packages before CI installs the TypeScript toolchain."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Any


PACKAGES = {
    "pnpm@10.15.1": {
        "name": "pnpm",
        "version": "10.15.1",
        "license": "MIT",
        "source": "github.com/pnpm/pnpm",
        "bins": {"pnpm", "pnpx"},
    },
    "typescript@5.9.3": {
        "name": "typescript",
        "version": "5.9.3",
        "license": "Apache-2.0",
        "source": "github.com/microsoft/TypeScript",
        "bins": {"tsc", "tsserver"},
    },
    "@types/node@22.18.3": {
        "name": "@types/node",
        "version": "22.18.3",
        "license": "MIT",
        "source": "github.com/DefinitelyTyped/DefinitelyTyped",
        "bins": set(),
    },
}


def fail(message: str) -> None:
    raise SystemExit(message)


def package_metadata(package: str) -> dict[str, Any]:
    command = [
        "npm",
        "view",
        package,
        "name",
        "version",
        "license",
        "repository",
        "bin",
        "dependencies",
        "dist.integrity",
        "--json",
    ]
    result = subprocess.run(command, check=True, capture_output=True, text=True)
    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        fail(f"invalid npm metadata for {package}: {error}")
    if not isinstance(metadata, dict):
        fail(f"unexpected npm metadata shape for {package}")
    return metadata


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    records = {}
    for package, expected in PACKAGES.items():
        metadata = package_metadata(package)
        for field in ("name", "version", "license"):
            if metadata.get(field) != expected[field]:
                fail(
                    f"unexpected {field} for {package}: "
                    f"{metadata.get(field)!r}"
                )
        repository = json.dumps(metadata.get("repository", ""), sort_keys=True)
        if expected["source"].lower() not in repository.lower():
            fail(f"unexpected source repository for {package}: {repository}")
        bins = metadata.get("bin") or {}
        if set(bins) != expected["bins"]:
            fail(f"unexpected executable entrypoints for {package}: {sorted(bins)}")
        dist = metadata.get("dist")
        integrity = dist.get("integrity") if isinstance(dist, dict) else None
        if integrity is None:
            integrity = metadata.get("dist.integrity")
        if not isinstance(integrity, str) or not integrity.startswith("sha512-"):
            fail(f"missing registry integrity for {package}")
        records[package] = metadata

    evidence = {"schema_version": 1, "packages": records}
    arguments.output.write_text(
        json.dumps(evidence, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(evidence, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
