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
The [template catalog](docs/templates.md) records reusable kernel/rootfs inputs
under stable content-derived identities. The hosted OCI gate binds a
digest-pinned public image to reproducible rootfs bytes, selects it through the
existing HTTP `template` field, and boots the catalog's exact KVM kernel/rootfs
inputs after integrity verification. Its Btrfs source assets also pass a hosted
fs-verity gate covering signed tooling, kernel measurement, write rejection,
reflink semantics, and real Firecracker launch.

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
| Templates | Immutable build/list/inspect/delete catalog plus content-ID selection for direct Firecracker creates |
| Observability | Structured lifecycle and workload audit events |
| Agent surface | Rust CLI, first-party Agent Skill, and OpenAPI 3.1 contract |
| Testing | Process/API E2E and hosted nested-KVM E2E workflows |

Public port routing, domain policy, persistent volumes, multi-node scheduling,
and object storage are planned after the v0.1 runtime gate. The full boundary
is recorded in [Scope and acceptance](docs/scope.md).

## Measured performance

### Primary microVM comparison

GitHub Actions run
[`30432673085`](https://github.com/nya-a-cat/ferrobox/actions/runs/30432673085)
used one nested-KVM host, one vCPU, 512 MiB memory, the same Python 3.11
workload, and retained every sample:

| Startup or allocation boundary | P50 | P95 |
| --- | ---: | ---: |
| Ferrobox ready-pool HTTP allocation | 1.784 ms | 2.810 ms |
| Ferrobox snapshot-pool preparation | 961.798 ms | 1,038.732 ms |
| Direct Firecracker cold launch to guest ready | 1,684.991 ms | 2,501.760 ms |
| Cloud Hypervisor cold launch to guest ready | 2,518.585 ms | 2,540.687 ms |
| Kata QEMU complete cold `/bin/true` job | 1,481.160 ms | 1,579.662 ms |

| Warm execution boundary | `/bin/true` P50 / P95 | Python P50 / P95 |
| --- | ---: | ---: |
| Direct Firecracker guest protocol | 1.908 / 5.037 ms | 11.134 / 11.676 ms |
| Cloud Hypervisor guest protocol | 1.889 / 6.167 ms | 10.604 / 11.185 ms |
| Ferrobox snapshot pool, full runtime | 3.119 / 17.495 ms | 15.877 / 39.557 ms |
| Ferrobox fresh-boot pool, full runtime | 2.861 / 5.528 ms | 11.791 / 22.026 ms |
| Kata QEMU through containerd and shim-v2 | 31.118 / 33.237 ms | 56.028 / 61.262 ms |

The full Ferrobox snapshot runtime has 9.98 times lower `/bin/true` P50 and
3.53 times lower Python P50 than Kata QEMU in this run. The fresh-boot pool
reduces Ferrobox's execution tail further, with a preparation and memory cost:
five guests used 500,240 KiB RSS versus 338,644 KiB for the snapshot pool.

Direct Firecracker and Cloud Hypervisor are close at the shared guest-protocol
boundary. The remaining Ferrobox gap is concentrated in snapshot-backed memory
and runtime bookkeeping. The mandatory microVM gate stays red while the full
snapshot runtime trails the direct Cloud Hypervisor minimal-command median.

E2B remains a remote Firecracker product comparison. Its public startup figure
uses a different network and deployment boundary, so no same-host speedup ratio
is claimed. Exact samples and limitations are recorded in
[Performance evidence](docs/performance.md).

### Container and userspace-kernel controls

GitHub Actions run
[`30424993453`](https://github.com/nya-a-cat/ferrobox/actions/runs/30424993453)
measured the real HTTP control plane, Docker, and gVisor `runsc` on one hosted
nested-KVM runner:

| Five-sample P95 | Latency |
| --- | ---: |
| Ferrobox `POST /v1/sandboxes` from a ready pool | 4.558 ms |
| Ferrobox five-client concurrent create | 5.908 ms |
| Docker/runc container create + start | 139.797 ms |
| gVisor/runsc container create + start | 143.987 ms |

| 100-command result | P50 | P95 | Sequential throughput |
| --- | ---: | ---: | ---: |
| Ferrobox guest `/bin/true` RPC | 3.149 ms | 20.238 ms | 150.955 ops/s |
| Docker/runc `/bin/true` exec-to-exit | 35.478 ms | 37.945 ms | 28.043 ops/s |
| gVisor/runsc `/bin/true` exec-to-exit | 21.139 ms | 22.975 ms | 46.820 ops/s |

| 30-command Python 3.11 result | P50 | P95 | Sequential throughput |
| --- | ---: | ---: | ---: |
| Ferrobox `python3 -c "print(42)"` | 17.327 ms | 19.596 ms | 60.742 ops/s |
| Docker/runc `python3 -c "print(42)"` | 46.769 ms | 48.847 ms | 21.349 ops/s |
| gVisor/runsc `python3 -c "print(42)"` | 40.007 ms | 41.851 ms | 24.952 ops/s |

GitHub Actions run
[`30425743192`](https://github.com/nya-a-cat/ferrobox/actions/runs/30425743192)
measured a Python 3.11 write/read/verify/delete roundtrip for a 1 MiB file:

| 20-command file result | P50 | P95 | Sequential throughput |
| --- | ---: | ---: | ---: |
| Ferrobox | 20.049 ms | 30.875 ms | 44.558 ops/s |
| Docker/runc | 49.542 ms | 51.994 ms | 20.051 ops/s |
| gVisor/runsc | 45.934 ms | 48.097 ms | 21.724 ops/s |

The workflow uses direct HTTP, preloads template images, matches the available
resource settings, retains all raw samples, verifies the official gVisor
archive with SHA-512, and gates Ferrobox P95 below both Docker/runc and
gVisor/runsc for startup, minimal command execution, and Python execution.
Ferrobox sustained 2.85 times the Docker/runc Python throughput and 2.43 times
the gVisor/runsc Python throughput in this boundary. Percentiles use the
conservative nearest-rank method. Snapshot restore cost, ready-pool allocation,
burst wall time, and resident memory are reported separately in
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
- [Network isolation and DNS relay](docs/networking.md)
- [Snapshot, restore, clone, and rollback contract](docs/snapshots.md)
- [Agent Skill and CLI contract](docs/agent-skill.md)
- [OpenAPI and generated-client contract](docs/openapi.md)
- [Performance evidence](docs/performance.md)
- [SOTA parity program](docs/sota-roadmap.md)
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
