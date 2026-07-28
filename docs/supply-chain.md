# Supply-chain record

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
provided. Production deployment must copy the verified executables into a
root-owned directory that cannot be modified by the Firecracker runtime UID.

## Kernel and root filesystem

Ferrobox does not silently fetch a floating kernel or root filesystem. A
deployable template manifest records:

- source URL or build recipe revision;
- SHA-256 for the uncompressed kernel and ext4 root filesystem;
- architecture and guest kernel configuration;
- installed distribution package versions;
- SHA-256 of the injected `ferrobox-guest`;
- expected sandbox UID/GID and service definition.

The template preparation gate rejects assets without this manifest. This keeps
guest provenance separate from the Firecracker release archive.
