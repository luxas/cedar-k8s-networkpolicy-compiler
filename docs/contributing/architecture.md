# Architecture

The code map for contributors: what lives where and how data flows. The behavioural
promises the code must keep are pinned by [the test suite](testing.md); the design
rationale, decision by decision, is in `docs/plans/`.

## Data flow

```
                       load.rs (YAML/JSON files)
                                       │
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

`main.rs` is the clap CLI over all of it; `lib.rs` exposes the same modules as a library.

Module notes:

- **`compile.rs`** renders commented Cedar *text* and re-parses it (the comments are the
  reviewability; Cedar's programmatic syntax tree carries none — a recorded follow-up).
  `Direction` is the single source of the action names and of which endpoint a policy
  selects.
- **`expr.rs`** is the small expression AST behind that text: its constructors simplify
  as they build (constants absorbed, nesting flattened) so the output never reads
  `true && true && ...`, and its renderer hoists each clause's provenance comment above
  the clause.
- **`eval.rs`** is TPE-only; it exposes each outcome's verdict, its reasons mapped back to
  Kubernetes objects through the policy annotations, and the residual policies.

Neighbours: [testing](testing.md) · [the encoding](../concepts/encoding.md)
