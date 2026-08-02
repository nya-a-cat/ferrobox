# Network isolation and DNS relay contract

This contract covers Ferrobox `Internet` mode. The default `Disabled` mode has
no guest data interface.

## Per-sandbox topology

Each Internet-enabled sandbox owns one network namespace, TAP, bridge, veth
pair, `/24` subnet, and project-scoped nftables table. The guest receives
`10.200.X.2`, uses `10.200.X.1` as its gateway, and receives the same gateway
address as its only DNS server. Cleanup is keyed by sandbox ID and aborts DNS
work before deleting the nftables table, tagged host-forwarding rules, veth,
and namespace.

## DNS boundary

The node binds a DNS relay to UDP and TCP port 53 on that sandbox's gateway.
The relay accepts packets only from the assigned guest address. It forwards
wire-format DNS queries from host-originated sockets to at most three distinct
IPv4 resolvers discovered from the host resolver files, with `1.1.1.1` as the
last fallback. This supports host-local stubs, private VPC resolvers, and cloud
virtual resolvers without exposing their addresses to the guest.

Each upstream attempt has a two-second deadline. A sandbox can have at most 128
DNS exchanges in flight. Queries require a DNS header and at least one
question; replies require the matching transaction ID and response bit. An
exhausted upstream set returns `SERVFAIL`. UDP truncation is preserved so the
guest resolver can retry over TCP. Dropping the network lease aborts the relay
and its outstanding exchanges.

## Packet policy

The nftables input hook accepts guest traffic to the gateway only on UDP/TCP
port 53 and rejects every other host destination. The forwarding hook rejects
loopback, RFC1918, link-local and metadata, carrier-grade NAT, multicast,
reserved, host-management, and other sandbox ranges. Remaining public IPv4
traffic is allowed and source-NATed through the host. Two rules carrying a
`ferrobox:<sandbox>` comment admit that validated traffic through host FORWARD
policies installed by container engines or cloud images. Exact-match deletion
removes both rules with the sandbox.

Resolver answers do not bypass the forwarding policy. A hostname resolving to
a rejected address remains unreachable. FQDN allowlists, wildcard policy,
rebinding-aware answer pinning, and runtime policy updates remain separate SOTA
parity gates in `sota-roadmap.md`.

## GitHub acceptance

The hosted KVM job must prove all of the following in one run:

- the guest receives its sandbox gateway as the only resolver;
- HTTPS resolution and public egress succeed through UDP DNS;
- a wire-format query succeeds through TCP DNS;
- the metadata endpoint remains unreachable;
- deletion leaves no Firecracker process or Ferrobox network namespace;
- deletion leaves no tagged Ferrobox host-forwarding rule;
- failure evidence retains host resolver inputs, routes, interfaces, and the
  exact per-sandbox nftables table.
