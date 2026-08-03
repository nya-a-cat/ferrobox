#!/usr/bin/env python3
"""Record GitHub runner toolchains and verify the generated Gradle wrapper."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path


GRADLE_WRAPPER_SHA256 = (
    "498495120a03b9a6ab5d155f5de3c8f0d986a449153702fb80fc80e134484f17"
)
GRADLE_DISTRIBUTION_SHA256 = (
    "ed1a8d686605fd7c23bdf62c7fc7add1c5b23b2bbc3721e661934ef4a4911d7c"
)
GRADLE_DISTRIBUTION_URL = (
    "https\\://services.gradle.org/distributions/gradle-8.14.3-all.zip"
)


def fail(message: str) -> None:
    raise SystemExit(message)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def version(command: list[str]) -> str:
    result = subprocess.run(command, check=True, capture_output=True, text=True)
    lines = [line.strip() for line in (result.stdout + result.stderr).splitlines()]
    return next((line for line in lines if line), "")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--kotlin-root", type=Path, required=True)
    parser.add_argument("--gradle-wrapper", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    wrapper_jar = arguments.kotlin_root / "gradle/wrapper/gradle-wrapper.jar"
    wrapper_properties = (
        arguments.kotlin_root / "gradle/wrapper/gradle-wrapper.properties"
    )
    if sha256(wrapper_jar) != GRADLE_WRAPPER_SHA256:
        fail("generated Gradle wrapper JAR is not the recognized Gradle 8.9 binary")
    properties = wrapper_properties.read_text(encoding="utf-8")
    if f"distributionUrl={GRADLE_DISTRIBUTION_URL}\n" not in properties:
        fail("generated Gradle distribution URL drift")

    evidence = {
        "schema_version": 1,
        "runner": {
            "arch": os.environ.get("RUNNER_ARCH"),
            "image_os": os.environ.get("ImageOS"),
            "image_version": os.environ.get("ImageVersion"),
        },
        "toolchains": {
            "cargo": version(["cargo", "--version"]),
            "corepack": version(["corepack", "--version"]),
            "dotnet": version(["dotnet", "--version"]),
            "go": version(["go", "version"]),
            "gradle": version([str(arguments.gradle_wrapper), "--version"]),
            "java": version(["java", "-version"]),
            "maven": version(["mvn", "--version"]),
            "node": version(["node", "--version"]),
            "pnpm": version(["corepack", "pnpm@10.15.1", "--version"]),
            "python": version(
                ["uv", "run", "--no-project", "--python", "3.12", "python", "--version"]
            ),
            "rustc": version(["rustc", "--version"]),
            "uv": version(["uv", "--version"]),
        },
        "verified_gradle": {
            "distribution_sha256": GRADLE_DISTRIBUTION_SHA256,
            "distribution_url": GRADLE_DISTRIBUTION_URL.replace("\\:", ":"),
            "wrapper_jar_release": "8.9",
            "wrapper_jar_sha256": GRADLE_WRAPPER_SHA256,
        },
    }
    arguments.output.write_text(
        json.dumps(evidence, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(evidence, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
