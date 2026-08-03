#!/usr/bin/env python3
"""Build and verify the deterministic OpenAPI projection used for SDK codegen."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
from typing import Any


VARIANTS = (
    "exited",
    "signaled",
    "timed_out",
    "output_limit_exceeded",
)
VARIANT_SCHEMAS = {
    "exited": "ExecTerminationExited",
    "signaled": "ExecTerminationSignaled",
    "timed_out": "ExecTerminationTimedOut",
    "output_limit_exceeded": "ExecTerminationOutputLimitExceeded",
}
STRICT_TERMINATION = {
    "oneOf": [
        {"$ref": f"#/components/schemas/{VARIANT_SCHEMAS[kind]}"}
        for kind in VARIANTS
    ],
    "discriminator": {
        "propertyName": "kind",
        "mapping": {
            kind: f"#/components/schemas/{VARIANT_SCHEMAS[kind]}"
            for kind in VARIANTS
        },
    },
}
PROJECTED_TERMINATION = {
    "type": "object",
    "additionalProperties": False,
    "required": ["kind"],
    "properties": {
        "kind": {"type": "string", "enum": list(VARIANTS)},
        "exit_code": {
            "type": "integer",
            "format": "int32",
            "description": "Present only when kind is exited.",
        },
        "signal": {
            "type": "integer",
            "format": "int32",
            "description": "Present only when kind is signaled.",
        },
    },
    "description": "Code-generation projection of the strict command-termination union.",
}
EXPECTED_OVERLAY = {
    "components": {
        "schemas": {
            "ExecTermination": {
                "oneOf": None,
                "discriminator": None,
                **PROJECTED_TERMINATION,
            }
        }
    }
}


def fail(message: str) -> None:
    raise SystemExit(message)


def merge_patch(target: Any, patch: Any) -> Any:
    """Apply RFC 7396 JSON Merge Patch without mutating either input."""
    if not isinstance(patch, dict):
        return copy.deepcopy(patch)
    result = copy.deepcopy(target) if isinstance(target, dict) else {}
    for key, value in patch.items():
        if value is None:
            result.pop(key, None)
        else:
            result[key] = merge_patch(result.get(key), value)
    return result


def validate_strict_contract(document: dict[str, Any]) -> None:
    schemas = document.get("components", {}).get("schemas")
    if not isinstance(schemas, dict):
        fail("authoritative OpenAPI components.schemas is missing")
    if schemas.get("ExecTermination") != STRICT_TERMINATION:
        fail("authoritative ExecTermination union drifted from the strict contract")

    expected_variants = {
        "exited": {
            "required": ["kind", "exit_code"],
            "properties": {
                "kind": {"type": "string", "enum": ["exited"]},
                "exit_code": {"type": "integer", "format": "int32"},
            },
        },
        "signaled": {
            "required": ["kind", "signal"],
            "properties": {
                "kind": {"type": "string", "enum": ["signaled"]},
                "signal": {"type": "integer", "format": "int32"},
            },
        },
        "timed_out": {
            "required": ["kind"],
            "properties": {
                "kind": {"type": "string", "enum": ["timed_out"]},
            },
        },
        "output_limit_exceeded": {
            "required": ["kind"],
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["output_limit_exceeded"],
                },
            },
        },
    }
    for kind, expected in expected_variants.items():
        name = VARIANT_SCHEMAS[kind]
        actual = schemas.get(name)
        required_shape = {
            "type": "object",
            "additionalProperties": False,
            **expected,
        }
        if actual != required_shape:
            fail(f"authoritative {name} schema drifted from the strict contract")


def project_document(
    document: dict[str, Any], overlay: dict[str, Any]
) -> dict[str, Any]:
    validate_strict_contract(document)
    if overlay != EXPECTED_OVERLAY:
        fail("code-generation overlay drifted from the reviewed merge patch")
    projected = merge_patch(document, overlay)
    termination = projected.get("components", {}).get("schemas", {}).get(
        "ExecTermination"
    )
    if termination != PROJECTED_TERMINATION:
        fail("projected ExecTermination schema is unexpected")
    return projected


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_object(path: Path, label: str) -> tuple[bytes, dict[str, Any]]:
    try:
        raw = path.read_bytes()
        value = json.loads(raw)
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read {label} {path}: {error}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object: {path}")
    return raw, value


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("overlay", type=Path)
    parser.add_argument("output", type=Path)
    arguments = parser.parse_args()

    if arguments.output.exists():
        fail(f"projection output already exists: {arguments.output}")
    source_raw, document = read_object(arguments.source, "authoritative OpenAPI")
    overlay_raw, overlay = read_object(arguments.overlay, "code-generation overlay")
    projected = project_document(document, overlay)
    projection_raw = (
        json.dumps(projected, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")
    arguments.output.write_bytes(projection_raw)

    evidence = {
        "source_sha256": sha256(source_raw),
        "overlay_sha256": sha256(overlay_raw),
        "projection_sha256": sha256(projection_raw),
        "termination_kinds": list(VARIANTS),
    }
    print(json.dumps(evidence, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
