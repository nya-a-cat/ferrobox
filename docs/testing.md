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
- first-party Agent Skill contract validation;
- Rust CLI create/exec/file/delete end-to-end verification;
- Firecracker/Jailer checksum and version verification.
- full-commit GitHub Action pin verification across every workflow.

The process E2E checks `print(42)`, argv literal handling, file round-trip,
path traversal rejection, deletion, and token redaction from audit records.

The CLI/Agent Skill E2E repeats the user-facing flow through the compiled
`ferrobox` binary. It captures the one-time token, proves inspection and literal
argv handling, round-trips and lists a host file, verifies the
running-to-paused-to-running state machine and paused-command rejection, deletes
the exact sandbox, checks post-delete rejection, and confirms that the audit log
omits the bearer token. The static skill gate also validates its frontmatter,
UI metadata, security invariants, command set, and remote-installer policy.

[Standard CI run 30763552790](https://github.com/nya-a-cat/ferrobox/actions/runs/30763552790)
passed the Process/API, Agent Skill contract, and CLI/Agent Skill E2E steps at
commit `8854525`.

[Standard CI run 30763757911](https://github.com/nya-a-cat/ferrobox/actions/runs/30763757911)
passed formatting, Clippy, the Skill gate, and the extended CLI lifecycle E2E
at commit `4b3daae`. Its state sequence was running, paused, rejected execution
with HTTP 409, resumed, running, then deleted.

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

The supply-chain gate also fetches a pinned Syft release and produces SPDX 2.3
SBOMs for the Rust source dependency graph, mounted Python rootfs, and exact
Docker comparator image. It emits an in-toto Statement v1 inventory covering
the built Ferrobox binaries, installed VMMs, gVisor sidecars, Kata-selected
assets, host comparator runtimes, guest kernel/rootfs, workload image digest,
and all SBOM hashes. The statement is accepted only after every serialized
local-file digest is independently recomputed. Its outcome is reported beside
the deferred performance gates and enforced at the final convergence step.

Run 30762230170 verified this gate at commit `6fb848c` and retained artifact
`8838016271`. The in-toto predicate contains five subjects, nineteen executed
files, six guest assets, six upstream inputs, and three SPDX 2.3 SBOMs. The
SBOMs report 277 source packages, 263 rootfs packages, and 141 comparator-image
packages. Standard CI run 30762230183 and Live Snapshot KVM run 30762230199
passed for the same revision. The full KVM result remained red after Kata
cleanup hit its outer deadline and left the Kata JSON absent; the supply-chain,
Internet, container, and file-workload outcomes passed independently.

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
Percentiles use the nearest-rank definition; with five samples, P95 is the
largest retained sample.
When `FERROBOX_SNAPSHOT_ROOT` is set, the functional path prepares a snapshot
and the later benchmark measures restore-to-ready preparation and ready-pool
allocation independently.

The workflow also starts the real HTTP API with five ready microVMs and retains
`ferrobox-http-benchmark.json`. Its create samples include HTTP parsing, runtime
allocation, control-plane state registration, token issuance, audit writing,
JSON serialization, and loopback response transfer.

The same job pulls a cached Python image, records its resolved Docker digest,
and measures Docker Engine and gVisor through the same Unix-socket HTTP
harness. Each artifact retains five create-and-start samples, one hundred
`/bin/true` samples, thirty Python samples, twenty file-workload samples, and
twenty archive write/read samples. Direct Firecracker, CPU-capped Firecracker,
Cloud Hypervisor, and Kata QEMU controls run later on the same hosted KVM job.

The microVM leadership gate runs CPU-capped direct Firecracker, Cloud
Hypervisor, Cloud Hypervisor, and CPU-capped direct Firecracker in ABBA order.
All four cohorts use the identical guest probe and cloned-client sequence.
Their raw series are pooled into 200 `/bin/true` and 60 Python samples per VMM,
then nearest-rank P50/P95 are recomputed; both percentiles must be lower for
Firecracker. The full Ferrobox runtime has separate overhead limits:
minimal-command P50 within 25% of pooled direct Firecracker, minimal-command
P95 at or below 15 ms, and Python P50/P95 within 10%. Snapshot preparation,
HTTP allocation, and full-runtime hot execution retain their Kata and Cloud
Hypervisor boundary checks.

Run 30761222885 passed the formal ABBA aggregation and microVM gate at commit
`52d3ef6`. Artifact `8837570647` contains all four source cohorts and the
pooled result; final enforcement left only the HTTP file-API gate red.

Measurement steps validate artifact schemas and sample counts without applying
leadership claims. All performance gates run after every comparator has
finished, continue long enough to expose every failed dimension, and are then
enforced together. A red gate therefore still uploads the complete available
matrix instead of stopping at the first slower comparison.

Kata containerd management calls have TERM and KILL deadlines. The Kata step
also continues to the shared enforcement point when its shim path fails or
hits the outer step deadline. Later container, file, and Internet-policy checks
still run, the missing or failed Kata result makes the final job red, and the
containerd log is retained with the other evidence.

The hosted `ctr` comparator uses Kata's documented single sandbox-cgroup mode.
This avoids accumulating per-container cgroups on the nested-KVM runner. The
exact derived Kata configuration is retained next to `containerd.log`.

Internet-policy E2E has its own deferred gate. A failure retains the host
resolver inputs, IPv4 forwarding state, namespace interfaces and routes, and
project-scoped nftables tables in `network-diagnostics.txt`; final enforcement
reports its outcome with every performance gate.

The network gate requires the guest to receive its sandbox gateway as the only
resolver, resolve and fetch public HTTPS through UDP DNS, complete an explicit
wire-format TCP DNS query, reject the metadata endpoint, and clean up the VM
and namespace. Cleanup also proves that no tagged host-forwarding rule remains.
DNS runs through the same host-originated relay path used on cloud-specific
GitHub runners.

## Live snapshot KVM CI

`.github/workflows/snapshots.yml` is an independent hosted-KVM workflow for
full running state. It validates memory-resident process continuation, rootfs
consistency, source divergence, paginated metadata, token separation, restore
and clone isolation, same-ID rollback, source-independent lifetime,
paused-source preservation, digest failure closure, audit redaction, and final
VM, jail, cgroup, and snapshot cleanup. Each run uploads its result and a
sanitized schema-1 manifest.

Partial-batch cleanup uses the opt-in `fault-injection` Cargo feature. A test
API reads `FERROBOX_TEST_CLONE_FAILURE_FILE`; a valid integer requests failure
after that many clones have launched. Production builds leave the feature
disabled and contain no active injection branch. Resource mismatch failures
print the before/after Firecracker PID, jail root, cgroup leaf, and network
namespace sets so hosted evidence identifies the leaked resource class.

The hosted pool benchmark launches several VMs together. Each launch must pass
an accepting Firecracker `/version` request after socket creation; a transient
Unix-socket connect race is retried within the existing API deadline. Failed
launch cleanup drains the exact cgroup leaf and records controller diagnostics
if the leaf remains busy. Transport failures include the Firecracker HTTP
method and API path, and each VM reuses one Unix HTTP client across the full
configuration sequence.

The current hosted proof is
[Live Snapshot KVM E2E run 30758919945](https://github.com/nya-a-cat/ferrobox/actions/runs/30758919945)
for commit `87d5456`. It completed all sixteen checks and uploaded artifact
`8836989482`; standard CI for the same commit passed in run `30758919931`.

The HTTP artifact also retains a five-request concurrent burst with each
request's latency and total wall time. Sequential and concurrent P95 must both
remain below 80 ms.

## Hosted density evidence

The KVM workflow starts a zero-pool API on a dedicated jail and runtime root,
then `scripts/benchmark-density.py` accumulates 1, 5, 10, and 25 live snapshot-
restored sandboxes. The JSON artifact retains all create-to-ready samples,
five-sample host `MemAvailable` medians, Firecracker RSS/PSS/USS, cgroup
`memory.current`, kernel version, commit identity, sandbox specification, and
cleanup state. Schema validation requires exact tier counts, live Firecracker
processes, positive PSS/USS/controller totals, a positive 25-instance host
delta, and zero cgroup leaves after deletion.

Schema 2 also retains nearest-rank create-to-ready P50/P95/P99, sequential
total time, and throughput. At the 25-instance tier, the first regression gate
limits host delta to 64 MiB, PSS and USS to 40 MiB each, and cgroup current to
48 MiB per sandbox. Runs 30760119705, 30760452371, and 30761222885 independently
passed schema 2 at 25 live sandboxes and returned all cgroups to zero. Their
evidence artifacts are `8837241226`, `8837336923`, and `8837581831`.

## Completion rule

Workflow presence is not evidence of success. Ferrobox remains incomplete until
the GitHub check runs are green, KVM output is retained, and the final
requirement audit maps each claim to a workflow result or artifact.
