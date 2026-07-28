# Architecture

```text
Rust SDK / CLI / external agent
             |
             | HTTP + bearer token
             v
       ferrobox-api
       state + TTL + audit
             |
             | SandboxRuntime
             v
       ferrobox-node
  Jailer + Firecracker + cgroup
       |             |
       | UDS HTTP    | virtio-vsock
       v             v
 Firecracker API   ferrobox-guest
                   process + files
```

## Trust boundary

`ferrobox-api` and `ferrobox-node` are trusted control-plane components.
`ferrobox-guest` is trusted code inside each microVM and executes untrusted
workloads as UID 1000. A workload has no host directory mount. Its control
channel is a Firecracker virtio-vsock device; its optional data network uses an
isolated TAP device in a per-sandbox network namespace.

The process runtime runs workloads directly on the development host. Starting
the API with that backend requires an explicit development flag.

## Lifecycle

```text
Creating -> Running -> Pausing -> Paused -> Resuming -> Running
    |          |                                    |
    +----------+---------------+--------------------+
                               v
                           Deleting -> Deleted

Any live state may move to Failed when creation or runtime control fails.
Failed may move to Deleting for cleanup.
```

The API creates a random bearer token, stores only its SHA-256 digest, and
returns the plaintext once. State becomes `Running` only after the guest
responds to health and idempotent initialization. A background reaper deletes
the full microVM when its TTL expires.

## Runtime interface

HTTP handlers depend on `SandboxRuntime`. The interface covers create,
execution, signal delivery, file read/write/list, pause, resume, and delete.
`ProcessRuntime` supports deterministic API and state-machine tests.
`FirecrackerRuntime` owns the security boundary and production lifecycle.

## Guest protocol

The host opens the Firecracker vsock UDS, writes `CONNECT <port>\n`, verifies
the `OK <host-port>\n` acknowledgement, and then runs the gRPC/HTTP2 guest
protocol over that byte stream. Each new RPC channel repeats the UDS handshake,
which permits reconnection after a VM restore.

The guest exposes health, idempotent initialization, server-streamed process
events, signal delivery, file write, server-streamed file read, and directory
listing.

## Network modes

`Disabled` creates no guest data interface. `Internet` creates a dedicated
network namespace and TAP interface, then applies project-scoped nftables
rules. Public egress is permitted after private, link-local metadata,
control-plane, host-management, and other-sandbox ranges are rejected. Cleanup
is keyed by sandbox ID and is idempotent.

Host networking changes are ephemeral. The implementation never edits a
persistent distribution firewall configuration.
