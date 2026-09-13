# np2cedar — Kubernetes NetworkPolicy in Cedar

Compiles Kubernetes `NetworkPolicy` objects into a [Cedar](https://cedarpolicy.com) policy
set, builds the matching entity store from `Pod` and `Namespace` objects, and answers
questions about them — "may this pod talk to that one?", "what can this pod reach?" —
with Cedar's authorizer and its type-aware partial evaluation.

## Why

NetworkPolicy is hard to reason about from the YAML alone. Policies are additive; a pod is
default-allow until the first policy selects it and default-deny after; every connection
needs **two** independent permissions — the source's egress rules and the destination's
ingress rules; and selectors, `ipBlock`s and named ports each carry their own edge cases.
Misreading any of that tends to be wrong in the one direction that matters most: traffic
you believed blocked, flowing.

Compiling to Cedar turns those semantics into artifacts a machine can be precise about:

- The compiled policy set is **readable and reviewable**: one annotated `permit` per rule,
  each traceable back to the Kubernetes object and rule index it came from
  ([the encoding](concepts/encoding.md)).
- Verdicts are **exact, with honest unknowns**. When the answer depends on something the
  cluster did not say — a pod with no recorded address — the tool answers `UNKNOWN` and
  prints the residual condition naming exactly what is missing, instead of guessing
  ([partial evaluation](concepts/partial-evaluation.md)).

## What

One binary (and a library) over three stages, fed from files or a
[live cluster](guides/live-cluster.md):

```
NetworkPolicies ──► np2cedar compile  ──► policies.cedar ─┐
Pods, Namespaces ─► np2cedar entities ──► entities.json ──┤
                                                          ├─► check      may A talk to B?
                                                          └─► reachable  what can A reach?
```

- **`compile` / `entities`** translate the Kubernetes objects
  ([translation rules](concepts/translation-rules.md),
  [schema](reference/schema.md)). Every generated policy set is strictly validated
  before it is returned.
- **`check`** decides one connection, both directions ANDed
  ([guide](guides/checking-connectivity.md)); **`reachable`** leaves the peer
  symbolic and prints residual policies describing what qualifies
  ([guide](guides/reachability.md)).

## How

```sh
np2cedar compile   -f examples/bookinfo -o policies.cedar
np2cedar entities  -f examples/bookinfo -o entities.json
np2cedar check     --policies policies.cedar --entities entities.json \
                   --from default/productpage --to default/details --port 9080
```

Both directions allow — the ingress line of the output names the rule that decided it
(`default/details-allow-productpage ingress[0]`), the egress line the default-allow
catch-all — and the combined verdict is `=> ALLOWED`, exit code 0. The
[quickstart](quickstart.md) shows the full output and continues from here.

When a pod's address was never recorded, the verdict is honestly `UNKNOWN`:

```sh
np2cedar check -f tests/data/unknown-pod-ip --from default/gateway --to default/mystery --port 443
# => UNKNOWN (the rule allows egress only to the internet; mystery's address is unknown)
```

Start at the [quickstart](quickstart.md) — it covers building the tool and the first
queries.

## Topics

Everything here is organized along two axes: **topic** (rows below) and **depth** — a
guide shows how to use a feature, a deep dive explains how and why it works, a reference
enumerates. If you know what kind of reader you are, start from
[your reading path](#reading-paths-by-persona) instead.

| Topic | Guide (use it) | Deep dive (understand it) | Reference |
| --- | --- | --- | --- |
| Getting started | [quickstart](quickstart.md) | — | [CLI](reference/cli.md) |
| Compiling NetworkPolicy to Cedar | [quickstart](quickstart.md) | [the encoding](concepts/encoding.md) · [translation rules](concepts/translation-rules.md) | [schema](reference/schema.md) |
| May this pod talk to that one? | [checking connectivity](guides/checking-connectivity.md) | [partial evaluation](concepts/partial-evaluation.md) | [CLI: check](reference/cli.md#check) |
| What can this pod reach? | [reachability](guides/reachability.md) | [partial evaluation](concepts/partial-evaluation.md) | [CLI: reachable](reference/cli.md#reachable) |
| Reading a live cluster | [live cluster](guides/live-cluster.md) | — | [CLI: sources](reference/cli.md#sources) |
| The codebase | [testing](contributing/testing.md) | [architecture](contributing/architecture.md) | — |

## Reading paths, by persona

**The operator** — you have a cluster and want answers about it.
[Quickstart](quickstart.md) → [checking connectivity](guides/checking-connectivity.md) →
[live cluster](guides/live-cluster.md) → [reachability](guides/reachability.md). Keep the
[CLI reference](reference/cli.md) at hand.

**The policy author** — you write NetworkPolicies and want to know what they really mean.
[The encoding](concepts/encoding.md) → [translation rules](concepts/translation-rules.md)
(the edge cases live here: `policyTypes` defaulting, `NotIn` and absent keys, named
ports) → [checking connectivity](guides/checking-connectivity.md) → the verdict-level
subtleties in [partial evaluation](concepts/partial-evaluation.md).

**The formal-methods reader** — you care about what is being decided, and how exactly.
[The encoding](concepts/encoding.md) → [partial evaluation](concepts/partial-evaluation.md).
The places where the tool deliberately answers `UNKNOWN` are there.

**The contributor** — you want to change the code without breaking its promises.
[Architecture](contributing/architecture.md) → [testing](contributing/testing.md).

## Conventions in these pages

Command output shown in an output block is real, verbatim output generated from the
fixtures named next to it (`examples/bookinfo`, `tests/data/*`) — if it drifts, that is a
documentation bug. Output quoted inside a shell block as `#` comment lines is annotation
and may be abridged; schematic Cedar (a `permit(...)` with literal `...`) is a shape, not
output.
Relative links only; every page opens with a one-paragraph summary and links its
neighbouring guide, deep dive and reference.

## Validation

The suite checks the compiler against hand-derived truth tables, an independent
hand-written translation, and the API server's own defaulting. What that does and does
not establish is spelled out in [testing](contributing/testing.md).

## Caveats and follow-ups

Not modelled, and out of scope for now:

- Stateful connections and return traffic — each direction is judged on its own.
- Host-network pods, and the kubelet's health-check traffic, which bypasses NetworkPolicy.
- Cross-address-family `isInRange` returns false, matching Kubernetes.
- Cluster-specific behaviour beyond the API contract: some CNIs allow node-local traffic
  that the spec leaves implementation-defined.

Planned:

1. **Build policies with `cedar_policy::pst`** rather than rendering text and re-parsing
   it. Blocked on comments: the PST carries no trivia, so the inline `// spec.podSelector`
   annotations would have to move into Cedar annotations or be dropped.
2. **Multiple pod addresses.** `status.podIPs` is a list (dual-stack); today the first
   entry is used and `Pod.ip` models one address. Generalise to a set, matching if any
   address does.
