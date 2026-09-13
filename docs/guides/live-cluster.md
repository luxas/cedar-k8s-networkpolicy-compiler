# Reading a live cluster: `--cluster`

Every command that takes `-f` also takes `--cluster`, which lists NetworkPolicies, Pods
and Namespaces straight from the Kubernetes API — no intermediate files needed:

```sh
np2cedar compile   --cluster -o policies.cedar
np2cedar check     --cluster --from default/web --to default/api --port 8080
np2cedar reachable --cluster --from default/web
```

| Flag | |
| --- | --- |
| `--kubeconfig <PATH>` | an explicit kubeconfig (default: `$KUBECONFIG`, then the usual place) |
| `--context <CTX>` | kubeconfig context (default: the current one) |
| `-n, --namespace <NS>` | restrict the **pods** fetched; repeatable |
| `-l, --selector <SEL>` | label selector on the pod listing |
| `--include-terminated` | keep `Succeeded`/`Failed` pods |

With no kubeconfig at all it falls back to the in-cluster service account, so the same
binary works from inside a pod.

## The choices that keep the answer sound

**`--namespace` narrows the pods, never the policies.** NetworkPolicies and Namespaces
are always read cluster-wide. A policy in another namespace still isolates the pods it
selects, so reading a subset would silently turn "isolated" into "not isolated" — wrong
in the one direction that lets traffic through unnoticed. If the API server refuses the
cluster-wide namespace read, the tool warns and synthesises namespaces from pod metadata,
which is enough unless a policy selects on some namespace label other than
`kubernetes.io/metadata.name`.

**Pods in a terminal phase are skipped.** A `Succeeded` or `Failed` pod is gone and its
recorded address has most likely been handed to another pod; keeping it would answer
questions about traffic that cannot happen. `--include-terminated` keeps them anyway.
`Pending` pods *are* kept: "no address yet" is unknown rather than absent, and the
[residual machinery](../concepts/partial-evaluation.md) says so correctly.

**Host-network pods are kept but warned about.** Their address is their node's, and
NetworkPolicy's pod-level semantics do not apply to them the way this model assumes.

## RBAC

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: np2cedar
rules:
  - apiGroups: [""]
    resources: [pods, namespaces]
    verbs: [list]
  - apiGroups: [networking.k8s.io]
    resources: [networkpolicies]
    verbs: [list]
```

## Building without the client

The Kubernetes client is a default-on `cluster` Cargo feature. Turning it off leaves a
pure offline compiler with no HTTP or TLS stack — what you want when embedding the
library:

```sh
cargo build --no-default-features
```

The `--cluster` flag still exists in that build, and explains itself rather than
vanishing.

## A local cluster for development

```sh
hack/kind-up.sh            # create the kind cluster, apply tests/data/cluster
hack/integration-test.sh   # ... run the live tests, then tear it down
hack/kind-down.sh
```

`hack/kind-up.sh` installs `kind` if missing and writes a repo-local, gitignored
`.kube/config`, so running the suite never retargets your real `kubectl` context. CI runs
the same `kind-up`/`kind-down` scripts around its own `cargo test --features integration`
step; the `hack/integration-test.sh` wrapper is the local convenience, not what CI calls —
see [testing](../contributing/testing.md).

Neighbours: [quickstart](../quickstart.md) · [CLI reference](../reference/cli.md)
