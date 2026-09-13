# Follow-ups: public-4-symbolic-discharge

## PR description

Turn most `UNKNOWN` verdicts back into answers. An unknown pod address is not arbitrary —
it lies in the cluster's pod CIDR — and the new repeatable `--pod-cidr` flag on `check`
and `reachable` hands exactly that constraint to a symbolic evaluator backed by cvc5.
Residual rules that cannot match any pod address are pruned, and when that settles a
direction the verdict collapses, exactly rather than heuristically:

```sh
np2cedar check -f tests/data/unknown-pod-ip --from default/gateway --to default/mystery \
               --port 443 --pod-cidr 10.244.0.0/16
```

```
note: no status.podIPs for default/mystery; their addresses are left unknown, so ipBlock rules over them evaluate to a residual rather than a decision
note: default/egress-to-internet egress[0] can never match a pod address in 10.244.0.0/16
ingress  default/gateway -> default/mystery  TCP:443  ALLOW  (catch-all ingress (not isolated by any NetworkPolicy))
egress   default/gateway -> default/mystery  TCP:443  DENY  (default/egress-to-internet egress[0], no residual rule can match a pod address in 10.244.0.0/16)
=> DENIED (blocked on egress)
```

With `--pod-cidr 172.16.0.0/12` the same query is `ALLOWED`; with `10.0.0.0/7`, which
straddles the rule's exception, it honestly stays `UNKNOWN`. This is the first use of
Cedar's symbolic machinery on the compiled policies: the same SMT encoding later drives
the synthesis of effective connectivity. It also brings in the pinned cedar-woodpecker
fork as a git dependency, since the evaluator is unpublished.

## Review findings

1. **The discharger spawns one cvc5 per `Discharger`, and `check` creates one per
   query.** Fine interactively; a batch caller evaluating many pairs would want to reuse
   the discharger across outcomes, which the API allows but `main.rs` does not exercise.
   Document the reuse pattern, or add a `check` mode over many pairs.
2. **`refine` clears `residual_conditions` on collapse but keeps `reasons` growing.** The
   appended "allows every pod address in ..." reason is right, but on `ALLOW` the original
   `describe(id)` reason is now listed twice (once from the residual, once from the
   collapse note), as the quickstart output shows. Dedup or reword.
3. **Errors from a residual that cannot be typechecked go to stderr from the library.**
   `symbolic.rs` uses `eprintln!` for the "kept unknown" warning; library users get no
   programmatic signal. Return the warnings alongside the pruned list, as
   `cluster::Fetched` does.
4. **The example duplicates a small `Solver` wrapper** to see the script; a `tee`
   constructor on `LocalSolver` upstream would make the example three lines and would
   also capture what the solver emits on its own behalf. Worth proposing to the fork.
5. **The CIDR assumption uses `||` over CIDRs but `--pod-cidr` accepts duplicates and
   overlaps silently.** Harmless for the solver; a warning on overlapping CIDRs would
   catch a typo like passing the same range twice with a wrong prefix.
6. **`Outcome` now carries core-crate types (`cedar_policy_core::ast::Expr`) in its public
   API**, which leaks the fork's version pin to library users. A newtype or an opaque
   handle would keep the public surface on `cedar_policy` only.
7. **The fork pin has no automated drift check.** `cargo tree -i cedar-policy-core` must
   show one node; a CI step asserting that would catch a future patch-section mistake
   before the type-bridge errors do.
