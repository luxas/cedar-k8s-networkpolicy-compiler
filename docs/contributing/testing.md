# Testing

What each suite establishes, and the conventions that keep the suite honest. The
architecture the tests pin down is in [architecture](architecture.md).

```sh
cargo test                            # no cluster needed: unit + snapshot + semantics + cross-check + solver
cargo test --no-default-features      # the offline build, without the client
cargo test --features integration     # adds the live-cluster tests
hack/integration-test.sh              # ... including standing the cluster up first
hack/e2e-test.sh                      # empirical, one-shot: Cilium enforces, kubesonde probes, teardown
hack/e2e-up.sh                        # ... or stand the Cilium cluster up once, and then
KUBECONFIG=.kube/e2e-config \
  cargo test --features e2e           # ... compare against it repeatedly while iterating
cargo insta review                    # accept changed snapshots
```

"No cluster needed" is not "needs nothing": the default `cargo test` drives cvc5 (fail-loud,
below), and the first build fetches the cedar fork from its pinned commit
([architecture](architecture.md#the-cedar-fork)).

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
- **`tests/symbolic.rs`** and **`tests/connect.rs`** drive the solver-backed features
  (`--pod-cidr` discharge, connect synthesis) against fixtures, snapshot the synthesized
  output, and assert every escalation is sound and every synthesized permit
  strict-validates against the bundled schema. They run by default and **fail loudly
  when cvc5 is missing** — a silently skipped solver test would let a broken setup look
  green, so there is deliberately no skip path.

## The live-cluster suite

**`tests/cluster.rs`** (feature `integration`) needs a real API server and **fails
rather than skips** when the cluster is unreachable. Its most interesting assertion:
compiling the same NetworkPolicies from the cluster and from the file produces
byte-identical Cedar. The API server applies its own defaulting (`policyTypes`,
`protocol`, `podSelector`) on write, so that only holds while this repo's reading of the
API contract matches Kubernetes' own — if it ever fails, the divergence is the finding.

## Empirical validation, and its limits

Everything above checks the tool against *a reading* of the API contract — tables and
translations produced by the same hands. **`tests/e2e.rs`** (feature `e2e`) checks
against reality: a kind cluster running **Cilium** enforces
`tests/data/e2e/manifests.yaml`, [kubesonde](https://github.com/kubesonde/kubesonde)
injects nmap ephemeral containers and probes every pod pair, and every observed
Allow/Deny must match np2cedar's verdict. The fixture is chosen so the observed matrix
contains allows, an ingress-caused denial, and — the interesting one — an *egress*-caused
denial, which exercises the two-action conjunction against a dataplane rather than
against the author's reasoning.

Being precise about what that establishes:

- It validates against **Cilium** — one implementation, not the specification. A failure
  means one of np2cedar, kubesonde or Cilium is wrong: investigate, never route around.
- **TCP only** (nmap reports UDP as `open|filtered`, which decides nothing), **pod to
  pod, one namespace**, and **only where a listener exists** — every fixture pod serves
  every port it declares, which is what makes an observed `Deny` mean *policy*.
- The test refuses to pass vacuously: all twelve ordered pairs must have been probed and
  both outcomes must occur, so an empty probe run or an all-deny cluster fails rather
  than quietly agreeing.

Neighbours: [architecture](architecture.md) ·
[translation rules](../concepts/translation-rules.md)
