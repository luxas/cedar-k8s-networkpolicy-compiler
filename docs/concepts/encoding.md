# The encoding: Kubernetes NetworkPolicy as Cedar

How `np2cedar compile` maps NetworkPolicy's semantics onto Cedar, and why each piece has
the shape it does. The per-construct mapping tables are in
[translation rules](translation-rules.md); the schema types are catalogued in the
[schema reference](../reference/schema.md).

## Two actions, ANDed by the caller

Kubernetes requires **both** ends to agree — a connection succeeds only if the source's
egress rules permit it *and* the destination's ingress rules permit it. Cedar's `permit`
is a disjunction, so that conjunction cannot live inside a single request. So there are
two actions, and the caller ANDs the decisions:

```
allowed(source -> destination, ctx)  ==  ingress(source, destination, ctx) == Allow
                                     &&  egress (source, destination, ctx) == Allow
```

Both requests use the same principal (source) and resource (destination); only the action
differs. Within one direction the semantics are a pure disjunction — additive policies —
which is exactly what a set of `permit`s gives. So each NetworkPolicy rule compiles 1:1
into one readable, annotated Cedar policy, and additivity falls out for free: two
policies selecting the same pod produce two permits, which Cedar ORs.

(The conjunction itself can also be reified and synthesized away — that is the
[`connect` action](connect-synthesis.md).)

## The three kinds of compiled policy

**A catch-all per direction**, carrying Kubernetes' default-allow. A pod is *isolated*
the moment any policy of that type selects it, so the catch-all excludes exactly those
pods:

```cedar
@k8sKind("catch-all")
@k8sDirection("ingress")
permit (principal, action == Action::"ingress", resource)
unless {
  resource is Pod &&
  // default/details-allow-productpage
  (
    resource.namespace == Namespace::"default" &&
    resource.labels.hasTag("app") &&
    resource.labels.getTag("app") == "details"
  )
};
```

The `resource is Pod &&` guard is load-bearing: a non-Pod endpoint is never isolated,
because NetworkPolicy governs pods only.

**One `permit` per rule**, annotated so a decision maps back to the object it came from
— the `@k8sPolicy`/`@k8sDirection`/`@k8sRule` annotations are what `check` prints as its
reasons:

```cedar
@k8sPolicy("default/details-allow-productpage")
@k8sDirection("ingress")
@k8sRule("0")
permit (principal, action == Action::"ingress", resource is Pod)
when {
  // spec.podSelector, in the policy's namespace default
  (resource.namespace == Namespace::"default" && ...) &&
  // ingress[0].from (any of)
  (principal is Pod && ...) &&
  // ingress[0].ports (any of)
  (context.protocol == Protocol::"TCP" && context.port == 9080)
};
```

**Nothing at all** for an isolating policy with no rules. The catch-all's `unless`
already excludes those pods, and with no permit to match they are denied. That is how
`default-deny-all` compiles.

## Schema shapes, and the constraints that forced them

The full type catalogue lives in the [schema reference](../reference/schema.md); three
shapes carry the encoding's weight:

- **Labels and named ports are tag-carrying side entities** (`StringStringMap`,
  `StringLongMap`), because Cedar records have no dynamic keys — and every `getTag` is
  guarded by a `hasTag`, since Cedar's `getTag` *errors* on a missing tag rather than
  returning a default, and `&&` short-circuits.
- **`Pod.ip` is an entity reference (`IpAddr`), not an inline `ipaddr`**, so an address
  can be unknown independently of its pod — Cedar partial entities are all-or-nothing per
  entity. This one shape is what makes [partial evaluation](partial-evaluation.md)'s
  honest unknowns possible.
- **`Protocol` is an enum entity**, giving the solver a finite domain and turning a typo
  into a validation error instead of a silently-false comparison.

One production note: policies are generated as commented Cedar *text* and re-parsed — the
inline `// spec.podSelector` comments are most of what makes the output reviewable, and
Cedar's programmatic syntax tree carries no comments (a recorded follow-up).

## The compiler checks itself

Every generated policy set is parsed and validated with `ValidationMode::Strict` before
it is returned; a failure there is reported as a compiler bug, not a user error. Entity
UIDs are namespaced paths — `Pod::"default/productpage"`, `Namespace::"default"` — chosen
over `metadata.uid` so residual policies stay readable and `--from ns/pod` addresses them
directly.

Neighbours: [translation rules](translation-rules.md) ·
[partial evaluation](partial-evaluation.md) · [schema reference](../reference/schema.md)
