#!/usr/bin/env python3
"""Exercise the generated Python SDK against the loopback process backend."""

from __future__ import annotations

import base64
import json
import os
from pathlib import Path

import ferrobox_client
from ferrobox_client.api.commands_api import CommandsApi
from ferrobox_client.api.files_api import FilesApi
from ferrobox_client.api.sandboxes_api import SandboxesApi
from ferrobox_client.exceptions import ApiException
from ferrobox_client.models.create_sandbox_request import CreateSandboxRequest
from ferrobox_client.models.execute_command_request import ExecuteCommandRequest
from ferrobox_client.models.network_request import NetworkRequest
from ferrobox_client.models.write_file_request import WriteFileRequest


def client(api_url: str, token: str | None = None) -> ferrobox_client.ApiClient:
    return ferrobox_client.ApiClient(
        ferrobox_client.Configuration(host=api_url, access_token=token)
    )


def enum_value(value: object) -> object:
    return getattr(value, "value", value)


def main() -> None:
    api_url = os.environ["FERROBOX_API_URL"]
    audit_path = Path(os.environ["FERROBOX_AUDIT_LOG"])
    evidence_path = Path(os.environ["FERROBOX_OPENAPI_SDK_EVIDENCE"])
    sandbox_id = None
    token = None

    with client(api_url) as unauthenticated:
        created = SandboxesApi(unauthenticated).create_sandbox(
            CreateSandboxRequest(
                template="python",
                cpu_count=1,
                memory_mb=512,
                timeout_seconds=120,
                network=NetworkRequest(internet_access=False),
            )
        )
    sandbox_id = created.sandbox_id
    token = created.token
    assert isinstance(token, str) and token
    assert enum_value(created.state) == "running"

    try:
        with client(api_url, token) as authenticated:
            sandboxes = SandboxesApi(authenticated)
            commands = CommandsApi(authenticated)
            files = FilesApi(authenticated)

            inspected = sandboxes.get_sandbox(sandbox_id)
            assert inspected.sandbox_id == sandbox_id
            assert enum_value(inspected.state) == "running"

            executed = commands.execute_command(
                sandbox_id,
                ExecuteCommandRequest(
                    argv=["python3", "-c", "print(40 + 2)"],
                    cwd="/home/sandbox",
                    environment={},
                    timeout_seconds=30,
                    max_output_bytes=1048576,
                ),
            )
            assert executed.stdout == "42\n"
            assert base64.b64decode(executed.stdout_base64) == b"42\n"

            payload = b"generated-openapi-client\n"
            written = files.write_file(
                sandbox_id,
                WriteFileRequest(
                    path="/home/sandbox/openapi.txt",
                    content_base64=base64.b64encode(payload).decode("ascii"),
                    overwrite=False,
                ),
            )
            assert written.bytes_written == len(payload)
            read = files.read_file(
                sandbox_id,
                path="/home/sandbox/openapi.txt",
                offset=0,
                max_bytes=1048576,
            )
            assert base64.b64decode(read.content_base64) == payload
            assert read.eof is True

            sandboxes.delete_sandbox(sandbox_id)
            try:
                sandboxes.get_sandbox(sandbox_id)
            except ApiException as error:
                assert error.status == 404
            else:
                raise AssertionError("deleted sandbox remained addressable")

        audit = audit_path.read_text(encoding="utf-8")
        assert token not in audit
        assert '"operation":"delete"' in audit
        evidence = {
            "schema_version": 1,
            "language": "python",
            "sandbox_id": str(sandbox_id),
            "checks": [
                "generated-model-create",
                "bearer-auth-inspect",
                "typed-command-execution",
                "lossless-base64-output",
                "typed-file-roundtrip",
                "delete-and-stale-handle-rejection",
                "credential-redaction",
            ],
        }
        evidence_path.write_text(
            json.dumps(evidence, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(json.dumps(evidence, indent=2, sort_keys=True))
        sandbox_id = None
        token = None
    finally:
        if sandbox_id is not None and token is not None:
            try:
                with client(api_url, token) as authenticated:
                    SandboxesApi(authenticated).delete_sandbox(sandbox_id)
            except ApiException:
                pass


if __name__ == "__main__":
    main()
