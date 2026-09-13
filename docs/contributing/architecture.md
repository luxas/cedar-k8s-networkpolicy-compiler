# Architecture

The code map for contributors: what lives where, how data flows, and the dependency
story. The behavioural promises the code must keep are pinned by
[the test suite](testing.md); the design rationale, decision by decision, is in
`docs/plans/`.

## Data flow

```
             load.rs (YAML/JSON files)         cluster.rs (live API, feature "cluster")
                        └──────────────┬──────────────┘
                             Loaded { network_policies, pods, namespaces }
                                       │
              ┌────────────────────────┴─────────────────────┐
        compile.rs (+ expr.rs, peer.rs,                 entities.rs
         port.rs, selector.rs)                                │
              │  Cedar text → parse → strict validate         │
              ▼                                               ▼
        PolicySet (annotated permits)               entity JSON (+ unknown IpAddrs)
              │                                               │
              └───────────────┬───────────────────────────────┘
                              ▼
                eval.rs — TPE per direction: Verdict + residual conditions
                              │
                              ▼
                symbolic.rs — Discharger: assumptions + cvc5 refine
                              (--pod-cidr on check/reachable)
```

`main.rs` is the clap CLI over all of it; `lib.rs` exposes the same modules as a library
(`cargo build --no-default-features` drops the Kubernetes client entirely).

Module notes:

- **`compile.rs`** renders commented Cedar *text* and re-parses it (the comments are the
  reviewability; Cedar's programmatic syntax tree carries none — a recorded follow-up).
  `Direction` is the single source of the action names and of which endpoint a policy
  selects.
- **`expr.rs`** is the small expression AST behind that text: its constructors simplify
  as they build (constants absorbed, nesting flattened) so the output never reads
  `true && true && ...`, and its renderer hoists each clause's provenance comment above
  the clause.
- **`cluster.rs`** returns the same `Loaded` the file loader does, so nothing downstream
  knows where the objects came from. It is synchronous from the outside: the tokio
  runtime lives and dies inside `fetch`. Pagination (`drain`) is kept free of `kube`
  types so the loop that decides when to stop is unit-tested without a cluster.
- **`eval.rs`** is TPE-only; it exposes each outcome's verdict, its reasons mapped back to
  Kubernetes objects through the policy annotations, the residual policies, and their
  bare conditions plus request environment so the symbolic layer can re-evaluate them
  without re-deriving anything.
- **`symbolic.rs`** owns the solver bootstrap (a current-thread tokio runtime — the
  solver spawn needs the reactor even from synchronous code) and the discharge rules.

## The cedar fork

The symbolic evaluator is unpublished: the crates.io `cedar-policy-symcc` release predates
it. `Cargo.toml` therefore takes `cedar-policy-symcc` from the
[cedar-woodpecker](https://github.com/luxas/cedar-woodpecker) repository — the Cedar
workspace plus the evaluator — as a git dependency pinned to a commit, with
`[patch.crates-io]` pointing `cedar-policy` and `cedar-policy-core` at the same commit.
The patch is mandatory: the fork's crates pin each other by path, and the type bridges
between the public TPE wrappers and the core types only line up when the whole graph
resolves to a single instance of each crate (`cargo tree -i cedar-policy-core` must show
exactly one node).

The pin is deliberate: tests byte-encode the evaluator's output, so a moving tip would
break unrelated changes — or silently re-record changed semantics. Bump the `rev` in
`Cargo.toml` together with the affected snapshots, in one commit. cvc5 is installed in
every CI job that runs `cargo test`, because the solver-backed suites run by default and
fail loudly without it — by design.

Neighbours: [testing](testing.md) · [the encoding](../concepts/encoding.md)
