# Architecture

The code map for contributors: what lives where and how data flows. The behavioural
promises the code must keep are pinned by [the test suite](testing.md); the design
rationale, decision by decision, is in `docs/plans/`.

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
  Kubernetes objects through the policy annotations, and the residual policies.

Neighbours: [testing](testing.md) · [the encoding](../concepts/encoding.md)
