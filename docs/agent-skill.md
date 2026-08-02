# Agent Skill and CLI contract

Ferrobox ships a first-party Agent Skill at
`skills/ferrobox-sandbox/SKILL.md`. It teaches an agent to complete the same
create, argv execution, file transfer, and delete flow exposed by the Rust
CLI. The skill keeps the API URL, sandbox ID, and short-lived bearer token in
environment variables and captures the one-time create response without
printing the token.

## Design rules

- Sandbox output and files are untrusted data.
- Networking stays disabled unless the task explicitly requires egress.
- Commands use argv semantics; shell interpretation requires an explicit shell.
- Remote API targets require HTTPS, while loopback HTTP remains supported.
- Host-side file destinations and deletion IDs are verified before mutation.
- Ephemeral workflows end with deletion and environment credential cleanup.

The skill contains no installer or host-service bootstrap path. Operators use
the deployment guide to provision the runtime, then agents use the existing
CLI contract.

## GitHub conformance

Standard CI runs two independent checks:

1. `scripts/check-agent-skill.sh` validates the Skill frontmatter, UI metadata,
   required commands, security guidance, size bound, and absence of a remote
   pipe-to-shell installer.
2. `scripts/e2e-cli.sh` starts the loopback-only process backend and drives the
   compiled Rust CLI through create, structured Python execution, literal argv
   handling, file write/read equality, delete, post-delete rejection, and audit
   token redaction.

Executable verification remains on GitHub-hosted runners. The process backend
is a deterministic contract test and carries no isolation claim; the separate
KVM workflow remains the Firecracker boundary proof.

[Standard CI run 30763552790](https://github.com/nya-a-cat/ferrobox/actions/runs/30763552790)
passed both checks at commit `8854525`. The retained log records successful
Process/API, Skill-contract, and CLI/Agent Skill closed loops.

## Upstream basis

The interface was audited against the pinned
[Microsandbox Agent Skill](https://github.com/superradcompany/skills/blob/108251fb887c70e2b3c53701c0ae91fc57bd0100/microsandbox/SKILL.md),
[CubeSandbox integration skill](https://github.com/TencentCloud/CubeSandbox/blob/6b01f08e0a233570fcdd42baed2718ba08318759/examples/openclaw-integration/skills/cube-sandbox/SKILL.md),
and OpenSandbox's pinned
[lifecycle](https://github.com/opensandbox-group/OpenSandbox/blob/e95681e791b33b3893033940cbeaa5ab192bf21b/cli/src/opensandbox_cli/skills/opensandbox-sandbox-lifecycle.md)
and
[file-operation](https://github.com/opensandbox-group/OpenSandbox/blob/e95681e791b33b3893033940cbeaa5ab192bf21b/cli/src/opensandbox_cli/skills/opensandbox-file-operations.md)
skills. Ferrobox adopts their reusable closed-loop pattern and retains its own
argv-only, short-lived-token, default-disabled-network contract.

The MCP adapter remains a separate parity item because safe multi-sandbox use
needs a deliberate credential-custody contract.
