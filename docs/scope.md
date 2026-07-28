# Scope and acceptance

## MVP

Ferrobox v0.1 is a single-node Linux/KVM runtime. Its completion gate is a
real Firecracker microVM that becomes ready through the guest service, executes
`python3 -c "print(42)"`, round-trips a file, and is fully removed.

The MVP includes:

- one Firecracker microVM and one short-lived token per sandbox;
- a common runtime contract with process and Firecracker implementations;
- argv-based execution with cwd, environment, timeout, output cap, exit code,
  and streamed stdout/stderr events;
- confined read, write, and directory-listing operations below
  `/home/sandbox`;
- create, execute, file read/write, pause/resume, and delete lifecycle states;
- a default-disabled network and a restricted public-egress mode;
- cgroup, Jailer, TTL cleanup, and structured audit records.

The process backend exercises API behavior on development hosts. It provides no
workload isolation and cannot satisfy the KVM completion gate.

## Later phases

Template snapshots and restore, public port routing, an egress proxy, domain
policy, PostgreSQL, Redis, multi-node placement, object storage, and failure
recovery follow the MVP. Multi-region operation, billing, Kubernetes, GPU,
browser desktops, shared persistent volumes, registries, and organization
management are outside v0.1.

## Evidence required before declaring v0.1 complete

1. Formatting, linting, unit tests, locked workspace build, and a static-musl
   guest build pass.
2. Firecracker and Jailer version and checksum provenance are recorded.
3. KVM end-to-end output proves VM readiness, command behavior, UID 1000,
   file confinement, network policy, independent concurrent VMs, TTL expiry,
   and cleanup.
4. The audit log contains lifecycle and workload events without bearer tokens.
