# Plan: discharge address residuals with the symbolic evaluator (`--pod-cidr`)

## Context

np2cedar answers connection queries through Cedar's partial evaluation. A pod whose
`status.podIPs` is absent has an unknown address, so `ipBlock` rules over it yield
residuals and an `UNKNOWN` verdict. But an unknown pod address is *constrained*: it lies
in the cluster's pod CIDR. This feature asks a symbolic evaluator whether a residual
rule's CIDR-minus-exceptions can ever intersect the pod CIDR — a rule allowing egress to
`0.0.0.0/0 except 10.0.0.0/8` can never match a peer *pod* when the pod CIDR is inside
`10.0.0.0/8`, so the rule is pruned and `UNKNOWN` becomes a definite verdict.

The evaluator is `cedar_policy_symcc::evaluator::Evaluator` from the
[cedar-woodpecker](https://github.com/luxas/cedar-woodpecker) fork of Cedar: it accepts
boolean Cedar expressions and a partial entity store as assumptions, and its async
`evaluate` returns, per expression, the exact set of outcomes (`True`/`False`/`Error`) the
expression can take over every input satisfying the assumptions, decided by cvc5.

## The dependency

The crates.io `cedar-policy-symcc` release predates the evaluator, so `Cargo.toml` takes
the crate from the fork as a git dependency pinned to a commit. The fork pins
`cedar-policy` and `cedar-policy-core` by path at `=4.12.0`, so `[patch.crates-io]`
points both at the same commit — otherwise the graph would hold two instances of each
crate and the `AsRef` bridges from the public TPE wrappers to the core types would not
line up (`cargo tree -i cedar-policy-core` must show one node). `tokio` becomes
unconditional (the evaluator is async); the `cluster` feature keeps only `kube`.

The pin is bumped deliberately, together with any test that byte-encodes the evaluator's
output. CI installs cvc5 in every job that runs `cargo test`.

## Semantic design

Assumptions attach in two ways:

1. **Per entity, global**, for every `IpAddr` entity in the store whose attributes are
   unknown: `IpAddr::"<id>".addr.isInRange(ip("<cidr1>")) || ...`. The invariant this
   keys on: an attribute-less `IpAddr` entity is always an unknown *pod* address —
   `IpEndpoint`s are always emitted with concrete addresses. The unknown set is derived
   from the `PartialEntities` themselves, so replayed `--entities` files work too.
2. **Per call**, for `reachable`'s fully symbolic Pod peer: `resource.ip.addr.isInRange(..)`
   as an extra assumption of that one evaluation, in the request environment whose
   resource type is `Pod`. No `is Pod` guard is needed, and an `IpEndpoint` environment
   is never given it.

The entity store itself is assumed too (open-world: known addresses participate, the
symbolic peer stays unconstrained). The request is deliberately *not* assumed:
assumptions persist across evaluations and one query's two directions differ in action;
partial evaluation has already folded the concrete request into the residuals anyway.

**Verdict collapse**, sound because the compiler emits permits only (the default-allow is a
catch-all `permit ... unless`, never a `forbid`): per direction with verdict `UNKNOWN`,
each residual permit's condition is evaluated under the assumptions.

- Some condition is always true → that permit applies on every admissible completion →
  `ALLOW`.
- No condition can be true (an erroring permit never applies) → `DENY`.
- Otherwise still `UNKNOWN`, with never-true residuals pruned from the output and named.

`UnsatisfiableAssumptions` (a `--pod-cidr` contradicting addresses the store pins) is a
user-facing error, never a verdict. A residual the evaluator cannot re-typecheck is kept
unknown with a warning: collapsing it would be unsound, keeping it is merely imprecise.

## Code

- `src/eval.rs` stays TPE-only; `Outcome` gains each residual's bare condition
  (`residual_conditions`) and the request environment (`env`), so the symbolic layer
  re-evaluates exactly what partial evaluation left over.
- New `src/symbolic.rs`: `Discharger::new(entities, pod_cidrs)` spawns cvc5 inside a
  current-thread tokio runtime (`LocalSolver::cvc5` spawns through tokio and needs the
  reactor even from synchronous code), registers the assumptions, and
  `refine(policies, outcome, symbolic_resource)` applies the collapse rules and returns
  the pruned rules' descriptions. `parse_cidr` validates `address/prefix` for clap, and
  the library entry point re-checks its inputs because the strings are spliced into
  generated Cedar.
- `src/main.rs`: a repeatable `--pod-cidr` on `check` and `reachable`. `check` spawns the
  solver only when a direction is `UNKNOWN`; `reachable` prints pruned rules and possibly
  collapsed verdicts.
- `examples/smt_inspect.rs`: a wrapping `Solver` that tees the SMT-LIB script the
  compiler writes, for reading what is actually sent to cvc5.

## Verification

`tests/symbolic.rs`, on the `unknown-pod-ip` fixture (`gateway` egress-isolated by
`0.0.0.0/0 except 10.0.0.0/8`; `mystery` with no recorded address):

- `10.244.0.0/16` (inside the exception) collapses egress to `DENY`;
- `172.16.0.0/12` (outside) collapses to `ALLOW`;
- `10.0.0.0/7` (straddling) stays `UNKNOWN` — the evaluator is exact, not heuristic;
- dual-stack `10.244.0.0/16` + `fd00::/8` still denies (`isInRange` across families is
  false, as in Kubernetes);
- the `reachable` shape prunes the internet-only rule and collapses to `DENY`.

These run by default and fail loudly when cvc5 is missing; a silently skipped solver test
would let a broken setup look green. End to end: the three `check` invocations above exit
1, 0 and 2 respectively, and `reachable ... --pod-cidr 10.244.0.0/16` names the pruned
rule.
