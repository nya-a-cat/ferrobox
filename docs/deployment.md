# Linux/KVM deployment

## Host gate

Ferrobox production mode requires an x86_64 Linux host with:

- read/write access to `/dev/kvm`;
- cgroup v2 with `cpu`, `memory`, `io`, and `pids` controllers;
- `/dev/net/tun`, `ip`, `nft`, and `iptables` with the comment and conntrack
  extensions;
- Firecracker and Jailer `1.15.1` from the pinned archive;
- a root-owned guest kernel and Python rootfs manifest;
- a dedicated unprivileged Firecracker UID and GID.

Run `ferrobox-node check` before creating a sandbox. The check reports each
missing prerequisite and never changes the host.

## Architecture portability boundary

The GitHub host-architecture matrix validates the Rust workspace, native static
guest, Process/API lifecycle, and CLI lifecycle on Linux x86_64 and Linux
aarch64. Its aarch64 leg exercises the explicitly unsafe process backend and
records `/dev/kvm` as an observational runner capability. The production
Firecracker deployment contract above currently applies to Linux x86_64.
Aarch64 Firecracker assets, guest images, Jailer behavior, and KVM lifecycle
evidence remain an open production gate. GitHub architecture run `30781691279`
verified the native aarch64 build and process lifecycle and reported no
`/dev/kvm` device on its `ubuntu-24.04-arm` runner.

The `macos-15` M1 gate builds the host-side API and CLI and exercises their
shared process-backend lifecycle. That backend retains its explicit unsafe
development classification with isolation `none`. A production macOS microVM
backend remains open.

The `windows-11-arm` gate provides the same host-side API/CLI and unsafe
process-backend coverage on native Windows ARM64. It is a portability gate;
Windows production isolation and WSL2 runtime support remain open. GitHub run
`30784854537` verified this leg inside the four-platform v3 evidence contract.

## Storage placement

Keep source code wherever the developer prefers. Place Cargo build output,
microVM rootfs copies, sockets, and runtime state on a native Linux filesystem.
DrvFS/9p paths under `/mnt/c` have incompatible ownership semantics for Jailer
and materially slower random I/O.

Suggested development layout:

```text
/opt/ferrobox/
  bin/firecracker
  bin/jailer
  images/vmlinux
  images/python.ext4
  images/python.manifest.json
  templates/records/

/var/lib/ferrobox/
  jailer/
  runtime/
  audit/
```

All `/opt/ferrobox` inputs and their parent directories must be root-owned and
unwritable by the runtime UID. Each writable rootfs is copied into its own jail.

Point `FERROBOX_TEMPLATE_STORE` or the standalone node's `--template-store` at
the operator-owned template directory. The HTTP API accepts
`--template-store /absolute/path` when it constructs the Firecracker backend.
Relative paths are rejected. Each catalog record contains absolute
kernel/rootfs locators, so moving an input requires deleting and rebuilding its
catalog record. Equal descriptor and artifact bytes reproduce the same template
ID after that cycle. The current catalog uses a single-writer operating
contract for each store directory.

The configured `--kernel` and `--rootfs` remain required. With a catalog
configured, they serve the reserved legacy `python` template and anchor
snapshot-kernel compatibility; every other template reference must resolve to
an immutable catalog alias or ID. Without a catalog, the existing configured
rootfs path continues to serve legacy template names. Every catalog record must
name the configured kernel digest; its rootfs may differ. Keep the store,
records, kernel, and rootfs inputs root-owned and unwritable by the runtime UID.
Direct alias/ID creation reads and hashes the complete kernel and rootfs before
each launch, so operators should budget that I/O until the authenticated
constant-time measurement gate is implemented.

`scripts/build-oci-rootfs.sh` creates a deterministic ext4 identity from the
verified OCI and injected guest inputs. It uses the OCI config `created` value
as its source-date epoch. Images without `created` use the recorded epoch
`946684800` (2000-01-01 UTC). Operators that require another fixed epoch may set
`FERROBOX_OCI_SOURCE_DATE_EPOCH` to a positive decimal integer; that value is
part of the derived filesystem identity and the retained evidence.

The hosted builder supplies the verified e2fsprogs 1.47.4 `mke2fs`, `e2fsck`,
`dumpe2fs`, configuration, and source manifest through the corresponding
`FERROBOX_*` build variables. It materializes the injected tree as a GNU tar
with name ordering, numeric ownership, and a fixed modification time before
passing that tar to `mke2fs -d`. The archive root is explicitly `root:root`
mode `0755`, allowing UID 1000 workloads to traverse the filesystem. Operators
reproducing the OCI builder should run `scripts/build-e2fsprogs.sh` and supply
the same five paths; schema-3 build evidence records their hashes.

## Jailer

Production mode always invokes Jailer. The Jailer creates the mount/PID
isolation, joins the per-sandbox network namespace, applies cgroup limits,
switches to the dedicated UID/GID, and starts Firecracker with its default
seccomp filter.

Jailer places each v2 cgroup at `/sys/fs/cgroup/ferrobox/<jailer-id>`.
Ferrobox retains that physical Jailer ID independently from the public sandbox
ID so an in-place rollback can retire the old VM exactly. VM termination waits
for the child process, drains remaining members through the leaf's cgroup v2
`cgroup.kill` control, removes the exact leaf with `rmdir` semantics, and never
recursively deletes the shared cgroup tree. Launch-failure cleanup follows the
same bounded path and retains `cgroup.events` plus `cgroup.procs` diagnostics
on failure. The abnormal-exit path allows up to five seconds for PID-namespace
children to leave the cgroup after `cgroup.kill`. This implements the operator
cleanup responsibility defined by the [Firecracker Jailer contract](https://github.com/firecracker-microvm/firecracker/blob/main/docs/jailer.md).

Startup treats the API socket inode as an intermediate state. Ferrobox retries
the Firecracker version request until the configured API deadline, then begins
machine configuration or snapshot loading only after the API accepts requests.
Ordinary Firecracker control calls retain a five-second fail-fast deadline. Full
snapshot creation has a dedicated five-minute deadline because Firecracker
writes the configured guest memory to storage before replying.
Each VM retains one Hyper Unix-socket client for its complete lifecycle so the
configuration sequence uses one bounded connection pool.

Direct Firecracker execution is available only behind an explicit unsafe
development option. The API refuses to expose that mode on a non-loopback
listener.

## Network cleanup

Ferrobox prefixes every ephemeral netns, TAP, and nftables object with a
validated sandbox identifier. Creation and deletion are idempotent. Startup
reconciliation removes only resources whose ownership marker matches the local
runtime record; existing Docker, WSL, and distribution firewall tables remain
outside scope.

## WSL2 development note

WSL2 can provide nested KVM and is useful for the full local acceptance test.
Run privileged node operations through the WSL distribution. The HTTP API and
CLI remain ordinary-user processes; the narrow node service owns privileged
Jailer and network operations.
