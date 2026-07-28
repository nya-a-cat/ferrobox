# Ferrobox

Ferrobox is a single-node Rust runtime for running AI-agent workloads inside
one Firecracker microVM per sandbox. It exposes an E2B-style HTTP API for
sandbox lifecycle, argv-based command execution, and confined file access.

The first release targets Linux hosts with KVM. A process backend exists for
API development and tests; it does not provide a security boundary.

## MVP acceptance path

```text
POST /v1/sandboxes
  -> Firecracker starts a microVM
  -> the guest agent reports ready over virtio-vsock
  -> python3 -c "print(42)" returns 42
  -> file upload/download round-trips
  -> DELETE removes the VM and its ephemeral resources
```

Implementation and verified commands are documented under `docs/`. Until the
KVM end-to-end transcript passes, the project must be treated as incomplete.

