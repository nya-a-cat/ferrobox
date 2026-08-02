# OpenAPI and generated-client contract

`openapi/ferrobox-v1.json` is the checked-in contract for the implemented v1
HTTP surface. It describes the existing handlers without changing their route,
payload, state, or authorization semantics.

## Contract boundary

The document uses OpenAPI 3.1.2 with the JSON Schema 2020-12 dialect. Its
eighteen operations cover health, sandbox lifecycle, command execution,
confined file access, and snapshot state branching. Every operation has one
stable `operationId`, one success response, a structured default error, and an
explicit credential scope:

- two public bootstrap operations use no credential;
- eleven sandbox-owned operations use the source sandbox bearer token;
- five snapshot-owned operations use the distinct snapshot bearer token.

One-time response credentials are marked `readOnly`, `password`, and
`x-ferrobox-sensitive`. Generated examples, conformance evidence, and audit
records must omit their values.

This is the initial v1 description of already implemented behavior. It has no
client migration requirement. A later incompatible route or schema change
requires a new reviewed contract and migration note.

## GitHub conformance

Standard CI uses three independent checks:

1. OpenAPI Generator validates the document and emits C#, Go, Java, Kotlin,
   Python, Rust, and TypeScript Fetch source trees from the same input.
2. `scripts/check-openapi.py` compares the specification with the exact Axum
   route/method set, resolves every local reference, checks operation IDs,
   request bodies, path parameters, error responses, credential scopes, and
   sensitive fields, then hashes each generated source tree.
3. The generated Python client runs through `uv` against the loopback-only
   process backend and completes create, inspect, typed command execution,
   lossless output, file roundtrip, delete, stale-handle rejection, and audit
   credential-redaction checks.

The Python dependency graph is locked on the GitHub runner with an
`exclude-newer` cutoff and retained with the generated source. The process
backend supplies deterministic API behavior and carries no workload-isolation
claim.

## Pinned tooling

The workflow downloads the official OpenAPI Generator v7.22.0 release JAR only
from its GitHub release, then checks its 31,390,141-byte size and SHA-256
`37f23217f40cabac50c435312ea1d3ff5e61271092edb210695cd6e876a7cc8c` before
execution. The release tag resolves to verified commit
`f4d1cb8c15e1bc0476c75bcbc3febf1edec89b25`.

Python setup uses `astral-sh/setup-uv` v9.0.0 at full commit
`c771a70e6277c0a99b617c7a806ffedaca235ff9` and fixes uv to 0.12.1 with Python
3.12. All validation, generation, locking, installation, and execution occur on
GitHub-hosted runners.

## Current parity boundary

The generated trees are retained conformance outputs. Stable packaged SDKs and
the full language execution matrix remain open work. Diagnostics, authenticated
ingress, and richer egress-policy endpoints also remain outside the current
HTTP surface, so the broader OpenAPI and SDK roadmap rows remain Partial.
