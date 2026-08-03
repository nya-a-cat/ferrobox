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
| OCI images | Pull and run pinned public/private OCI images | Partial | Public digest-pinned image boots and executes on GitHub; private-registry custody and API image selection pass their contracts |
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
| SDKs | Rust, Python, TypeScript, Go, Java/Kotlin, and C# surfaces | Verified | Language matrix passes one remote GitHub scenario and versioned packages pass install smoke tests |
| CLI, MCP, skills | Human CLI, MCP server, and agent-facing skill package | Partial | Each surface completes create/exec/file/delete against the same API |
| Browser/desktop | Chromium/Playwright, VNC desktop, and VS Code Web templates | Missing | Browser and desktop smoke tests use authenticated exposed endpoints |
| GPU | Explicit GPU allocation and isolation policy | Missing | Provider contract reports device identity, quota, cleanup, and unsupported modes |
| Single-node operation | Auditable standalone deployment | Verified | Fresh GitHub runner installs pinned inputs and passes full KVM E2E |
| Multi-node placement | Capacity-aware scheduling, failure recovery, and node fencing | Missing | Two-node GitHub topology proves placement, drain, loss, and reconciliation behavior |
| Kubernetes | CRDs/controller, pools, RuntimeClass, and workload-provider integration | Missing | Kind-based API conformance and cleanup suite passes on GitHub |
| Audit and diagnostics | Lifecycle/workload audit, reasoned state, metrics, logs, traces | Partial | Request IDs correlate API, runtime, guest, network, and audit events |
| Tenant security | API keys, per-sandbox credentials, endpoint credentials, and organization boundaries | Partial | Cross-tenant authorization matrix and credential-redaction suite pass |
| Supply-chain inventory | Action SHA pins, pinned binaries/images, checksums, manifests, SBOM, and update policy | Verified | In-toto provenance covers every executable, guest asset, image, and SBOM used by release E2E |
| Release integrity | Digest-bound releases, keyless signatures, provenance attestations, and consumer verification | Partial | GitHub verifies signed source/binary artifacts and OCI digests against fixed workflow and repository identities |

The CLI and first-party `ferrobox-sandbox` Agent Skill now have a shared
[GitHub-hosted create/exec/file/delete conformance path](https://github.com/nya-a-cat/ferrobox/actions/runs/30763552790)
at commit `8854525`. The row remains Partial until the MCP surface passes the
same scenario with an approved credential custody contract.

The CLI command group also covers the complete snapshot API surface: create,
paginated list, inspect, verify, restore, clone, rollback, and delete. The Live
Snapshot KVM workflow enforces its separate conformance path.

The implemented eighteen-operation HTTP surface now has one checked-in OpenAPI
3.1 contract. Standard CI validates it with a digest-pinned official generator,
emits seven language source trees, compares it with the Axum route set, and
drives seven generated package consumers through the existing API. Runs
[30766020434](https://github.com/nya-a-cat/ferrobox/actions/runs/30766020434)
and
[30766116507](https://github.com/nya-a-cat/ferrobox/actions/runs/30766116507)
at commit `9cb5c54` independently produced equal file counts and tree hashes for
all seven clients; each run also passed an internal byte-for-byte regeneration
gate. Standard CI now contains a shared-process runtime gate for all seven
generated clients, with per-language dependency locks and sanitized aggregate
evidence. Its first attempt,
[run 30774266980](https://github.com/nya-a-cat/ferrobox/actions/runs/30774266980),
passed Go, Python, TypeScript, and generated-source immutability while isolating
C#, Java, Kotlin, and Rust codegen/cache blockers.
[Run 30774849266](https://github.com/nya-a-cat/ferrobox/actions/runs/30774849266)
removed the Java and Rust source-generation failures, repeated the three passing
clients, and preserved generated-source immutability. It narrowed the remaining
set to C#/Rust tagged-union runtime deserialization, Java's dynamic offline
Surefire provider, and Kotlin external-runtime prefetch. The audited projection
and dependency corrections passed in
[run 30777303301](https://github.com/nya-a-cat/ferrobox/actions/runs/30777303301)
at commit `86bb3cc`. All seven generated clients completed the same seven-check
scenario through one API process, produced distinct UUIDv7 sandboxes, retained
complete dependency evidence, and preserved byte-identical source trees. The
generated-source runtime matrix is Verified. Run
[30779954220](https://github.com/nya-a-cat/ferrobox/actions/runs/30779954220)
at commit `8bf9a62` built stable `0.1.0` packages for C#, Go, Java, Kotlin,
Python, Rust, and TypeScript, installed each package in a separate consumer,
passed the shared lifecycle, linked all package hashes to the consumer records,
and preserved byte-identical generated sources. The SDK row is Verified under
its declared acceptance gate. The OpenAPI row remains Partial because
diagnostics, ingress, and richer egress specifications are outstanding.

OCI image parity now has a verified public-image slice. Runs
[30769681608](https://github.com/nya-a-cat/ferrobox/actions/runs/30769681608),
[30769811422](https://github.com/nya-a-cat/ferrobox/actions/runs/30769811422),
and
[30769812772](https://github.com/nya-a-cat/ferrobox/actions/runs/30769812772)
independently passed at commit `8a0c5dc`. Each run verified the same repository
and platform manifest digests, flattened rootfs SHA-256
`6118c08463cec1d2abf919ae45a79f2390ecd45366c394aa22cab80ab457e9d8`,
7,237 extracted members, 183 safely rooted absolute symbolic links, injected
guest/init identities, and ten KVM lifecycle checks. The checks include UID
1000 Python execution, file write/read/list, pause-time rejection, resume,
post-resume execution, credential redaction, and resource cleanup.

The three original generated ext4 files have distinct SHA-256 values because
their filesystem metadata was generated per run. Every image passed read-only
`e2fsck` and the same microVM contract. Strict run
[30771042568](https://github.com/nya-a-cat/ferrobox/actions/runs/30771042568)
confirmed that GitHub Ubuntu 24.04's e2fsprogs 1.47.0 still emitted different
bytes after all exposed directory-input parameters were fixed. The current
workflow builds signed, checksum-pinned e2fsprogs 1.47.4 with libarchive,
normalizes the injected tree into a sorted fixed-time tar, imports that tar
directly, and requires two complete builds to be byte-identical before KVM boot.
Run
[30771721838](https://github.com/nya-a-cat/ferrobox/actions/runs/30771721838)
passed that byte gate with equal SHA-256
`f3580f2126bbbc3aa5b869be3bfabeba7746d3274956b58b1a54c5ca60ae0f2f`,
then exposed the extraction root's `0700` mode during UID 1000 execution. The
builder now fixes and records the archive root as `root:root 0755`. Run
[30771989462](https://github.com/nya-a-cat/ferrobox/actions/runs/30771989462)
verified two byte-identical ext4 builds with SHA-256
`3ed9c8fc9e746916bee5cf72681b30f0f61d70b142e039e016164dec4a2c8c14`,
read-only `e2fsck`, the ten-check real KVM lifecycle, and final resource cleanup.
Artifact `8840831703` has archive digest
`sha256:04e63c6419489d2c7bcfd34ea4b6211fcdb9648ea3fadbd06230ef9bc0794615`.
Private-registry authentication and a public image-selection API remain open.

The four verified state-branching rows and the complete CLI snapshot surface
are backed by
[Live Snapshot KVM E2E run 30764234734](https://github.com/nya-a-cat/ferrobox/actions/runs/30764234734)
at commit `b80d25b`. Its `live-snapshot-evidence` artifact `8838607163` records
schema 1, seventeen API checks, and seven CLI checks, including process-memory
continuation, independent restore, clone isolation, same-ID rollback, fault
cleanup, integrity failure, credential separation, and final resource cleanup.
Standard CI for the same commit passed in
[run 30764234706](https://github.com/nya-a-cat/ferrobox/actions/runs/30764234706).

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
