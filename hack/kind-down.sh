#!/usr/bin/env bash
# Tear down the local e2e cluster and its kubeconfig.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

ensure_kind
if kind get clusters 2>/dev/null | grep -qx "$CLUSTER_NAME"; then
    log "deleting cluster $CLUSTER_NAME"
    kind delete cluster --name "$CLUSTER_NAME"
else
    log "cluster $CLUSTER_NAME does not exist"
fi
rm -f "$KUBECONFIG"
