//! Truth tables derived by hand from the Kubernetes NetworkPolicy semantics.
//!
//! Unlike the snapshots, nothing here is generated: each row states what Kubernetes
//! would do, so a compiler change that alters behaviour fails rather than
//! re-recording itself.

mod common;

use k8s_networkpolicy_smt::compile::Direction;
use k8s_networkpolicy_smt::eval::Verdict::{Allow, Deny, Unknown};

#[test]
fn default_deny_all() {
    // An isolating policy with no rules denies everything for the pods it selects,
    // in both directions, and leaves other namespaces alone.
    common::fixture("default-deny-all").assert_table(&[
        ("locked/a", "locked/b", "TCP", 80, Deny),
        ("locked/a", "open/c", "TCP", 80, Deny),
        ("open/c", "locked/a", "TCP", 80, Deny),
        ("open/c", "open/d", "TCP", 80, Allow),
    ]);
}

#[test]
fn allow_same_ns() {
    common::fixture("allow-same-ns").assert_table(&[
        ("default/web", "default/api", "TCP", 8080, Allow),
        // The port and the protocol both have to match.
        ("default/web", "default/api", "TCP", 8081, Deny),
        ("default/web", "default/api", "UDP", 8080, Deny),
        // A source the `from` does not name.
        ("default/other", "default/api", "TCP", 8080, Deny),
        // `other` is selected by no policy, so it is not isolated.
        ("default/web", "default/other", "TCP", 8080, Allow),
    ]);
}

#[test]
fn namespace_selector() {
    common::fixture("namespace-selector").assert_table(&[
        ("monitoring/prom", "prod/db", "TCP", 5432, Allow),
        // namespaceSelector and podSelector in one peer: both must match.
        ("platform/scraper", "prod/db", "TCP", 5432, Allow),
        ("platform/other", "prod/db", "TCP", 5432, Deny),
        ("dev/x", "prod/db", "TCP", 5432, Deny),
        // An empty namespaceSelector selects every namespace.
        ("dev/x", "stage/db", "TCP", 5432, Allow),
    ]);
}

#[test]
fn match_expressions() {
    common::fixture("match-expressions").assert_table(&[
        // `fe` has neither `quarantine` nor `zone`; DoesNotExist and NotIn both
        // match an absent key.
        ("default/fe", "default/target", "TCP", 80, Allow),
        ("default/mid", "default/target", "TCP", 80, Allow),
        ("default/db", "default/target", "TCP", 80, Deny),
        ("default/fe-quarantined", "default/target", "TCP", 80, Deny),
        ("default/fe-dmz", "default/target", "TCP", 80, Deny),
        // `db` carries no `app` label, so the policy's podSelector misses it and it
        // is never isolated.
        ("default/fe", "default/db", "TCP", 80, Allow),
    ]);
}

#[test]
fn ipblock_egress() {
    common::fixture("ipblock-egress").assert_table(&[
        // Outside the cluster, inside the CIDR, outside the exception.
        ("default/gateway", "1.1.1.1", "TCP", 443, Allow),
        ("default/gateway", "10.1.2.3", "TCP", 443, Deny),
        // The same test applied to pods, whose addresses we were given.
        ("default/gateway", "default/elsewhere", "TCP", 443, Allow),
        ("default/gateway", "default/internal", "TCP", 443, Deny),
        // `internal` is not selected by the (Egress-only) policy.
        ("default/internal", "default/elsewhere", "TCP", 443, Allow),
    ]);
}

#[test]
fn unknown_pod_ip_yields_a_residual() {
    let fixture = common::fixture("unknown-pod-ip");
    assert_eq!(fixture.unknown_ips, vec!["default/mystery".to_string()]);

    fixture.assert_table(&[
        ("default/gateway", "default/known", "TCP", 443, Allow),
        // We were never told `mystery`'s address, and the rule turns on it.
        ("default/gateway", "default/mystery", "TCP", 443, Unknown),
    ]);

    // Ingress is still decided; only egress, which reads the address, is not.
    let outcomes = fixture.outcomes("default/gateway", "default/mystery", "TCP", 443);
    let egress = outcomes
        .iter()
        .find(|o| o.direction == Direction::Egress)
        .unwrap();
    assert_eq!(egress.verdict, Unknown);
    assert!(
        egress.residuals.iter().any(|r| r.contains("isInRange")),
        "expected a residual naming the undecided address test, got {:?}",
        egress.residuals
    );
    let ingress = outcomes
        .iter()
        .find(|o| o.direction == Direction::Ingress)
        .unwrap();
    assert_eq!(ingress.verdict, Allow);
}

#[test]
fn port_range() {
    // `endPort` is inclusive at both ends.
    common::fixture("port-range").assert_table(&[
        ("default/client", "default/svc", "TCP", 29999, Deny),
        ("default/client", "default/svc", "TCP", 30000, Allow),
        ("default/client", "default/svc", "TCP", 30005, Allow),
        ("default/client", "default/svc", "TCP", 30010, Allow),
        ("default/client", "default/svc", "TCP", 30011, Deny),
    ]);
}

#[test]
fn named_port() {
    common::fixture("named-port").assert_table(&[
        ("default/client", "default/api", "TCP", 9100, Allow),
        ("default/client", "default/api", "TCP", 8080, Deny),
        // A container port name cannot resolve outside the cluster, so the egress
        // rule can never match an IpEndpoint.
        ("default/client", "1.1.1.1", "TCP", 9100, Deny),
        // The Egress-only policy on `client` does not isolate its ingress.
        ("default/api", "default/client", "TCP", 9100, Allow),
    ]);
}

#[test]
fn additive() {
    // Two policies over the same pod are unioned, and neither alone denies.
    common::fixture("additive").assert_table(&[
        ("default/web", "default/api", "TCP", 80, Allow),
        ("default/cron", "default/api", "TCP", 80, Allow),
        ("default/other", "default/api", "TCP", 80, Deny),
    ]);
}

#[test]
fn egress_defaulting() {
    // `policyTypes` omitted with an `egress` section present: Egress applies
    // because of the section, and Ingress applies because it always does — which
    // isolates `worker` for ingress with no ingress rules to let anything back in.
    common::fixture("egress-defaulting").assert_table(&[
        ("default/worker", "default/cache", "TCP", 6379, Allow),
        ("default/worker", "default/other", "TCP", 6379, Deny),
        ("default/other", "default/worker", "TCP", 6379, Deny),
    ]);
}

#[test]
fn cross_namespace_proves_the_conjunction() {
    let fixture = common::fixture("cross-namespace");
    fixture.assert_table(&[("a/front", "b/back", "TCP", 80, Deny)]);

    // The point of the fixture: one side says yes, the other says no.
    assert_eq!(
        fixture.direction("a/front", "b/back", "TCP", 80, Direction::Egress),
        Allow
    );
    assert_eq!(
        fixture.direction("a/front", "b/back", "TCP", 80, Direction::Ingress),
        Deny
    );
}

#[test]
fn protocol_enum() {
    // A UDP rule does not admit TCP or SCTP on the same port.
    common::fixture("protocol-enum").assert_table(&[
        ("default/client", "default/dns", "UDP", 53, Allow),
        ("default/client", "default/dns", "TCP", 53, Deny),
        ("default/client", "default/dns", "SCTP", 53, Deny),
    ]);
}
