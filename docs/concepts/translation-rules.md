# Translation rules, construct by construct

The exact mapping from NetworkPolicy constructs to Cedar conditions, including the edge
cases Kubernetes hides in prose. The surrounding structure — which policies get emitted
at all — is in [the encoding](encoding.md).

## `LabelSelector`

Used for `spec.podSelector`, `peer.podSelector` and `peer.namespaceSelector`. Every
`getTag` is guarded by a `hasTag`, because Cedar's `getTag` *errors* on a missing tag and
`&&` short-circuits.

| Kubernetes | Cedar |
| --- | --- |
| `matchLabels: {k: v}` | `X.labels.hasTag("k") && X.labels.getTag("k") == "v"` |
| `Exists` / `DoesNotExist` | `X.labels.hasTag("k")` / `!X.labels.hasTag("k")` |
| `In [a,b]` | `X.labels.hasTag("k") && ["a","b"].contains(X.labels.getTag("k"))` |
| `NotIn [a,b]` | `!(X.labels.hasTag("k") && [..].contains(..))` — **matches an absent key**, per Kubernetes, which is why the whole conjunction is negated, not just the membership |
| `{}` or absent | `true` |

## `NetworkPolicyPeer`

`ipBlock` is mutually exclusive with the selectors, and a peer setting none of the three
is invalid — both are compile errors here, not silent guesses.

| Kubernetes | Cedar |
| --- | --- |
| `ipBlock: {cidr, except}` | `X.ip.addr.isInRange(ip("<cidr>")) && !X.ip.addr.isInRange(ip("<e>")) && ...` |
| `podSelector` only | `X is Pod && X.namespace == Namespace::"<policy ns>" && <sel>` |
| `namespaceSelector` only | `X is Pod && <sel over X.namespace.labels>` |
| both | `X is Pod && <nsSel(X.namespace)> && <podSel(X)>` |
| `from`/`to` empty or absent | clause omitted — all peers |

The scoping rules matter: a `podSelector` **without** a `namespaceSelector` binds only
the policy's own namespace, while adding a `namespaceSelector` *replaces* that scoping
rather than narrowing it — and an empty `namespaceSelector: {}` selects every namespace.
Every namespace automatically carries the `kubernetes.io/metadata.name` label, so
selecting one namespace by name works; `np2cedar entities` synthesises that label for
namespaces it was not given.

## `NetworkPolicyPort`

| Kubernetes | Cedar |
| --- | --- |
| `protocol` (defaults to `TCP`) | `context.protocol == Protocol::"TCP"` |
| numeric `port` | `context.port == N` |
| `port` + `endPort` | `context.port >= N && context.port <= E` — inclusive at both ends; `endPort` requires a numeric `port` |
| named `port` | `resource is Pod && resource.namedPorts.hasTag("http") && context.port == resource.namedPorts.getTag("http")` |
| `ports` empty or absent | clause omitted — all ports |

A named port always resolves against the **destination**, which is `resource` in both
directions; on an egress rule aimed at an `IpEndpoint` the `resource is Pod` guard makes
the clause `false` — correct, a container port name cannot resolve outside the cluster.
`endPort` with a named port is rejected, as Kubernetes rejects it.

## `policyTypes` defaulting

Per the API contract: when unset, **Ingress always applies**, and **Egress applies
exactly when an `egress` section is present** — even an empty one. The sharp edge: a
policy with only an `egress` section still isolates its pods for ingress, with no ingress
rules to let anything back in. (`tests/data/egress-defaulting` pins this.)

An earlier shipped bug is worth its regression note: **empty `from`/`to`/`ports` means
unrestricted, i.e. `true`** — not an empty disjunction, which is `false` and silently
denied everything the rule meant to allow.

Neighbours: [the encoding](encoding.md) ·
[testing](../contributing/testing.md) (how these rules are pinned)
