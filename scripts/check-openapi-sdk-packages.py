#!/usr/bin/env python3
"""Validate versioned SDK packages and bind them to lifecycle smoke evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import tarfile
import zipfile
from email.parser import BytesParser
from pathlib import Path, PurePosixPath
from typing import Any
from xml.etree import ElementTree


LANGUAGES = ("csharp", "go", "java", "kotlin", "python", "rust", "typescript")
PACKAGE_FIELDS = {
    "generator",
    "import_name",
    "language",
    "package_id",
    "primary_artifact",
    "registry_format",
}


def fail(message: str) -> None:
    raise SystemExit(message)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"invalid JSON {path}: {error}")


def safe_relative_path(value: str) -> Path:
    pure = PurePosixPath(value)
    if pure.is_absolute() or ".." in pure.parts or not pure.parts:
        fail(f"unsafe package artifact path: {value}")
    return Path(*pure.parts)


def zip_names(path: Path) -> set[str]:
    try:
        with zipfile.ZipFile(path) as archive:
            names = set(archive.namelist())
    except (OSError, zipfile.BadZipFile) as error:
        fail(f"invalid ZIP package {path}: {error}")
    for name in names:
        pure = PurePosixPath(name)
        if pure.is_absolute() or ".." in pure.parts:
            fail(f"unsafe ZIP member in {path}: {name}")
    return names


def validate_csharp(path: Path, package_id: str, version: str) -> dict[str, str]:
    with zipfile.ZipFile(path) as archive:
        nuspecs = [name for name in archive.namelist() if name.endswith(".nuspec")]
        if len(nuspecs) != 1:
            fail(f"NuGet package must contain one nuspec: {path}")
        root = ElementTree.fromstring(archive.read(nuspecs[0]))
        identity = root.find(".//{*}metadata/{*}id")
        package_version = root.find(".//{*}metadata/{*}version")
        if identity is None or identity.text != package_id:
            fail(f"NuGet package ID drift: {path}")
        if package_version is None or package_version.text != version:
            fail(f"NuGet package version drift: {path}")
        expected_dll = f"lib/net10.0/{package_id}.dll"
        if expected_dll not in archive.namelist():
            fail(f"NuGet package lacks {expected_dll}")
    return {"package_id": package_id, "version": version}


def validate_go(path: Path, package_id: str, version: str) -> dict[str, str]:
    version_with_v = f"v{version}"
    version_root = path.parent
    mod_path = version_root / f"{version_with_v}.mod"
    info_path = version_root / f"{version_with_v}.info"
    list_path = version_root / "list"
    if mod_path.read_text(encoding="utf-8").splitlines()[0] != f"module {package_id}":
        fail("Go proxy module identity drift")
    info = read_json(info_path)
    if info != {"Time": "1970-01-01T00:00:00Z", "Version": version_with_v}:
        fail("Go proxy version info drift")
    if list_path.read_text(encoding="utf-8") != version_with_v + "\n":
        fail("Go proxy version list drift")
    names = zip_names(path)
    prefix = f"{package_id}@{version_with_v}/"
    if not names or any(not name.startswith(prefix) for name in names):
        fail("Go module ZIP prefix drift")
    if prefix + "go.mod" not in names or not any(name.endswith(".go") for name in names):
        fail("Go module ZIP lacks module metadata or source")
    return {"package_id": package_id, "version": version_with_v}


def validate_maven(
    path: Path,
    package_id: str,
    version: str,
    expected_class: str,
) -> dict[str, str]:
    names = zip_names(path)
    if expected_class not in names:
        fail(f"Maven package lacks {expected_class}: {path}")
    return {"package_id": package_id, "version": version}


def validate_python(path: Path, package_id: str, version: str) -> dict[str, str]:
    names = zip_names(path)
    metadata_names = [name for name in names if name.endswith(".dist-info/METADATA")]
    if len(metadata_names) != 1 or "ferrobox_client/__init__.py" not in names:
        fail(f"Python wheel structure drift: {path}")
    with zipfile.ZipFile(path) as archive:
        metadata = BytesParser().parsebytes(archive.read(metadata_names[0]))
    normalized_name = re.sub(r"[-_.]+", "-", metadata["Name"].lower())
    if normalized_name != package_id or metadata["Version"] != version:
        fail(f"Python wheel identity drift: {path}")
    return {"package_id": normalized_name, "version": metadata["Version"]}


def validate_rust(path: Path, package_id: str, version: str) -> dict[str, str]:
    try:
        with tarfile.open(path, mode="r:gz") as archive:
            names = set(archive.getnames())
            prefix = f"{package_id}-{version}"
            cargo_member = archive.extractfile(f"{prefix}/Cargo.toml")
            if cargo_member is None or f"{prefix}/src/lib.rs" not in names:
                fail(f"Cargo package structure drift: {path}")
            cargo_toml = cargo_member.read().decode("utf-8")
    except (OSError, tarfile.TarError, UnicodeDecodeError) as error:
        fail(f"invalid Cargo package {path}: {error}")
    if not re.search(rf'^name = "{re.escape(package_id)}"$', cargo_toml, re.MULTILINE):
        fail(f"Cargo package name drift: {path}")
    if not re.search(rf'^version = "{re.escape(version)}"$', cargo_toml, re.MULTILINE):
        fail(f"Cargo package version drift: {path}")
    return {"package_id": package_id, "version": version}


def validate_typescript(path: Path, package_id: str, version: str) -> dict[str, str]:
    try:
        with tarfile.open(path, mode="r:gz") as archive:
            package_json = archive.extractfile("package/package.json")
            names = set(archive.getnames())
            if package_json is None:
                fail(f"npm package lacks package.json: {path}")
            metadata = json.loads(package_json.read().decode("utf-8"))
    except (OSError, tarfile.TarError, UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid npm package {path}: {error}")
    if metadata.get("name") != package_id or metadata.get("version") != version:
        fail(f"npm package identity drift: {path}")
    if "package/dist/index.js" not in names or "package/dist/index.d.ts" not in names:
        fail(f"npm package lacks compiled entrypoints: {path}")
    return {"package_id": package_id, "version": version}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--contract", type=Path, required=True)
    parser.add_argument("--packages-dir", type=Path, required=True)
    parser.add_argument("--evidence-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    contract = read_json(arguments.contract)
    if set(contract) != {"packages", "schema_version", "version"}:
        fail("unexpected SDK package contract fields")
    if contract["schema_version"] != 1 or contract["version"] != "0.1.0":
        fail("unexpected SDK package contract version")
    packages = contract["packages"]
    if not isinstance(packages, list) or len(packages) != len(LANGUAGES):
        fail("SDK package contract must contain seven packages")
    if tuple(package.get("language") for package in packages) != LANGUAGES:
        fail("SDK package contract language order drift")

    records = []
    for package in packages:
        if not isinstance(package, dict) or set(package) != PACKAGE_FIELDS:
            fail("unexpected SDK package entry fields")
        language = package["language"]
        path = arguments.packages_dir / safe_relative_path(package["primary_artifact"])
        if not path.is_file() or path.stat().st_size == 0:
            fail(f"missing {language} package artifact: {path}")
        if language == "csharp":
            parsed_identity = validate_csharp(
                path, package["package_id"], contract["version"]
            )
        elif language == "go":
            parsed_identity = validate_go(
                path, package["package_id"], contract["version"]
            )
        elif language == "java":
            parsed_identity = validate_maven(
                path,
                package["package_id"],
                contract["version"],
                "io/github/nyaacat/ferrobox/client/ApiClient.class",
            )
        elif language == "kotlin":
            parsed_identity = validate_maven(
                path,
                package["package_id"],
                contract["version"],
                "io/github/nyaacat/ferrobox/kotlin/infrastructure/ApiClient.class",
            )
        elif language == "python":
            parsed_identity = validate_python(
                path, package["package_id"], contract["version"]
            )
        elif language == "rust":
            parsed_identity = validate_rust(
                path, package["package_id"], contract["version"]
            )
        else:
            parsed_identity = validate_typescript(
                path, package["package_id"], contract["version"]
            )

        consumer_evidence = arguments.evidence_dir / f"{language}.json"
        evidence = read_json(consumer_evidence)
        if evidence.get("language") != language or evidence.get("schema_version") != 1:
            fail(f"{language} consumer evidence identity drift")
        records.append(
            {
                **package,
                "artifact": {
                    "sha256": sha256(path),
                    "size_bytes": path.stat().st_size,
                },
                "consumer_evidence_sha256": sha256(consumer_evidence),
                "parsed_identity": parsed_identity,
            }
        )

    result = {
        "schema_version": 1,
        "version": contract["version"],
        "package_count": len(records),
        "consumer_smoke_count": len(records),
        "contract": {
            "name": arguments.contract.name,
            "sha256": sha256(arguments.contract),
        },
        "packages": records,
    }
    arguments.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
