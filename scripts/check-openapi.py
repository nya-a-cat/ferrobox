#!/usr/bin/env python3
"""Validate the checked-in Ferrobox OpenAPI contract and emitted SDK trees."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Any

from openapi_codegen_projection import project_document


EXPECTED_OPERATIONS = {
    ("get", "/healthz"): ("health", "none"),
    ("post", "/v1/sandboxes"): ("createSandbox", "none"),
    ("get", "/v1/sandboxes/{id}"): ("getSandbox", "sandbox"),
    ("delete", "/v1/sandboxes/{id}"): ("deleteSandbox", "sandbox"),
    ("post", "/v1/sandboxes/{id}/commands"): ("executeCommand", "sandbox"),
    ("put", "/v1/sandboxes/{id}/files"): ("writeFile", "sandbox"),
    ("get", "/v1/sandboxes/{id}/files"): ("readFile", "sandbox"),
    ("get", "/v1/sandboxes/{id}/directories"): ("listDirectory", "sandbox"),
    ("post", "/v1/sandboxes/{id}/pause"): ("pauseSandbox", "sandbox"),
    ("post", "/v1/sandboxes/{id}/resume"): ("resumeSandbox", "sandbox"),
    ("post", "/v1/sandboxes/{id}/snapshots"): ("createSnapshot", "sandbox"),
    ("get", "/v1/sandboxes/{id}/snapshots"): ("listSnapshots", "sandbox"),
    (
        "post",
        "/v1/sandboxes/{id}/rollback/{snapshot_id}",
    ): ("rollbackSnapshot", "sandbox"),
    ("get", "/v1/snapshots/{snapshot_id}"): ("getSnapshot", "snapshot"),
    ("delete", "/v1/snapshots/{snapshot_id}"): ("deleteSnapshot", "snapshot"),
    (
        "post",
        "/v1/snapshots/{snapshot_id}/verify",
    ): ("verifySnapshot", "snapshot"),
    (
        "post",
        "/v1/snapshots/{snapshot_id}/restore",
    ): ("restoreSnapshot", "snapshot"),
    (
        "post",
        "/v1/snapshots/{snapshot_id}/clones",
    ): ("cloneSnapshot", "snapshot"),
}

BODY_OPERATIONS = {
    "createSandbox",
    "executeCommand",
    "writeFile",
    "createSnapshot",
    "restoreSnapshot",
    "cloneSnapshot",
}

HTTP_METHODS = {"delete", "get", "patch", "post", "put"}
EXPECTED_CLIENTS = {
    "csharp",
    "go",
    "java",
    "kotlin",
    "python",
    "rust",
    "typescript-fetch",
}


def fail(message: str) -> None:
    raise SystemExit(message)


def resolve_pointer(document: dict[str, Any], reference: str) -> Any:
    if not reference.startswith("#/"):
        fail(f"external OpenAPI reference is forbidden: {reference}")
    current: Any = document
    for raw_part in reference[2:].split("/"):
        part = raw_part.replace("~1", "/").replace("~0", "~")
        if not isinstance(current, dict) or part not in current:
            fail(f"dangling OpenAPI reference: {reference}")
        current = current[part]
    return current


def check_references(document: dict[str, Any], value: Any) -> None:
    if isinstance(value, dict):
        reference = value.get("$ref")
        if reference is not None:
            if not isinstance(reference, str):
                fail("$ref must be a string")
            resolve_pointer(document, reference)
        for child in value.values():
            check_references(document, child)
    elif isinstance(value, list):
        for child in value:
            check_references(document, child)


def parameter_names(document: dict[str, Any], operation: dict[str, Any]) -> set[str]:
    names = set()
    for parameter in operation.get("parameters", []):
        if "$ref" in parameter:
            parameter = resolve_pointer(document, parameter["$ref"])
        if parameter.get("in") == "path":
            names.add(parameter.get("name"))
            if parameter.get("required") is not True:
                fail(f"path parameter is not required: {parameter.get('name')}")
    return names


def path_placeholders(path: str) -> set[str]:
    return {
        segment[1:-1]
        for segment in path.split("/")
        if segment.startswith("{") and segment.endswith("}")
    }


def check_operations(document: dict[str, Any]) -> list[str]:
    actual: dict[tuple[str, str], tuple[str, str]] = {}
    operation_ids = set()
    inherited_security = document.get("security")

    for path, path_item in document.get("paths", {}).items():
        if not isinstance(path_item, dict):
            fail(f"path item must be an object: {path}")
        for method, operation in path_item.items():
            if method not in HTTP_METHODS:
                continue
            operation_id = operation.get("operationId")
            scope = operation.get("x-ferrobox-credential-scope")
            if not isinstance(operation_id, str) or not operation_id:
                fail(f"missing operationId for {method.upper()} {path}")
            if operation_id in operation_ids:
                fail(f"duplicate operationId: {operation_id}")
            operation_ids.add(operation_id)
            actual[(method, path)] = (operation_id, scope)

            security = operation.get("security", inherited_security)
            expected_security = [] if scope == "none" else [{"bearerAuth": []}]
            if security != expected_security:
                fail(f"incorrect security for {operation_id}: {security!r}")
            if scope not in {"none", "sandbox", "snapshot"}:
                fail(f"invalid credential scope for {operation_id}: {scope!r}")
            if len(operation.get("tags", [])) != 1:
                fail(f"operation must have exactly one tag: {operation_id}")

            responses = operation.get("responses", {})
            if "default" not in responses:
                fail(f"operation lacks the structured default error: {operation_id}")
            if not any(str(status).startswith("2") for status in responses):
                fail(f"operation lacks a success response: {operation_id}")
            has_body = "requestBody" in operation
            if has_body != (operation_id in BODY_OPERATIONS):
                fail(f"request-body contract mismatch: {operation_id}")

            placeholders = path_placeholders(path)
            if parameter_names(document, operation) != placeholders:
                fail(f"path-parameter contract mismatch: {operation_id}")

    if actual != EXPECTED_OPERATIONS:
        missing = sorted(set(EXPECTED_OPERATIONS) - set(actual))
        extra = sorted(set(actual) - set(EXPECTED_OPERATIONS))
        changed = sorted(
            key
            for key in set(actual) & set(EXPECTED_OPERATIONS)
            if actual[key] != EXPECTED_OPERATIONS[key]
        )
        fail(f"operation drift: missing={missing}, extra={extra}, changed={changed}")
    return sorted(operation_ids)


def router_operations(source_path: Path) -> set[tuple[str, str]]:
    source = source_path.read_text(encoding="utf-8")
    start = source.find("pub fn router")
    end = source.find(".layer(", start)
    if start < 0 or end < 0:
        fail("unable to locate the Axum router definition")
    router = source[start:end]
    operations: set[tuple[str, str]] = set()
    cursor = 0
    while True:
        call = router.find(".route(", cursor)
        if call < 0:
            break
        index = call + len(".route(")
        depth = 1
        in_string = False
        escaped = False
        while index < len(router) and depth:
            character = router[index]
            if in_string:
                if escaped:
                    escaped = False
                elif character == "\\":
                    escaped = True
                elif character == '"':
                    in_string = False
            elif character == '"':
                in_string = True
            elif character == "(":
                depth += 1
            elif character == ")":
                depth -= 1
            index += 1
        if depth:
            fail("unterminated .route call in Axum router")
        arguments = router[call + len(".route(") : index - 1]
        match = re.match(r'\s*"([^"]+)"\s*,(.*)\Z', arguments, re.DOTALL)
        if match is None:
            fail(f"unsupported .route declaration: {arguments!r}")
        path, handlers = match.groups()
        methods = re.findall(r"(?:^|\.)\s*(delete|get|patch|post|put)\s*\(", handlers)
        if not methods:
            fail(f"route has no recognized method: {path}")
        operations.update((method, path) for method in methods)
        cursor = index
    return operations


def check_schemas(document: dict[str, Any]) -> int:
    schemas = document.get("components", {}).get("schemas", {})
    if not isinstance(schemas, dict) or not schemas:
        fail("components.schemas must be non-empty")
    for name, schema in schemas.items():
        if schema.get("type") == "object" and "additionalProperties" not in schema:
            fail(f"object schema lacks an explicit additionalProperties policy: {name}")

    for response_name in ("CreateSandboxResponse", "CreateSnapshotResponse"):
        token = schemas[response_name]["properties"]["token"]
        if token.get("readOnly") is not True or token.get("x-ferrobox-sensitive") is not True:
            fail(f"one-time credential is not marked sensitive: {response_name}.token")
    return len(schemas)


def hash_client_tree(root: Path) -> dict[str, dict[str, Any]]:
    if not root.is_dir():
        fail(f"generated client root does not exist: {root}")
    names = {path.name for path in root.iterdir() if path.is_dir()}
    if names != EXPECTED_CLIENTS:
        fail(
            "generated client set drift: "
            f"missing={sorted(EXPECTED_CLIENTS - names)}, "
            f"extra={sorted(names - EXPECTED_CLIENTS)}"
        )

    result = {}
    for name in sorted(names):
        client_root = root / name
        files = sorted(path for path in client_root.rglob("*") if path.is_file())
        if len(files) < 5:
            fail(f"generated client tree is unexpectedly small: {name}")
        digest = hashlib.sha256()
        for path in files:
            relative = path.relative_to(client_root).as_posix().encode()
            data = path.read_bytes()
            digest.update(len(relative).to_bytes(8, "big"))
            digest.update(relative)
            digest.update(len(data).to_bytes(8, "big"))
            digest.update(data)
        result[name] = {"file_count": len(files), "tree_sha256": digest.hexdigest()}
    return result


def check_codegen_projection(
    root: Path, authoritative: dict[str, Any]
) -> dict[str, Any]:
    if not root.is_dir():
        fail(f"generated client root does not exist: {root}")
    expected_files = {
        ".ferrobox-codegen-openapi.json",
        ".ferrobox-codegen-overlay.json",
    }
    actual_files = {path.name for path in root.iterdir() if path.is_file()}
    if actual_files != expected_files:
        fail(
            "generated-root metadata drift: "
            f"missing={sorted(expected_files - actual_files)}, "
            f"extra={sorted(actual_files - expected_files)}"
        )

    overlay_path = root / ".ferrobox-codegen-overlay.json"
    projection_path = root / ".ferrobox-codegen-openapi.json"
    overlay_raw = overlay_path.read_bytes()
    projection_raw = projection_path.read_bytes()
    try:
        overlay = json.loads(overlay_raw)
        projection = json.loads(projection_raw)
    except json.JSONDecodeError as error:
        fail(f"invalid generated-root projection metadata: {error}")
    if not isinstance(overlay, dict) or not isinstance(projection, dict):
        fail("generated-root projection metadata must contain JSON objects")

    expected_projection = project_document(authoritative, overlay)
    if projection != expected_projection:
        fail("retained code-generation projection does not match its merge patch")
    kinds = projection["components"]["schemas"]["ExecTermination"]["properties"][
        "kind"
    ]["enum"]
    return {
        "overlay_sha256": hashlib.sha256(overlay_raw).hexdigest(),
        "projection_sha256": hashlib.sha256(projection_raw).hexdigest(),
        "termination_kinds": kinds,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("spec", type=Path)
    parser.add_argument("--router-source", type=Path, required=True)
    parser.add_argument("--generated-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    raw_spec = arguments.spec.read_bytes()
    document = json.loads(raw_spec)
    if document.get("openapi") != "3.1.2":
        fail(f"unexpected OpenAPI version: {document.get('openapi')!r}")
    if document.get("info", {}).get("version") != "0.1.0":
        fail("unexpected Ferrobox API version")
    if document.get("jsonSchemaDialect") != "https://json-schema.org/draft/2020-12/schema":
        fail("unexpected JSON Schema dialect")

    check_references(document, document)
    operation_ids = check_operations(document)
    source_operations = router_operations(arguments.router_source)
    if source_operations != set(EXPECTED_OPERATIONS):
        fail(
            "Axum router drift: "
            f"missing={sorted(set(EXPECTED_OPERATIONS) - source_operations)}, "
            f"extra={sorted(source_operations - set(EXPECTED_OPERATIONS))}"
        )
    schema_count = check_schemas(document)
    codegen_projection = check_codegen_projection(arguments.generated_root, document)
    generated_clients = hash_client_tree(arguments.generated_root)
    scopes = {"none": 0, "sandbox": 0, "snapshot": 0}
    for _, scope in EXPECTED_OPERATIONS.values():
        scopes[scope] += 1

    evidence = {
        "schema_version": 1,
        "openapi_version": document["openapi"],
        "api_version": document["info"]["version"],
        "spec_sha256": hashlib.sha256(raw_spec).hexdigest(),
        "operation_count": len(operation_ids),
        "operation_ids": operation_ids,
        "schema_count": schema_count,
        "credential_scopes": scopes,
        "codegen_projection": codegen_projection,
        "generated_clients": generated_clients,
    }
    arguments.output.write_text(
        json.dumps(evidence, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(evidence, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
