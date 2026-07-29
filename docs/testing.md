# Verification

All executable verification runs in GitHub Actions to keep development-host
resource usage low and results reproducible.

## Standard CI

`.github/workflows/ci.yml` performs:

- lockfile generation;
- `cargo fmt --check`;
- workspace tests;
- Clippy with warnings denied;
- locked workspace build;
- static `x86_64-unknown-linux-musl` guest build;
- process-backend HTTP end-to-end verification;
- Firecracker/Jailer checksum and version verification.

The process E2E checks `print(42)`, argv literal handling, file round-trip,
path traversal rejection, deletion, and token redaction from audit records.

## KVM CI

`.github/workflows/kvm.yml` requires `/dev/kvm`, builds the static guest and a
Python Debian rootfs, fetches the pinned Firecracker/Jailer release, downloads the pinned
Firecracker CI guest kernel and verifies its SHA-256, and runs:

```text
Jailer -> Firecracker -> guest READY -> python3 prints 42 -> delete -> leak check
```

Each run uploads the kernel key/hash and rootfs build manifest as provenance
evidence. The workflow rejects any kernel whose SHA-256 differs from the pinned
supply-chain record.

The same hosted-KVM job emits `ferrobox-benchmark.json` with:

- five sorted ready-pool preparation samples plus P50 and P95;
- ready-pool size and summed Firecracker resident memory;
- five sorted create-to-ready samples plus P50 and P95;
- all `/bin/true` execution samples plus P50 and P95;
- one Python execution sample;
- five sorted deletion samples plus P50 and P95;
- total benchmark time.

The benchmark runs after the functional KVM path and applies conservative
regression ceilings. Competitor targets are tracked separately so an
unverified marketing number cannot silently become a project success claim.
When `FERROBOX_SNAPSHOT_ROOT` is set, the functional path prepares a snapshot
and the later benchmark measures restore-to-ready preparation and ready-pool
allocation independently.

The workflow also starts the real HTTP API with five ready microVMs and retains
`ferrobox-http-benchmark.json`. Its create samples include HTTP parsing, runtime
allocation, control-plane state registration, token issuance, audit writing,
JSON serialization, and loopback response transfer.

The same job pulls a cached Python image, records its resolved Docker digest,
and measures Docker Engine `create` plus `start` through its Unix-socket HTTP
API. `docker-benchmark.json` retains five samples. The comparison gate requires
Ferrobox HTTP create P95 to remain below Docker create-and-start P95.

## Completion rule

Workflow presence is not evidence of success. Ferrobox remains incomplete until
the GitHub check runs are green, KVM output is retained, and the final
requirement audit maps each claim to a workflow result or artifact.
