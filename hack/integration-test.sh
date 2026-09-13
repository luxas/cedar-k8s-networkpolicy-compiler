#!/usr/bin/env bash
# The full live-cluster run: stand up a cluster, test against it, tear it down.
# Set KEEP_CLUSTER=1 to leave it running afterwards.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

"$REPO_ROOT/hack/kind-up.sh"

if [[ "${KEEP_CLUSTER:-0}" != "1" ]]; then
    trap '"$REPO_ROOT/hack/kind-down.sh"' EXIT
fi

log "cargo test --features integration"
cd "$REPO_ROOT"
cargo test --features integration
