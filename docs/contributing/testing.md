# Testing

What each suite establishes, and the conventions that keep the suite honest. The
architecture the tests pin down is in [architecture](architecture.md).

```sh
cargo test                            # no cluster needed: unit + snapshot + semantics + cross-check
cargo test --no-default-features      # the offline build, without the client
cargo test --features integration     # adds the live-cluster tests
hack/integration-test.sh              # ... including standing the cluster up first
cargo insta review                    # accept changed snapshots
```

## The cluster-free suites

- **`tests/compile_snapshots.rs`** pins the generated `.cedar` and entity JSON for the
  fixtures in `tests/data/` (insta snapshots).
- **`tests/semantics.rs`** holds truth tables **derived by hand** from the Kubernetes
  semantics. They are *not* snapshots: if a compiler change alters behaviour they must
  fail, and the resolution is to fix the compiler or argue the table wrong — never to
  re-record silently.
- **`tests/handwritten.rs`** cross-checks the compiler against
  `examples/handwritten/policies.cedar`, an independent hand translation of the same
  NetworkPolicy, over the whole bookinfo grid.

## The live-cluster suite

**`tests/cluster.rs`** (feature `integration`) needs a real API server and **fails
rather than skips** when the cluster is unreachable. Its most interesting assertion:
compiling the same NetworkPolicies from the cluster and from the file produces
byte-identical Cedar. The API server applies its own defaulting (`policyTypes`,
`protocol`, `podSelector`) on write, so that only holds while this repo's reading of the
API contract matches Kubernetes' own — if it ever fails, the divergence is the finding.

Everything above checks the tool against *a reading* of the API contract — tables and
translations produced by the same hands, plus the API server's agreement on defaulting.
That is a real check on the compiler's consistency, not on whether that reading matches
what a cluster's network plugin does to packets.

Neighbours: [architecture](architecture.md) ·
[translation rules](../concepts/translation-rules.md)
