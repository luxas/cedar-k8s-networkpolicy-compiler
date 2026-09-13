//! `NetworkPolicyPort` -> Cedar condition.

use anyhow::{Result, bail};
use k8s_openapi::api::networking::v1::NetworkPolicyPort;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use crate::expr::{Expr, entity_uid, string_lit};

/// Translate one `ports` entry into a condition over `context`.
///
/// A named port always resolves against the *destination*, which is `resource` in
/// both directions. On an egress rule aimed at an `IpEndpoint` the `resource is Pod`
/// guard makes the whole clause `false` — correct, a container port name cannot
/// resolve to an address outside the cluster.
pub fn port_expr(port: &NetworkPolicyPort) -> Result<Expr> {
    let protocol = port.protocol.as_deref().unwrap_or("TCP");
    if !matches!(protocol, "TCP" | "UDP" | "SCTP") {
        bail!("ports: unknown protocol {protocol:?} (expected TCP, UDP or SCTP)");
    }
    let mut parts = vec![Expr::atom(format!(
        "context.protocol == {}",
        entity_uid("Protocol", protocol)
    ))];

    match (&port.port, port.end_port) {
        (None, None) => {}
        (None, Some(_)) => bail!("ports: `endPort` requires `port`"),

        (Some(IntOrString::Int(n)), None) => {
            parts.push(Expr::atom(format!("context.port == {n}")));
        }
        (Some(IntOrString::Int(n)), Some(end)) => {
            if end < *n {
                bail!("ports: `endPort` ({end}) must be >= `port` ({n})");
            }
            parts.push(Expr::atom(format!("context.port >= {n}")));
            parts.push(Expr::atom(format!("context.port <= {end}")));
        }

        (Some(IntOrString::String(name)), None) => {
            let lit = string_lit(name);
            parts.push(Expr::atom("resource is Pod".to_string()));
            parts.push(Expr::atom(format!("resource.namedPorts.hasTag({lit})")));
            parts.push(Expr::atom(format!(
                "context.port == resource.namedPorts.getTag({lit})"
            )));
        }
        (Some(IntOrString::String(name)), Some(_)) => {
            bail!("ports: `endPort` cannot be combined with the named port {name:?}")
        }
    }

    Ok(Expr::and(parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_defaults_to_tcp() {
        assert_eq!(
            port_expr(&NetworkPolicyPort::default()).unwrap().render(0),
            "context.protocol == Protocol::\"TCP\""
        );
    }

    #[test]
    fn numeric_port_and_range() {
        let single = NetworkPolicyPort {
            port: Some(IntOrString::Int(8080)),
            protocol: Some("UDP".into()),
            ..Default::default()
        };
        assert_eq!(
            port_expr(&single).unwrap().render(0),
            "context.protocol == Protocol::\"UDP\" &&\ncontext.port == 8080"
        );

        let range = NetworkPolicyPort {
            port: Some(IntOrString::Int(32000)),
            end_port: Some(32100),
            ..Default::default()
        };
        let rendered = port_expr(&range).unwrap().render(0);
        assert!(rendered.contains("context.port >= 32000"));
        assert!(rendered.contains("context.port <= 32100"));
    }

    #[test]
    fn named_port_resolves_against_the_destination() {
        let named = NetworkPolicyPort {
            port: Some(IntOrString::String("http".into())),
            ..Default::default()
        };
        let rendered = port_expr(&named).unwrap().render(0);
        assert!(rendered.contains("resource is Pod"));
        assert!(rendered.contains("context.port == resource.namedPorts.getTag(\"http\")"));
    }

    #[test]
    fn rejects_invalid_combinations() {
        let named_range = NetworkPolicyPort {
            port: Some(IntOrString::String("http".into())),
            end_port: Some(9000),
            ..Default::default()
        };
        assert!(port_expr(&named_range).is_err());

        let backwards = NetworkPolicyPort {
            port: Some(IntOrString::Int(9000)),
            end_port: Some(8000),
            ..Default::default()
        };
        assert!(port_expr(&backwards).is_err());

        let dangling_end = NetworkPolicyPort {
            end_port: Some(9000),
            ..Default::default()
        };
        assert!(port_expr(&dangling_end).is_err());

        let bad_proto = NetworkPolicyPort {
            protocol: Some("ICMP".into()),
            ..Default::default()
        };
        assert!(port_expr(&bad_proto).is_err());
    }
}
