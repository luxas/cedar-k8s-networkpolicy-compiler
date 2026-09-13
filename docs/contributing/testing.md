# Testing

What each suite establishes, and the conventions that keep the suite honest. The
architecture the tests pin down is in [architecture](architecture.md).

```sh
cargo test                            # unit + snapshot + semantics + cross-check
cargo insta review                    # accept changed snapshots
```

## The suites

- **`tests/compile_snapshots.rs`** pins the generated `.cedar` and entity JSON for the
  fixtures in `tests/data/` (insta snapshots).
- **`tests/semantics.rs`** holds truth tables **derived by hand** from the Kubernetes
  semantics. They are *not* snapshots: if a compiler change alters behaviour they must
  fail, and the resolution is to fix the compiler or argue the table wrong — never to
  re-record silently.
- **`tests/handwritten.rs`** cross-checks the compiler against
  `examples/handwritten/policies.cedar`, an independent hand translation of the same
  NetworkPolicy, over the whole bookinfo grid.

Everything above checks the tool against *a reading* of the API contract — tables and
translations produced by the same hands. That is a real check on the compiler's
consistency, not on whether that reading matches what a cluster does to packets.

Neighbours: [architecture](architecture.md) ·
[translation rules](../concepts/translation-rules.md)
