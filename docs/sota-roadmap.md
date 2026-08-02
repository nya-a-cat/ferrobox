# SOTA parity program

Ferrobox targets the verified capability union of Microsandbox, CubeSandbox,
and OpenSandbox. A capability is complete only when its public contract,
security boundary, GitHub-hosted conformance test, and operator documentation
are all present.

This program preserves the compact single-node Firecracker runtime as the
core. Additional runtimes and developer surfaces remain separable components
behind explicit contracts.

## Fixed comparison baseline

The initial audit was performed on 2026-08-02 against these upstream states:

- [Microsandbox `001af7b`](https://github.com/superradcompany/microsandbox/tree/001af7b51e4e2e208985c0f0e317390c80587eb7), with v0.6.8 as the latest release;
- [CubeSandbox `6b01f08`](https://github.com/TencentCloud/CubeSandbox/tree/6b01f08e0a233570fcdd42baed2718ba08318759), with v0.6.0 as the latest release;
- [OpenSandbox `e95681e`](https://github.com/opensandbox-group/OpenSandbox/tree/e95681e791b33b3893033940cbeaa5ab192bf21b).

Primary upstream evidence:

- [Microsandbox README](https://github.com/superradcompany/microsandbox/blob/001af7b51e4e2e208985c0f0e317390c80587eb7/README.md), [snapshots](https://github.com/superradcompany/microsandbox/blob/001af7b51e4e2e208985c0f0e317390c80587eb7/docs/sandboxes/snapshots.mdx), and [secret isolation](https://github.com/superradcompany/microsandbox/blob/001af7b51e4e2e208985c0f0e317390c80587eb7/docs/security/secrets.mdx);
- [CubeSandbox architecture](https://github.com/TencentCloud/CubeSandbox/blob/6b01f08e0a233570fcdd42baed2718ba08318759/docs/architecture/overview.md), [reproducible benchmark report](https://github.com/TencentCloud/CubeSandbox/blob/6b01f08e0a233570fcdd42baed2718ba08318759/docs/blog/posts/2026-06-01-cubesandbox-perf-benchmark.md), [snapshot/clone/rollback](https://github.com/TencentCloud/CubeSandbox/blob/6b01f08e0a233570fcdd42baed2718ba08318759/docs/guide/snapshot-rollback-clone.md), and [security proxy](https://github.com/TencentCloud/CubeSandbox/blob/6b01f08e0a233570fcdd42baed2718ba08318759/docs/guide/security-proxy.md);
- [OpenSandbox architecture](https://github.com/opensandbox-group/OpenSandbox/blob/e95681e791b33b3893033940cbeaa5ab192bf21b/docs/architecture/index.md), [public specifications](https://github.com/opensandbox-group/OpenSandbox/tree/e95681e791b33b3893033940cbeaa5ab192bf21b/specs), and [roadmap](https://github.com/opensandbox-group/OpenSandbox/blob/e95681e791b33b3893033940cbeaa5ab192bf21b/ROADMAP.md).

Upstream movement is expected. Each parity release must refresh the pinned
commits and record any newly documented capability before making a current
comparison claim.

## Talk-backed design constraints

The implementation order also follows public systems talks with demonstrated
mechanisms:

- [AWS re:Invent 2019: Firecracker open-source innovation](https://www.youtube.com/watch?v=yDplzXEdBTI)
  demonstrates millisecond snapshot resume and burst-starting live microVMs.
  Ferrobox therefore treats post-initialization snapshots and restore-time
  identity injection as core runtime primitives.
- [KVM Forum 2019: Firecracker lessons and virtio-vsock](https://kvm-forum.qemu.org/2019/)
  covers the production isolation boundary and host/guest transport. Snapshot
  restore must re-establish control channels and uniqueness before a sandbox is
  exposed.
- [OSDI 2024: Sabre](https://www.usenix.org/conference/osdi24/presentation/lazarev)
  reports that working-set prefetch and hardware-assisted snapshot compression
  can reduce Firecracker restore cost. Ferrobox will measure page-fault and
  working-set behavior before selecting a prefetch or compression mechanism.
- [KVM Forum 2022: Booting Linux to userspace in 100 ms and beyond](https://kvm-forum.qemu.org/2022/)
  keeps kernel and userspace boot work visible as a separate cold-start track;
  ready-pool allocation cannot substitute for that measurement.

New mechanisms with only paper or product evidence remain candidates. They
enter the implementation plan after their contracts and reproducible evidence
have been audited.

## Published-number boundary

Upstream figures define investigation targets until the same-host workflow
reproduces them:

- Microsandbox's sub-100 ms README figure is guest boot on an Apple M1. It is
  a different host and boundary from Ferrobox's Linux HTTP create-to-ready.
- CubeSandbox's reproducible report measures 2-vCPU/2-GiB sandboxes on a
  96-logical-core bare-metal host. It reports serial create P95 of 57.4 ms and
  host-memory-delta density converging near 25 MB per live sandbox. Those
  measurements have stronger methodology than the shorter README headline.
- OpenSandbox currently supplies the broadest protocol and workload surface;
  its architecture documentation does not establish a same-host microVM
  latency lead.

Ferrobox will make no speedup or density ratio across unmatched host,
isolation, resource, image, warm-state, or timing boundaries.

## Capability acceptance matrix

Status meanings:

- **Verified**: implemented and exercised on a GitHub-hosted runner.
- **Partial**: some behavior exists; the union contract or evidence is incomplete.
- **Missing**: no supported implementation exists.

| Capability family | Upstream union to match | Ferrobox state | Acceptance gate |
| --- | --- | --- | --- |
| Hardware isolation | Dedicated microVM boundary | Verified | Firecracker/Jailer KVM E2E and cross-sandbox escape suite pass |
| Runtime abstraction | Embedded/local runtime plus server and Docker/Kubernetes providers | Partial | Each provider passes one shared lifecycle conformance suite |
| OCI images | Pull and run pinned public/private OCI images | Missing | Digest-pinned image boots and executes on GitHub without a bespoke rootfs build |
| Host architecture | Linux x86_64, Linux aarch64, macOS Apple Silicon, Windows/WSL2 where supported | Partial | Platform matrix reports the exact isolation backend and passes the shared smoke contract |
| Templates | Build, list, version, inspect, and delete reusable templates | Partial | Immutable template identity and provenance survive create/delete cycles |
| Ready pools | Bounded pre-provisioning with safe replenishment | Verified | Allocation, concurrent claim, replenishment, TTL, and leak gates pass |
| Lifecycle | Create, inspect, set TTL, pause, resume, hibernate, delete | Partial | State-transition contract passes for every runtime provider |
| User snapshots | Named, inspectable, portable snapshots | Verified | Create/list/get/delete and integrity verification pass |
| Live checkpoint | Capture memory, process, filesystem, and device state | Verified | Running Python state resumes with exact memory and file contents |
| Clone/fork | Create independent children from one checkpoint | Verified | Concurrent children preserve the checkpoint and isolate later mutations |
| Rollback | Restore an existing sandbox identity to a checkpoint | Verified | Memory, processes, files, token rotation, and stale-handle rejection pass |
| Incremental snapshots | Dirty-page and changed-block capture with bounded chain depth | Missing | 0/10/100/512 MiB dirty-state matrix retains size and latency evidence |
| Command execution | Argv and explicit-shell execution, timeout, signal, exit reasons | Verified | Shared positive/error/timeout/signal suite passes |
| Streaming execution | Incremental stdout/stderr and background process inspection | Partial | Slow producer is observable before exit; reconnect resumes from a cursor |
| Interactive sessions | PTY and persistent shell sessions over a streaming transport | Missing | Resize, stdin, disconnect/reconnect, and cleanup tests pass |
| Code interpreter | Stateful Jupyter-compatible contexts and streamed results | Missing | Variable state, rich output, interrupt, and restart tests pass |
| File API | Confined read/write/list plus streaming large transfers | Partial | Path-escape suite and 1 MiB/64 MiB streamed transfer gates pass |
| Volumes | Read-only/read-write host, managed persistent, and object-backed volumes | Missing | Mount policy, persistence, quota, isolation, and cleanup tests pass |
| Disabled networking | No guest data interface by default | Verified | Guest route/interface audit and blocked egress tests pass |
| Egress policy | FQDN, wildcard, IP, CIDR, DNS, and runtime policy updates | Partial | DNS rebinding, metadata, private-range, redirect, and policy-race suite passes |
| Secret protection | Host-side secret substitution/proxy with plaintext absent from guest | Missing | Files, env, process memory, logs, packet capture, and error paths contain no secret |
| Service exposure | Authenticated HTTP/WebSocket/TCP endpoints and port discovery | Missing | Route isolation, token rotation, WebSocket, expiry, and SSRF tests pass |
| E2B compatibility | E2B lifecycle, command, file, and code-interpreter behavior | Missing | Pinned E2B SDK conformance suite passes without application changes beyond endpoint/auth |
| OpenAPI contracts | Versioned lifecycle, execution, diagnostics, ingress, and egress specs | Partial | Generated clients and server contract tests share one checked-in specification |
| SDKs | Rust, Python, TypeScript, Go, Java/Kotlin, and C# surfaces | Partial | Language matrix passes the same remote GitHub sandbox scenario |
| CLI, MCP, skills | Human CLI, MCP server, and agent-facing skill package | Partial | Each surface completes create/exec/file/delete against the same API |
| Browser/desktop | Chromium/Playwright, VNC desktop, and VS Code Web templates | Missing | Browser and desktop smoke tests use authenticated exposed endpoints |
| GPU | Explicit GPU allocation and isolation policy | Missing | Provider contract reports device identity, quota, cleanup, and unsupported modes |
| Single-node operation | Auditable standalone deployment | Verified | Fresh GitHub runner installs pinned inputs and passes full KVM E2E |
| Multi-node placement | Capacity-aware scheduling, failure recovery, and node fencing | Missing | Two-node GitHub topology proves placement, drain, loss, and reconciliation behavior |
| Kubernetes | CRDs/controller, pools, RuntimeClass, and workload-provider integration | Missing | Kind-based API conformance and cleanup suite passes on GitHub |
| Audit and diagnostics | Lifecycle/workload audit, reasoned state, metrics, logs, traces | Partial | Request IDs correlate API, runtime, guest, network, and audit events |
| Tenant security | API keys, per-sandbox credentials, endpoint credentials, and organization boundaries | Partial | Cross-tenant authorization matrix and credential-redaction suite pass |
| Supply chain | Pinned binaries/images, checksums, manifests, SBOM, and update policy | Partial | Provenance artifact covers every executable and image used by release E2E |

The four verified state-branching rows are backed by
[Live Snapshot KVM E2E run 30758919945](https://github.com/nya-a-cat/ferrobox/actions/runs/30758919945)
at commit `87d5456`. Its `live-snapshot-evidence` artifact records schema 1 and
all sixteen contract checks, including process-memory continuation, independent
restore, clone isolation, same-ID rollback, fault cleanup, integrity failure,
and final resource cleanup. Standard CI for the same commit passed in
[run 30758919931](https://github.com/nya-a-cat/ferrobox/actions/runs/30758919931).

## SOTA evidence rules

Feature breadth and system performance use separate gates. A release-level
SOTA claim requires all applicable capability rows to be **Verified** and the
target performance row to pass on three independent GitHub-hosted runs.

Every runtime comparison must use:

- the same hosted runner, resource limit, guest image, workload, warm-up, and
  sample count;
- pinned source commits, release artifacts, images, and checksums;
- full raw samples with nearest-rank P50/P95/P99 and wall-clock throughput;
- explicit boundaries for cold boot, restore, ready-pool preparation,
  allocation, execution, and cleanup;
- retained failure artifacts and no early exit before all comparators finish.

The required performance matrix is:

| Dimension | Required evidence |
| --- | --- |
| Ready allocation | Serial and concurrent HTTP create-to-ready latency |
| Cold boot | Request to authenticated, executable guest |
| Snapshot restore | Restore to authenticated, executable guest |
| Checkpoint/clone/rollback | P50/P95 across dirty-state sizes and concurrency tiers |
| Hot execution | `/bin/true`, Python startup, output streaming, and sustained throughput |
| Files | 1 MiB and 64 MiB API transfer plus in-guest round-trip |
| Density | Host memory delta plus PSS/USS at 1/5/10/25 live sandboxes |
| Isolation cost | The same workload under Firecracker, another microVM VMM, gVisor, and runc |
| Network policy | DNS and HTTP latency with policy disabled, allowlisted, and secret-proxy modes |
| Recovery | Process, node, and controller failure detection and leak-free reconciliation time |

`VmRSS` cannot support a cross-project density claim because it double-counts
shared pages. Ferrobox will retain it for process diagnostics and add host
memory delta, PSS, and USS before comparing with CubeSandbox's published
per-instance memory results.

## Dependency-ordered delivery

1. **Evidence completeness**: finish every comparator, retain all artifacts,
   and report every failed gate in one GitHub run.
2. **State branching**: public snapshots, live checkpoint, clone/fork,
   rollback, integrity, and incremental storage.
3. **Security services**: domain/IP egress policy, DNS-rebinding resistance,
   secret proxy, and authenticated ingress.
4. **Developer contract**: OCI images, OpenAPI, E2B compatibility, SDKs, CLI,
   MCP, and skills.
5. **Interactive workloads**: streamed/background execution, PTY, persistent
   sessions, Jupyter, browser, desktop, and service endpoints.
6. **Persistent and distributed operation**: volumes, multi-node placement,
   Kubernetes provider, failure recovery, aarch64, and GPU provider contract.
7. **SOTA closure**: optimize the measured Pareto frontier for latency,
   throughput, density, checkpoint cost, and policy overhead.

Public protocol, database, network-policy, and concurrency changes require a
reviewed contract and migration note before implementation. Each delivery
stage remains a collection of small, independently reversible components.
