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

Command termination keeps the existing tagged JSON objects and gives every
variant a named schema, a singleton `kind` enum, and an explicit discriminator
mapping. This is equivalent to the prior anonymous `oneOf`/`const` validation
while giving all seven generators stable model names. The optional `cwd`
property keeps its `/home/sandbox` default and repeats the `SandboxPath` string
constraints inline so Java receives a valid quoted string initializer. Neither
representation changes an accepted request or emitted response.

## GitHub conformance

Standard CI uses three independent checks:

1. OpenAPI Generator validates the document and emits C#, Go, Java, Kotlin,
   Python, Rust, and TypeScript Fetch source trees from the same input.
2. `scripts/check-openapi.py` compares the specification with the exact Axum
   route/method set, resolves every local reference, checks operation IDs,
   request bodies, path parameters, error responses, credential scopes, and
   sensitive fields, then hashes each generated source tree.
3. Generated C#, Go, Java, Kotlin, Python, Rust, and TypeScript clients share
   one loopback-only API process. Every client completes create, inspect, typed
   command execution, lossless output, file roundtrip, delete, stale-handle
   rejection, and audit credential-redaction checks.

Each CI run generates all seven client trees twice in separate runner-temporary
directories and requires a recursive byte comparison before and after runtime
conformance. The C# project GUID is fixed. Every runtime operates on a temporary
copy, so dependency resolution, harness sources, build products, and caches do
not mutate either generated tree. Hidden generator metadata is included in the
artifact after a path-name review.

The retained SDK matrix contains one sanitized schema-1 record per language,
the seven distinct UUIDv7 sandbox identities, the shared audit-log hash, and
the SHA-256 of each dependency manifest or lock. Python uses a fixed
`exclude-newer` cutoff. C# uses NuGet locked mode; Go uses its checked-in
`go.sum`; Java resolves online then tests offline; Kotlin resolves its complete
runtime classpath while writing a strict Gradle lock, then builds and runs
offline; Rust fetches a generated `Cargo.lock` then runs offline; TypeScript
uses an exact pnpm package graph and runs the compiled CommonJS output under
Node. The process backend supplies deterministic API behavior and carries no
workload-isolation claim.

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

## Verified evidence

[Standard CI run 30766020434](https://github.com/nya-a-cat/ferrobox/actions/runs/30766020434)
and the independent
[workflow-dispatch run 30766116507](https://github.com/nya-a-cat/ferrobox/actions/runs/30766116507)
passed at commit `9cb5c54`. In each run, the first and replay generations were
byte-identical. The retained structural records also match across runs:

| Generator | Files | Stable tree SHA-256 |
| --- | ---: | --- |
| C# | 72 | `dd84a44c5b1a080d958b2461ad7338d2e6b2da410c3dcf61e451d87cb7f98a53` |
| Go | 49 | `1d4832bacd07fc0ca00a5a278f3770a0073ff1692024cd159760185ceeb92907` |
| Java | 72 | `25bf0fee4c4bec4e89ea7be66d25008d2b5bf092c99e8a000f497ca3ad30a5ff` |
| Kotlin | 62 | `911b4df0f16d77fcfccf37b6c9d75cb1739e76473da18abdc1d90b5284fb95a4` |
| Python | 60 | `3910c5a4346e9d5083a019d397196ed147bdb16820b69e411313e3a81bd1df3f` |
| Rust | 47 | `826a032a51a52506b1e0e31a174e982a9d5ae1669c55d9aaff8cc62f6304df52` |
| TypeScript Fetch | 42 | `2931b95440e21b9710a27addf7c1eeb6812e029550550878c5d2ecdbc43d8ae6` |

Both records contain specification SHA-256
`1dd72ea740209b89b26168452556794b96c805e5f231a3bbe273259e8adf66d8`,
18 operations, 30 schemas, and credential-scope counts of 2 public, 11
sandbox-owned, and 5 snapshot-owned operations. Both Python runtime records
contain the same seven check names and no bearer credential value.

Artifacts `8838973235` and `8839002111` retain the complete trees. Their
archive digests are respectively
`sha256:1d20ad923601d834b139a928272b6f07040967eb112faab75f3598d4eb22112d`
and
`sha256:b0d610a231d5899bfe61487aa3ce1b4aafd9ef05d06bd7b6ea7816cc66087e23`.
The archives expire on 2026-10-31. Archive-level digests include per-run
runtime evidence and packaging metadata; the language-tree hashes above are
the reproducibility boundary.

The first expanded runtime attempt,
[run 30774266980](https://github.com/nya-a-cat/ferrobox/actions/runs/30774266980)
at commit `a38a4af`, passed the complete scenario in Go, Python, and TypeScript
and preserved both generated source trees byte-for-byte. It isolated four
generator/runtime-integration blockers: anonymous termination variants in C#
and Rust, an unquoted Java default attached through `allOf`, and Kotlin runtime
artifacts that the dependency-report task had not downloaded. Artifact
`8841521780` retains the partial evidence with archive digest
`sha256:3d4a7b610cb95caaca4e3860fe822e8112be0adf3689fe3f5a7a1e1c5535f64f`.
The named discriminator, inline typed default, and runtime-classpath prefetch
are pending the next GitHub verification run.

## Current parity boundary

The generated trees are retained conformance outputs. The seven-language
runtime gate is implemented and requires a passing GitHub run before it becomes
verified evidence. Stable packaged SDKs remain open work. Diagnostics,
authenticated ingress, and richer egress-policy endpoints also remain outside
the current HTTP surface, so the broader OpenAPI and SDK roadmap rows remain
Partial.
