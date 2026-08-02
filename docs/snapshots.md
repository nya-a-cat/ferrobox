# Snapshot, restore, clone, and rollback contract

This document defines Ferrobox state branching. It combines CubeSandbox's
running-state semantics with Microsandbox's named, independently managed,
integrity-verifiable snapshot artifacts. The comparison baseline is pinned in
[`sota-roadmap.md`](sota-roadmap.md).

## Consistency point

A full snapshot contains Firecracker memory and device state plus the writable
rootfs from one VM pause point. A request against a running sandbox:

1. locks that sandbox's lifecycle record;
2. pauses the VM;
3. asks Firecracker for a full state and memory snapshot;
4. creates copy-on-write artifact copies of state, memory, and rootfs;
5. resumes the source VM;
6. hashes and atomically publishes the artifact directory.

A source that was paused remains paused. A failure before publication removes
the partial directory and restores a formerly running source. Operations that
already entered the guest may be represented in the captured memory state;
new operations wait on the lifecycle record until capture releases it.

## Artifact schema 1

Each snapshot lives below `<runtime-root>/snapshots/<snapshot-id>/`:

```text
manifest.json
vmstate
memory
rootfs.ext4
restore-token
```

`manifest.json` records schema version, snapshot and source IDs, optional name,
creation time, source state, sandbox specification, node and architecture,
Firecracker version, kernel SHA-256, and the size and SHA-256 of every runtime
artifact. `restore-token` contains the captured guest credential needed for an
authenticated identity rotation after restore. It is mode `0400`, is represented
in the manifest only by its SHA-256, and is excluded from user-facing responses.

Publication writes a sibling partial directory, syncs the files, and renames it
to the final snapshot ID. Listing ignores partial directories. Restore verifies
schema, compatibility, sizes, hashes, and credential hash before launching a
VM. Hash integrity detects corruption; artifact authenticity requires a future
node signing key.

## Identity and independence

The API issues a distinct snapshot bearer token and stores only its
digest. A snapshot token authorizes get, verify, restore, clone, and delete for
one snapshot. Creating and listing snapshots uses the source sandbox token.
Deleting the source does not delete its snapshots.

A restored VM first authenticates with the credential captured in memory, then
rotates to a new sandbox ID and random guest credential over vsock. Each clone
therefore has an independent outer token, guest token, jail, rootfs, cgroup,
vsock endpoint, and optional network lease.

API records and token digests are process-local in the current single-node
control plane. Runtime artifacts survive a daemon restart; token recovery and
cross-node restore require the later durable metadata service.

## HTTP surface

- `POST /v1/sandboxes/{id}/snapshots` creates a named full snapshot.
- `GET /v1/sandboxes/{id}/snapshots?limit=&cursor=` lists snapshots in stable
  creation order.
- `GET /v1/snapshots/{snapshot_id}` returns metadata.
- `POST /v1/snapshots/{snapshot_id}/verify` rehashes every artifact.
- `POST /v1/snapshots/{snapshot_id}/restore` creates one independent sandbox.
- `POST /v1/snapshots/{snapshot_id}/clones` creates 1-32 independent sandboxes.
- `DELETE /v1/snapshots/{snapshot_id}` deletes an unused snapshot.
- `POST /v1/sandboxes/{id}/rollback/{snapshot_id}` restores the source ID in
  place when the snapshot belongs to that sandbox.

Restore and clone requests set a fresh TTL. A batch clone returns tokens only
after every VM is ready; a partial failure deletes every VM created by that
request. Snapshot deletion conflicts while a restore, clone, or rollback holds
its runtime lease.

## Rollback safety

Rollback verifies the artifact and launches a replacement VM before changing
the runtime map. When the replacement is healthy, one write-locked swap keeps
the public sandbox ID, API token, and expiry record while replacing its guest
token and physical VM. The old VM is then terminated and cleaned. Launch or
verification failure leaves the original VM and API record unchanged.

## GitHub acceptance

The hosted KVM workflow must prove:

- memory and file values captured from a running Python workload reappear after
  restore;
- a process active at the pause point continues in the restored VM and produces
  an expected file side effect;
- the source continues independently and diverges after capture;
- deleting the source does not invalidate snapshot verify or restore;
- two clones receive distinct IDs and tokens and diverge independently;
- rollback preserves the public ID and token while restoring captured state;
- one-byte corruption makes verification and restore fail closed;
- a forced partial clone failure leaves no VM, jail, cgroup, network, or API
  record from that batch;
- running-source and paused-source capture both preserve their required source
  state;
- all partial directories and snapshot leases are absent after cleanup.

These checks run in `.github/workflows/snapshots.yml`; the runtime, guest,
artifact corruption, and resource observations all occur on the GitHub runner.
