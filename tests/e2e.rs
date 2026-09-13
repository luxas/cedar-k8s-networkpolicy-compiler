//! Empirical validation: does this tool agree with what a real CNI actually does?
//!
//! Everything else in this suite checks np2cedar against my own reading of the
//! Kubernetes API contract — hand-derived truth tables, a hand-written Cedar
//! translation, the API server's defaulting. This file checks it against packets.
//!
//! A kind cluster running Cilium enforces `tests/data/e2e/manifests.yaml`; kubesonde
//! injects nmap ephemeral containers and probes every pod pair, recording Allow or
//! Deny per (source, destination, port); and the test below asserts np2cedar's
//! verdict matches the observed one for every probe.
//!
//! What this establishes is narrower than "the tool is correct", and worth stating:
//!
//!   - It validates against *Cilium*, one implementation, not the specification. A
//!     disagreement implicates three tools and needs investigating, not suppressing.
//!   - TCP only. nmap's UDP result is `open|filtered`, which decides nothing.
//!   - Pod to pod, inside one namespace, because a Kubesonde object probes exactly
//!     one. Cross-namespace behaviour is covered by tests/cluster.rs.
//!   - Only where the destination genuinely serves the port, which the fixture
//!     guarantees: nmap cannot tell "policy dropped it" from "nothing is listening".
//!
//!     hack/e2e-test.sh                  # the whole cycle from nothing
//!     cargo test --features e2e         # against an already-prepared cluster

#![cfg(feature = "e2e")]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use k8s_networkpolicy_smt::cluster;
use k8s_networkpolicy_smt::eval::Verdict;
use k8s_networkpolicy_smt::load::Loaded;

const NAMESPACE: &str = "np2cedar-e2e";
const FIXTURE: &str = "tests/data/e2e";

// ---------------------------------------------------------------------------
// kubesonde's /probes output. Only the fields we actually judge on.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    items: Vec<ProbeItem>,
    #[serde(default)]
    errors: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeItem {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    resulting_action: String,
    #[serde(default)]
    source: Endpoint,
    #[serde(default)]
    destination: Endpoint,
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    port: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Endpoint {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    namespace: String,
}

/// One probe reduced to something directly comparable with a `check`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Observation {
    from: String,
    to: String,
    protocol: String,
    port: i64,
    observed: Verdict,
}

impl std::fmt::Display for Observation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} -> {} {}:{}",
            self.from, self.to, self.protocol, self.port
        )
    }
}

// ---------------------------------------------------------------------------

fn probes_path() -> PathBuf {
    std::env::var_os("E2E_PROBES")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join(".e2e/probes.json"))
}

fn kubeconfig() -> Option<PathBuf> {
    if std::env::var_os("KUBECONFIG").is_some() {
        return None;
    }
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join(".kube/e2e-config");
    local.exists().then_some(local)
}

fn read_probes() -> ProbeOutput {
    let path = probes_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "could not read kubesonde's results at {}: {error}\n\n\
             Run `hack/e2e-up.sh` to stand up the Cilium cluster and probe it, or set \
             $E2E_PROBES to an existing result file.",
            path.display()
        )
    });
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not kubesonde probe output: {error}", path.display()))
}

/// Ports each pod declares, and therefore actually serves.
type Served = BTreeMap<String, BTreeSet<i64>>;

/// Every container port the fixture's pods declare. `hack/e2e-up.sh` waits for the
/// pods to be Ready, so a declared port here is a served port.
fn served_ports(loaded: &Loaded) -> Served {
    loaded
        .pods
        .iter()
        .filter_map(|pod| {
            let name = pod.metadata.name.as_deref()?;
            let namespace = pod.metadata.namespace.as_deref()?;
            let ports = pod
                .spec
                .iter()
                .flat_map(|spec| spec.containers.iter())
                .flat_map(|container| container.ports.iter().flatten())
                .map(|port| i64::from(port.container_port))
                .collect::<BTreeSet<_>>();
            Some((format!("{namespace}/{name}"), ports))
        })
        .collect()
}

/// Keep only the probes we can honestly judge, and say why the others go.
fn observations(output: &ProbeOutput, served: &Served) -> Vec<Observation> {
    let mut kept: Vec<Observation> = output
        .items
        .iter()
        .filter(|item| item.kind == "Probe")
        // Pod-to-pod only: a Service probe is answered by whichever backend was
        // picked, and an Internet probe leaves the cluster entirely.
        .filter(|item| item.source.kind == "Pod" && item.destination.kind == "Pod")
        .filter(|item| {
            item.source.namespace == NAMESPACE && item.destination.namespace == NAMESPACE
        })
        // nmap reports UDP as `open|filtered`, which cannot decide anything.
        .filter(|item| item.protocol.eq_ignore_ascii_case("tcp"))
        // Traffic from a pod to itself never traverses policy enforcement.
        .filter(|item| item.source.name != item.destination.name)
        .filter_map(|item| {
            let port: i64 = item.port.parse().ok()?;
            // The confound this whole comparison turns on: nmap cannot tell "policy
            // dropped it" from "nothing is listening", and kubesonde falls back to
            // port 80 for probes carrying no port of their own. Judging one of those
            // would score a closed port as a policy denial. Only compare where the
            // destination genuinely serves.
            let destination = format!("{}/{}", item.destination.namespace, item.destination.name);
            if !served
                .get(&destination)
                .is_some_and(|ports| ports.contains(&port))
            {
                return None;
            }
            Some(Observation {
                from: format!("{}/{}", item.source.namespace, item.source.name),
                to: destination,
                protocol: item.protocol.to_uppercase(),
                port,
                observed: match item.resulting_action.as_str() {
                    "Allow" => Verdict::Allow,
                    "Deny" => Verdict::Deny,
                    other => panic!("kubesonde reported an unknown action {other:?}"),
                },
            })
        })
        .collect();
    kept.sort();
    kept.dedup();
    kept
}

/// np2cedar's model of the same cluster, built by fetching it live — so the cluster
/// path is exercised here too rather than re-reading the fixture YAML. The `Loaded`
/// comes back with it because the comparison needs the pods' declared ports.
fn model() -> (common::Fixture, Loaded) {
    let fetched = cluster::fetch(&cluster::Options {
        kubeconfig: kubeconfig(),
        namespaces: vec![NAMESPACE.to_string()],
        ..Default::default()
    })
    .unwrap_or_else(|error| {
        panic!(
            "could not reach the e2e cluster: {error:?}\n\n\
             Run `hack/e2e-up.sh`, or point KUBECONFIG at the Cilium cluster."
        )
    });

    // Only the fixture's own objects, so anything else Cilium or kubesonde installed
    // cannot affect the verdicts.
    let ours = |ns: &Option<String>| ns.as_deref() == Some(NAMESPACE);
    let loaded = Loaded {
        network_policies: fetched
            .loaded
            .network_policies
            .iter()
            .filter(|p| ours(&p.metadata.namespace))
            .cloned()
            .collect(),
        pods: fetched
            .loaded
            .pods
            .iter()
            .filter(|p| ours(&p.metadata.namespace))
            .cloned()
            .collect(),
        namespaces: fetched
            .loaded
            .namespaces
            .iter()
            .filter(|n| n.metadata.name.as_deref() == Some(NAMESPACE))
            .cloned()
            .collect(),
    };
    (common::fixture_from(FIXTURE, &loaded), loaded)
}

#[test]
fn the_probe_set_is_worth_comparing_against() {
    // A vacuous run is the main way a test like this rots: kubesonde produces
    // nothing, or everything comes back Deny, and the comparison passes while
    // proving nothing. Fail loudly instead.
    let output = read_probes();
    let (_, loaded) = model();
    let observations = observations(&output, &served_ports(&loaded));

    assert!(
        !observations.is_empty(),
        "kubesonde returned no comparable pod-to-pod TCP probes in {NAMESPACE} \
         ({} items, {} errors in total)",
        output.items.len(),
        output.errors.len()
    );

    let outcomes: BTreeSet<Verdict> = observations.iter().map(|o| o.observed).collect();
    assert!(
        outcomes.contains(&Verdict::Allow) && outcomes.contains(&Verdict::Deny),
        "the fixture is meant to produce both allowed and denied traffic, but every \
         probe came back the same way ({outcomes:?}); a comparison against this would \
         be vacuous"
    );

    // Four pods, so twelve ordered pairs on the one served port.
    let pairs: BTreeSet<(&str, &str)> = observations
        .iter()
        .map(|o| (o.from.as_str(), o.to.as_str()))
        .collect();
    assert_eq!(
        pairs.len(),
        12,
        "expected every ordered pair of the four fixture pods to be probed, got {pairs:?}"
    );
}

#[test]
fn every_probe_agrees_with_the_static_analysis() {
    let (model, loaded) = model();
    let observations = observations(&read_probes(), &served_ports(&loaded));

    let mut disagreements = Vec::new();
    let mut unknowns = Vec::new();

    for observation in &observations {
        let predicted = model.verdict(
            &observation.from,
            &observation.to,
            &observation.protocol,
            observation.port,
        );
        match predicted {
            // This fixture has every address known and no ipBlock rules, so an
            // Unknown means something is wrong -- and an unknown cannot be checked
            // against a packet either way.
            Verdict::Unknown => unknowns.push(format!("  {observation}")),
            _ if predicted != observation.observed => disagreements.push(format!(
                "  {observation}: np2cedar says {predicted}, Cilium did {}",
                observation.observed
            )),
            _ => {}
        }
    }

    assert!(
        unknowns.is_empty(),
        "np2cedar could not decide {} of {} probes, which this fixture should never \
         produce:\n{}",
        unknowns.len(),
        observations.len(),
        unknowns.join("\n")
    );
    assert!(
        disagreements.is_empty(),
        "{} of {} probes disagree with what Cilium actually did.\n\n\
         One of np2cedar, kubesonde or Cilium is wrong here; find out which before \
         changing this test.\n\n{}",
        disagreements.len(),
        observations.len(),
        disagreements.join("\n")
    );
}

#[test]
fn the_observed_matrix_is_the_one_the_fixture_was_designed_for() {
    // Independently of np2cedar: this is what tests/data/e2e/manifests.yaml is
    // supposed to produce, derived by hand from the policies. If Cilium disagrees
    // with this table, the table is what needs re-deriving -- that is the point of
    // running against a real dataplane.
    use Verdict::{Allow, Deny};
    let expected: &[(&str, &str, Verdict)] = &[
        ("web", "api", Allow),
        ("web", "cache", Deny),
        ("web", "lonely", Allow),
        ("api", "web", Allow),
        ("api", "cache", Deny),
        ("api", "lonely", Allow),
        // cache -> web is denied by cache's *egress* policy, not by web's ingress.
        ("cache", "web", Deny),
        ("cache", "api", Deny),
        ("cache", "lonely", Allow),
        ("lonely", "web", Allow),
        ("lonely", "api", Deny),
        ("lonely", "cache", Deny),
    ];

    let (_, loaded) = model();
    let observations = observations(&read_probes(), &served_ports(&loaded));
    let mut wrong = Vec::new();
    for (from, to, want) in expected {
        let from = format!("{NAMESPACE}/{from}");
        let to = format!("{NAMESPACE}/{to}");
        match observations
            .iter()
            .find(|o| o.from == from && o.to == to && o.port == 8080)
        {
            None => wrong.push(format!("  {from} -> {to} TCP:8080 was never probed")),
            Some(observed) if observed.observed != *want => wrong.push(format!(
                "  {from} -> {to} TCP:8080: expected {want}, Cilium did {}",
                observed.observed
            )),
            Some(_) => {}
        }
    }
    assert!(
        wrong.is_empty(),
        "Cilium's behaviour differs from the fixture's hand-derived matrix:\n{}",
        wrong.join("\n")
    );
}
