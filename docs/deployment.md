# Linux/KVM deployment

## Host gate

Ferrobox production mode requires an x86_64 Linux host with:

- read/write access to `/dev/kvm`;
- cgroup v2 with `cpu`, `memory`, `io`, and `pids` controllers;
- `/dev/net/tun`, `ip`, `nft`, and `iptables` with the comment and conntrack
  extensions;
- Firecracker and Jailer `1.16.1` from the pinned archive;
- a root-owned guest kernel and Python rootfs manifest;
- a dedicated unprivileged Firecracker UID and GID.

Run `ferrobox-node check` before creating a sandbox. The check reports each
missing prerequisite and never changes the host.

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

/var/lib/ferrobox/
  jailer/
  runtime/
  audit/
```

All `/opt/ferrobox` inputs and their parent directories must be root-owned and
unwritable by the runtime UID. Each writable rootfs is copied into its own jail.

## Jailer

Production mode always invokes Jailer. The Jailer creates the mount/PID
isolation, joins the per-sandbox network namespace, applies cgroup limits,
switches to the dedicated UID/GID, and starts Firecracker with its default
seccomp filter.

Jailer places each v2 cgroup at `/sys/fs/cgroup/ferrobox/<jailer-id>`.
Ferrobox retains that physical Jailer ID independently from the public sandbox
ID so an in-place rollback can retire the old VM exactly. VM termination waits
for the child process, removes the exact leaf with `rmdir` semantics, and never
recursively deletes the shared cgroup tree. Launch-failure cleanup follows the
same bounded path. This implements the operator cleanup responsibility defined
by the [Firecracker Jailer contract](https://github.com/firecracker-microvm/firecracker/blob/main/docs/jailer.md).

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
