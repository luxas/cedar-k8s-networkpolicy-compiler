//! Cross-check: the compiler's output must agree with a hand-written translation of
//! the same NetworkPolicy, decision for decision.

mod common;

use std::path::Path;

use cedar_policy::{PolicySet, ValidationMode, Validator};
use k8s_networkpolicy_smt::schema;

const PODS: [&str; 2] = ["default/productpage", "default/details"];
const PROTOCOLS: [&str; 3] = ["TCP", "UDP", "SCTP"];
const PORTS: [i64; 5] = [80, 100, 9080, 9081, 30000];

#[test]
fn compiled_bookinfo_agrees_with_the_hand_written_translation() {
    let compiled = common::fixture_at(Path::new("examples/bookinfo"));

    let text = std::fs::read_to_string("examples/handwritten/policies.cedar").unwrap();
    let hand_written: PolicySet = text.parse().expect("the hand-written policies must parse");
    let result = Validator::new(schema().clone()).validate(&hand_written, ValidationMode::Strict);
    assert!(
        result.validation_passed(),
        "the hand-written policies must pass strict validation: {:?}",
        result
            .validation_errors()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
    let hand_written = compiled.with_policies(hand_written);

    let mut disagreements = Vec::new();
    let mut rows = 0;
    for from in PODS {
        for to in PODS {
            for protocol in PROTOCOLS {
                for port in PORTS {
                    rows += 1;
                    let a = compiled.verdict(from, to, protocol, port);
                    let b = hand_written.verdict(from, to, protocol, port);
                    if a != b {
                        disagreements.push(format!(
                            "  {from} -> {to} {protocol}:{port}: compiled {a}, hand-written {b}"
                        ));
                    }
                }
            }
        }
    }
    assert!(
        disagreements.is_empty(),
        "{} of {rows} rows disagree:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
    assert!(rows > 0);
}
