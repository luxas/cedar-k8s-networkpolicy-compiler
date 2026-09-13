# Schema reference

The bundled Cedar schema is the single source of truth for every request the tool
evaluates; print it with `np2cedar schema`, or read `networkpolicy.cedarschema` at the
repo root. This page catalogues its types and the constraint behind each shape — the
narrative is in [the encoding](../concepts/encoding.md).

## Entity types

| Type | Shape | Why |
| --- | --- | --- |
| `Pod` | `{ name, namespace: Namespace, labels: StringStringMap, namedPorts: StringLongMap, ip: IpAddr }` | the subject of every question; `namespace` by entity reference makes same-namespace tests entity equality |
| `Namespace` | `{ name, labels: StringStringMap }` | `namespaceSelector` selects on namespace labels |
| `IpAddr` | `{ addr: ipaddr }` | an address that can be **unknown** independently of its pod — Cedar partial entities are all-or-nothing per entity ([why](../concepts/partial-evaluation.md)) |
| `IpEndpoint` | `{ ip: IpAddr }` | a cluster-external peer; shares the `ip` shape so `X.ip.addr` typechecks for either endpoint type |
| `StringStringMap`, `StringLongMap` | entities with `tags` | Cedar records have no dynamic keys; labels and named ports are tag lookups, every `getTag` guarded by `hasTag` |
| `Protocol` | `enum ["TCP", "UDP", "SCTP"]` | finite domain; typos become validation errors |

## Actions

| Action | principal | resource | context |
| --- | --- | --- | --- |
| `ingress`, `egress` | `[Pod, IpEndpoint]` | `[Pod, IpEndpoint]` | `{ port: Long, protocol: Protocol }` |

For `ingress` and `egress` alike, **principal is the traffic source and resource the
destination**; only the action changes between the two requests of one connection.

Neighbours: [the encoding](../concepts/encoding.md) · [CLI reference](cli.md)
