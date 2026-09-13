#!/usr/bin/env bash
# Stand up the local e2e cluster and load the test fixture into it.
# Idempotent: safe to re-run against a cluster that is already up.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

require docker
require kubectl
ensure_kind

if kind get clusters 2>/dev/null | grep -qx "$CLUSTER_NAME"; then
    log "cluster $CLUSTER_NAME already exists"
else
    log "creating cluster $CLUSTER_NAME"
    kind create cluster --name "$CLUSTER_NAME" --image "$KIND_NODE_IMAGE"
fi

log "writing kubeconfig to $KUBECONFIG"
mkdir -p "$(dirname "$KUBECONFIG")"
kind export kubeconfig --name "$CLUSTER_NAME" --kubeconfig "$KUBECONFIG"
chmod 600 "$KUBECONFIG"

# The namespaces owned by tests/data/cluster/manifests.yaml.
FIXTURE_NAMESPACES=(np2cedar-a np2cedar-b)

log "creating namespaces"
for ns in "${FIXTURE_NAMESPACES[@]}"; do
    kubectl get namespace "$ns" >/dev/null 2>&1 || kubectl create namespace "$ns" --save-config
done

# A namespace's `default` ServiceAccount is created asynchronously by the
# controller-manager, and the API server refuses to admit a Pod that has none. On a
# cold cluster the apply below loses that race, so wait for it rather than fail with
# a confusing "serviceaccount default not found".
log "waiting for the default service accounts"
for ns in "${FIXTURE_NAMESPACES[@]}"; do
    kubectl wait --namespace "$ns" --for=create serviceaccount/default --timeout=120s
done

log "applying tests/data/cluster/manifests.yaml"
kubectl apply -f "$REPO_ROOT/tests/data/cluster/manifests.yaml"

# The tests assert that every pod has a concrete address, which is only true once
# the pods are actually running.
log "waiting for pods to be ready"
for ns in "${FIXTURE_NAMESPACES[@]}"; do
    kubectl wait --namespace "$ns" --for=condition=Ready pod --all --timeout=180s
done

log "cluster ready; export KUBECONFIG=$KUBECONFIG"
