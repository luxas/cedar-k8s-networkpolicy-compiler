# Follow-ups: public-2-cluster

## PR description

Point np2cedar at a running cluster. Every command that took `-f` now also takes
`--cluster`, which lists NetworkPolicies, Pods and Namespaces straight from the Kubernetes
API, so a question about a live cluster needs no intermediate files:

```sh
hack/kind-up.sh                       # a kind cluster with tests/data/cluster applied
export KUBECONFIG=$PWD/.kube/config
np2cedar check --cluster --from np2cedar-a/web --to np2cedar-a/api --port 8080
np2cedar check --cluster --from np2cedar-a/crosser --to np2cedar-b/back --port 80
```

```
ingress  np2cedar-a/web -> np2cedar-a/api  TCP:8080  ALLOW  (np2cedar-a/allow-web-to-api ingress[0])
egress   np2cedar-a/web -> np2cedar-a/api  TCP:8080  ALLOW  (catch-all egress (not isolated by any NetworkPolicy))
=> ALLOWED
ingress  np2cedar-a/crosser -> np2cedar-b/back  TCP:80  DENY
egress   np2cedar-a/crosser -> np2cedar-b/back  TCP:80  ALLOW  (np2cedar-a/crosser-egress egress[0])
=> DENIED (blocked on ingress)
```

`-n` narrows the pods, never the policies — a policy elsewhere still isolates the pods it
selects, and reading a subset would answer "allowed" for traffic that is blocked.
Terminal-phase pods are skipped, host-network pods are flagged. The client is a default-on
`cluster` feature; `cargo build --no-default-features` keeps the pure offline compiler.
`hack/kind-up.sh` stands up a kind cluster with a fixture, and the live-cluster tests assert
that compiling the same policies from the cluster and from the file gives byte-identical
Cedar, which pins this repo's reading of the API contract to the API server's own
defaulting.

## Review findings

1. **A bare `--context` without `--kubeconfig` ignores `$KUBECONFIG`'s multi-file form
   only partially.** `Config::from_kubeconfig` honours `$KUBECONFIG`, but the error
   message on failure says "loading kubeconfig context" without the path list. Include
   the resolved path(s) in the error so a wrong `KUBECONFIG` is obvious.
2. **`PAGE_SIZE` is a constant.** Very large clusters may want a smaller page to bound
   memory and a larger one to bound round-trips; expose `--page-size` or an env override
   in `cluster::Options`.
3. **Warnings are printed to stderr only from `main.rs`.** Library users of
   `cluster::fetch` get them in `Fetched.warnings`, which is right, but `tests/cluster.rs`
   never asserts on the warnings (for example that a terminated pod in the fixture is
   reported). Add a `Succeeded` pod to the fixture and assert the warning.
4. **The host-network warning fires on every run for clusters with node agents.**
   That is correct but noisy on real clusters; consider summarising by count with
   `-v` for the full list.
5. **`only_fixture` in `tests/cluster.rs` duplicates the namespace-ownership filter
   three times.** A small `Loaded::retain_namespaces(&[&str])` helper in the library
   would serve the tests and users who want to scope after fetching.
6. **The integration job depends on the kind node image tag matching `k8s-openapi`'s
   `v1_34` feature.** Nothing checks this; a comment in `hack/lib.sh` tying
   `KIND_NODE_IMAGE` to the `k8s-openapi` feature (and a test that the server's
   version is 1.34) would stop them drifting apart silently.
7. **`--selector` is passed to the API but not documented as a Kubernetes label
   selector syntax.** The live-cluster guide's table should say it is the same syntax as
   `kubectl get pods -l`.
