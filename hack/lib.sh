# Shared settings for the hack/ scripts. Source, do not execute.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export REPO_ROOT

# Everything is overridable so CI, a laptop, and two developers sharing a machine
# can each point somewhere else.
export CLUSTER_NAME="${CLUSTER_NAME:-np2cedar}"
export KIND_VERSION="${KIND_VERSION:-v0.32.0}"
export KIND_NODE_IMAGE="${KIND_NODE_IMAGE:-kindest/node:v1.34.0}"

# The empirical e2e cluster is separate: Cilium needs `disableDefaultCNI`, and the
# integration cluster has to stay fast.
export E2E_CLUSTER_NAME="${E2E_CLUSTER_NAME:-np2cedar-e2e}"
export E2E_NAMESPACE="${E2E_NAMESPACE:-np2cedar-e2e}"
export CILIUM_CLI_VERSION="${CILIUM_CLI_VERSION:-v0.19.7}"
export CILIUM_VERSION="${CILIUM_VERSION:-1.18.3}"
export KUBESONDE_VERSION="${KUBESONDE_VERSION:-v0.8.24}"
# kubesonde's monitor image is published for amd64 only. Its command is overridden
# to `sh` and the process it would run is exec'd separately (and tolerated to fail),
# so any multi-arch image with a shell satisfies the "both containers Running" gate.
export KUBESONDE_MONITOR_IMAGE="${KUBESONDE_MONITOR_IMAGE:-alpine:3}"
export E2E_KUBECONFIG="${E2E_KUBECONFIG:-$REPO_ROOT/.kube/e2e-config}"
export E2E_PROBES="${E2E_PROBES:-$REPO_ROOT/.e2e/probes.json}"

# The kubeconfig is repo-local and gitignored, so running the integration suite
# never retargets the developer's real kubectl context, and so a cluster
# credential cannot be swept into a commit by `git add -A`.
export KUBECONFIG="${KUBECONFIG:-$REPO_ROOT/.kube/config}"

log() { printf '\033[1;34m==>\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

require() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required but not on PATH"
}

host_os()   { uname -s | tr '[:upper:]' '[:lower:]'; }
host_arch() {
    case "$(uname -m)" in
        x86_64|amd64) echo amd64 ;;
        arm64|aarch64) echo arm64 ;;
        *) die "unsupported architecture $(uname -m)" ;;
    esac
}

# These are the tools we install for the caller, because CI would otherwise need a
# second, divergent code path to get them -- and then CI would be testing a recipe
# nobody runs locally.
ensure_kind() {
    if command -v kind >/dev/null 2>&1; then
        return
    fi
    log "kind not found; installing $KIND_VERSION into $REPO_ROOT/.bin"
    mkdir -p "$REPO_ROOT/.bin"
    curl -fsSL -o "$REPO_ROOT/.bin/kind" \
        "https://kind.sigs.k8s.io/dl/${KIND_VERSION}/kind-$(host_os)-$(host_arch)"
    chmod +x "$REPO_ROOT/.bin/kind"
    export PATH="$REPO_ROOT/.bin:$PATH"
}

ensure_cilium_cli() {
    if command -v cilium >/dev/null 2>&1; then
        return
    fi
    log "cilium CLI not found; installing $CILIUM_CLI_VERSION into $REPO_ROOT/.bin"
    mkdir -p "$REPO_ROOT/.bin"
    local tarball="cilium-$(host_os)-$(host_arch).tar.gz"
    curl -fsSL -o "$REPO_ROOT/.bin/$tarball" \
        "https://github.com/cilium/cilium-cli/releases/download/${CILIUM_CLI_VERSION}/${tarball}"
    tar -xzf "$REPO_ROOT/.bin/$tarball" -C "$REPO_ROOT/.bin" cilium
    rm -f "$REPO_ROOT/.bin/$tarball"
    chmod +x "$REPO_ROOT/.bin/cilium"
    export PATH="$REPO_ROOT/.bin:$PATH"
}
