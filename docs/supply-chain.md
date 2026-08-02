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

Every external action in `ci.yml`, `kvm.yml`, and `snapshots.yml` uses a full
40-character commit SHA. A human-readable release or channel comment remains
beside each pin. The current set is:

| Action | Revision |
| --- | --- |
| `actions/checkout` | `3d3c42e5aac5ba805825da76410c181273ba90b1` (`v7.0.1`) |
| `actions/upload-artifact` | `043fb46d1a93c77aae656e7c1c64a875d1fc6a0a` (`v7.0.1`) |
| `actions/cache` restore/save | `55cc8345863c7cc4c66a329aec7e433d2d1c52a9` (`v6.1.0`) |
| `dtolnay/rust-toolchain` | `4cda84d5c5c54efe2404f9d843567869ab1699d4` (`stable`, resolved 2026-08-03) |
| `Swatinem/rust-cache` | `e18b497796c12c097a38f9edb9d0641fb99eee32` (`v2`) |

Updates require an upstream release review, a new full SHA, and successful
standard, snapshot, and KVM workflows. `scripts/check-action-pins.sh` rejects
any external workflow reference that is not a full lowercase commit SHA.

## Firecracker

Ferrobox pins Firecracker and Jailer to the same upstream release:

| Item | Value |
| --- | --- |
| Version | `1.16.1` |
| Release date | 2026-07-02 |
| Upstream | `firecracker-microvm/firecracker` |
| License | Apache-2.0 |
| x86_64 archive | `firecracker-v1.16.1-x86_64.tgz` |
| SHA-256 | `382a02a869e4d6d5cb14c40577f9545e8458021ea8b0b2d3fc10ec14d9c242e6` |

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

## SBOM generator

`scripts/fetch-syft.sh` installs the official Linux x86_64 Syft `v1.50.0`
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

The first hosted run for this revision is the verification gate for the new
schema. The capability remains partial until that artifact passes.
