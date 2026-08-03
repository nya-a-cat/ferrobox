#!/usr/bin/env python3
"""Record and validate one GitHub-hosted Ferrobox architecture leg."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import struct
import subprocess
from pathlib import Path
from typing import Any


LINUX_CHECKS = (
    "rust-toolchain",
    "native-build-tools",
    "action-pin-policy",
    "dependency-lock",
    "workspace-tests",
    "workspace-build",
    "static-guest-build",
    "process-api-lifecycle",
    "cli-lifecycle",
)
MACOS_CHECKS = (
    "rust-toolchain",
    "native-tools",
    "action-pin-policy",
    "dependency-lock",
    "host-tests",
    "host-build",
    "process-api-lifecycle",
    "cli-lifecycle",
)
CHECKS_BY_PLATFORM = {
    "linux-aarch64": LINUX_CHECKS,
    "linux-x86_64": LINUX_CHECKS,
    "macos-aarch64": MACOS_CHECKS,
}
ELF_MACHINES = {"aarch64": 183, "x86_64": 62}


def command(command: list[str]) -> dict[str, Any]:
    try:
        result = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            env={**os.environ, "LC_ALL": "C"},
        )
    except OSError as error:
        return {"ok": False, "error": type(error).__name__, "output": None}
    output = "\n".join(
        line.strip()
        for line in (result.stdout + result.stderr).splitlines()
        if line.strip()
    )
    return {
        "ok": result.returncode == 0,
        "error": None if result.returncode == 0 else f"exit-{result.returncode}",
        "output": output or None,
    }


def parse_checks(platform_id: str, values: list[str]) -> dict[str, str]:
    expected = CHECKS_BY_PLATFORM.get(platform_id)
    if expected is None:
        raise SystemExit(f"unknown platform identity: {platform_id}")
    checks: dict[str, str] = {}
    for value in values:
        name, separator, outcome = value.partition("=")
        if not separator or not name or not outcome:
            raise SystemExit(f"invalid check outcome: {value}")
        if name in checks:
            raise SystemExit(f"duplicate check outcome: {name}")
        checks[name] = outcome
    if tuple(checks) != expected:
        raise SystemExit(f"check order drift: {tuple(checks)}")
    return checks


def parse_rust_host(result: dict[str, Any]) -> str | None:
    if not result["ok"] or not result["output"]:
        return None
    match = re.search(r"^host: (\S+)$", result["output"], re.MULTILINE)
    return match.group(1) if match else None


def parse_lscpu(result: dict[str, Any]) -> dict[str, str | None]:
    wanted = {
        "Architecture": "architecture",
        "CPU(s)": "logical_cpus",
        "Vendor ID": "vendor_id",
        "Model name": "model_name",
        "Virtualization": "virtualization",
        "Hypervisor vendor": "hypervisor_vendor",
        "Byte Order": "byte_order",
    }
    parsed = {name: None for name in wanted.values()}
    if not result["ok"] or not result["output"]:
        return parsed
    try:
        document = json.loads(result["output"])
    except json.JSONDecodeError:
        return parsed
    for item in document.get("lscpu", []):
        field = str(item.get("field", "")).rstrip(":")
        if field in wanted:
            value = item.get("data")
            parsed[wanted[field]] = str(value) if value is not None else None
    return parsed


def command_value(arguments: list[str]) -> str | None:
    result = command(arguments)
    return result["output"] if result["ok"] else None


def parse_darwin_cpu() -> dict[str, str | None]:
    hypervisor_support = command_value(["sysctl", "-n", "kern.hv_support"])
    return {
        "architecture": command_value(["sysctl", "-n", "hw.machine"]),
        "logical_cpus": command_value(["sysctl", "-n", "hw.logicalcpu"]),
        "vendor_id": "Apple",
        "model_name": command_value(
            ["sysctl", "-n", "machdep.cpu.brand_string"]
        ),
        "virtualization": (
            "Hypervisor.framework" if hypervisor_support == "1" else None
        ),
        "hypervisor_vendor": "Apple" if hypervisor_support == "1" else None,
        "byte_order": "Little Endian",
    }


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def inspect_elf(path: Path, stable_path: str) -> dict[str, Any] | None:
    if not path.is_file():
        return None
    with path.open("rb") as stream:
        header = stream.read(64)
        if len(header) != 64 or header[:4] != b"\x7fELF":
            return {
                "path": stable_path,
                "size_bytes": path.stat().st_size,
                "sha256": sha256(path),
                "valid_elf64_little_endian": False,
            }
        if header[4] != 2 or header[5] != 1:
            return {
                "path": stable_path,
                "size_bytes": path.stat().st_size,
                "sha256": sha256(path),
                "valid_elf64_little_endian": False,
            }
        elf_machine = struct.unpack_from("<H", header, 18)[0]
        program_offset = struct.unpack_from("<Q", header, 32)[0]
        program_entry_size = struct.unpack_from("<H", header, 54)[0]
        program_entry_count = struct.unpack_from("<H", header, 56)[0]
        has_interpreter = False
        for index in range(program_entry_count):
            stream.seek(program_offset + index * program_entry_size)
            program_type = stream.read(4)
            if len(program_type) != 4:
                break
            if struct.unpack("<I", program_type)[0] == 3:
                has_interpreter = True
                break
    return {
        "path": stable_path,
        "size_bytes": path.stat().st_size,
        "sha256": sha256(path),
        "elf_machine": elf_machine,
        "executable": os.access(path, os.X_OK),
        "has_interpreter": has_interpreter,
        "valid_elf64_little_endian": True,
    }


def inspect_kvm() -> dict[str, Any]:
    path = Path("/dev/kvm")
    exists = path.exists()
    character_device = False
    if exists:
        try:
            character_device = stat.S_ISCHR(path.stat().st_mode)
        except OSError:
            pass
    openable = False
    open_error: dict[str, Any] | None = None
    if character_device:
        try:
            descriptor = os.open(path, os.O_RDWR | getattr(os, "O_CLOEXEC", 0))
        except OSError as error:
            open_error = {"errno": error.errno, "type": type(error).__name__}
        else:
            os.close(descriptor)
            openable = True
    return {
        "path": str(path),
        "exists": exists,
        "character_device": character_device,
        "readable": os.access(path, os.R_OK),
        "writable": os.access(path, os.W_OK),
        "openable_read_write": openable,
        "open_error": open_error,
        "firecracker_exercised": False,
    }


def required_environment(name: str, errors: list[str]) -> str | None:
    value = os.environ.get(name)
    if not value:
        errors.append(f"missing environment {name}")
    return value


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--platform-id", required=True)
    parser.add_argument("--runner-label", required=True)
    parser.add_argument("--expected-runner-arch", required=True)
    parser.add_argument(
        "--expected-runner-os", choices=("Linux", "macOS"), required=True
    )
    parser.add_argument("--expected-kernel", choices=("Darwin", "Linux"), required=True)
    parser.add_argument(
        "--expected-machine",
        choices=("aarch64", "arm64", "x86_64"),
        required=True,
    )
    parser.add_argument("--expected-rust-host", required=True)
    parser.add_argument("--static-guest-target")
    parser.add_argument("--static-guest", type=Path)
    parser.add_argument("--check", action="append", default=[])
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    errors: list[str] = []
    if bool(arguments.static_guest_target) != bool(arguments.static_guest):
        parser.error("static guest target and path must be provided together")

    checks = parse_checks(arguments.platform_id, arguments.check)
    for name, outcome in checks.items():
        if outcome != "success":
            errors.append(f"{name} outcome is {outcome}")

    runner_arch = required_environment("RUNNER_ARCH", errors)
    runner_os = required_environment("RUNNER_OS", errors)
    runner_environment = required_environment("RUNNER_ENVIRONMENT", errors)
    if runner_arch != arguments.expected_runner_arch:
        errors.append(
            f"runner arch {runner_arch!r} != {arguments.expected_runner_arch!r}"
        )
    if runner_os != arguments.expected_runner_os:
        errors.append(
            f"runner OS {runner_os!r} != {arguments.expected_runner_os!r}"
        )
    if runner_environment != "github-hosted":
        errors.append(f"runner environment {runner_environment!r} != 'github-hosted'")

    uname_system = command(["uname", "-s"])
    uname_machine = command(["uname", "-m"])
    uname_release = command(["uname", "-r"])
    machine = uname_machine["output"] if uname_machine["ok"] else None
    if not uname_system["ok"] or uname_system["output"] != arguments.expected_kernel:
        errors.append(f"uname did not report {arguments.expected_kernel}")
    if machine != arguments.expected_machine:
        errors.append(f"machine {machine!r} != {arguments.expected_machine!r}")

    rustc = command(["rustc", "-vV"])
    cargo = command(["cargo", "--version"])
    rust_host = parse_rust_host(rustc)
    if rust_host != arguments.expected_rust_host:
        errors.append(
            f"rust host {rust_host!r} != {arguments.expected_rust_host!r}"
        )

    guest = None
    if arguments.static_guest_target and arguments.static_guest:
        stable_guest_path = (
            f"target/{arguments.static_guest_target}/release/ferrobox-guest"
        )
        guest = inspect_elf(arguments.static_guest, stable_guest_path)
        expected_elf_machine = ELF_MACHINES.get(arguments.expected_machine)
        if guest is None:
            errors.append("static guest artifact is missing")
        else:
            if not guest.get("valid_elf64_little_endian"):
                errors.append("static guest is not a little-endian ELF64 binary")
            if guest.get("elf_machine") != expected_elf_machine:
                errors.append("static guest ELF machine does not match the runner")
            if guest.get("has_interpreter") is not False:
                errors.append("static guest has a dynamic interpreter")
            if guest.get("executable") is not True:
                errors.append("static guest is not executable")
    elif arguments.expected_kernel == "Linux":
        errors.append("Linux capability evidence requires a static guest")

    if arguments.expected_kernel == "Linux":
        cpu = parse_lscpu(command(["lscpu", "--json"]))
    else:
        cpu = parse_darwin_cpu()
    if cpu["architecture"] != arguments.expected_machine:
        errors.append("CPU architecture does not match the runner")

    source = {
        "repository": required_environment("GITHUB_REPOSITORY", errors),
        "commit": required_environment("GITHUB_SHA", errors),
        "ref": required_environment("GITHUB_REF", errors),
        "run_id": required_environment("GITHUB_RUN_ID", errors),
        "run_attempt": required_environment("GITHUB_RUN_ATTEMPT", errors),
        "workflow": required_environment("GITHUB_WORKFLOW", errors),
        "job": required_environment("GITHUB_JOB", errors),
    }
    if source["commit"] and not re.fullmatch(r"[0-9a-f]{40}", source["commit"]):
        errors.append("GitHub commit is not a full lowercase SHA")

    evidence = {
        "schema_version": 1,
        "contract": "ferrobox-host-capability-v1",
        "platform_id": arguments.platform_id,
        "source": source,
        "runner": {
            "label": arguments.runner_label,
            "os": runner_os,
            "arch": runner_arch,
            "environment": runner_environment,
            "image_os": os.environ.get("ImageOS"),
            "image_version": os.environ.get("ImageVersion"),
        },
        "system": {
            "kernel": uname_system["output"] if uname_system["ok"] else None,
            "kernel_release": uname_release["output"] if uname_release["ok"] else None,
            "machine": machine,
        },
        "cpu": cpu,
        "toolchain": {
            "rust_host": rust_host,
            "rustc": rustc["output"] if rustc["ok"] else None,
            "cargo": cargo["output"] if cargo["ok"] else None,
            "static_guest_target": arguments.static_guest_target,
        },
        "guest_artifact": guest,
        "kvm": inspect_kvm(),
        "verification": {
            "backend": "process",
            "isolation": "none",
            "unsafe_process_opt_in": True,
            "checks": checks,
            "complete": not errors,
            "errors": errors,
        },
    }
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(
        json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(json.dumps(evidence, indent=2, sort_keys=True))
    if errors:
        raise SystemExit("; ".join(errors))


if __name__ == "__main__":
    main()
