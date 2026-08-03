#!/usr/bin/env python3
"""Validate the seven generated-SDK runs and emit one sanitized matrix."""

from __future__ import annotations

import argparse
import hashlib
import json
import uuid
from pathlib import Path
from typing import Any


LANGUAGES = ("csharp", "go", "java", "kotlin", "python", "rust", "typescript")
CHECKS = (
    "generated-model-create",
    "bearer-auth-inspect",
    "typed-command-execution",
    "lossless-base64-output",
    "typed-file-roundtrip",
    "delete-and-stale-handle-rejection",
    "credential-redaction",
)
LOCK_FILES = {
    "csharp": (
        "csharp-library.packages.lock.json",
        "csharp-harness.packages.lock.json",
    ),
    "go": ("go.sum",),
    "java": (
        "java-pom.xml",
        "java-dependency-tree.txt",
        "java-surefire-provider.sha256",
    ),
    "kotlin": ("kotlin-gradle.lockfile", "kotlin-gradle-wrapper.properties"),
    "python": ("python-uv.lock",),
    "rust": ("rust-Cargo.lock",),
    "typescript": ("typescript-pnpm-lock.yaml",),
}
SENSITIVE_KEYS = {"access_token", "authorization", "bearer", "credential", "token"}


def fail(message: str) -> None:
    raise SystemExit(message)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def reject_sensitive_keys(value: Any, location: str) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key.lower() in SENSITIVE_KEYS:
                fail(f"sensitive key in retained evidence at {location}.{key}")
            reject_sensitive_keys(child, f"{location}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            reject_sensitive_keys(child, f"{location}[{index}]")


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"invalid JSON evidence {path}: {error}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence-dir", type=Path, required=True)
    parser.add_argument("--audit-log", type=Path, required=True)
    parser.add_argument("--locks-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    sandbox_ids: dict[str, str] = {}
    for language in LANGUAGES:
        path = arguments.evidence_dir / f"{language}.json"
        evidence = read_json(path)
        reject_sensitive_keys(evidence, language)
        if set(evidence) != {"checks", "language", "sandbox_id", "schema_version"}:
            fail(f"unexpected {language} evidence fields: {sorted(evidence)}")
        if evidence["schema_version"] != 1 or evidence["language"] != language:
            fail(f"unexpected {language} evidence identity")
        if tuple(evidence["checks"]) != CHECKS:
            fail(f"{language} check set drift")
        try:
            sandbox_id = uuid.UUID(evidence["sandbox_id"])
        except (AttributeError, TypeError, ValueError) as error:
            fail(f"invalid {language} sandbox ID: {error}")
        if sandbox_id.version != 7:
            fail(f"{language} sandbox identity is not UUIDv7")
        sandbox_ids[language] = str(sandbox_id)

    if len(set(sandbox_ids.values())) != len(LANGUAGES):
        fail("generated SDK runs did not use seven distinct sandboxes")

    audit_events = []
    try:
        lines = arguments.audit_log.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        fail(f"cannot read audit log: {error}")
    for line_number, line in enumerate(lines, start=1):
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError as error:
            fail(f"invalid audit event on line {line_number}: {error}")
        reject_sensitive_keys(event, f"audit[{line_number}]")
        audit_events.append(event)

    for language, sandbox_id in sandbox_ids.items():
        operations = {
            (event.get("operation"), event.get("outcome"))
            for event in audit_events
            if event.get("sandbox_id") == sandbox_id
        }
        if ("create", "succeeded") not in operations:
            fail(f"{language} lacks a successful audited create")
        if ("delete", "succeeded") not in operations:
            fail(f"{language} lacks a successful audited delete")

    manifests = {}
    for language, names in LOCK_FILES.items():
        records = []
        for name in names:
            path = arguments.locks_dir / name
            if not path.is_file() or path.stat().st_size == 0:
                fail(f"missing dependency manifest for {language}: {path}")
            records.append(
                {
                    "name": name,
                    "sha256": sha256(path),
                    "size_bytes": path.stat().st_size,
                }
            )
        manifests[language] = records

    matrix = {
        "schema_version": 1,
        "api_processes": 1,
        "client_count": len(LANGUAGES),
        "languages": list(LANGUAGES),
        "checks": list(CHECKS),
        "sandbox_ids": sandbox_ids,
        "audit": {
            "event_count": len(audit_events),
            "sha256": sha256(arguments.audit_log),
            "successful_creates": len(LANGUAGES),
            "successful_deletes": len(LANGUAGES),
        },
        "dependency_manifests": manifests,
    }
    arguments.output.write_text(
        json.dumps(matrix, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(matrix, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
