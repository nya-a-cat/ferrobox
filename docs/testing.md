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
- Firecracker/Jailer checksum and version verification;
- full-commit GitHub Action pin verification across every workflow;
- OpenAPI 3.1 validation, exact Axum route matching, and seven generated SDK
  source trees;
- generated C#, Go, Java, Kotlin, Python, Rust, and TypeScript SDKs completing
  the same create/inspect/exec/file/delete closed loop through one API process.

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

The OpenAPI gate uses `openapi/ferrobox-v1.json` as the authoritative input for
validation and exact source-route comparison. A reviewed deterministic merge
patch derives the code-generation view consumed by seven black-box generated
clients. The gate recalculates that projection from the retained overlay and
records both SHA-256 values. It retains a structural evidence document, seven
sanitized runtime documents, one aggregate matrix, the shared sanitized audit
log, dependency manifests, a toolchain record, the generator manifest, the
overlay, the projected document, and all seven generated source trees. It
compares both generated trees again after runtime to prove that harness builds
did not alter the source evidence. Each independently calculated projection
must equal one job-stable runner input, keeping its path identical across both
generator invocations. See [`openapi.md`](openapi.md) for the credential and
supply-chain boundary.

[Standard CI run 30766020434](https://github.com/nya-a-cat/ferrobox/actions/runs/30766020434)
and independent
[run 30766116507](https://github.com/nya-a-cat/ferrobox/actions/runs/30766116507)
passed this gate at commit `9cb5c54`. Each run performed two byte-identical
generations before the Python runtime test. Artifacts `8838973235` and
`8839002111` retain complete hidden and visible generator output; their seven
language file counts and tree hashes match across the two runs. The Python
tree contains 60 retained source/metadata/lock files and no runtime bytecode or
installation cache.

The expanded language gate runs every client against the same loopback API
process and the same typed scenario. Its aggregate gate requires seven distinct
UUIDv7 sandboxes, the same seven check names per language, a successful audited
create and delete for each identity, no credential-shaped evidence keys, and a
non-empty dependency manifest for every toolchain. All builds, dependency
resolution, and execution occur on the GitHub runner. A passing run for this
expanded gate is pending.

[Diagnostic run 30774266980](https://github.com/nya-a-cat/ferrobox/actions/runs/30774266980)
at commit `a38a4af` completed Go, Python, and TypeScript, retained their three
sanitized seven-check records, and passed the post-runtime source-immutability
gate. C# reached the API then failed its anonymous-union converter; Java and
Rust rejected invalid generated source; Kotlin wrote a lock but lacked cached
runtime JARs for offline execution. The follow-up keeps the wire contract and
uses named discriminated variants, a typed inline `cwd` default, and explicit
Gradle runtime-classpath prefetch.

[Diagnostic run 30774849266](https://github.com/nya-a-cat/ferrobox/actions/runs/30774849266)
at commit `2f0540d` again completed Go, Python, and TypeScript and passed both
deterministic-generation and post-runtime immutability gates. C# and Rust now
compiled and reached runtime deserialization, exposing target-specific handling
of the strict tagged union. Java compiled and stopped at its offline Surefire
provider lookup. Kotlin's prefetch resolved the local unbuilt classes directory
alongside external dependencies. Artifact `8841711323`, archive digest
`sha256:d9cd88cef12f2aacbc3012f29359a2c6802956257cfe0833f24ba895f37a7293`,
retains the diagnostic evidence through 2026-11-01. The next run uses an audited
flat code-generation projection, explicit Surefire provider prefetch with a
retained JAR digest, and Kotlin external-runtime resolution. A passing result is
pending.

## OCI image KVM CI

`.github/workflows/oci.yml` runs entirely on a GitHub-hosted Ubuntu 24.04
nested-KVM runner. It requires a digest-qualified `linux/amd64` image, verifies
the index, selected manifest, config, layer digests and sizes, builds a hardened
flattened rootfs, injects the static guest and init, creates an ext4 image, runs
read-only `e2fsck`, and boots the result through the real HTTP/Firecracker path.

The rootfs stage performs two independent builds from the digest-qualified
registry reference. It builds the signed, checksum-pinned e2fsprogs 1.47.4
source with libarchive, verifies tar-input reproducibility in a focused smoke
gate, and materializes the injected rootfs as a sorted, fixed-time GNU tar with
numeric ownership. It fixes the filesystem UUID, directory hash seed, locale,
and lazy-initialization settings, then requires equal ext4 bytes and equal
schema-3 deterministic fields. The retained
`oci-rootfs-reproducibility.json` records both SHA-256 values, `cmp` status, and
the complete ext4 identity used by the KVM test.

The API flow verifies UID 1000 Python execution, literal argv execution, file
write/read/root-directory listing, paused-command rejection, resume followed by
a fresh guest command, delete/stale-handle behavior, audit credential redaction,
and final process/network cleanup. A failure records only its stage, HTTP status,
error code, and message.

Runs
[30769681608](https://github.com/nya-a-cat/ferrobox/actions/runs/30769681608),
[30769811422](https://github.com/nya-a-cat/ferrobox/actions/runs/30769811422),
and
[30769812772](https://github.com/nya-a-cat/ferrobox/actions/runs/30769812772)
passed independently at commit `8a0c5dc`. Their artifacts are `8840115730`,
`8840141572`, and `8840143404`, with archive digests
`sha256:cc390daf989cd8b688d701e02d2c2cb18e9c7a95ec1b51a247eed6737e5b5b2e`,
`sha256:aa9d04a9c80f63051c028daa9582aca70d64bc849566faa9577d8bc6841f4ecf`,
and
`sha256:766291a6845d432cd48e742f482d821528d7b597f0bdf5d520eb7b6eda2c15a6`.
All three retained the same source, manifest, flattened-rootfs, guest, init,
Python-version, member-count, link-rewrite, and ten-check values. Their ext4
byte hashes differ; all three report `e2fsck_read_only: true`.

[Run 30771042568](https://github.com/nya-a-cat/ferrobox/actions/runs/30771042568)
then exercised the first strict two-build gate with Ubuntu 24.04's e2fsprogs
1.47.0. Both OCI materializations completed with equal configured fields, while
the ext4 hashes were
`38d7be2ac9b07822200da21145aa039d15355f433930c8948ae5dc921b47cdff`
and
`75e4df23361612612d973156277ecaeffc614e2fb7ac26193184af1e80b43fa0`.
The workflow stopped before KVM at the byte-equality gate. Artifact `8840543144`
retains both schema-2 records and the first differing-byte report. This is the
directory-import baseline for the pinned 1.47.4 tar-input path.

[Run 30771721838](https://github.com/nya-a-cat/ferrobox/actions/runs/30771721838)
passed signed-source verification, the focused tar smoke gate, and the complete
OCI byte-equality gate at commit `92a61a7`. Both ext4 files have SHA-256
`f3580f2126bbbc3aa5b869be3bfabeba7746d3274956b58b1a54c5ca60ae0f2f`.
The real KVM flow then reported `guest spawn: Permission denied (os error 13)`
for the UID 1000 Python command. Tar input had preserved the extraction staging
root's mode. The revised gate fixes and records `root:root 0755`.

[Run 30771989462](https://github.com/nya-a-cat/ferrobox/actions/runs/30771989462)
passed the complete contract at commit `6c4d140`. Its two ext4 images are
byte-identical with SHA-256
`3ed9c8fc9e746916bee5cf72681b30f0f61d70b142e039e016164dec4a2c8c14`;
the retained record has empty `cmp_detail`, equal schema-3 deterministic fields,
and `e2fsck_read_only: true`. The real KVM path passed all ten checks, including
UID 1000 Python 3.11.15, argv execution, file operations, pause/reject/resume,
stale-handle rejection, redaction, and process/network cleanup. Artifact
`8840831703` has archive digest
`sha256:04e63c6419489d2c7bcfd34ea4b6211fcdb9648ea3fadbd06230ef9bc0794615`.

[Standard CI run 30769681624](https://github.com/nya-a-cat/ferrobox/actions/runs/30769681624)
passed formatting, tests, Clippy, builds, process/CLI/OpenAPI conformance, and
the Firecracker/Jailer `1.15.1` host gate for the same commit.

[Live Snapshot KVM run 30769681594](https://github.com/nya-a-cat/ferrobox/actions/runs/30769681594)
also passed the complete running/paused snapshot, restore, clone, rollback,
integrity, credential, and cleanup contract at `8a0c5dc`. Artifact `8840293443`
has archive digest
`sha256:062ffefef493979c193cdcf7f633a965ee1e7869b5c84f34e9cfb8722dfc5c8f`.

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

The workflow also builds the Rust CLI and runs a second snapshot closed loop
after the exhaustive API suite. That CLI path covers create/list/inspect/verify,
same-ID rollback, independent restore, two clones, preserved file state,
credential redaction, and final snapshot artifact cleanup. Its uploaded
`snapshot-cli-e2e.json` contains only IDs, counts, and check names.

The current hosted proof is
[Live Snapshot KVM E2E run 30764234734](https://github.com/nya-a-cat/ferrobox/actions/runs/30764234734)
for commit `b80d25b`. It completed seventeen API checks plus the seven-check CLI
closed loop and uploaded artifact `8838607163`, whose archive digest is
`sha256:fb0141120e4ca9a6acd8073c11cfa4f3c0281adf3e143d05798b38cad57267d0`.
Standard CI for the same commit passed in run `30764234706`.

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
