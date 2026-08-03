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
mapping. The authoritative contract accepts exactly the four tagged variants.
OpenAPI Generator v7.22.0 misgenerates this strict union in its C# and Rust
targets. `openapi/ferrobox-codegen-overlay.json` is a reviewed RFC 7396 merge
patch used only for SDK generation. It projects `ExecTermination` into a closed
object with one required four-value `kind` enum plus optional `exit_code` and
`signal` fields. `scripts/openapi_codegen_projection.py` refuses authoritative
contract drift, overlay drift, and an unexpected projected shape. It preserves
authoritative schema-property order because generated C#, Go, and Rust
constructors use positional arguments. The strict contract remains the HTTP
validation boundary, and the emitted JSON shape is unchanged.

The optional `cwd` property keeps its `/home/sandbox` default and repeats the
`SandboxPath` string constraints inline so Java receives a valid quoted string
initializer. This representation preserves the accepted request set.

## GitHub conformance

Standard CI uses three independent checks:

1. OpenAPI Generator validates the authoritative document. The deterministic
   projector derives the reviewed code-generation view and emits C#, Go, Java,
   Kotlin, Python, Rust, and TypeScript Fetch source trees from it.
2. `scripts/check-openapi.py` compares the specification with the exact Axum
   route/method set, resolves every local reference, checks operation IDs,
   request bodies, path parameters, error responses, credential scopes, and
   sensitive fields, recalculates the projection from the retained overlay,
   then hashes each generated source tree.
3. CI builds one versioned native package for each generated C#, Go, Java,
   Kotlin, Python, Rust, and TypeScript client. A separate consumer installs
   that package, then all seven consumers share one loopback-only API process
   and complete create, inspect, typed command execution, lossless output, file
   roundtrip, delete, stale-handle rejection, and audit credential-redaction
   checks.

Each CI run generates all seven client trees twice in separate runner-temporary
directories and requires a recursive byte comparison before and after runtime
conformance. Both independent projections must equal one job-stable runner
input before generation, preventing output-directory paths from entering the
generated README files. The C# project GUID is fixed. Every runtime operates on
a temporary copy, so dependency resolution, harness sources, build products,
and caches do not mutate either generated tree. The retained hidden metadata
includes the exact merge patch and projected OpenAPI document; structural
evidence records their SHA-256 values. Hidden generator metadata is included
in the artifact after a path-name review.

The retained SDK matrix contains one sanitized schema-1 record per language,
the seven distinct UUIDv7 sandbox identities, the shared audit-log hash, and
the SHA-256 of each dependency manifest or lock. Per-language JSON records use
canonical BOM-free UTF-8. Python uses a fixed
`exclude-newer` cutoff. C# uses NuGet locked mode; Go uses its checked-in
`go.sum`; Java resolves online, explicitly prefetches the Surefire JUnit
Platform provider at `2.22.2`, records its JAR SHA-256, then tests offline;
Kotlin resolves every resolvable generated-project configuration while writing
a strict Gradle lock, then builds and runs offline; Rust fetches a generated
`Cargo.lock` then runs offline; TypeScript uses exact package and consumer pnpm
graphs and runs the compiled CommonJS output under Node. The package manifest
binds every package SHA-256 to its consumer lifecycle record. The process
backend supplies deterministic API behavior and carries no workload-isolation
claim.

## Versioned package contract

`openapi/ferrobox-sdk-packages.json` is the checked-in schema-1 identity
contract for the generated `0.1.0` packages:

| Language | Package identity | GitHub package form |
| --- | --- | --- |
| C# | `Ferrobox.Client` | NuGet `.nupkg` |
| Go | `github.com/nya-a-cat/ferrobox/sdk/go` | deterministic Go module proxy ZIP |
| Java | `io.github.nyaacat.ferrobox:ferrobox-java-client` | Maven JAR and POM |
| Kotlin | `io.github.nyaacat.ferrobox:ferrobox-kotlin-client` | Maven repository with JAR, sources, module, POM, and checksums |
| Python | `ferrobox-client` / `ferrobox_client` | wheel |
| Rust | `ferrobox-client` / `ferrobox_client` | Cargo `.crate` |
| TypeScript | `@nya-a-cat/ferrobox` | npm `.tgz` |

Each consumer crosses the package boundary used by its ecosystem. C# compares
the packed NuGet bytes with the global-package cache entry. Go resolves only
through the generated `file://` module proxy. Java and Kotlin resolve from
runner-local Maven repositories. Python installs the wheel into a clean uv
virtual environment. Rust extracts the `.crate` before the consumer build.
TypeScript installs the generated tarball through a frozen pnpm graph.

`scripts/check-openapi-sdk-packages.py` parses every package container, checks
its embedded name, version, and compiled entrypoint or marker class, rejects
unsafe ZIP paths, and records size plus SHA-256. The result links each package
to the SHA-256 of the seven-check lifecycle record produced by its installed
consumer. CI writes only runner-temporary package repositories and uploads them
as evidence. No external registry receives a package in this gate.

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
The named variants and inline typed default removed the original Java and Rust
source-generation failures in
[run 30774849266](https://github.com/nya-a-cat/ferrobox/actions/runs/30774849266)
at commit `2f0540d`. Go, Python, and TypeScript again completed the scenario, and
the generated trees remained byte-identical. C# and Rust compiled, then exposed
two target-specific discriminator deserialization defects. Java compiled and
reached its offline test phase, where Maven lacked the dynamically selected
Surefire JUnit Platform provider. Kotlin's prefetch included the unbuilt local
classes directory. Artifact `8841711323` retains this narrowed diagnostic with
archive digest
`sha256:d9cd88cef12f2aacbc3012f29359a2c6802956257cfe0833f24ba895f37a7293`;
it expires on 2026-11-01.

The completed follow-up,
[run 30777303301](https://github.com/nya-a-cat/ferrobox/actions/runs/30777303301)
at commit `86bb3cc`, passed the entire Standard CI job. C#, Go, Java, Kotlin,
Python, Rust, and TypeScript each completed the same seven-check lifecycle
against one API process. The aggregate matrix records seven distinct UUIDv7
sandboxes, 35 sanitized audit events, seven successful creates, seven successful
deletes, and one dependency record set per language. The post-runtime recursive
comparison proved that both generated source roots remained byte-identical.

The structural record binds authoritative specification SHA-256
`70e9400c4089000b4757df3228143d68fd9bf65aea4fb36a3417b30381368b0a`,
overlay SHA-256
`82ea593b150854333d031e2bdb23bb613cbd97ee275e1cfdc2e42261aa00a733`,
and projected-document SHA-256
`eed1203f24517729bb693bc66e6a9f80efaf072cc7d74838e2bd48f5ca6ead3b`.
The retained generated trees are:

| Generator | Files | Tree SHA-256 |
| --- | ---: | --- |
| C# | 73 | `6be04a969fe0655ecff9548a2c6370738b4f941b7e535d155e02f46155fbe9d7` |
| Go | 50 | `a4fb086a5f602b0a07fc8a8610104934f49387b5e36905ef2953048427935a80` |
| Java | 73 | `a7ed92d9317248f2ae1be17697741bee69df609d1253f9c5de8e4a2ab8c94c2d` |
| Kotlin | 63 | `8ca201694ea36e97c660b9c4f00ee6da1a5014ff977a65f2a59dc88fc3c188ff` |
| Python | 60 | `6c644fd086a4f8a0a6a848cc19b3eadca4fa8dd31bc4d7fc3bdf9bcdaf4a5f3d` |
| Rust | 48 | `e107456cb7db27059efa1fcd383a3913aae5289b0791283cb35fcf1fab7ffbca` |
| TypeScript Fetch | 43 | `3fb622af39b4ef84d9ca6ac5b5cf0c5e2b3a7c9b12be486b88b35251ba94a1bd` |

Artifact `8842515354` retains the generated roots, projection metadata, seven
runtime records, aggregate matrix, audit log, dependency manifests, and
toolchain record. Its archive digest is
`sha256:78a7ce80c5f3e3d6a177d81744e0b09580119d76569ac8e6c6435d556ca3331f`,
and it expires on 2026-11-01.

[Standard CI run 30779954220](https://github.com/nya-a-cat/ferrobox/actions/runs/30779954220)
at commit `8bf9a62` passed the versioned package gate. Its seven independent
consumers installed the NuGet package, Go module ZIP, Java Maven artifact,
Kotlin Maven artifact, Python wheel, Cargo crate, and npm tarball before running
the same lifecycle. The aggregate matrix records seven package smokes, seven
distinct UUIDv7 sandboxes, 35 sanitized audit events, seven successful creates,
seven successful deletes, and the dependency records for both package builds
and consumers. The package-contract SHA-256 is
`8444a4e5f0d6209e7d95f6eea73419c78f26a182f5bc5df2f7c15a889c347cea`;
the package-manifest SHA-256 is
`c0b873f372b4e0906c4951eb01f0500d827576e16f9501ff795d43c3d1034236`.

| Package | SHA-256 |
| --- | --- |
| `Ferrobox.Client.0.1.0.nupkg` | `8dc9de2b2fa6eae568ba3b61544e884ebd25088fcdf94a675a3368f076691ca4` |
| Go module `v0.1.0.zip` | `fb512f606502c36dcba99b4c096906efbe6506f546ebfd2cf7d84a9e6a1a4e05` |
| `ferrobox-java-client-0.1.0.jar` | `e8c55ea535f67910db05a6872ffbdfd8b51b92b78313b8564adc303ec7f9a575` |
| `ferrobox-kotlin-client-0.1.0.jar` | `2befff8f19239e4a8ba89987756d1e8f1cb932fd2594605d31250965f724e97e` |
| `ferrobox_client-0.1.0-py3-none-any.whl` | `619cc1a5c86381075f302e9c1e2c2f180c6c7a25e007eb7201f7606a13b243a1` |
| `ferrobox-client-0.1.0.crate` | `9fb7cd376ba22a9098ad105cf5dbdf78823f4b4e3bf1618eee0ab86b19fd4a1a` |
| `nya-a-cat-ferrobox-0.1.0.tgz` | `6faf22d052fdeca580a0b8853eb089243a988ad71337dea4e7fe9e21d054e08a` |

Artifact `8843358862` retains the packages, package contract and manifest,
generated roots, projection, locks, runtime records, aggregate matrix, audit
log, and toolchain record. Its archive digest is
`sha256:202af788f8ad5a41f9276ea53c9fb6ca95183f0cb0bf137ad524bb07f41f44a2`,
and it expires on 2026-11-01. The current stable generated-tree hashes are C#
`8bbc5d6a95af7687fbd24befb0fca2508625a525dee92a43a8e1eee848802b5f`,
Go `211711093fe9f88e49ea1ec5bb085e3507c58bc5f55503fb7d41053aee198c49`,
Java `d6093bf23312501efd33b9da5d8209dad19555edaab0ae3370a4f5fc68b26d52`,
Kotlin `e29f62a32e7997ca59dd50e675e9b2ece30d09e0b990595a4e55acd03b55ab96`,
Python `ee8b26b656b14a46e6cded2b52cec5ab7c3bba80665e4bb60705e8bc94ba12b9`,
Rust `2e3cc3eb37a1aec158ac85798628b0034b30c16e21c036f8af87eeac528d27b9`,
and TypeScript Fetch
`e90f5f6714b7dfe752d90e2d56ea85adf9694f2fce1606965406cefc9a3c8c38`.

## Current parity boundary

The generated trees are retained conformance outputs, and their seven-language
runtime and versioned-package gates are Verified. The SDK roadmap row now meets
its acceptance gate. Diagnostics, authenticated ingress, and richer
egress-policy endpoints remain outside the current HTTP surface, so the broader
OpenAPI row remains Partial. External registry publication, keyless signing,
and consumer-side signature verification remain under the release-integrity
row and need an approved credential and GitHub permission contract.
