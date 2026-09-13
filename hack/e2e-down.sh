#!/usr/bin/env bash
# Tear down the empirical e2e cluster and everything it left behind.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

ensure_kind
if kind get clusters 2>/dev/null | grep -qx "$E2E_CLUSTER_NAME"; then
    log "deleting cluster $E2E_CLUSTER_NAME"
    kind delete cluster --name "$E2E_CLUSTER_NAME"
else
    log "cluster $E2E_CLUSTER_NAME does not exist"
fi
rm -f "$E2E_KUBECONFIG" "$E2E_PROBES" "${E2E_PROBES}.tmp"
