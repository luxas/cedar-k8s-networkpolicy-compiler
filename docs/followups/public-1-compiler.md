# Follow-ups: public-1-compiler

## PR description

Compile Kubernetes `NetworkPolicy` objects into Cedar and ask questions about them.
`np2cedar compile` turns a set of policies into a readable, annotated Cedar policy set
that encodes the two-sided, additive, default-allow-until-isolated semantics of
NetworkPolicy; `np2cedar entities` builds the matching entity store from `Pod` and
`Namespace` objects; `np2cedar check` decides one connection by ANDing both directions,
and `np2cedar reachable` leaves the peer symbolic and prints the residual policies that
describe what a pod may reach. When the answer depends on something the manifests never
stated — a pod with no recorded address — the tool answers `UNKNOWN` and prints exactly
what is missing, rather than guessing.

```sh
np2cedar compile  -f examples/bookinfo -o policies.cedar
np2cedar entities -f examples/bookinfo -o entities.json
np2cedar check    --policies policies.cedar --entities entities.json \
                  --from default/productpage --to default/details --port 9080
```

```
ingress  default/productpage -> default/details  TCP:9080  ALLOW  (default/details-allow-productpage ingress[0])
egress   default/productpage -> default/details  TCP:9080  ALLOW  (catch-all egress (not isolated by any NetworkPolicy))
=> ALLOWED
```

This is the foundation everything later builds on: the compiled Cedar is the intermediate
representation that generic Cedar tooling — partial evaluation here, symbolic evaluation
and policy synthesis later — can reason about.

## Review findings

1. **Replayed policy files are parsed but not validated.** `--policies P` goes through
   `read_policies` in `src/main.rs`, which only parses. A hand-edited or stale `.cedar`
   file that no longer matches the bundled schema would be evaluated anyway, and TPE's
   behaviour on ill-typed policies is not what a user expects. Validate replayed policy
   sets with `ValidationMode::Strict` against `schema()` as `compile` does, and fail with
   a clear error naming the file.
2. **Port numbers are not range-checked.** `port_expr` accepts any `i32` for `port` and
   `endPort`; `check --port` accepts any `i64`. Kubernetes rejects ports outside
   1–65535. Reject them at compile time and at the CLI, so a typo surfaces as an error
   rather than as a policy that can never match.
3. **Named-port collisions across containers are only caught when the numbers differ.**
   `entities::named_ports` errors when one name maps to two different container ports
   but silently merges init-container and regular-container ports into one map. That is
   the right model for NetworkPolicy, but the doc comment should say so and a fixture
   should pin it.
4. **The CLI has no end-to-end test.** Exit codes 0/1/2/3 and the exact line format of
   `check` are documented and script-facing, but only the library layer is tested. Add a
   test that runs the built binary over `examples/bookinfo` and `tests/data/unknown-pod-ip`
   and asserts the exit code and the `=>` line.
5. **`reachable` with a bare-IP intent has no form.** `check --to` accepts an IP, but
   `reachable` can only leave a `Pod` symbolic; there is no way to ask "which external
   addresses may this pod reach?" with an `IpEndpoint` peer. Consider a `--to-kind`
   switch or a second symbolic pass over `IpEndpoint`.
6. **The `Commented` wrapper is dropped on constants** (`Expr::commented`), which is right
   for output but means an `ipBlock` rule whose peers all simplify away loses its
   provenance comment entirely. Low priority; worth a note in `expr.rs`.
7. **`Direction::selected`/`peer` return variable names as strings**, and `rule_policy`
   compares them with `==` to decide the `is Pod` guards. An enum for the two request
   variables would make this a match instead of string comparison.
8. **Snapshots and the semantics tables use the same fixtures.** That is fine, but the
   snapshots' `.entities` files are large and mostly repetitive JSON; a compact
   representation (one entity per line) would make snapshot diffs reviewable.
