# Plan: synthesize the implied `connect` policies with cedar-woodpecker

## Context

np2cedar decides one direction at a time; "can this pod talk to that pod" is the
caller-side conjunction of egress and ingress. cedar-woodpecker synthesizes the implied
Cedar policies of privilege-escalation paths, and conjunctions of permissions are
first-class there: a `Transition` takes multiple *sources* (cubes disjoined within a
source, conjoined across sources) plus a `when` condition identifying the requests.

This feature reifies the conjunction as a schema-level `connect` action (Pod to Pod)
with **no compiled policies**, and a `np2cedar connect` subcommand that runs woodpecker's
pipeline to synthesize the `permit(..., action == Action::"connect", ...)` policies:
exactly the cases in which a Pod may talk to another Pod, requiring both the egress
permission and the ingress permission for the same (source, destination, port, protocol).

## Verified woodpecker mechanics (load-bearing)

- The canonical flow: `LoadedSchema::parse(text, false)` → `evaluator_with(compiler,
  &schema, &assumptions, entities)` → `source_cubes(&policies, &schema, &mut evaluator,
  budgets)` → `Transition`s → `Setup { schema, schema_json, assumptions, entities,
  budgets }` → `escalate(&cubes, &transitions, setup, evaluator.into_compiler())` →
  `Vec<Escalation>` (`.to_text()` / `.to_json()`, `sound` flag).
- `escalate` extends the schema itself; the caller's `Setup.schema_json` is the ORIGINAL
  schema's JSON and must already declare the target action. So the subcommand parses the
  bundled schema via `LoadedSchema::parse(SCHEMA_SRC, false)` (both forms are needed),
  not `crate::schema()`.
- **Intermediate requests' contexts are reified.** The extended schema adds a
  `context{i}` attribute typed as the source action's context, and a source cube's
  `context.port == 8080` becomes `context.context{i}.port == 8080`. It does *not*
  implicitly bind to the target's port: the transition's `when` must equate them, or the
  elimination silently turns the conjunct into `true` — dropping the port constraint
  while still printing `[sound]`.
- `evaluator_with(entities: None)` with no assumptions skips every per-environment
  satisfiability probe; np2cedar passes no entities, so the synthesis is over every
  possible pod. Output is deterministic (cubes and principal types sorted), so insta
  snapshots are safe. Budgets default to 4096 cubes and 65536 split nodes; a very large
  policy set exceeds them with a clean error.

## The schema

```cedar
action "connect" appliesTo {
    principal: [Pod],
    resource: [Pod],
    context: { port: Long, protocol: Protocol },
};
```

`connect` carries its own `{port, protocol}` so the transition can equate them with each
intermediate request's context and the synthesized policies can talk about the connect
port. Nothing else changes: compile validation is policy-directional, every emitted policy
pins ingress or egress, and no snapshot embeds the schema.

## `src/connect.rs`

- `synthesize(policies, pod_cidrs) -> Result<Vec<Escalation>>`: validates the CIDRs
  (the strings are spliced into generated Cedar, so the library must not trust its
  caller), rejects any input policy whose action scope goes beyond ingress/egress (a
  previous synthesis fed back in would otherwise be silently ignored while the header
  still claimed completeness), then runs the pipeline inside the same runtime-plus-cvc5
  bootstrap the discharger uses.
- `transition`: sources `[egress, ingress]` in packet-flow order, target `connect`,
  principal `Pod`, and a `when` composed as expressions from the identification of the
  three requests — `context.resource{i} == resource` for both legs, and one equality per
  context attribute, `context.context{i}["<attr>"] == context["<attr>"]`. The attribute
  list is read from the schema's JSON, with a hard guard that ingress, egress and connect
  declare identical context attributes: a future attribute is equated automatically or
  fails loudly, never dropped.
- With `--pod-cidr`, the CIDR memberships for both endpoints are conjoined into the same
  `when` (both endpoints are Pods there by construction) — not registered as evaluator
  assumptions, which would wrongly constrain the `IpEndpoint` environments too.

## CLI

`np2cedar connect (-f ... | --cluster | --policies P --entities E) [--pod-cidr CIDR]...
[--json] [-o out.cedar]`. The connect arm resolves policies only — no entity store is
built or read, so no misleading unknown-address note — and prints each escalation's
provenance, path and `[sound]`/`[NOT SOUND]` marker as woodpecker renders them, under a
header naming the number of paths.

## Verification

`tests/connect.rs`, cvc5-loud like `tests/symbolic.rs`, snapshots the joined `to_text()`
output and asserts every escalation is sound. As an oracle independent of the fork's own
re-validation, every synthesized permit must strict-validate against the bundled schema —
which among other things proves every `getTag` is still behind its `hasTag` guard, the
property a conjunct-reordering fork bump could silently break.

1. `allow-same-ns`, no CIDR — the egress side is the unconditional catch-all, so connect
   is the ingress conditions with the rule's port constraint carried onto connect's own
   context by the `when` equalities.
2. `cross-namespace`, no CIDR — a genuine conjunction of an egress permit and an ingress
   permit.
3. `ipblock-egress`, with and without `--pod-cidr 10.244.0.0/16` — the
   `0.0.0.0/0 except 10.0.0.0/8` path drops out under the CIDR conjuncts.

Manual: `np2cedar connect -f tests/data/allow-same-ns` prints readable synthesized
permits with provenance comments; `--json` parses as JSON; feeding a previous synthesis
back in is rejected. `check`/`reachable` behaviour is byte-identical.
