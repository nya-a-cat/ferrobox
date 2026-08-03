# Immutable template catalog

Ferrobox currently provides an immutable host-side catalog for already-built
Firecracker kernel and ext4 rootfs inputs. The capability status is **Partial**:
catalog lifecycle and integrity are verified, while OCI construction and
runtime resolution remain open gates.

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

## Security and provenance boundary

- Source references contain credential-free registry or artifact identities.
- Private-registry credentials and pull mechanics stay in the future OCI build
  service contract.
- The store and referenced artifacts are trusted host assets owned by the
  operator and unwritable by the sandbox runtime UID.
- External artifact changes become visible through `inspect` as digest and size
  mismatches.
- Runtime creation still uses its configured kernel/rootfs and the existing
  `python`/`oci-python` selection. Catalog resolution requires an approved
  runtime/API contract change.
- Active-use checks and node-replica cleanup enter with runtime resolution and
  multi-node distribution.

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
