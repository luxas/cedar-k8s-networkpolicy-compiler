#!/usr/bin/env bash
# Stand up the empirical e2e cluster: kind without a CNI, Cilium as the CNI,
# kubesonde to probe, the fixture to probe, and the probe results on disk.
#
# Idempotent, and slow the first time -- Cilium and the probe images are large.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

require docker
require kubectl
require curl
ensure_kind
ensure_cilium_cli

# Deliberately not the integration cluster's kubeconfig: the two must not clobber
# each other, and neither should touch the developer's real one.
export KUBECONFIG="$E2E_KUBECONFIG"
mkdir -p "$(dirname "$E2E_KUBECONFIG")" "$(dirname "$E2E_PROBES")"

if kind get clusters 2>/dev/null | grep -qx "$E2E_CLUSTER_NAME"; then
    log "cluster $E2E_CLUSTER_NAME already exists"
else
    log "creating cluster $E2E_CLUSTER_NAME (no CNI; Cilium follows)"
    kind create cluster --name "$E2E_CLUSTER_NAME" \
        --image "$KIND_NODE_IMAGE" \
        --config "$REPO_ROOT/hack/kind-cilium.yaml"
fi
kind export kubeconfig --name "$E2E_CLUSTER_NAME" --kubeconfig "$E2E_KUBECONFIG"
chmod 600 "$E2E_KUBECONFIG"

if kubectl -n kube-system get daemonset cilium >/dev/null 2>&1; then
    log "cilium already installed"
else
    log "installing cilium $CILIUM_VERSION"
    cilium install --version "$CILIUM_VERSION"
fi
log "waiting for cilium"
cilium status --wait

log "installing kubesonde $KUBESONDE_VERSION"
# Upstream's manifest pins the controller to :latest, which would make a run
# irreproducible; pin it to the same tag as the manifest itself.
curl -fsSL "https://raw.githubusercontent.com/kubesonde/kubesonde/${KUBESONDE_VERSION}/kubesonde.yaml" \
    | sed "s|ghcr.io/kubesonde/controller:latest|ghcr.io/kubesonde/controller:${KUBESONDE_VERSION}|" \
    | kubectl apply -f -
kubectl -n kubesonde-system rollout status deployment/kubesonde-controller-manager --timeout=300s

log "applying tests/data/e2e/manifests.yaml"
# As in kind-up.sh: a fresh namespace has no `default` ServiceAccount for a moment,
# and a Pod cannot be admitted without one.
kubectl get namespace "$E2E_NAMESPACE" >/dev/null 2>&1 \
    || kubectl create namespace "$E2E_NAMESPACE" --save-config
kubectl wait --namespace "$E2E_NAMESPACE" --for=create serviceaccount/default --timeout=120s
kubectl apply -f "$REPO_ROOT/tests/data/e2e/manifests.yaml"

# Every declared port must have a live listener before probing starts, or a Deny
# would mean "nothing is listening" rather than "policy said no".
log "waiting for the fixture pods to serve"
kubectl wait --namespace "$E2E_NAMESPACE" --for=condition=Ready pod --all --timeout=300s

log "asking kubesonde to probe $E2E_NAMESPACE"
kubectl apply -f - <<EOF
apiVersion: security.kubesonde.io/v1
kind: Kubesonde
metadata:
  name: np2cedar
  namespace: "$E2E_NAMESPACE"
spec:
  namespace: "$E2E_NAMESPACE"
  probe: all
  monitorImage: "$KUBESONDE_MONITOR_IMAGE"
EOF

log "port-forwarding to the kubesonde controller"
kubectl -n kubesonde-system port-forward deployment/kubesonde-controller-manager 2709:2709 \
    >/dev/null 2>&1 &
PF_PID=$!
cleanup() { kill "$PF_PID" 2>/dev/null || true; }
trap cleanup EXIT

for _ in $(seq 1 30); do
    curl -sf "http://localhost:2709/probes" -o /dev/null 2>/dev/null && break
    sleep 2
done
curl -sf "http://localhost:2709/probes" -o /dev/null 2>/dev/null \
    || die "the kubesonde probes endpoint never became reachable"

# Probing is asynchronous with no completion signal, so settle for "the result stopped
# changing": a non-empty item count, unchanged across consecutive polls.
log "waiting for the probe results to settle"
previous=-1
stable=0
for attempt in $(seq 1 60); do
    if curl -sf "http://localhost:2709/probes" -o "${E2E_PROBES}.tmp" 2>/dev/null; then
        count=$(python3 -c "import json,sys; print(len((json.load(open(sys.argv[1])).get('items') or [])))" \
            "${E2E_PROBES}.tmp" 2>/dev/null || echo 0)
        if [[ "$count" -gt 0 && "$count" -eq "$previous" ]]; then
            stable=$((stable + 1))
        else
            stable=0
        fi
        log "  poll $attempt: $count probe(s), stable for $stable"
        if [[ "$stable" -ge 2 ]]; then
            mv "${E2E_PROBES}.tmp" "$E2E_PROBES"
            log "probe results written to $E2E_PROBES"
            log "cluster ready; export KUBECONFIG=$E2E_KUBECONFIG"
            exit 0
        fi
        previous="$count"
    fi
    sleep 10
done

rm -f "${E2E_PROBES}.tmp"
die "kubesonde produced no stable probe results within the timeout; check
  kubectl --kubeconfig $E2E_KUBECONFIG -n kubesonde-system logs deployment/kubesonde-controller-manager"
