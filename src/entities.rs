//! `Pod`/`Namespace` -> Cedar partial-entity JSON.
//!
//! The output is deliberately the *partial* entity format: an entity whose `attrs`
//! key is absent has unknown attributes. That is how a Pod with no `status.podIPs`
//! is represented, and it is why every evaluation path goes through TPE rather than
//! `Authorizer::is_authorized` — concrete `Entities` parsing rejects such entities.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use k8s_openapi::api::core::v1::{Namespace, Pod};
use serde_json::{Value, json};

/// The label Kubernetes sets on every namespace, so `namespaceSelector` can pick a
/// namespace out by name without the object having been handed to us.
const NAMESPACE_NAME_LABEL: &str = "kubernetes.io/metadata.name";

pub struct BuiltEntities {
    pub json: Value,
    /// `namespace/name` of every pod whose address we left unknown.
    pub unknown_ips: Vec<String>,
}

/// Cedar entity UID of a Pod.
pub fn pod_uid(namespace: &str, name: &str) -> String {
    format!("{namespace}/{name}")
}

/// Build the entity store. Namespaces referenced by a pod but not supplied are
/// synthesised carrying only the name label Kubernetes would have applied.
pub fn build(pods: &[Pod], namespaces: &[Namespace]) -> Result<BuiltEntities> {
    let mut by_name: BTreeMap<String, Option<&Namespace>> = BTreeMap::new();
    for ns in namespaces {
        let Some(name) = ns.metadata.name.clone() else {
            bail!("Namespace is missing metadata.name");
        };
        by_name.insert(name, Some(ns));
    }

    let mut out: Vec<Value> = Vec::new();
    let mut unknown_ips: Vec<String> = Vec::new();

    // Sorted, so the output depends only on the objects and not on the order they
    // arrived in. A cluster listing has server-determined order, which would
    // otherwise make the same cluster produce two different entity files.
    let mut sorted: Vec<(String, String, &Pod)> = pods
        .iter()
        .map(|pod| {
            let Some(name) = pod.metadata.name.clone() else {
                bail!("Pod is missing metadata.name");
            };
            let Some(namespace) = pod.metadata.namespace.clone() else {
                bail!("Pod {name} is missing metadata.namespace");
            };
            Ok((namespace, name, pod))
        })
        .collect::<Result<_>>()?;
    sorted.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));

    for (namespace, name, pod) in sorted {
        by_name.entry(namespace.clone()).or_insert(None);

        let uid = pod_uid(&namespace, &name);
        let labels_id = format!("pod/{uid}");
        let ports_id = format!("ports/{uid}");

        out.push(json!({
            "uid": json!({ "type": "Pod", "id": uid }),
            "attrs": {
                "name": name,
                "namespace": entity_ref("Namespace", &namespace),
                "labels": entity_ref("StringStringMap", &labels_id),
                "namedPorts": entity_ref("StringLongMap", &ports_id),
                "ip": entity_ref("IpAddr", &uid),
            },
            "parents": [],
            "tags": {},
        }));
        out.push(tag_entity(
            "StringStringMap",
            &labels_id,
            pod.metadata
                .labels
                .iter()
                .flatten()
                .map(|(k, v)| (k.clone(), json!(v))),
        ));
        out.push(tag_entity(
            "StringLongMap",
            &ports_id,
            named_ports(pod)?.into_iter().map(|(k, v)| (k, json!(v))),
        ));

        match pod_ip(pod) {
            Some(addr) => out.push(ip_addr_entity(&uid, Some(addr))),
            None => {
                out.push(ip_addr_entity(&uid, None));
                unknown_ips.push(uid);
            }
        }
    }

    for (name, ns) in &by_name {
        let labels_id = format!("ns/{name}");
        out.push(json!({
            "uid": json!({ "type": "Namespace", "id": name }),
            "attrs": { "name": name, "labels": entity_ref("StringStringMap", &labels_id) },
            "parents": [],
            "tags": {},
        }));

        let mut labels: BTreeMap<String, String> = ns
            .and_then(|ns| ns.metadata.labels.clone())
            .unwrap_or_default();
        labels
            .entry(NAMESPACE_NAME_LABEL.to_string())
            .or_insert_with(|| name.clone());
        out.push(tag_entity(
            "StringStringMap",
            &labels_id,
            labels.into_iter().map(|(k, v)| (k, json!(v))),
        ));
    }

    Ok(BuiltEntities {
        json: Value::Array(out),
        unknown_ips,
    })
}

/// An `IpEndpoint` plus its address, for a peer outside the cluster. Its address is
/// always known: it is the thing the caller asked about.
pub fn ip_endpoint_entities(addr: &str) -> Vec<Value> {
    let id = format!("ip/{addr}");
    vec![
        json!({
            "uid": json!({ "type": "IpEndpoint", "id": addr }),
            "attrs": { "ip": entity_ref("IpAddr", &id) },
            "parents": [],
            "tags": {},
        }),
        ip_addr_entity(&id, Some(addr)),
    ]
}

/// `Some(addr)` emits a concrete address; `None` omits `attrs` entirely, which the
/// partial-entity format reads as "unknown".
fn ip_addr_entity(id: &str, addr: Option<&str>) -> Value {
    let mut entity = json!({
        "uid": json!({ "type": "IpAddr", "id": id }),
        "parents": [],
        "tags": {},
    });
    if let Some(addr) = addr {
        entity["attrs"] = json!({ "addr": { "__extn": { "fn": "ip", "arg": addr } } });
    }
    entity
}

fn tag_entity(ty: &str, id: &str, tags: impl Iterator<Item = (String, Value)>) -> Value {
    json!({
        "uid": json!({ "type": ty, "id": id }),
        "attrs": {},
        "parents": [],
        "tags": tags.collect::<serde_json::Map<_, _>>(),
    })
}

fn entity_ref(ty: &str, id: &str) -> Value {
    json!({ "__entity": { "type": ty, "id": id } })
}

/// `status.podIPs` is the canonical (dual-stack) list and `status.podIP` mirrors its
/// first entry. We model a single address for now — see docs/README.md, planned item 2.
///
/// Absent means *unknown*, never "no address": we never invent one.
fn pod_ip(pod: &Pod) -> Option<&str> {
    let status = pod.status.as_ref()?;
    status
        .pod_ips
        .as_ref()
        .and_then(|ips| ips.first())
        .map(|entry| entry.ip.as_str())
        .or(status.pod_ip.as_deref())
}

/// Named container ports, which `NetworkPolicyPort.port` may refer to by name.
fn named_ports(pod: &Pod) -> Result<BTreeMap<String, i32>> {
    let Some(spec) = &pod.spec else {
        return Ok(BTreeMap::new());
    };
    let mut ports = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let containers = spec
        .containers
        .iter()
        .chain(spec.init_containers.iter().flatten());
    for container in containers {
        for port in container.ports.iter().flatten() {
            let Some(name) = &port.name else { continue };
            if !seen.insert(name.clone()) && ports[name] != port.container_port {
                bail!(
                    "Pod {}: port name {name:?} is used for two different container ports",
                    pod.metadata.name.as_deref().unwrap_or("<unnamed>")
                );
            }
            ports.insert(name.clone(), port.container_port);
        }
    }
    Ok(ports)
}
