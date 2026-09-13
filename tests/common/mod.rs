//! Shared fixture loading for the integration tests.
#![allow(dead_code)]

use std::net::IpAddr;
use std::path::PathBuf;

use cedar_policy::{EntityUid, PartialEntities, PartialEntityUid, PolicySet};
use serde_json::Value;

use k8s_networkpolicy_smt::compile::{Direction, compile};
use k8s_networkpolicy_smt::entities::{build, ip_endpoint_entities};
use k8s_networkpolicy_smt::eval::{Outcome, Verdict, combine, context, evaluate};
use k8s_networkpolicy_smt::load::{Loaded, load};
use k8s_networkpolicy_smt::schema;

pub struct Fixture {
    pub name: String,
    /// The generated `.cedar` text.
    pub source: String,
    pub entities_json: Value,
    pub policy_set: PolicySet,
    /// `namespace/name` of pods whose address was left unknown.
    pub unknown_ips: Vec<String>,
}

/// Compile and build the entity store for `tests/data/<name>`.
///
/// `compile` parses and strictly validates what it generates, so a fixture that
/// reaches the end of this function has already passed both.
pub fn fixture(name: &str) -> Fixture {
    fixture_at(&PathBuf::from("tests/data").join(name))
}

/// As [`fixture`], for a manifest directory anywhere in the repo.
pub fn fixture_at(dir: &std::path::Path) -> Fixture {
    let name = dir.display().to_string();
    let loaded =
        load(&[dir.to_path_buf()]).unwrap_or_else(|e| panic!("{name}: loading manifests: {e:?}"));
    fixture_from(&name, &loaded)
}

/// Compile and build a fixture from objects that are already in hand — from files,
/// or straight from a cluster.
pub fn fixture_from(name: &str, loaded: &Loaded) -> Fixture {
    let compiled =
        compile(&loaded.network_policies).unwrap_or_else(|e| panic!("{name}: compiling: {e:?}"));
    let built = build(&loaded.pods, &loaded.namespaces)
        .unwrap_or_else(|e| panic!("{name}: building entities: {e:?}"));
    Fixture {
        name: name.to_string(),
        source: compiled.source,
        entities_json: built.json,
        policy_set: compiled.policy_set,
        unknown_ips: built.unknown_ips,
    }
}

impl Fixture {
    /// The same entities and fixture name, but a different policy set — for
    /// cross-checking the compiler against a hand-written translation.
    pub fn with_policies(&self, policy_set: PolicySet) -> Fixture {
        Fixture {
            name: format!("{} (alternative policies)", self.name),
            source: policy_set.to_string(),
            entities_json: self.entities_json.clone(),
            policy_set,
            unknown_ips: self.unknown_ips.clone(),
        }
    }

    /// Evaluate both directions. `from` is a `namespace/name` pod reference; `to` is
    /// either that or a bare IP address, which becomes an `IpEndpoint`.
    pub fn outcomes(&self, from: &str, to: &str, protocol: &str, port: i64) -> Vec<Outcome> {
        let mut json = self.entities_json.clone();
        let principal = pod_euid(from);
        let resource = match to.parse::<IpAddr>() {
            Ok(address) => {
                let address = address.to_string();
                json.as_array_mut()
                    .expect("the entity store is a JSON array")
                    .extend(ip_endpoint_entities(&address));
                format!(r#"IpEndpoint::"{address}""#).parse().unwrap()
            }
            Err(_) => pod_euid(to),
        };

        let entities = PartialEntities::from_json_value(json, schema())
            .unwrap_or_else(|e| panic!("{}: loading entities: {e:?}", self.name));
        let context = context(port, protocol).unwrap();

        Direction::ALL
            .into_iter()
            .map(|direction| {
                evaluate(
                    &self.policy_set,
                    &entities,
                    PartialEntityUid::from_concrete(principal.clone()),
                    PartialEntityUid::from_concrete(resource.clone()),
                    Some(&context),
                    direction,
                )
                .unwrap_or_else(|e| panic!("{}: evaluating {direction}: {e:?}", self.name))
            })
            .collect()
    }

    pub fn verdict(&self, from: &str, to: &str, protocol: &str, port: i64) -> Verdict {
        combine(&self.outcomes(from, to, protocol, port))
    }

    pub fn direction(
        &self,
        from: &str,
        to: &str,
        protocol: &str,
        port: i64,
        direction: Direction,
    ) -> Verdict {
        self.outcomes(from, to, protocol, port)
            .into_iter()
            .find(|o| o.direction == direction)
            .expect("both directions are evaluated")
            .verdict
    }

    /// Check a whole truth table at once, reporting every disagreement rather than
    /// stopping at the first.
    pub fn assert_table(&self, rows: &[(&str, &str, &str, i64, Verdict)]) {
        let failures: Vec<String> = rows
            .iter()
            .filter_map(|(from, to, protocol, port, expected)| {
                let actual = self.verdict(from, to, protocol, *port);
                (actual != *expected).then(|| {
                    format!("  {from} -> {to} {protocol}:{port}: expected {expected}, got {actual}")
                })
            })
            .collect();
        assert!(
            failures.is_empty(),
            "{}: {} of {} rows disagree with Kubernetes:\n{}",
            self.name,
            failures.len(),
            rows.len(),
            failures.join("\n")
        );
    }
}

fn pod_euid(reference: &str) -> EntityUid {
    format!(r#"Pod::"{reference}""#)
        .parse()
        .unwrap_or_else(|e| panic!("{reference:?} is not a pod reference: {e:?}"))
}
