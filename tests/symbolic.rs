//! The symbolic discharger: `--pod-cidr` collapsing TPE residuals via cvc5.
//!
//! These run by default and fail loudly when cvc5 is missing, in the spirit of
//! the semantics tables: a silently skipped solver test would let a broken
//! setup look green. Install cvc5 or point `$CVC5` at the binary.
//!
//! The fixture: `default/gateway` (10.244.0.5) is egress-isolated by a rule
//! allowing `0.0.0.0/0 except 10.0.0.0/8`; `default/mystery` has no recorded
//! address, so without more information every query about it is `Unknown`.

mod common;

use cedar_policy::{EntityUid, PartialEntities, PartialEntityUid};
use common::fixture;
use k8s_networkpolicy_smt::compile::Direction;
use k8s_networkpolicy_smt::eval::{Verdict, evaluate};
use k8s_networkpolicy_smt::schema;
use k8s_networkpolicy_smt::symbolic::Discharger;

#[test]
fn a_cidr_inside_the_exception_collapses_to_deny() {
    let f = fixture("unknown-pod-ip");
    assert_eq!(
        f.verdict("default/gateway", "default/mystery", "TCP", 80),
        Verdict::Unknown,
        "without --pod-cidr the address is unconstrained"
    );
    // 10.244.0.0/16 lies wholly inside the rule's `except 10.0.0.0/8`: no pod
    // address can match, so the only residual permit can never apply.
    assert_eq!(
        f.direction_with_pod_cidrs(
            "default/gateway",
            "default/mystery",
            "TCP",
            80,
            Direction::Egress,
            &["10.244.0.0/16"]
        ),
        Verdict::Deny
    );
    assert_eq!(
        f.verdict_with_pod_cidrs(
            "default/gateway",
            "default/mystery",
            "TCP",
            80,
            &["10.244.0.0/16"]
        ),
        Verdict::Deny
    );
}

#[test]
fn a_cidr_outside_the_exception_collapses_to_allow() {
    // 172.16.0.0/12 is inside 0.0.0.0/0 and disjoint from 10.0.0.0/8: the rule
    // matches every address the pod could have.
    let f = fixture("unknown-pod-ip");
    assert_eq!(
        f.verdict_with_pod_cidrs(
            "default/gateway",
            "default/mystery",
            "TCP",
            80,
            &["172.16.0.0/12"]
        ),
        Verdict::Allow
    );
}

#[test]
fn a_cidr_straddling_the_exception_stays_unknown() {
    // 10.0.0.0/7 covers both 10.0.0.0/8 (inside the except) and 11.0.0.0/8
    // (outside): the answer genuinely depends on where in the range the address
    // falls, and an exact evaluator must say so rather than guess.
    let f = fixture("unknown-pod-ip");
    assert_eq!(
        f.direction_with_pod_cidrs(
            "default/gateway",
            "default/mystery",
            "TCP",
            80,
            Direction::Egress,
            &["10.0.0.0/7"]
        ),
        Verdict::Unknown
    );
}

#[test]
fn dual_stack_with_a_v6_range_still_denies() {
    // `isInRange` across address families is false (matching Kubernetes), so a
    // v6 pod address cannot match the rule's `0.0.0.0/0` any more than a
    // 10.244.0.0/16 one can escape its `except`.
    let f = fixture("unknown-pod-ip");
    assert_eq!(
        f.verdict_with_pod_cidrs(
            "default/gateway",
            "default/mystery",
            "TCP",
            80,
            &["10.244.0.0/16", "fd00::/8"]
        ),
        Verdict::Deny
    );
}

#[test]
fn reachable_prunes_the_internet_only_rule() {
    // The `reachable` shape: a fully symbolic Pod peer, whose address is only
    // known to lie in the pod CIDR (the `symbolic_resource` flag of `refine`).
    let f = fixture("unknown-pod-ip");
    let entities = PartialEntities::from_json_value(f.entities_json.clone(), schema())
        .expect("the fixture entities load");
    let gateway: EntityUid = r#"Pod::"default/gateway""#.parse().unwrap();
    let peer = PartialEntityUid::new("Pod".parse().unwrap(), None);

    let mut outcome = evaluate(
        &f.policy_set,
        &entities,
        PartialEntityUid::from_concrete(gateway),
        peer,
        None,
        Direction::Egress,
    )
    .unwrap();
    assert_eq!(outcome.verdict, Verdict::Unknown);
    assert_eq!(outcome.residuals.len(), 1, "{:?}", outcome.residuals);

    let mut discharger =
        Discharger::new(&entities, &["10.244.0.0/16".to_string()]).unwrap_or_else(|e| {
            panic!("starting the discharger (is cvc5 installed, or $CVC5 set?): {e:?}")
        });
    let pruned = discharger
        .refine(&f.policy_set, &mut outcome, true)
        .unwrap();

    assert_eq!(pruned.len(), 1, "{pruned:?}");
    assert!(
        pruned[0].contains("egress-to-internet"),
        "the pruned policy is named for its Kubernetes object: {pruned:?}"
    );
    assert_eq!(
        outcome.verdict,
        Verdict::Deny,
        "nothing else could allow gateway to reach a pod peer"
    );
    assert!(outcome.residuals.is_empty());
}
