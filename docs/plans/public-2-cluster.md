# Plan: read NetworkPolicies, Pods and Namespaces from a live cluster

## Context

Every input so far comes from YAML on disk, which makes the tool an offline exercise
rather than something usable against the thing it is meant to analyse. This feature adds
a live source — NetworkPolicies, Pods and Namespaces straight from the Kubernetes API —
so `np2cedar check --cluster --from a/b --to c/d --port 80` answers a question about the
cluster you are pointed at, with no intermediate files.

The client goes behind a default-on Cargo feature so the offline compiler stays free of
an HTTP and TLS stack. The feature also adds what keeps it honest: setup scripts shared by
a developer laptop and CI, and a CI job that stands up a kind cluster and runs the live
tests against it.

## Dependencies and the `cluster` feature

```toml
[features]
default     = ["cluster"]
cluster     = ["dep:kube", "dep:tokio"]
integration = ["cluster"]          # opts the live-cluster tests in

[dependencies]
kube  = { version = "4.2", optional = true }   # default features: client, rustls-tls, ring
tokio = { version = "1", features = ["rt"], optional = true }
```

`kube` 4.2 pairs with the `k8s-openapi ^0.28` already pinned. Its default features are
the right set; `runtime` (watchers, reflectors) is not needed for list calls. `main` stays
synchronous: `cluster::fetch` builds a current-thread tokio runtime and blocks on it.

## `src/cluster.rs`

Returns the same `load::Loaded { network_policies, pods, namespaces }` the file loader
produces, so `compile`, `entities::build` and `eval` are untouched.

- **Client.** An explicit `--kubeconfig` is read without touching `$KUBECONFIG`;
  `--context` selects a context; with neither, `Config::infer()` tries the kubeconfig and
  then the in-cluster service account, so the same binary works from a laptop and from a
  pod.
- **Scoping.** `--namespace` restricts only the Pod listing. NetworkPolicies and
  Namespaces are always read cluster-wide: a policy in another namespace still isolates
  the pods it selects, so a partial read would silently turn "isolated" into "not
  isolated" — wrong in the direction that lets traffic through unnoticed. A refused
  namespace list degrades to synthesising namespaces from pod metadata, with a warning; a
  refused pod or policy list is fatal.
- **Pagination.** `drain` follows `continue` tokens until the server stops handing them
  out, treating an empty token as the end (some servers send one). It is kept free of
  `kube` types so it is unit-tested without a cluster.
- **Pod filtering.** `Succeeded`/`Failed` pods are skipped by default (their recorded
  address has most likely been reassigned; `--include-terminated` keeps them). `Pending`
  pods are kept: no address yet is *unknown*, which the residual machinery already
  represents. Host-network pods are kept but warned about.

## CLI

`--cluster` joins `-f` as a source on every command, and `check`/`reachable` also accept
`-f` directly, so offline and live are symmetric:

```
np2cedar compile   (-f <path|dir|->... | --cluster)  [-o policies.cedar]
np2cedar entities  (-f <path|dir|->... | --cluster)  [-o entities.json]
np2cedar check     (-f ... | --cluster | --policies P --entities E) --from .. --to .. --port N
np2cedar reachable (-f ... | --cluster | --policies P --entities E) --from ..
```

A clap `ArgGroup` makes exactly one source mandatory. The cluster flags (`--kubeconfig`,
`--context`, `-n`, `-l`, `--include-terminated`) are registered even without the feature,
so the argument graph is identical in both builds and `--cluster` explains itself rather
than vanishing; they are rejected by hand when passed without `--cluster`, because clap's
`requires` cannot express a dependency on a boolean flag.

Two behaviours found by testing rather than reasoning: `entities::build` must sort pods
by `(namespace, name)`, since a server-ordered listing would otherwise make the same
cluster produce two different entity files; and the binary is now actually named
`np2cedar`.

## Scripts and CI

One code path for laptop and CI, so a green CI run means the scripts a developer uses
actually work:

| Script | Does |
| --- | --- |
| `hack/kind-up.sh` | installs `kind` if absent, creates cluster `np2cedar` if absent, writes a repo-local gitignored `.kube/config`, applies `tests/data/cluster/manifests.yaml`, waits for the pods to be Ready. Idempotent. |
| `hack/kind-down.sh` | deletes the cluster and the kubeconfig |
| `hack/integration-test.sh` | `kind-up`, `cargo test --features integration`, `kind-down` (`KEEP_CLUSTER=1` to keep it) |

The repo-local kubeconfig means running the suite never retargets the developer's real
`kubectl` context, and a cluster credential cannot be swept into a commit. A fresh
namespace has no `default` ServiceAccount for a moment, so `kind-up.sh` waits for it
before applying pods.

CI: the `test` job adds `cargo test --no-default-features`; a new `integration` job runs
`hack/kind-up.sh`, `cargo test --features integration`, dumps cluster state on failure,
and tears down.

## Verification

`tests/cluster.rs` (feature `integration`) runs and **fails** — never skips — when the
cluster is unreachable:

1. the fixture's pods, policies and namespaces come back and compile;
2. every Ready pod has a concrete address — the one thing no file-based test can show;
3. a hand-derived truth table over the live cluster, including the cross-namespace
   conjunction and the API server's defaulting of `protocol` and `policyTypes`;
4. compiling the same NetworkPolicies from the cluster and from the file yields
   byte-identical Cedar, so our defaulting is asserted to match the API server's;
5. `-n` trims the pods but never the policies.

Unit tests without a cluster: `drain` over token chains, empty tokens, empty middle pages
and mid-chain errors; terminal-phase and host-network detection; the clap argument graph,
including the by-hand rejection of cluster flags without `--cluster`.
