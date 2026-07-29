# Ferrobox

**A small, auditable Firecracker sandbox runtime for AI agents, written in Rust.**

[![CI](https://github.com/nya-a-cat/ferrobox/actions/workflows/ci.yml/badge.svg)](https://github.com/nya-a-cat/ferrobox/actions/workflows/ci.yml)
[![KVM E2E](https://github.com/nya-a-cat/ferrobox/actions/workflows/kvm.yml/badge.svg)](https://github.com/nya-a-cat/ferrobox/actions/workflows/kvm.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

Ferrobox runs untrusted agent workloads inside short-lived Firecracker
microVMs. Each sandbox receives an isolated root filesystem, resource limits,
a private control channel, a short-lived token, and an optional restricted
egress network.

The project provides an E2B-style HTTP interface for creating environments,
executing argv-based commands, transferring files, controlling lifecycle, and
collecting audit events. It targets private infrastructure and single-node
deployments where a compact, inspectable runtime is preferable to a large
orchestration stack.

> [!IMPORTANT]
> Ferrobox is under active development. Standard Rust and process-backend CI
> passes. The Firecracker KVM end-to-end workflow remains the release gate for
> v0.1; follow the KVM badge for its current result.

## Why Ferrobox?

AI agents need a place to execute generated code with a tighter boundary than
the agent process itself. Ferrobox concentrates on that runtime boundary:

- one Firecracker microVM per sandbox;
- Rust control-plane, node runtime, and guest service;
- direct argv execution with no implicit shell;
- confined file access below `/home/sandbox`;
- default-disabled networking and restricted public egress;
- ready-state snapshots, reflink rootfs clones, and an optional ready pool;
- Jailer, cgroup v2, seccomp, TTL cleanup, and structured audit events;
- a local process backend for deterministic API tests;
- no Kubernetes dependency for the single-node MVP.

Ferrobox is independent of model providers and agent frameworks. Any client
that can call HTTP can use it.

## Architecture

```text
Agent / SDK / CLI
        |
        | HTTP + per-sandbox bearer token
        v
  ferrobox-api
  state, TTL, audit
        |
        | SandboxRuntime
        v
  ferrobox-node
  Jailer, cgroup, network
        |
        +---- Firecracker API over Unix socket
        |
        v
  Firecracker microVM
  ferrobox-guest
  process and file RPC over virtio-vsock
```

The host control channel uses virtio-vsock. User network traffic, when enabled,
uses a TAP device inside a per-sandbox network namespace. The guest service
runs as root for environment control and launches submitted commands as
UID/GID 1000.

See [Architecture](docs/architecture.md) for lifecycle, trust boundaries, and
protocol details.

## Current capabilities

| Area | v0.1 capability |
| --- | --- |
| Lifecycle | Create, inspect, pause, resume, TTL expiry, delete |
| Commands | Argv, cwd, environment, timeout, signals, output limits |
| Output | Structured exit reason, UTF-8 display, lossless base64 |
| Files | Confined read, write, and directory listing |
| Isolation | Firecracker, Jailer, cgroup v2, default seccomp |
| Control plane | Per-sandbox tokens stored as SHA-256 digests |
| Network | Disabled by default; restricted public egress mode |
| Startup | Firecracker snapshot restore and configurable ready pool |
| Observability | Structured lifecycle and workload audit events |
| Testing | Process/API E2E and hosted nested-KVM E2E workflows |

Public port routing, domain policy, persistent volumes, multi-node scheduling,
and object storage are planned after the v0.1 runtime gate. The full boundary
is recorded in [Scope and acceptance](docs/scope.md).

## Measured startup performance

GitHub Actions run
[`30423815234`](https://github.com/nya-a-cat/ferrobox/actions/runs/30423815234)
measured the real HTTP control plane, Docker, and gVisor `runsc` on one hosted
nested-KVM runner:

| Five-sample P95 | Latency |
| --- | ---: |
| Ferrobox `POST /v1/sandboxes` from a ready pool | 4.597 ms |
| Ferrobox five-client concurrent create | 7.641 ms |
| gVisor container create + start through Docker Engine | 141.948 ms |

| Twenty-sample P95 | Latency |
| --- | ---: |
| Ferrobox guest `/bin/true` RPC | 21.408 ms |
| gVisor `/bin/true` exec-to-exit through Docker Engine | 25.621 ms |

The workflow uses direct HTTP, preloads template images, matches the available
resource settings, retains raw JSON, verifies the official gVisor archive with
SHA-512, and gates Ferrobox P95 below both Docker/runc and gVisor/runsc for
startup and command execution. The retained Docker/runc comparison is also
documented in the detailed evidence. Percentiles use the conservative
nearest-rank method. Snapshot restore cost, ready-pool allocation, burst wall
time, and resident memory are reported separately in
[Performance evidence](docs/performance.md).

## API example

Create a Python sandbox:

```bash
curl --fail --silent \
  --header 'content-type: application/json' \
  --data '{
    "template": "python",
    "cpu_count": 1,
    "memory_mb": 512,
    "timeout_seconds": 300,
    "network": {"internet_access": false}
  }' \
  http://127.0.0.1:8080/v1/sandboxes
```

The response contains a `sandbox_id` and a bearer `token`. Execute Python with
those values:

```bash
curl --fail --silent \
  --header "authorization: Bearer ${FERROBOX_TOKEN}" \
  --header 'content-type: application/json' \
  --data '{
    "argv": ["python3", "-c", "print(40 + 2)"],
    "cwd": "/home/sandbox",
    "environment": {},
    "timeout_seconds": 30,
    "max_output_bytes": 1048576
  }' \
  "http://127.0.0.1:8080/v1/sandboxes/${FERROBOX_SANDBOX_ID}/commands"
```

Commands are executed as argument arrays. Shell syntax is available only when
the caller explicitly invokes a shell:

```json
{"argv": ["/bin/bash", "-lc", "python3 main.py"]}
```

See the [HTTP API reference](docs/api.md) for file and lifecycle endpoints.

## Repository layout

```text
crates/
  ferrobox-api/       HTTP API, authorization, state, TTL, audit
  ferrobox-cli/       Command-line API client
  ferrobox-core/      Runtime contract and shared domain types
  ferrobox-guest/     Guest process and file service
  ferrobox-node/      Process and Firecracker runtime backends
  ferrobox-protocol/  Protobuf/gRPC guest protocol
scripts/
  build-python-rootfs.sh
  benchmark-kvm.sh
  e2e-process.sh
  e2e-kvm.sh
  fetch-firecracker.sh
docs/
  api.md
  architecture.md
  deployment.md
  scope.md
  security.md
  supply-chain.md
  testing.md
```

## Build and verification

Executable verification is performed by GitHub Actions to keep development
hosts lightweight and make the evidence reproducible.

The standard workflow checks formatting, unit and integration tests, Clippy
with warnings denied, locked workspace builds, a static musl guest build, the
process-backed HTTP flow, and pinned Firecracker tooling.

The KVM workflow runs on a hosted Linux runner with `/dev/kvm`. It builds a
Python rootfs, verifies the pinned kernel and Firecracker checksums, boots the
guest through Jailer, executes `python3 -c "print(42)"`, and checks cleanup.
It also retains phase-level create, hot-exec, Python-exec, delete, and total
lifecycle timings as a JSON benchmark artifact.

```text
Jailer -> Firecracker -> guest READY -> execute -> delete -> leak check
```

Workflow files:

- [Standard CI](.github/workflows/ci.yml)
- [Firecracker KVM E2E](.github/workflows/kvm.yml)
- [Verification policy](docs/testing.md)
- [Performance evidence](docs/performance.md)
- [Supply-chain pins](docs/supply-chain.md)

## Linux host requirements

The Firecracker backend requires an x86_64 Linux host with:

- read/write access to `/dev/kvm`;
- cgroup v2;
- `/dev/net/tun`, `ip`, and `nft`;
- the pinned Firecracker and Jailer release;
- a trusted guest kernel and rootfs template;
- a dedicated unprivileged UID/GID for Firecracker.

Runtime images, jail roots, sockets, and writable disks should live on a native
Linux filesystem. Detailed paths and ownership requirements are in the
[deployment guide](docs/deployment.md).

The process backend is available for API development:

```bash
cargo run -p ferrobox-api -- \
  --backend process \
  --unsafe-process-runtime \
  --listen 127.0.0.1:8080
```

It runs commands on the host and provides no workload isolation. The API
restricts this mode to a loopback listener.

## Security model

Ferrobox treats submitted commands, descendants, uploaded files, and network
traffic as untrusted. The host, node daemon, API daemon, guest kernel, rootfs
template, Firecracker, Jailer, and guest service are trusted components.

Important controls include:

- a dedicated microVM, rootfs, cgroup, vsock CID, and token per sandbox;
- `openat2` path confinement with beneath, no-magic-link, and no-cross-device
  resolution;
- an unprivileged guest workload account;
- byte limits for command output and file transfer;
- VM-wide termination on TTL expiry;
- private, link-local, metadata, host, and sandbox network rejection;
- audit records that omit bearer-token material.

Read [Security model](docs/security.md) before exposing Ferrobox to untrusted
users. Remote API deployments require TLS termination and operator
authentication in front of the service.

## Project status

The v0.1 milestone is reached when both workflows are green and the retained
KVM evidence demonstrates the full acceptance path. Until then, Ferrobox is
suitable for development and review, with production isolation claims pending
the KVM release gate.

## License

Ferrobox is licensed under the
[Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0).
