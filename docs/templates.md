# Immutable template catalog

Ferrobox provides an immutable host-side catalog for Firecracker kernel and
ext4 rootfs inputs. The hosted public-OCI workflow now constructs a
byte-reproducible rootfs, registers the resulting kernel/rootfs pair, and
verifies that the catalog record names the exact files used by the KVM
lifecycle. The capability status is **Partial**: catalog lifecycle, integrity,
public-OCI construction, exact runtime-artifact binding, and immutable-ID
selection for direct Firecracker creates are verified. Hosted Btrfs source-asset
fs-verity and real-KVM compatibility are also verified. Asynchronous build
status, template-specific ready pools, other runtime providers, private-registry
credential custody, distributed artifact delivery, and trusted fs-verity digest
binding in the runtime identity contract remain open gates.

## Upstream semantics carried forward

The contract follows the fixed comparison baseline in
[SOTA parity program](sota-roadmap.md):

- [Microsandbox image commands](https://github.com/superradcompany/microsandbox/blob/001af7b51e4e2e208985c0f0e317390c80587eb7/docs/cli/image-commands.mdx)
  expose pull/load/save/list/inspect/remove and retain shared content-addressed
  layers. Its manifest rows retain digest, platform, layer count, and size.
- [CubeSandbox template creation](https://github.com/TencentCloud/CubeSandbox/blob/6b01f08e0a233570fcdd42baed2718ba08318759/docs/guide/tutorials/template-from-image.md)
  separates OCI pull/rootfs build, guest boot/snapshot, and registration. It
  exposes asynchronous status, artifact SHA-256, deterministic specification
  fingerprints, unique creation-time aliases, list/info/render, and deletion.
- [OpenSandbox lifecycle specification](https://github.com/opensandbox-group/OpenSandbox/blob/e95681e791b33b3893033940cbeaa5ab192bf21b/specs/sandbox-lifecycle.yml)
  accepts image or snapshot startup sources with explicit platform constraints;
  its Kubernetes pools carry full workload templates.

The first Ferrobox slice concentrates these semantics into one small catalog
component: stable identity, version, source provenance, platform, artifact
integrity, inspection, and lifecycle.

## Identity contract

`ferrobox-template-v1` serializes one fixed-order descriptor containing:

- human name and version;
- source kind (`oci`, `file`, or `snapshot`), reference, and `sha256:` digest;
- Linux target architecture (`x86_64` or `aarch64`) and `firecracker` runtime;
- kernel media type, SHA-256, and size;
- rootfs media type, SHA-256, and size.

SHA-256 over the compact descriptor JSON produces `spec_digest`. The public
ID is `tpl-` plus its first 60 hexadecimal characters, retaining 240 bits while
fitting the existing 64-character sandbox template field. The full digest is
stored and recalculated whenever the record is loaded.

Absolute kernel/rootfs paths live in a separate `locations` object. Directory
changes therefore leave identity unchanged when descriptor fields and file
bytes match. Alias is also outside the digest and receives a single immutable
binding inside one catalog. Re-registering the same descriptor and alias is
idempotent; alias reassignment and a second alias for an existing identity are
rejected.

## Storage and lifecycle

Each alias owns one JSON record:

```text
<store>/
  records/
    python-3-12.json
```

The writer uses a same-directory temporary file plus no-clobber persistence.
Catalog readers validate schema, status, contract, filename/alias agreement,
full descriptor digest, and derived template ID before returning a record.
Each store has a single-writer operating contract; cross-process transaction
coordination remains part of the future multi-node catalog gate.

`inspect` streams both artifacts through SHA-256 and compares digest plus byte
size. Its JSON result keeps descriptor validity separate from kernel/rootfs
validity for precise diagnostics. `delete` removes the record and returns
`artifacts_preserved: true`; operators retain ownership of kernel/rootfs files.

## CLI

Set the catalog location once:

```bash
export FERROBOX_TEMPLATE_STORE=/var/lib/ferrobox/templates
```

Register verified local inputs:

```bash
ferrobox template build \
  --name python \
  --version 3.12.0 \
  --alias python-3-12 \
  --source-kind oci \
  --source-reference docker.io/library/python:3.12-slim \
  --source-digest sha256:<oci-manifest-digest> \
  --target-arch x86_64 \
  --kernel /opt/ferrobox/images/vmlinux \
  --rootfs /opt/ferrobox/images/python.ext4
```

List, inspect by alias or ID, and delete:

```bash
ferrobox template list
ferrobox template inspect python-3-12
ferrobox template inspect tpl-<60-hex-characters>
ferrobox template delete python-3-12
```

Every command emits machine-readable JSON. `build` performs a complete artifact
verification before returning `ready`.

## Runtime selection

Configure the Firecracker API with the absolute catalog path:

```bash
ferrobox-api --backend firecracker \
  --template-store /var/lib/ferrobox/templates \
  <other-firecracker-options>
```

The standalone node accepts the same `--template-store` option or
`FERROBOX_TEMPLATE_STORE`. A create request selects the immutable record through
the existing field:

```json
{
  "template": "tpl-2a4a8bfe7412552c0ec6dcaf7cc2dc258dfccacef05c162149bc80827071",
  "cpu_count": 1,
  "memory_mb": 512,
  "timeout_seconds": 300,
  "network": {"internet_access": false}
}
```

For a `tpl-...` request, the node loads the exact content-derived ID, validates
the descriptor and host architecture, requires the recorded kernel digest to
equal the configured Firecracker/snapshot kernel digest, then streams the
kernel and rootfs through SHA-256 before cloning them into the jail. Catalog
aliases are reserved for build/list/inspect/delete administration. Existing
legacy template names continue to select the configured kernel/rootfs.

Immutable-ID creates currently take the direct cold-boot path. The legacy
`python` snapshot template and ready pool cannot satisfy a catalog-ID request.
Unknown IDs fail with `404 not_found`; incompatible platform or kernel records
fail with `501 unsupported`; corrupt, unreadable, or tampered records fail with
`503 unavailable` using sanitized messages.

## Security and provenance boundary

- Source references contain credential-free registry or artifact identities.
- Private-registry credentials and pull mechanics stay in the future OCI build
  service contract.
- The store and referenced artifacts are trusted host assets owned by the
  operator and unwritable by the sandbox runtime UID.
- External artifact changes become visible through `inspect` as digest and size
  mismatches.
- Runtime selection accepts only the content-derived ID. Each direct create
  revalidates the descriptor and artifact bytes before Firecracker launch.
- Dynamic templates share the configured kernel digest so restored snapshots
  and direct-created guests stay inside one kernel compatibility contract.
- The hosted Btrfs gate enables fs-verity on the catalog kernel/rootfs, matches
  offline digests to repeated kernel measurements, rejects writes, and boots the
  same source paths through Firecracker. A reflink clone retains identical bytes
  and drops fs-verity metadata, matching the Linux copy contract.
- Full SHA-256 over a 1 GiB rootfs remains on the direct-create path. Runtime use
  of constant-time measurement requires a trusted expected fs-verity digest in
  the template identity contract and a defined unsupported-filesystem policy.
- Active-use deletion checks and node-replica cleanup enter with multi-node
  distribution.

## GitHub evidence

[Standard CI run 30786711526](https://github.com/nya-a-cat/ferrobox/actions/runs/30786711526)
passed formatting, workspace tests, Clippy, build, and the seven-check template
E2E at commit `5f1519a`. The retained aggregate uses contract
`ferrobox-template-catalog-evidence-v1` and binds template ID
`tpl-5f7f28630d7a32f54bd33965bbd37be172af5968a5353a47c3fca3068693`
to full descriptor digest
`sha256:5f7f28630d7a32f54bd33965bbd37be172af5968a5353a47c3fca30686931432`.

Artifact `8845497128` has archive digest
`sha256:46ba9e64288e7d51015e1825805610b5fa5c1eae2a0aff838e796583e97c87aa`
and expires on 2026-11-01. It retains build, list, valid inspection, deliberate
tamper inspection, alias-conflict stderr, deletion, same-identity rebuild, and
final deletion evidence.

[OCI KVM run 30787903798](https://github.com/nya-a-cat/ferrobox/actions/runs/30787903798)
then passed the real public-image binding at commit `639666c`. The workflow
materialized the digest-pinned Python image twice, produced byte-identical ext4
SHA-256
`3ed9c8fc9e746916bee5cf72681b30f0f61d70b142e039e016164dec4a2c8c14`,
and registered the same descriptor below two independent catalog locations.
Both records derived template ID
`tpl-2a4a8bfe7412552c0ec6dcaf7cc2dc258dfccacef05c162149bc80827071`
and full specification digest
`sha256:2a4a8bfe7412552c0ec6dcaf7cc2dc258dfccacef05c162149bc808270717abf`.

The retained runtime record points to `/mnt/ferrobox-oci/images/vmlinux` and
`/mnt/ferrobox-oci/images/oci-python.ext4`. The KVM gate rehashed both paths,
matched kernel SHA-256
`e20e46d0c36c55c0d1014eb20576171b3f3d922260d9f792017aeff53af3d4f2`
and the ext4 SHA-256 above, then passed all twelve lifecycle checks with Python
3.11.15 running as UID 1000. Artifact `8845975100` has archive digest
`sha256:681fe9dde6d9f2c34bc54dc906e277f57ad96149820347351f2695b5e760ed0c`
and expires on 2026-11-01.

[OCI KVM run 30789867561](https://github.com/nya-a-cat/ferrobox/actions/runs/30789867561)
passed the fifteen-check runtime-selection contract at commit `274a417`. The
HTTP request used the immutable ID above, while the configured fallback rootfs
was a deliberately invalid 45-byte file with SHA-256
`990278af04fe88cd43f527f0f16f3077fe509f9ae38c48d734591c7ceba42b2d`.
The resolver selected `/mnt/ferrobox-oci/images/vmlinux` and
`/mnt/ferrobox-oci/images/oci-python.ext4`, booted Python 3.11.15 as UID 1000,
and rejected an unknown all-zero template ID with `404 not_found`. Artifact
`8846706558` has archive digest
`sha256:419b7d98006bfa6307591c773ee2961e847f0d38a63ec04447cffb751bb3b353`
and expires on 2026-11-01.

[Standard CI run 30789867545](https://github.com/nya-a-cat/ferrobox/actions/runs/30789867545)
passed formatting, workspace tests, Clippy, builds, template E2E, OpenAPI, and
all seven generated SDK consumers for the same commit. Host architecture run
[30789867548](https://github.com/nya-a-cat/ferrobox/actions/runs/30789867548)
passed the Linux x86_64, Linux aarch64, macOS Apple Silicon, and Windows ARM64
matrix.

[OCI KVM run 30793770602](https://github.com/nya-a-cat/ferrobox/actions/runs/30793770602)
passed the sixteen-check source-integrity and runtime-selection contract at
commit `705f0ed`. The workflow built fsverity-utils 1.7 from a release archive
whose checksum is signed by kernel.org, matched its extracted Git tree to
`849ba951347671baf7691000e94dfcdffb36fe56`, and passed upstream `make check`
under the same fixed build flags as the installed binary. On GitHub Btrfs, the
kernel and 1 GiB rootfs produced fs-verity digests
`sha256:346294cd981e5f4bc7af2d4d68f47a3987ccd421af0d1e44d4717403275ce2fa`
and
`sha256:43a04b8f68916525a67b9595ad6e8dfb02a7421db1e0738f9f5bb23362bf1f14`.
Their 31-sample measurement P95 values were 1,022 and 1,046 microseconds. The
API remeasured both source paths immediately before launch and booted Python
3.11.15 through the exact immutable ID. Artifact `8848166249` has archive digest
`sha256:ca006cf19ad009a5574421d242f43562840e2cdf0b682466b873ca11880a6ba0`
and expires on 2026-11-01.
