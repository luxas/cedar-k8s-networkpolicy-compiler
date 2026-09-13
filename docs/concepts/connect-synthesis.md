# Connect synthesis: reifying the conjunction

How `np2cedar connect` derives, in closed form, every case in which a Pod may reach a
Pod. The user-facing view is [the guide](../guides/connect.md); this page is the
pipeline, the transition, and the soundness story.

## The `connect` action

The schema declares a third action, `connect` (Pod→Pod, with the same
`{port, protocol}` context as ingress and egress), for which the compiler emits **no
policies at all** — and the tool rejects input policies that name it. Its permits exist
only as synthesized output: `connect` is the reified statement "egress allows this AND
ingress allows this, for the same request".

## The pipeline

The synthesis is cedar-woodpecker's privilege-escalation synthesis, specialized to one
transition:

1. **Cubes.** The compiled permits are split into their DNF cubes per request
   environment — each cube one conjunctive "way" a permission can hold — and pruned with
   the symbolic evaluator so only satisfiable ways remain.
2. **The transition.** One rule says: a Pod holding an egress permission (source 1) and
   an ingress permission (source 2) implicitly holds a connect permission, under a
   condition `when` that identifies the three requests. Cubes are **disjoined within** a
   source (any way of holding egress) and **conjoined across** sources (egress AND
   ingress) — an unsatisfiable side kills the path, which is exactly the semantics
   wanted.
3. **Combination and pruning.** Each egress-cube × ingress-cube pair becomes a candidate
   path over an *extended* schema whose connect context carries the intermediate
   requests (`context.action{i}`, `context.resource{i}`, `context.context{i}`); cvc5
   discards the unsatisfiable ones.
4. **Elimination.** The intermediate requests are reduced away — equalities propagated,
   set operations over the same intermediate combined — leaving a condition over the
   connect request alone: the synthesized permit.

## The identification, and why the equalities are load-bearing

The transition's `when` states, explicitly:

```
context.resource1 == resource && context.resource2 == resource &&
context.context1.port == context.port && context.context1.protocol == context.protocol &&
context.context2.port == context.port && context.context2.protocol == context.protocol
```

— both legs are about the connect request's destination, on the connect request's port
and protocol. The context equalities exist because woodpecker reifies each source
request's context as `context.context{i}`: a source cube's `context.port == 8080`
becomes `context.context2.port == 8080`, and **without** an equality binding it to the
target's port, the elimination would silently drop it as `true` — an over-approximation
that would still print `[sound]`, in a security-analysis tool. So np2cedar does not
hand-maintain the list: the equalities are generated from the schema's own context
attributes, with a hard guard that ingress, egress and connect declare identical context
shapes. A future context attribute is equated automatically or fails loudly — never
dropped.

With `--pod-cidr`, the CIDR memberships for both endpoints are conjoined into the same
`when` (both endpoints are Pods there by construction) — not registered as evaluator
assumptions, which would wrongly constrain the `IpEndpoint` environments too.

## Soundness

Every escalation is checked: the path (assumed true) must imply the synthesized policy's
condition. `[sound]` means the printed permit is implied by its path; `[NOT SOUND]` marks
a path the elimination could only over-approximate — read such a permit as an upper
bound, not a guarantee. The provenance comment (which cubes combined) and the raw path
are printed precisely so a surprising permit can be audited.

## Open-world artifacts, and limits

- With no entities fed in, **labels and identity are independent**: a synthesized
  condition can pair `resource.namespace.labels.getTag("kubernetes.io/metadata.name") ==
  "b"` with `resource.namespace != Namespace::"b"` — honest under open-world reasoning,
  even if no real cluster ever exhibits it. Tying the metadata label to identity is a
  candidate future schema-level assumption.
- The combination multiplies the two directions' cube counts, and DNF splitting has
  budgets; a very large policy set can exceed them, which surfaces as a clean error
  rather than a wrong answer.

Neighbours: [the guide](../guides/connect.md) ·
[symbolic discharge](symbolic-discharge.md) · [the encoding](encoding.md)
