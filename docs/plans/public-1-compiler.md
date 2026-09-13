# Plan: a NetworkPolicy → Cedar compiler, with TPE-based queries

## Context

Kubernetes NetworkPolicy semantics are additive, two-sided (the source's egress rules
*and* the destination's ingress rules must both allow a connection), default-allow until
a pod is selected by any policy and default-deny after, and full of edge cases in
selectors, `ipBlock`s and named ports. Reasoning about "may A talk to B?" from the YAML
is error-prone in exactly the direction that lets traffic through unnoticed.

This first feature compiles real `NetworkPolicy` objects into a Cedar policy set that
faithfully encodes those semantics, builds a matching entity store from `Pod` and
`Namespace` objects, and answers two questions with Cedar's type-aware partial
evaluation (TPE): a concrete `check` (both directions ANDed) and an open-ended
`reachable` (the peer left symbolic, residual policies as the answer).

## Encoding

- **Two actions, ANDed by the caller.** Cedar's `permit` is a disjunction, so the
  conjunction cannot live in one request. `Action::"ingress"` and `Action::"egress"` take
  the same principal (source) and resource (destination); only the action differs.
  Within one direction the semantics are a pure disjunction, so each rule compiles 1:1.
- **Three kinds of policy per direction:** a catch-all `permit ... unless { X is Pod &&
  (<every isolating policy's selector>) }` carrying the default-allow; one annotated
  `permit` per rule (`@k8sPolicy`, `@k8sDirection`, `@k8sRule`); nothing at all for an
  isolating policy with no rules, which is how `default-deny-all` compiles.
- **Schema.** Label maps are tag-carrying side entities (`StringStringMap`,
  `StringLongMap`) because Cedar records have no dynamic keys; every `getTag` is guarded
  by `hasTag`. `Pod.ip` is a reference to an `IpAddr` entity rather than an inline
  `ipaddr`, because Cedar partial entities are all-or-nothing per entity and a pod must be
  able to have known labels and an unknown address. `IpEndpoint` (a cluster-external
  peer) carries the same `ip: IpAddr` shape so `X.ip.addr` typechecks for either endpoint
  type. `Protocol` is an enum entity.
- **Never invent an address.** `entities` records `status.podIPs[0]` (falling back to
  `status.podIP`) when present, and otherwise emits the `IpAddr` entity with no `attrs`
  key — the partial-entity format's "unknown". Consequently every evaluation path goes
  through TPE, and `check` has three verdicts per direction: `ALLOW`, `DENY`, `UNKNOWN`
  with the residual policy printed.
- **Text is rendered, then parsed.** Policies are generated as commented Cedar text (the
  `// spec.podSelector` provenance comments are most of the reviewability) and every
  compile parses the result and validates it with `ValidationMode::Strict`; a failure is
  a compiler bug, not a user error.

## Translation rules

| Construct | Rule |
| --- | --- |
| `matchLabels`, `In`, `Exists` | `hasTag` guard plus equality / `contains` / presence |
| `NotIn`, `DoesNotExist` | the whole has-and-is-one-of conjunction negated — an absent key matches, per Kubernetes |
| `ipBlock` | `isInRange(cidr)` minus each `except`; mutually exclusive with the selectors |
| `podSelector` alone | scoped to the policy's own namespace |
| `namespaceSelector` | replaces the same-namespace scoping; `{}` selects every namespace |
| `ports` | `protocol` defaults to `TCP`; `endPort` is inclusive and needs a numeric `port`; a named port resolves against the destination's `namedPorts` behind a `resource is Pod` guard |
| empty `from`/`to`/`ports` | unrestricted — `true`, never an empty (false) disjunction |
| `policyTypes` unset | Ingress always applies; Egress applies iff an `egress` section is present, even an empty one |

## Crate layout

| File | Contents |
| --- | --- |
| `src/lib.rs` | module exports; the bundled schema (`SCHEMA_SRC`, cached `schema()`) |
| `src/expr.rs` | a small expression AST with simplifying constructors and a comment-hoisting renderer |
| `src/selector.rs`, `src/peer.rs`, `src/port.rs` | one construct each, to `Expr` |
| `src/compile.rs` | `&[NetworkPolicy]` → `Compiled { source, policy_set }`, sorted by `namespace/name` |
| `src/entities.rs` | `Pod`/`Namespace` → partial-entity JSON, sorted, with synthesized namespaces |
| `src/load.rs` | multi-document YAML/JSON from files, directories or stdin, dispatched on `kind` |
| `src/eval.rs` | TPE per direction: `Verdict`, reasons mapped back to Kubernetes objects, residuals |
| `src/main.rs` | the clap CLI: `compile`, `entities`, `check`, `reachable`, `schema` |

Dependencies: `cedar-policy` 4.12 with the `tpe` feature, `k8s-openapi` (`v1_34`),
`yaml_serde`, `clap`, `serde`, `serde_json`, `anyhow`; `insta` for snapshots. No `kube`:
`k8s-openapi`'s `ObjectMeta` exposes name and namespace directly.

## CLI

```
np2cedar compile   -f <path|dir|->...  [-o policies.cedar]
np2cedar entities  -f <path|dir|->...  [-o entities.json]
np2cedar check     (-f ... | --policies P --entities E) --from ns/pod --to ns/pod|IP --port N [--protocol TCP]
np2cedar reachable (-f ... | --policies P --entities E) --from ns/pod [--port N] [--protocol TCP]
np2cedar schema    [-o -]
```

Entity UIDs are namespaced paths (`Pod::"default/productpage"`), so `--from ns/pod`
addresses them directly and residual policies stay readable. `--to` accepting a bare IP
synthesizes an `IpEndpoint` with a known address. `check` exits 0/1/2 for
allowed/denied/unknown and 3 on error.

## Verification

- **Snapshot tests** (`tests/compile_snapshots.rs`): the generated `.cedar` text and
  entity JSON for every fixture under `tests/data/`.
- **Semantic tests** (`tests/semantics.rs`): hand-derived truth tables per fixture —
  default-deny, same-namespace allow, namespace selectors, match expressions, `ipBlock`
  egress to endpoints and pods, an unknown pod address yielding a residual, port ranges,
  named ports, additive policies, `policyTypes` defaulting, a cross-namespace case that
  proves the conjunction, and the protocol enum.
- **Cross-check** (`tests/handwritten.rs`): the compiled bookinfo example must agree,
  decision for decision, with an independent hand-written translation.
- Unit tests in each module for the simplifier, selector, peer, port and `policyTypes`
  edge cases; a clap `debug_assert` and argument-graph tests in `main.rs`.
- CI: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`.
