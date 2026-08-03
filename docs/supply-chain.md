# Supply-chain record

Ferrobox records the exact project artifacts, isolation inputs, comparator
runtimes, guest assets, and workload images exercised by the hosted KVM gate.
The evidence is produced on GitHub and retained with the performance artifacts.

## Comparison baseline

The pinned upstream audit in [SOTA parity program](sota-roadmap.md) establishes
three useful reference points:

- Microsandbox pins release-workflow actions to full commit SHAs.
- CubeSandbox validates a release manifest containing component, guest-image,
  kernel version, and kernel digest identities.
- OpenSandbox documents keyless GitHub/Sigstore provenance, digest-bound cosign
  signatures for release images, and consumer verification commands.

Ferrobox combines action SHA pins, input digests, an execution inventory, and
standard SBOMs in the KVM evidence path. Keyless signed release attestations
remain a separate parity item because they require additional GitHub token
permissions.

## GitHub Actions inputs

Every external action in `architecture.yml`, `ci.yml`, `kvm.yml`, `oci.yml`, and
`snapshots.yml` uses a full 40-character commit SHA. A human-readable release
or channel comment remains beside each pin. The current set is:

| Action | Revision |
| --- | --- |
| `actions/checkout` | `3d3c42e5aac5ba805825da76410c181273ba90b1` (`v7.0.1`) |
| `actions/download-artifact` | `3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c` (`v8.0.1`) |
| `actions/upload-artifact` | `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` (`v7.0.1`) |
| `actions/cache` restore/save | `55cc8345863c7cc4c66a329aec7e433d2d1c52a9` (`v6.1.0`) |
| `dtolnay/rust-toolchain` | `4cda84d5c5c54efe2404f9d843567869ab1699d4` (`stable`, resolved 2026-08-03) |
| `Swatinem/rust-cache` | `e18b497796c12c097a38f9edb9d0641fb99eee32` (`v2`) |

Updates require an upstream release review, a new full SHA, and successful
standard, snapshot, and KVM workflows. `scripts/check-action-pins.sh` rejects
any external workflow reference that is not a full lowercase commit SHA. Its
streaming reader is exercised on Ubuntu Bash, macOS Bash 3.2, and Windows ARM64
Git Bash.

## Firecracker

Ferrobox pins Firecracker and Jailer to the same upstream release:

| Item | Value |
| --- | --- |
| Version | `1.15.1` |
| Release date | 2026-04-07 |
| Upstream | `firecracker-microvm/firecracker` |
| License | Apache-2.0 |
| x86_64 archive | `firecracker-v1.15.1-x86_64.tgz` |
| SHA-256 | `d4a32ab2322d887ca1bc4a4e7afa9cc35393e6362dfc2b3becb389d362e4275a` |

The pin stays on `1.15.1` while upstream
[issue #6074](https://github.com/firecracker-microvm/firecracker/issues/6074)
remains open for a `1.16.x` Pause/Resume vsock livelock. Promotion to a newer
release requires the hosted KVM lifecycle gate to pass pause, rejected execution
while paused, resume, and a post-resume guest command.

[Standard CI run 30769681624](https://github.com/nya-a-cat/ferrobox/actions/runs/30769681624),
three independent OCI KVM runs, and
[Live Snapshot KVM run 30769681594](https://github.com/nya-a-cat/ferrobox/actions/runs/30769681594)
passed this pin at commit `8a0c5dc`. The snapshot run completed the running and
paused source paths, restore, clone, rollback, integrity, credential, and cleanup
contract and retained artifact `8840293443`.

`scripts/fetch-firecracker.sh` uses a fixed HTTPS URL, verifies the pinned hash
before parsing, rejects absolute and parent-traversing archive members, extracts
into a fresh temporary directory, and installs only the two expected
executables. It never pipes network content into a shell.

The script installs into the user cache unless an explicit destination is
provided. Production deployment copies the verified executables into a
root-owned directory that the Firecracker runtime UID cannot modify.

## Other hosted isolation inputs

The KVM comparison fixes every downloaded runtime and workload identity:

| Input | Pin |
| --- | --- |
| Guest kernel | `firecracker-ci/v1.15/x86_64/vmlinux-6.1.155`, SHA-256 `e20e46d0c36c55c0d1014eb20576171b3f3d922260d9f792017aeff53af3d4f2` |
| Cloud Hypervisor | `v53.0`, SHA-256 `448af3d4e59b22c2987f7df94c213ad40fb53a10d437e42b5ee6c4fce7c29ecc` |
| gVisor | `release-20260721.0`, SHA-512 `1e951f8d9dd2198e16ad66066fac0db42943ac8e8ca35c7173a20f0fbc859b8185c33c478ae0dc3c4e76b8c06d99ed118286caf43ad207c96578b42af62cae72` |
| Kata Containers | `3.31.0`, SHA-256 `68c2786a0b97023f62f3eca02dc868b78a794e5469d4ddd6cc9e0bd4a7212b0b` |
| Python comparator image | `python:3.11-slim-bookworm`, repository digest `sha256:b18992999dbe963a45a8a4da40ac2b1975be1a776d939d098c647482bcad5cba` |

The workflow rejects a Python tag resolving to another repository digest.
gVisor uses its immutable dated release path and verifies both the upstream
checksum file and the repository-pinned SHA-512. Downloaded archives receive a
path-traversal or expected-prefix check before extraction.

## Kernel and root filesystem

The GitHub KVM gate verifies the guest kernel hash before installation. The
Python rootfs build manifest records:

- distribution, suite, and architecture;
- sandbox UID and GID;
- SHA-256 of the injected `ferrobox-guest`;
- SHA-256 of the ext4 root filesystem.

The unified evidence record separately hashes the installed kernel and rootfs,
so copied or modified runtime assets fail the final verification loop.

## Template catalog provenance

The schema-1 template descriptor binds a credential-free source kind,
reference, and SHA-256 to the target operating system, architecture, runtime,
human version, kernel digest/size, and rootfs digest/size. Canonical struct JSON
produces a full specification SHA-256. Its first 240 bits form the public
template ID; the complete digest stays available for collision and corruption
checks. Host file locators remain outside this identity payload.

Standard CI run
[30786711526](https://github.com/nya-a-cat/ferrobox/actions/runs/30786711526)
proved descriptor integrity, artifact re-verification, alias immutability,
metadata-only deletion, and deterministic identity after rebuild. Artifact
`8845497128` has archive digest
`sha256:46ba9e64288e7d51015e1825805610b5fa5c1eae2a0aff838e796583e97c87aa`
and expires on 2026-11-01. Registry credentials, remote pulls, and build-system
secrets stay outside this catalog contract.

OCI KVM run
[30787903798](https://github.com/nya-a-cat/ferrobox/actions/runs/30787903798)
binds the resolved public platform manifest to template ID
`tpl-2a4a8bfe7412552c0ec6dcaf7cc2dc258dfccacef05c162149bc80827071`
and full specification digest
`sha256:2a4a8bfe7412552c0ec6dcaf7cc2dc258dfccacef05c162149bc808270717abf`.
Registration below independent build and runtime paths converged on that same
identity. Inspection then rehashed the exact kernel and rootfs paths passed to
the KVM lifecycle. Artifact `8845975100` preserves both catalog records, the
runtime inspection, aggregate binding, OCI provenance, reproducibility record,
and twelve-check KVM result under archive digest
`sha256:681fe9dde6d9f2c34bc54dc906e277f57ad96149820347351f2695b5e760ed0c`.

## OCI root filesystem pipeline

The hosted OCI gate fixes the public Python conformance image to repository
digest
`sha256:b18992999dbe963a45a8a4da40ac2b1975be1a776d939d098c647482bcad5cba`
and selects the `linux/amd64` manifest
`sha256:28255a3ace7eb4c48bc1b57b90af29e1bc82b4fd6c60614a8e3dce61b87ff941`.
The builder verifies descriptor bytes, index-to-manifest linkage, config
identity and platform, layer count against diff IDs, and every layer's digest,
size, and supported media type before extraction.

Rootfs export uses `go-containerregistry` v0.21.8 source archive SHA-256
`54d520389ab2e7dbaceafb94fbe5ba151ae51e2dc613d3f3f58689d3bbfce984`.
The repository patch admits a valid root directory tar entry while retaining
the release's unsafe-path, digest-verification, and opaque-whiteout fixes. The
workflow applies the patch, runs its focused upstream regression tests, builds
`crane` twice with byte equality, and retains source, toolchain, patch, test,
and build records. Upstream publishes no SLSA provenance for this source
archive, so that absence stays explicit in `BUILDINFO.txt`.

`scripts/safe-extract-tar.py` applies Python 3.12's data filter plus Ferrobox
limits for traversal, absolute member paths, links, duplicates, special files,
member count, and logical bytes. Rootfs-contained absolute links are rewritten
to safe archive-relative targets. Three independent runs produced the same
flattened tar SHA-256
`6118c08463cec1d2abf919ae45a79f2390ecd45366c394aa22cab80ab457e9d8`
and extraction counts. Those runs exposed per-run ext4 metadata as the
remaining byte-reproducibility gap.

`scripts/build-e2fsprogs.sh` builds e2fsprogs 1.47.4 from the official kernel.org
release archive at tag target
`7ee1d505ef3b37831215f490411f346fe57e9053`. The archive size is 7,337,236 bytes
and its SHA-256 is
`fd5bf388cbdbe006a3d3b318d983b2948382440acc85a87f1e7d108653e8db0b`.
The script verifies the clear-signed published checksum with exact signer
fingerprint `B8868C80BA62A1FFFAF5FDA9632D3A06589DA6B1`, safely extracts the source,
forces direct libarchive support, and records compiler, library, binary,
configuration, signature, extraction, and build evidence. A focused smoke gate
imports the same fixed tar twice, requires equal ext4 bytes, checks the image
read-only, and reads the imported fixture through the newly built `debugfs`.

The rootfs builder turns the fully injected tree into a GNU tar with lexical
name ordering, numeric UID/GID fields, and the selected source-date timestamp.
The archive root is fixed to UID/GID 0 and mode `0755`. It derives the
filesystem UUID and directory hash seed from that tar identity, sets
`SOURCE_DATE_EPOCH` and `E2FSPROGS_FAKE_TIME`, fixes `LC_ALL=C`, and disables lazy
inode-table and journal initialization. `mke2fs -d` consumes the tar directly.
The workflow performs two complete OCI pull, extraction, injection, tar, and
ext4 builds; it requires byte equality and equal schema-3 deterministic records.
Read-only `e2fsck` and the real KVM lifecycle remain mandatory after this gate.
The controls follow the
[Reproducible Builds system-image guidance](https://reproducible-builds.org/docs/system-images/),
the upstream
[mke2fs tar-input contract](https://github.com/tytso/e2fsprogs/blob/v1.47.4/misc/mke2fs.8.in),
and the
[e2fsprogs 1.47.4 release directory](https://mirrors.edge.kernel.org/pub/linux/kernel/people/tytso/e2fsprogs/v1.47.4/).

GitHub OCI run
[30771989462](https://github.com/nya-a-cat/ferrobox/actions/runs/30771989462)
verified this complete chain at commit `6c4d140`. The two independent ext4
builds both produced SHA-256
`3ed9c8fc9e746916bee5cf72681b30f0f61d70b142e039e016164dec4a2c8c14`
and then passed the real ten-check KVM lifecycle. Artifact `8840831703` has
archive digest
`sha256:04e63c6419489d2c7bcfd34ea4b6211fcdb9648ea3fadbd06230ef9bc0794615`.

Run `30787903798` preserved the same reproducible ext4 identity and extended
the KVM gate to twelve checks. The new checks require a content-derived
template identity and exact equality between catalog artifact descriptors and
the files used for boot. The kernel is 44,279,576 bytes with SHA-256
`e20e46d0c36c55c0d1014eb20576171b3f3d922260d9f792017aeff53af3d4f2`;
the rootfs is 1,073,741,824 bytes with the ext4 SHA-256 above.

## SBOM generator

`scripts/fetch-syft.sh` installs the official Linux x86_64
[Syft `v1.50.0`](https://github.com/anchore/syft/releases/tag/v1.50.0)
archive after verifying SHA-256
`bf7b29ff57f06da30918266a0e1c2885a8f99784798d1bdb1628886aa015d788`.
It applies the same HTTPS, temporary-directory, and archive-path controls used
for Firecracker.

`scripts/generate-e2e-sboms.sh` creates three SPDX 2.3 JSON documents:

1. source dependencies from the checked-out repository and `Cargo.lock`;
2. installed packages inside the read-only mounted Python ext4 rootfs;
3. installed packages in the exact local Docker comparator image.

The gate requires a non-empty package set and document namespace in each SBOM.
Syft update checks are disabled during evidence generation.

## OpenAPI generator and generated-SDK tooling pins

Standard CI generates developer-client evidence with OpenAPI Generator v7.22.0.
`scripts/fetch-openapi-generator.sh` downloads the official GitHub release JAR,
requires size 31,390,141 bytes, verifies SHA-256
`37f23217f40cabac50c435312ea1d3ff5e61271092edb210695cd6e876a7cc8c`, and
checks the executable-reported version before generation. The signed release
commit is `f4d1cb8c15e1bc0476c75bcbc3febf1edec89b25`.

The same workflow pins `astral-sh/setup-uv` v9.0.0 to full commit
`c771a70e6277c0a99b617c7a806ffedaca235ff9`, fixes uv to 0.12.1 and Python to
3.12, and disables the action cache. The generated Python dependency graph is
resolved with a fixed upload-date cutoff and locked before execution.

Every generated client runs from a temporary copy. NuGet, Go, Maven, Gradle,
uv, Cargo, and pnpm dependency records are retained separately by SHA-256, and
the pristine source trees must still match their independent replay after all
seven runs. The authoritative strict termination union, reviewed RFC 7396
code-generation overlay, and resulting projected document are retained
together. The projector verifies their exact shapes, and structural evidence
records the overlay and projection SHA-256 values. C# and Rust consume the
projected closed object with a four-value `kind` enum, avoiding generator
v7.22.0 discriminator defects while preserving the runtime JSON fields.

Java resolves its declared graph, explicitly prefetches
`org.apache.maven.surefire:surefire-junit-platform:2.22.2` with strict checksum
handling, records the provider JAR SHA-256, then executes offline. Rust executes
from its fetched locked graph. Kotlin resolves every resolvable
generated-project configuration while writing its strict lock before offline
build and execution; TypeScript uses frozen pnpm state.

The generated Kotlin wrapper selects Gradle 8.14.3. CI recognizes the embedded
wrapper JAR as the official Gradle 8.9 binary with SHA-256
`498495120a03b9a6ab5d155f5de3c8f0d986a449153702fb80fc80e134484f17`
and adds the official 8.14.3 complete-distribution SHA-256
`ed1a8d686605fd7c23bdf62c7fc7add1c5b23b2bbc3721e661934ef4a4911d7c`
to the temporary wrapper properties before execution. The values come from
Gradle's [release checksum reference](https://gradle.org/release-checksums/).

Before TypeScript installation, CI queries npm metadata for exact pnpm
10.15.1, TypeScript 5.9.3, and `@types/node` 22.18.3 packages. It requires the
expected name, version, license, source repository, executable entrypoints,
and registry `sha512` integrity, then creates separate frozen package-build and
consumer pnpm locks. The metadata and GitHub runner toolchain versions are
retained with the matrix.

The checked-in `openapi/ferrobox-sdk-packages.json` fixes seven package
identities and version `0.1.0`. Standard CI materializes a NuGet package, a
deterministic Go module proxy entry, Java and Kotlin Maven artifacts, a Python
wheel, a Cargo crate, and an npm tarball. Each artifact is consumed through its
native package boundary before the lifecycle test. The package validator parses
embedded metadata and compiled entrypoints, records size and SHA-256, and binds
the artifact to the consumer evidence SHA-256. All repositories and package
files live under the runner's temporary evidence root. This gate uses no
registry credential and performs no external publication.

Standard CI runs
[30766020434](https://github.com/nya-a-cat/ferrobox/actions/runs/30766020434)
and
[30766116507](https://github.com/nya-a-cat/ferrobox/actions/runs/30766116507)
verified these pins at commit `9cb5c54`. The generator ran twice per job with
byte-identical output, and the seven retained tree hashes matched across the
two independent jobs. Artifacts `8838973235` and `8839002111` include hidden
generator control files after a path-name review and omit Python runtime cache
material.

[Standard CI run 30777303301](https://github.com/nya-a-cat/ferrobox/actions/runs/30777303301)
verified the completed seven-language runtime chain at commit `86bb3cc`. Its
structural record binds overlay SHA-256
`82ea593b150854333d031e2bdb23bb613cbd97ee275e1cfdc2e42261aa00a733`
to projected-document SHA-256
`eed1203f24517729bb693bc66e6a9f80efaf072cc7d74838e2bd48f5ca6ead3b`.
The explicitly fetched Surefire JUnit Platform provider JAR has SHA-256
`c33490024cd816e0c2c27331a68ba82e4c023d5255bdfa4ac71ba5998e13079d`;
its retained coordinate-and-digest record has SHA-256
`1aa411e4adcf11e5a5efe3cc67c165d44f4bb6c31efe0846d5b26c48e9a76a1c`.
All seven dependency record sets passed the aggregate check, and the pristine
generated roots remained byte-identical after execution. Artifact `8842515354`
has archive digest
`sha256:78a7ce80c5f3e3d6a177d81744e0b09580119d76569ac8e6c6435d556ca3331f`
and expires on 2026-11-01.

[Standard CI run 30779954220](https://github.com/nya-a-cat/ferrobox/actions/runs/30779954220)
verified the package chain at commit `8bf9a62`. The package contract has SHA-256
`8444a4e5f0d6209e7d95f6eea73419c78f26a182f5bc5df2f7c15a889c347cea`,
and the seven-entry package evidence manifest has SHA-256
`c0b873f372b4e0906c4951eb01f0500d827576e16f9501ff795d43c3d1034236`.
The aggregate gate accepted seven parsed package identities, seven consumer
smokes, complete build and consumer dependency records, and seven linked
lifecycle records. Artifact `8843358862` has archive digest
`sha256:202af788f8ad5a41f9276ea53c9fb6ca95183f0cb0bf137ad524bb07f41f44a2`
and expires on 2026-11-01.

## E2E provenance schema v1

`scripts/generate-e2e-provenance.sh` emits
`ferrobox-e2e-provenance.intoto.json` with the in-toto Statement v1 envelope
and predicate type
`https://github.com/nya-a-cat/ferrobox/e2e-provenance/v1`.

Its subjects are the node, API, static guest, microVM probe, and generated
rootfs. The predicate contains:

- repository commit, workflow identity and hash, `Cargo.lock` hash, and rootfs
  recipe hash;
- GitHub run identity and runner environment;
- every project, VMM, gVisor sidecar, Kata, Docker/containerd, and SBOM binary
  executed inside the declared E2E boundary;
- the Ferrobox and Kata guest assets selected by their runtime configuration;
- the digest-closed Python workload image;
- upstream archive URLs and pinned digests;
- all three SBOM identities, package counts, namespaces, sizes, and hashes.

The generator rejects duplicate executable names, malformed digests, missing
required subjects, missing Kata or gVisor runtime assets, an image digest
mismatch, and an empty SBOM. It then re-hashes every local file named by the
statement and compares it with the serialized digest.

GitHub runner base-image utilities and build-only operating-system packages sit
outside this evidence boundary. Their runner image identity and Linux kernel
release remain recorded as environment fields.

## Signing boundary

The current in-toto statement is unsigned evidence retained by the GitHub
artifact service. Release-level keyless signing needs `id-token`,
`attestations`, and current GitHub artifact-metadata write permissions plus a
consumer verification policy. That permission change stays pending explicit
approval. The parity matrix tracks release integrity separately from inventory
coverage.

## Update policy

Before a parity release or pinned-input update:

1. inspect the official upstream release notes and asset list;
2. obtain the asset digest from the official release channel and record it in
   source control;
3. update the immutable URL, version check, and documentation together;
4. run standard CI, snapshot KVM, and full comparison KVM on GitHub;
5. inspect the retained provenance and SBOM schemas, subjects, package counts,
   and digest verification result;
6. preserve the prior pin in Git history as the rollback point.

GitHub KVM run
[30762230170](https://github.com/nya-a-cat/ferrobox/actions/runs/30762230170)
verified schema v1 at commit `6fb848c`. Its `kvm-evidence` artifact
`8838016271` contains the in-toto statement plus all three SPDX documents. The
statement records five subjects, nineteen executed files, six guest assets, six
upstream inputs, and the fixed Python image digest. The source, rootfs, and
image SBOMs contain 277, 263, and 141 packages respectively. The supply-chain
step passed its schema and complete local re-hash loop.

The run's final aggregate remained red because Kata cleanup reached its
12-minute outer deadline after the final Python batch, which left its benchmark
JSON absent, and the independent HTTP file-API gate failed. Standard CI run
[30762230183](https://github.com/nya-a-cat/ferrobox/actions/runs/30762230183)
and Live Snapshot KVM run
[30762230199](https://github.com/nya-a-cat/ferrobox/actions/runs/30762230199)
passed for the same commit. Supply-chain inventory is verified; the
release-integrity row remains partial pending the permission-reviewed signing
phase.
