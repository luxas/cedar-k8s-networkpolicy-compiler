# Follow-ups: public-5-connect

## PR description

Answer "who can reach whom?" in closed form. `np2cedar connect` synthesizes the set of
`permit(..., action == Action::"connect", ...)` policies — every case in which a Pod may
talk to another Pod, requiring both the egress permission and the ingress permission for
the same source, destination, port and protocol — using cedar-woodpecker's
privilege-escalation synthesis over the compiled ingress/egress permits, with cvc5
pruning the impossible combinations:

```sh
np2cedar connect -f tests/data/allow-same-ns
```

```
// Synthesized connect policies: how a Pod may reach a Pod, requiring
// the egress and the ingress permission for the same request. 4 path(s).

// connect: (principal: Pod, action: Action::"egress", resource: Pod) via policy1.cube0; (principal: Pod, action: Action::"ingress", resource: Pod) via policy0.cube0, policy0.cube1, policy0.cube2, policy2.cube0 -> (principal: Pod, action: Action::"connect", resource: Pod) [sound]
// path: (context.action1 == Action::"egress") && (context.action2 == Action::"ingress") && (context.resource2.namespace == Namespace::"default") && context.resource2.labels.hasTag("app") && (context.resource2.labels.getTag("app") == "api") && (principal.namespace == Namespace::"default") && principal.labels.hasTag("app") && (principal.labels.getTag("app") == "web") && (context.context2.protocol == Protocol::"TCP") && (context.context2.port == 8080) && (context.resource1 == resource) && (context.resource2 == resource) && (context.context1.port == context.port) && (context.context1.protocol == context.protocol) && (context.context2.port == context.port) && (context.context2.protocol == context.protocol)
@woodpecker("connect: policy1.cube0, policy0.cube0, policy0.cube1, policy0.cube2, policy2.cube0")
permit(
  principal is Pod,
  action == Action::"connect",
  resource is Pod
) when {
  resource.labels.hasTag("app") && (principal.labels.hasTag("app") && ((resource.namespace == Namespace::"default") && ((resource.labels.getTag("app") == "api") && ((principal.namespace == Namespace::"default") && ((principal.labels.getTag("app") == "web") && ((context.port == 8080) && (context.protocol == Protocol::"TCP")))))))
};
```

The first synthesized permit reads: a `web` pod in `default` may connect to an `api` pod
in `default` on TCP 8080 — the ingress rule's port, carried onto the connect request's
own context. Each permit carries its provenance (which egress and ingress cubes combined),
the un-eliminated path, and a soundness marker. `--pod-cidr` prunes rules that can only
match the world outside the cluster; `--json` emits the escalations as data. This is the
capstone of the story: the two-sided NetworkPolicy semantics, compiled to Cedar, reduced
by generic Cedar tooling to a closed-form description of effective connectivity.

## Review findings

1. **The synthesis ignores the entity store**, so labels and identity are independent:
   a permit can pair `namespace.labels.getTag("kubernetes.io/metadata.name") == "b"` with
   `namespace != Namespace::"b"`. A schema-level assumption tying the metadata label to
   the namespace identity would remove these open-world artifacts; feeding the store's
   entities into `Setup.entities` would specialize the answer to one cluster snapshot.
   Both are worth offering as flags.
2. **Budgets are not exposed.** `Budgets::default()` (4096 cubes, 65536 split nodes) is
   fine for fixtures; a real cluster's policy set will hit it, and the only remedy is to
   edit the source. Add `--max-cubes` / `--max-split-nodes`.
3. **All nine request environments are cubed, though only (Pod, egress|ingress, Pod)
   feed the transition.** Filtering `source_cubes` to the two environments the transition
   consumes would cut solver work roughly in half on large inputs.
4. **The synthesized `when` conditions nest deeply to the right** (`a && (b && (c && ...))`),
   which is how woodpecker's `and_chain` renders; a flattening pass before printing
   would make the output as reviewable as the compiler's own `.cedar`.
5. **`resolve_policies` leaves `--entities` mandatory but unread.** The argument graph is
   shared with `check`, so `connect --policies P` still demands `--entities E`. A
   dedicated `Store` variant for policy-only commands would stop asking for a file that
   is never opened.
6. **The output header counts paths, not permits.** With `--json` the count is implicit;
   in text form, a summary line per synthesized permit (source pods, destination pods,
   port) would make a large result skimmable without reading each `when`.
7. **The provenance and path comments are single very long lines.** The guide already
   warns about this; wrapping the `path:` comment at `&&` boundaries would keep it
   readable in a terminal without changing the Cedar.
