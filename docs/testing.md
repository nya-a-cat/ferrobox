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
- immutable template build/list/inspect/delete, alias, tamper, and rebuild verification;
- Firecracker/Jailer checksum and version verification;
- full-commit GitHub Action pin verification across every workflow;
- OpenAPI 3.1 validation, exact Axum route matching, and seven generated SDK
  source trees;
- versioned C#, Go, Java, Kotlin, Python, Rust, and TypeScript packages, with
  independent consumers completing the same create/inspect/exec/file/delete
  closed loop through one API process.

The process E2E checks `print(42)`, argv literal handling, file round-trip,
path traversal rejection, deletion, and token redaction from audit records.

The template E2E runs entirely below `RUNNER_TEMP`. It builds a versioned
record from synthetic kernel/rootfs fixtures and credential-free OCI
provenance, lists it, inspects it by content-derived ID, changes the rootfs and
requires a hash/size mismatch, restores the fixture, rejects alias reassignment,
deletes the metadata while retaining both inputs, and rebuilds the exact same
identity. Run
[30786711526](https://github.com/nya-a-cat/ferrobox/actions/runs/30786711526)
passed all seven checks at commit `5f1519a`. Evidence artifact `8845497128`
has archive digest
`sha256:46ba9e64288e7d51015e1825805610b5fa5c1eae2a0aff838e796583e97c87aa`
and retains the initial, tampered, deleted, and rebuilt JSON records through
2026-11-01.

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
package contract and package hashes, all seven native package artifacts, the
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
resolution, and execution occur on the GitHub runner. The verified result is
recorded after the diagnostic history below.

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
retains the diagnostic evidence through 2026-11-01. The audited flat
code-generation projection, explicit Surefire provider prefetch, complete
Gradle lock resolution, preserved schema-property order, and BOM-free evidence
encoding closed these failures.

[Standard CI run 30777303301](https://github.com/nya-a-cat/ferrobox/actions/runs/30777303301)
at commit `86bb3cc` passed the expanded gate. All seven generated clients
completed the common seven-check scenario against one API process. The matrix
contains seven distinct UUIDv7 identities, 35 sanitized audit events, seven
successful creates, seven successful deletes, and every required dependency
manifest. Both recursive generated-tree comparisons passed, including the
post-runtime check. Artifact `8842515354` has archive digest
`sha256:78a7ce80c5f3e3d6a177d81744e0b09580119d76569ac8e6c6435d556ca3331f`
and retains the source, runtime, lock, projection, audit, and toolchain evidence
through 2026-11-01.

[Standard CI run 30779954220](https://github.com/nya-a-cat/ferrobox/actions/runs/30779954220)
at commit `8bf9a62` passed the package-consumer extension of this gate. Seven
versioned `0.1.0` packages were built in their native ecosystem formats and
installed by separate consumers before the common lifecycle ran. The matrix
binds all seven package hashes to all seven sanitized consumer records and
retains build plus consumer dependency state. It reports seven distinct UUIDv7
sandboxes, 35 audit events, seven successful creates, and seven successful
deletes. Both generated roots remained byte-identical after packaging and
execution. Artifact `8843358862` has archive digest
`sha256:202af788f8ad5a41f9276ea53c9fb6ca95183f0cb0bf137ad524bb07f41f44a2`
and retains the complete evidence through 2026-11-01.

## Host architecture CI

`.github/workflows/architecture.yml` runs the shared Process/API and CLI
lifecycle on GitHub-hosted Linux x86_64, Linux aarch64, macOS Apple Silicon,
and Windows ARM64. Both Linux legs also run the complete workspace test/build
and produce a native static musl guest. The macOS and Windows legs test the
five host-side crates and build the native API and CLI binaries.

Every leg retains its runner label, GitHub architecture, image identity, kernel,
normalized CPU description, native Rust host, observational `/dev/kvm` state,
and every step outcome. Linux evidence additionally binds the static guest ELF
machine, size, and SHA-256. All four identify the exercised backend as the
unsafe process backend with isolation `none`.

The final convergence job downloads all four records, requires the same
repository, commit, ref, run, and common smoke checks, validates each platform's
build evidence, and uploads one versioned
`ferrobox-host-architecture-matrix` document. Every test step is allowed to
finish diagnostically; its exact GitHub outcome is recorded and the leg fails
after the evidence is written when any outcome is incomplete.

[Host architecture run 30781691279](https://github.com/nya-a-cat/ferrobox/actions/runs/30781691279)
passed at commit `b456d08`. Both native legs completed all nine checks and the
final convergence accepted the two records. The aarch64 runner was
`ubuntu24-arm64` image `20260719.67.1` on a four-vCPU ARM Neoverse-N2; its
5,179,856-byte guest has ELF machine 183 and SHA-256
`960acc6e8398562afa4817d97fea4a70bf7120b6b8eef77ed0098808e8d5191f`.
The x86_64 runner was `ubuntu24` image `20260720.247.2` on a four-vCPU AMD EPYC
7763; its 5,144,288-byte guest has ELF machine 62 and SHA-256
`32314ad282c3d54a3cbb76ff9a6056a2d81f5ead94aa87c1235ac7b61b728060`.

Artifact `8843888799` retains the aggregate plus both source records with
archive digest
`sha256:7927a042a66fa02ecfe9d7afc6211fc011f9b53f54a57347190a2326715bd2bf`.
The aarch64 and x86_64 evidence documents have SHA-256
`656df8439d1f7a5fe41a44c330a260ad36d8f9dbfb599f614daa3cf23dfe3bfe`
and `e4f49796261a9f45a133984a8f21ea645064ee2d2f3933237bb907dc4f19bdbf`.
All three artifacts expire on 2026-11-01.

The retained capability observation found no `/dev/kvm` device on the aarch64
runner. The x86_64 runner exposed a character device whose initial unprivileged
open returned errno 13; the separate Firecracker gate applies the documented
runner permission setup before its KVM test. Linux aarch64 production microVM
support, an Apple Silicon microVM backend, Windows production isolation, and
WSL2 runtime support remain open host-architecture gates. GitHub documents all
four exercised runner labels in its
[hosted-runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).

The same workflow exercises the host-side crates and shared Process/API plus
CLI lifecycle on a `macos-15` M1 runner. Diagnostic run `30782286698` completed
every functional check and isolated the Bash 4-only `mapfile` used by the action
pin policy. Run `30782442876` passed after the pin checker moved to Bash 3.2
compatible streaming input and cleared the macOS leg for the machine-readable
cross-runner convergence contract.

[Host architecture run 30782762889](https://github.com/nya-a-cat/ferrobox/actions/runs/30782762889)
passed the formal three-platform v2 contract at commit `9f0e8d1`. Its macOS leg
records `macos15` image `20260727.0256.1`, Darwin `24.6.0`, three logical CPUs,
`Apple M1 (Virtual)`, and Rust host `aarch64-apple-darwin`. The macOS evidence
has SHA-256
`c562c3ba00b2920996b8ec8886dde1791d0abed851c3c4359d1855ab245479df`;
its hardware-virtualization observation and `/dev/kvm` fields are unavailable.
The evidence therefore makes no macOS hardware-isolation claim.

Aggregate artifact `8844232131` retains all three source records and the
accepted v2 matrix with archive digest
`sha256:f4bd711e659670967466caaa7a78075928534cf64e7b38814155c7b367b5c39b`.
The macOS source artifact is `8844229804`, archive digest
`sha256:342535585b10167fa3745c5ccc82a8eece5eae14eab635eb7f6024b8472cb7d8`.
All four artifacts expire on 2026-11-01.

[Host architecture run 30784445732](https://github.com/nya-a-cat/ferrobox/actions/runs/30784445732)
passed the `windows-11-arm` diagnostic at commit `ad14972`. The native ARM64
Rust tests/build and both shared process lifecycles completed successfully on
image `20260727.122.1` with Rust host `aarch64-pc-windows-msvc`.

[Host architecture run 30784854537](https://github.com/nya-a-cat/ferrobox/actions/runs/30784854537)
passed the formal four-platform v3 contract at commit `d373e93`. Its Windows
record reports Windows 11 ARM64, four logical CPUs, image
`20260727.122.1`, and all eight checks complete. The Windows source document
has SHA-256
`a418e3c94cbf74a873acef32ab903bb45f22889fb969c25fa551dc78ff681171`;
the accepted matrix document has SHA-256
`06e143b738500c547f658dbc2d3c5013698bf6beff8894efa2c3fda88055a40b`.

Aggregate artifact `8844936585` retains all four source records and the v3
matrix with archive digest
`sha256:c71b81759f5f28a398b93657123f5eff312704af0f8f12a8065b99f2319831cc`.
Windows source artifact `8844933901` has archive digest
`sha256:83978106366f3439f2d0bfc7c86b594d6fcd7cd574e6ddaf6cac7f0e70c30f8c`.
All five artifacts expire on 2026-11-01. The Windows record identifies the
unsafe process backend with isolation `none` and makes no hardware-isolation
claim.

## OCI image KVM CI

`.github/workflows/oci.yml` runs entirely on a GitHub-hosted Ubuntu 24.04
nested-KVM runner. It requires a digest-qualified `linux/amd64` image, verifies
the index, selected manifest, config, layer digests and sizes, builds a hardened
flattened rootfs, injects the static guest and init, creates an ext4 image, runs
read-only `e2fsck`, registers the exact kernel/rootfs pair in the immutable
template catalog, enables and measures fs-verity on the Btrfs source assets, and
boots the result through the real HTTP/Firecracker path.

The rootfs stage performs two independent builds from the digest-qualified
registry reference. It builds the signed, checksum-pinned e2fsprogs 1.47.4
source with libarchive, verifies tar-input reproducibility in a focused smoke
gate, and materializes the injected rootfs as a sorted, fixed-time GNU tar with
numeric ownership. It fixes the filesystem UUID, directory hash seed, locale,
and lazy-initialization settings, then requires equal ext4 bytes and equal
schema-3 deterministic fields. The retained
`oci-rootfs-reproducibility.json` records both SHA-256 values, `cmp` status, and
the complete ext4 identity used by the KVM test.

The API flow selects the immutable template ID, rejects an unknown ID, proves
that catalog assets override an invalid configured fallback, then verifies UID
1000 Python execution, literal argv execution, file write/read/root-directory
listing, paused-command rejection, resume followed by a fresh guest command,
delete/stale-handle behavior, audit credential redaction, and final
process/network cleanup. Before launch it reruns constant-time fs-verity
measurement on both protected source paths and compares the results with the
retained offline digests. A failure records only its stage, HTTP status, error
code, and message.

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

[Run 30787903798](https://github.com/nya-a-cat/ferrobox/actions/runs/30787903798)
passed the extended twelve-check contract at commit `639666c`. It registered
the digest-pinned OCI descriptor once from build-stage files and once from the
actual runtime files. Both locations produced template ID
`tpl-2a4a8bfe7412552c0ec6dcaf7cc2dc258dfccacef05c162149bc80827071`
and full specification digest
`sha256:2a4a8bfe7412552c0ec6dcaf7cc2dc258dfccacef05c162149bc808270717abf`.
The runtime inspection verified kernel SHA-256
`e20e46d0c36c55c0d1014eb20576171b3f3d922260d9f792017aeff53af3d4f2`
and rootfs SHA-256
`3ed9c8fc9e746916bee5cf72681b30f0f61d70b142e039e016164dec4a2c8c14`
against `/mnt/ferrobox-oci/images/vmlinux` and
`/mnt/ferrobox-oci/images/oci-python.ext4` before boot. The guest then reported
Python 3.11.15 and completed every prior lifecycle assertion plus
`content-derived-template-identity` and `template-runtime-artifact-match`.
Artifact `8845975100` has archive digest
`sha256:681fe9dde6d9f2c34bc54dc906e277f57ad96149820347351f2695b5e760ed0c`
and expires on 2026-11-01.

[Run 30789867561](https://github.com/nya-a-cat/ferrobox/actions/runs/30789867561)
passed the fifteen-check runtime-resolution contract at commit `274a417`. The
successful request used template ID
`tpl-2a4a8bfe7412552c0ec6dcaf7cc2dc258dfccacef05c162149bc80827071`;
an all-zero unknown ID returned `404 not_found`. The configured rootfs was a
deliberately invalid 45-byte file at
`/home/runner/work/_temp/invalid-default-rootfs.ext4` with SHA-256
`990278af04fe88cd43f527f0f16f3077fe509f9ae38c48d734591c7ceba42b2d`.
The resolved catalog inputs were `/mnt/ferrobox-oci/images/vmlinux` and
`/mnt/ferrobox-oci/images/oci-python.ext4`; the guest booted and reported
`oci-python=3.11.15`. Artifact `8846706558` has archive digest
`sha256:419b7d98006bfa6307591c773ee2961e847f0d38a63ec04447cffb751bb3b353`
and expires on 2026-11-01. Standard CI run
[30789867545](https://github.com/nya-a-cat/ferrobox/actions/runs/30789867545)
and four-platform architecture run
[30789867548](https://github.com/nya-a-cat/ferrobox/actions/runs/30789867548)
passed for the same commit.

[Run 30793770602](https://github.com/nya-a-cat/ferrobox/actions/runs/30793770602)
passed the sixteen-check fs-verity/KVM contract at commit `705f0ed`. The signed
fsverity-utils 1.7 source matched upstream Git tree
`849ba951347671baf7691000e94dfcdffb36fe56`, the installed binary and upstream
checks shared the same fixed build flags, and both source assets were on Btrfs.
The 44,279,576-byte kernel measured at 920/1,022 microseconds P50/P95; the
1,073,741,824-byte rootfs measured at 936/1,046 microseconds. Every one of 31
measurements per asset matched its offline digest, writable opens returned
`EPERM`, and a byte-identical rootfs reflink dropped verity metadata. The API
remeasured the protected source paths before boot and produced sandbox
`019fc68b-8469-7ea2-b94c-f1a781a02613` with `oci-python=3.11.15`. Artifact
`8848166249` has archive digest
`sha256:ca006cf19ad009a5574421d242f43562840e2cdf0b682466b873ca11880a6ba0`
and expires on 2026-11-01. Standard CI run
[30793770601](https://github.com/nya-a-cat/ferrobox/actions/runs/30793770601)
and four-platform architecture run
[30793770628](https://github.com/nya-a-cat/ferrobox/actions/runs/30793770628)
also passed at the code head.

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
