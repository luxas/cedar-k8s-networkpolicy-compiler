# Symbolic discharge: how `--pod-cidr` collapses residuals

Under the hood of [discharging unknowns](../guides/discharging-unknowns.md): the
symbolic evaluator, where the assumptions attach, the collapse rules, and why they are
sound. This is the machinery of `cedar-policy-symcc`'s evaluator driving cvc5 — addresses
become SMT bit-vectors, so CIDR containment is decided exactly by the solver.

## The assumptions, and where they attach

The evaluator accepts arbitrary boolean Cedar expressions as assumptions, plus the
partial entity store itself (open-world: entities it lists are pinned to their known
parts; everything else stays unconstrained). np2cedar registers:

1. **The entity store**, so concrete addresses, labels and namespaces participate.
2. **Per unknown address, a CIDR membership**: for every `IpAddr` entity whose attributes
   are unknown — [always an unknown *pod* address](partial-evaluation.md) —
   `IpAddr::"ns/name".addr.isInRange(ip("<cidr>")) || ...` over the given CIDRs.
3. For `reachable`'s fully symbolic Pod peer, the same membership over
   `resource.ip.addr` — attached per evaluation call, only in the request environment
   whose resource *is* a Pod. An `IpEndpoint` environment is never given it: external
   addresses are not pod addresses, and constraining them would be wrong.

Notably absent: the request itself is never assumed. Assumptions persist across
evaluations, and one query's two directions differ in their action — and a concrete
request is already folded into the residuals by partial evaluation anyway.

## The collapse rules, and why they are sound

The compiler emits **permits only** — the default-allow is a catch-all `permit` with an
`unless`, never a `forbid` — and that is the precondition for the whole scheme. Per
direction with verdict `UNKNOWN`, each residual permit's condition is evaluated under the
assumptions; the evaluator returns, for every condition, the exact set of outcomes
(`True`/`False`/`Error`) it can take over all inputs satisfying them:

- Some condition is **always true** → that permit applies on every admissible completion
  → the direction collapses to `ALLOW`.
- **No** condition can be true (an erroring permit never applies) → no permit can apply
  → `DENY`.
- Otherwise → still `UNKNOWN`, but residuals that can never be true are pruned from the
  output, and `reachable` names them.

Introducing a `forbid` anywhere would invalidate this reasoning — the collapse rules are
tied to the all-permit encoding, deliberately.

Two more properties worth knowing:

- **Exactness.** The outcome sets are exact given an exact solver, not over-approximated:
  a CIDR straddling a rule's exception honestly stays `UNKNOWN`, because both outcomes
  are genuinely reachable.
- **Contradiction is an error, not a verdict.** A `--pod-cidr` that contradicts addresses
  the store already pins is reported as such; nothing collapses.
- **A residual the evaluator cannot re-typecheck is kept unknown** with a warning —
  collapsing it would be unsound, keeping it is merely imprecise.

Requires cvc5 (`$CVC5` or on `$PATH`). The solver is spawned only when there is something
to discharge.

Neighbours: [partial evaluation](partial-evaluation.md) ·
[discharging unknowns](../guides/discharging-unknowns.md) ·
[connect synthesis](connect-synthesis.md) (the same solver, synthesizing rather than
discharging)
