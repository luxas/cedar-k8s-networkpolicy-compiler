# Shared settings for the hack/ scripts. Source, do not execute.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export REPO_ROOT

# Everything is overridable so CI, a laptop, and two developers sharing a machine
# can each point somewhere else.
export CLUSTER_NAME="${CLUSTER_NAME:-np2cedar}"
export KIND_VERSION="${KIND_VERSION:-v0.32.0}"
export KIND_NODE_IMAGE="${KIND_NODE_IMAGE:-kindest/node:v1.34.0}"

# The kubeconfig is repo-local and gitignored, so running the integration suite
# never retargets the developer's real kubectl context, and so a cluster
# credential cannot be swept into a commit by `git add -A`.
export KUBECONFIG="${KUBECONFIG:-$REPO_ROOT/.kube/config}"

log() { printf '\033[1;34m==>\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

require() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required but not on PATH"
}

# kind is the one tool we will install for the caller, because CI would otherwise
# need a second, divergent code path to get it.
ensure_kind() {
    if command -v kind >/dev/null 2>&1; then
        return
    fi
    log "kind not found; installing $KIND_VERSION into $REPO_ROOT/.bin"
    mkdir -p "$REPO_ROOT/.bin"
    local os arch
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    case "$(uname -m)" in
        x86_64|amd64) arch=amd64 ;;
        arm64|aarch64) arch=arm64 ;;
        *) die "unsupported architecture $(uname -m)" ;;
    esac
    curl -fsSL -o "$REPO_ROOT/.bin/kind" \
        "https://kind.sigs.k8s.io/dl/${KIND_VERSION}/kind-${os}-${arch}"
    chmod +x "$REPO_ROOT/.bin/kind"
    export PATH="$REPO_ROOT/.bin:$PATH"
}
