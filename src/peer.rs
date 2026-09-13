//! `NetworkPolicyPeer` -> Cedar condition.

use anyhow::{Result, bail};
use k8s_openapi::api::networking::v1::NetworkPolicyPeer;

use crate::expr::{Expr, entity_uid, string_lit};
use crate::selector::selector_expr;

/// Translate one `from`/`to` entry into a condition over `subject`, which is
/// `"principal"` for an ingress rule's `from` and `"resource"` for an egress
/// rule's `to`. `policy_ns` is the namespace of the owning NetworkPolicy.
///
/// `ipBlock` is mutually exclusive with the two selectors, and reads `subject.ip`,
/// which typechecks for both `Pod` and `IpEndpoint`. The selector forms need
/// `subject.labels`, which only `Pod` has, hence the `subject is Pod` guard.
pub fn peer_expr(peer: &NetworkPolicyPeer, subject: &str, policy_ns: &str) -> Result<Expr> {
    let has_selector = peer.pod_selector.is_some() || peer.namespace_selector.is_some();

    match (&peer.ip_block, has_selector) {
        (Some(_), true) => {
            bail!("peer: `ipBlock` cannot be combined with `podSelector`/`namespaceSelector`")
        }
        (None, false) => {
            bail!("peer: must set exactly one of `ipBlock`, `podSelector` or `namespaceSelector`")
        }

        (Some(block), false) => {
            let mut parts = vec![in_range(subject, &block.cidr)];
            for except in block.except.iter().flatten() {
                parts.push(in_range(subject, except).negate());
            }
            Ok(Expr::and(parts))
        }

        (None, true) => {
            let mut parts = vec![Expr::atom(format!("{subject} is Pod"))];
            match &peer.namespace_selector {
                // No namespaceSelector: the podSelector binds only pods in the
                // policy's own namespace.
                None => parts.push(Expr::atom(format!(
                    "{subject}.namespace == {}",
                    entity_uid("Namespace", policy_ns)
                ))),
                Some(ns_selector) => parts.push(selector_expr(
                    Some(ns_selector),
                    &format!("{subject}.namespace.labels"),
                )?),
            }
            parts.push(selector_expr(
                peer.pod_selector.as_ref(),
                &format!("{subject}.labels"),
            )?);
            Ok(Expr::and(parts))
        }
    }
}

fn in_range(subject: &str, cidr: &str) -> Expr {
    Expr::atom(format!(
        "{subject}.ip.addr.isInRange(ip({}))",
        string_lit(cidr)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::networking::v1::IPBlock;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;

    #[test]
    fn ip_block_subtracts_exceptions() {
        let peer = NetworkPolicyPeer {
            ip_block: Some(IPBlock {
                cidr: "0.0.0.0/0".into(),
                except: Some(vec!["10.0.0.0/8".into()]),
            }),
            ..Default::default()
        };
        assert_eq!(
            peer_expr(&peer, "resource", "default").unwrap().render(0),
            "resource.ip.addr.isInRange(ip(\"0.0.0.0/0\")) &&\n\
             !resource.ip.addr.isInRange(ip(\"10.0.0.0/8\"))"
        );
    }

    #[test]
    fn pod_selector_alone_is_scoped_to_the_policy_namespace() {
        let peer = NetworkPolicyPeer {
            pod_selector: Some(LabelSelector::default()),
            ..Default::default()
        };
        let rendered = peer_expr(&peer, "principal", "team-a").unwrap().render(0);
        assert_eq!(
            rendered,
            "principal is Pod &&\nprincipal.namespace == Namespace::\"team-a\""
        );
    }

    #[test]
    fn namespace_selector_reads_namespace_labels() {
        let peer = NetworkPolicyPeer {
            namespace_selector: Some(LabelSelector {
                match_labels: Some([("team".to_string(), "a".to_string())].into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let rendered = peer_expr(&peer, "principal", "default").unwrap().render(0);
        assert!(rendered.contains("principal.namespace.labels.hasTag(\"team\")"));
        // A namespaceSelector replaces, rather than adds to, the same-namespace check.
        assert!(!rendered.contains("principal.namespace =="));
    }

    #[test]
    fn rejects_ip_block_combined_with_a_selector_and_the_empty_peer() {
        let both = NetworkPolicyPeer {
            ip_block: Some(IPBlock {
                cidr: "10.0.0.0/8".into(),
                except: None,
            }),
            pod_selector: Some(LabelSelector::default()),
            ..Default::default()
        };
        assert!(peer_expr(&both, "principal", "default").is_err());
        assert!(peer_expr(&NetworkPolicyPeer::default(), "principal", "default").is_err());
    }
}
