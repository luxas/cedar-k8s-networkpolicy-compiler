//! Tests that need a real Kubernetes API server.
//!
//! Gated behind the `integration` feature. When it is on these tests **run and
//! fail** if the cluster is unreachable — they never skip, so a broken CI cluster
//! cannot masquerade as a passing build.
//!
//!     hack/integration-test.sh          # stand up a cluster, test, tear it down
//!     cargo test --features integration # against whatever is already configured

#![cfg(feature = "integration")]

mod common;

use std::path::{Path, PathBuf};

use k8s_networkpolicy_smt::cluster::{self, Fetched};
use k8s_networkpolicy_smt::compile::{Direction, compile};
use k8s_networkpolicy_smt::eval::Verdict::{Allow, Deny};
use k8s_networkpolicy_smt::load::Loaded;

/// The namespaces `tests/data/cluster/manifests.yaml` owns.
const NAMESPACES: [&str; 2] = ["np2cedar-a", "np2cedar-b"];
const FIXTURE: &str = "tests/data/cluster";

/// Where `hack/kind-up.sh` writes the kubeconfig. Used when `$KUBECONFIG` is unset,
/// so a bare `cargo test --features integration` works without the wrapper script.
fn kubeconfig() -> Option<PathBuf> {
    if std::env::var_os("KUBECONFIG").is_some() {
        return None;
    }
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join(".kube/config");
    local.exists().then_some(local)
}

fn options() -> cluster::Options {
    cluster::Options {
        kubeconfig: kubeconfig(),
        ..Default::default()
    }
}

/// Fetch, failing loudly with the underlying error when the cluster is not there.
fn fetch(options: &cluster::Options) -> Fetched {
    cluster::fetch(options).unwrap_or_else(|error| {
        panic!(
            "could not reach the cluster: {error:?}\n\n\
             These tests require one. Run `hack/kind-up.sh`, or point KUBECONFIG at \
             a cluster with tests/data/cluster/manifests.yaml applied."
        )
    })
}

/// Only the objects belonging to the fixture, so anything else in the cluster —
/// another developer's namespace, an operator's own policies — cannot affect the
/// result.
fn only_fixture(loaded: &Loaded) -> Loaded {
    let ours = |namespace: &Option<String>| {
        namespace
            .as_deref()
            .is_some_and(|ns| NAMESPACES.contains(&ns))
    };
    Loaded {
        network_policies: loaded
            .network_policies
            .iter()
            .filter(|p| ours(&p.metadata.namespace))
            .cloned()
            .collect(),
        pods: loaded
            .pods
            .iter()
            .filter(|p| ours(&p.metadata.namespace))
            .cloned()
            .collect(),
        namespaces: loaded
            .namespaces
            .iter()
            .filter(|n| {
                n.metadata
                    .name
                    .as_deref()
                    .is_some_and(|n| NAMESPACES.contains(&n))
            })
            .cloned()
            .collect(),
    }
}

#[test]
fn the_fixture_is_present_and_compiles() {
    let loaded = only_fixture(&fetch(&options()).loaded);

    let mut pods: Vec<String> = loaded
        .pods
        .iter()
        .map(|p| {
            format!(
                "{}/{}",
                p.metadata.namespace.clone().unwrap(),
                p.metadata.name.clone().unwrap()
            )
        })
        .collect();
    pods.sort();
    assert_eq!(
        pods,
        [
            "np2cedar-a/api",
            "np2cedar-a/crosser",
            "np2cedar-a/defaulted",
            "np2cedar-a/other",
            "np2cedar-a/web",
            "np2cedar-b/back",
        ],
        "is tests/data/cluster/manifests.yaml applied? run hack/kind-up.sh"
    );
    assert_eq!(loaded.network_policies.len(), 4);
    assert_eq!(loaded.namespaces.len(), 2);

    // Compiling strictly validates, so this covers the whole pipeline.
    common::fixture_from(FIXTURE, &loaded);
}

#[test]
fn live_pods_have_concrete_addresses() {
    // The one thing no file-based test can show: that `status.podIPs` really is
    // populated and really does reach the entity store as a known address.
    let fixture = common::fixture_from(FIXTURE, &only_fixture(&fetch(&options()).loaded));
    assert!(
        fixture.unknown_ips.is_empty(),
        "every Ready pod should have an address from the API server, but these did not: {:?}",
        fixture.unknown_ips
    );
}

#[test]
fn truth_table_over_the_live_cluster() {
    let fixture = common::fixture_from(FIXTURE, &only_fixture(&fetch(&options()).loaded));
    fixture.assert_table(&[
        ("np2cedar-a/web", "np2cedar-a/api", "TCP", 8080, Allow),
        ("np2cedar-a/web", "np2cedar-a/api", "TCP", 8081, Deny),
        ("np2cedar-a/web", "np2cedar-a/api", "UDP", 8080, Deny),
        ("np2cedar-a/other", "np2cedar-a/api", "TCP", 8080, Deny),
        // `other` is selected by no policy, so it is not isolated.
        ("np2cedar-a/web", "np2cedar-a/other", "TCP", 8080, Allow),
        // Egress allows it, np2cedar-b's ingress does not. The conjunction denies.
        ("np2cedar-a/crosser", "np2cedar-b/back", "TCP", 80, Deny),
        // crosser's egress is confined to np2cedar-b.
        ("np2cedar-a/crosser", "np2cedar-a/api", "TCP", 8080, Deny),
        // `defaulted-egress` omits `protocol`, which means TCP...
        (
            "np2cedar-a/defaulted",
            "np2cedar-a/other",
            "TCP",
            9090,
            Allow,
        ),
        (
            "np2cedar-a/defaulted",
            "np2cedar-a/other",
            "UDP",
            9090,
            Deny,
        ),
        // ... and omits `policyTypes`, which still isolates `defaulted` for ingress,
        // with no ingress rule to let anything back in.
        (
            "np2cedar-a/other",
            "np2cedar-a/defaulted",
            "TCP",
            9090,
            Deny,
        ),
    ]);
}

#[test]
fn the_cross_namespace_denial_is_ingress_not_egress() {
    let fixture = common::fixture_from(FIXTURE, &only_fixture(&fetch(&options()).loaded));
    assert_eq!(
        fixture.direction(
            "np2cedar-a/crosser",
            "np2cedar-b/back",
            "TCP",
            80,
            Direction::Egress
        ),
        Allow
    );
    assert_eq!(
        fixture.direction(
            "np2cedar-a/crosser",
            "np2cedar-b/back",
            "TCP",
            80,
            Direction::Ingress
        ),
        Deny
    );
}

#[test]
fn cluster_and_file_compile_identically() {
    // The API server applies its own defaulting to `policyTypes`, `protocol` and
    // `podSelector` before storing an object, and `defaulted-egress` in the fixture
    // leans on all of it. If our defaulting agrees with Kubernetes', the generated
    // Cedar is the same whether the objects came from the YAML or from the API.
    //
    // A failure here is a real discrepancy between our reading of the API contract
    // and the API server's own, not a snapshot to re-record.
    let from_cluster = compile(&only_fixture(&fetch(&options()).loaded).network_policies).unwrap();
    let from_file = common::fixture_at(Path::new(FIXTURE));

    assert_eq!(
        from_cluster.source, from_file.source,
        "compiling the same NetworkPolicies from the cluster and from the file \
         disagreed, which means our defaulting differs from the API server's"
    );
}

#[test]
fn namespace_scoping_trims_pods_but_never_policies() {
    let scoped = fetch(&cluster::Options {
        namespaces: vec!["np2cedar-a".to_string()],
        ..options()
    });

    let namespaces: Vec<&str> = scoped
        .loaded
        .pods
        .iter()
        .filter_map(|p| p.metadata.namespace.as_deref())
        .collect();
    assert!(
        namespaces.iter().all(|ns| *ns == "np2cedar-a"),
        "-n should restrict the pod listing, but got pods from {namespaces:?}"
    );

    // The soundness property: a policy in another namespace still isolates the pods
    // it selects, so narrowing the pods must not narrow the policies. If it did,
    // np2cedar-b/back would look un-isolated and traffic would appear to be allowed.
    let policies: Vec<String> = scoped
        .loaded
        .network_policies
        .iter()
        .map(|p| {
            format!(
                "{}/{}",
                p.metadata.namespace.clone().unwrap_or_default(),
                p.metadata.name.clone().unwrap_or_default()
            )
        })
        .collect();
    assert!(
        policies.iter().any(|p| p == "np2cedar-b/deny-all-ingress"),
        "policies must stay cluster-wide even when -n narrows the pods; got {policies:?}"
    );
}
