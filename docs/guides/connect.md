# Who can reach whom: `np2cedar connect`

`connect` answers the general question in closed form: it **synthesizes** the set of
`permit(..., action == Action::"connect", ...)` policies — every case in which a Pod may
talk to another Pod, requiring both the egress permission and the ingress permission for
the same source, destination, port and protocol. The synthesis pipeline
(cedar-woodpecker's privilege-escalation synthesis, driving cvc5) is explained in
[the deep dive](../concepts/connect-synthesis.md).

```sh
np2cedar connect (-f <files> | --cluster | --policies P --entities E) \
                 [--pod-cidr CIDR]... [--json] [-o out.cedar]
```

## Reading the output

Real output, from `tests/data/allow-same-ns` (an ingress rule allowing `web → api` on
TCP 8080 in `default`; nothing restricts egress):

```cedar
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

(The comment lines are long and scroll — they are emitted as single lines, quoted here
verbatim.)

Each synthesized permit carries three things:

- **Provenance** — which egress cubes and which ingress cubes combined to imply it, by
  policy id, so a surprising permit can be traced to its sources.
- **The path** — the un-eliminated condition over the extended request, before the
  intermediate requests were reduced away.
- **A soundness marker** — `[sound]` means the printed policy is implied by the path;
  `[NOT SOUND]` flags a policy the reduction had to over-approximate, so treat it as an
  upper bound. What makes a path unsound is covered in
  [the deep dive](../concepts/connect-synthesis.md#soundness).

Note the ingress rule's `port 8080` ends up as a condition on the **connect** request's
own context: the synthesis identifies all three requests' ports and protocols.

## Flags

- `--pod-cidr` constrains both endpoints' addresses, so rules that only match the world
  outside the cluster — an egress allow to `0.0.0.0/0 except 10.0.0.0/8` — drop out of
  the synthesis entirely ([the same flag, on queries](discharging-unknowns.md)).
- `--json` emits the same escalations as JSON; `-o` writes to a file.

## Two constraints worth knowing

- **Input must be ingress/egress policies only.** `connect` permits are synthesized
  output, never input — feeding a previous synthesis (or any policy naming another
  action) back in is rejected with an error rather than silently ignored.
- **The answer covers every possible pod, not one cluster snapshot.** The store's
  entities are not fed into the synthesis (yet), so conditions are over labels and
  namespaces symbolically; a pod that exists today with `app: web` satisfies the permit
  above, but so would any future one.

Neighbours: [reachability](reachability.md) ·
[connect synthesis](../concepts/connect-synthesis.md) ·
[CLI reference](../reference/cli.md#connect)
