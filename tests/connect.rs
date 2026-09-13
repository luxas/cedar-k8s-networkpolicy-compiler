//! The synthesized `connect` policies: cedar-woodpecker over the compiled
//! ingress/egress permits.
//!
//! These run by default and fail loudly when cvc5 is missing, like
//! tests/symbolic.rs: a silently skipped solver test would let a broken setup
//! look green. Install cvc5 or point `$CVC5` at the binary.

mod common;

use cedar_policy::{PolicySet, ValidationMode, Validator};
use k8s_networkpolicy_smt::connect::synthesize;
use k8s_networkpolicy_smt::schema;

fn synthesized(fixture_name: &str, cidrs: &[&str]) -> String {
    let f = common::fixture(fixture_name);
    let cidrs: Vec<String> = cidrs.iter().map(|c| c.to_string()).collect();
    let escalations = synthesize(&f.policy_set, &cidrs).unwrap_or_else(|e| {
        panic!("{fixture_name}: synthesizing (is cvc5 installed, or $CVC5 set?): {e:?}")
    });
    assert!(
        escalations.iter().all(|e| e.sound),
        "{fixture_name}: an unsound escalation path"
    );
    // An oracle independent of the fork's own re-validation: every synthesized
    // permit must strict-validate against the bundled schema, which among other
    // things proves every getTag is still behind its hasTag guard — the property
    // a conjunct-reordering fork bump could silently break.
    let mut synthesized_set = PolicySet::new();
    for escalation in &escalations {
        synthesized_set
            .add(escalation.policy.clone())
            .unwrap_or_else(|e| panic!("{fixture_name}: collecting synthesized policies: {e:?}"));
    }
    let validation =
        Validator::new(schema().clone()).validate(&synthesized_set, ValidationMode::Strict);
    assert!(
        validation.validation_passed(),
        "{fixture_name}: a synthesized policy fails strict validation: {:?}",
        validation.validation_errors().collect::<Vec<_>>()
    );
    escalations
        .iter()
        .map(|e| e.to_text())
        .collect::<Vec<_>>()
        .join("\n")
}

// Only ingress is restricted (same-namespace rule); the egress side is the
// unconditional catch-all, so connect is the ingress conditions with the rule's
// port constraint carried onto connect's own context by the transition's
// context equalities.
#[test]
fn allow_same_ns() {
    insta::assert_snapshot!(synthesized("allow-same-ns", &[]));
}

// Both directions restricted somewhere: a genuine conjunction of an egress
// permit and an ingress permit.
#[test]
fn cross_namespace() {
    insta::assert_snapshot!(synthesized("cross-namespace", &[]));
}

// The internet-only egress rule (0.0.0.0/0 except 10.0.0.0/8) survives when
// nothing constrains pod addresses...
#[test]
fn ipblock_egress_unconstrained() {
    insta::assert_snapshot!(synthesized("ipblock-egress", &[]));
}

// ...and drops out of the synthesis when the pod CIDR sits inside its except:
// no pod-to-pod connect can ride that rule.
#[test]
fn ipblock_egress_with_pod_cidr() {
    insta::assert_snapshot!(synthesized("ipblock-egress", &["10.244.0.0/16"]));
}
