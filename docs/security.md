# Security model

## Supported boundary

The Firecracker backend is the Ferrobox isolation boundary. It assigns one
microVM, Jailer process, writable rootfs, cgroup, vsock CID, token, and optional
network namespace to each sandbox. The process backend is a developer test
double and carries no isolation claim.

Ferrobox v0.1 assumes the Linux host, node daemon, API daemon, guest kernel,
rootfs template, Jailer, Firecracker, and guest service are trusted. The
submitted command, its descendants, uploaded files, and network traffic are
untrusted.

## Mandatory controls

- Jailer runs a matching statically linked Firecracker version with default
  seccomp enabled.
- The VM rootfs is never a host bind mount.
- Host cgroup v2 limits CPU, memory, I/O, and process count.
- Guest commands run as UID/GID 1000 with a guest-side process limit.
- The default network mode has no data network interface.
- Internet mode rejects loopback, link-local, RFC1918, carrier-grade NAT,
  multicast, metadata, host-management, control-plane, and sandbox ranges
  before allowing public egress.
- Internet mode exposes only a per-sandbox UDP/TCP DNS relay on the guest
  gateway; every other guest-to-host packet is rejected.
- Tokens are random, per-sandbox, short-lived, stored only as hashes by the API,
  and omitted from logs.
- Command timeout or sandbox TTL expiry terminates the entire microVM.
- Output and file transfer have byte limits.
- Every lifecycle and workload-control operation produces a structured audit
  event.

## File API

Guest file operations are rooted at `/home/sandbox`. Linux resolution uses
`openat2` with `RESOLVE_BENEATH`, `RESOLVE_NO_MAGICLINKS`, and
`RESOLVE_NO_XDEV`, followed by file-type checks. Requests reject:

- absolute guest paths and `..` components;
- symlinks and magic links;
- device nodes, FIFOs, and sockets;
- cross-mount traversal;
- uploads above the configured limit;
- recursive traversal without a fixed entry and depth budget.

Tests place sentinels outside the workspace and create malicious symlinks to
prove the rejection behavior.

## API exposure

The API binds to loopback by default. Remote deployment requires TLS
termination and operator authentication in front of Ferrobox. The v0.1 bearer
token authorizes only a single sandbox. Error responses avoid host paths and
token material.

## Explicit non-claims

The MVP has no multi-tenant organization authorization, encrypted persistent
volume, confidential-computing guarantee, FQDN allowlist or answer-pinning
policy, browser isolation, GPU isolation, or cross-node recovery guarantee.
Those capabilities require separate threat models and acceptance tests.

Snapshot artifacts contain guest memory and filesystem data and are therefore
sensitive host data. Their directories are root-owned, runtime files are
read-only after publication, and the captured restore credential is mode
`0400`. SHA-256 verification provides corruption detection. Encryption at rest,
artifact signing, tenant-scoped durable metadata, and cross-node key management
remain separate security gates.
