#!/usr/bin/env bash
# The full empirical run: cluster with Cilium, probe it, compare, tear down.
# Set KEEP_CLUSTER=1 to leave it standing.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

"$REPO_ROOT/hack/e2e-up.sh"

if [[ "${KEEP_CLUSTER:-0}" != "1" ]]; then
    trap '"$REPO_ROOT/hack/e2e-down.sh"' EXIT
fi

log "cargo test --features e2e"
cd "$REPO_ROOT"
KUBECONFIG="$E2E_KUBECONFIG" cargo test --features e2e
