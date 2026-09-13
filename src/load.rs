//! Loading Kubernetes objects from YAML/JSON files, directories or stdin.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use k8s_openapi::api::core::v1::{Namespace, Pod};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use serde::Deserialize as _;
use yaml_serde::Value;

#[derive(Default)]
pub struct Loaded {
    pub network_policies: Vec<NetworkPolicy>,
    pub pods: Vec<Pod>,
    pub namespaces: Vec<Namespace>,
}

/// Read every object from `paths`. Each entry is a file, a directory (recursed for
/// `*.yaml`/`*.yml`/`*.json`, in sorted order so output is reproducible), or `-` for
/// stdin. Multi-document YAML is split, and objects of different kinds may share a
/// file. JSON parses too, since YAML is a superset of it.
pub fn load(paths: &[PathBuf]) -> Result<Loaded> {
    let mut loaded = Loaded::default();
    for path in paths {
        if path.as_os_str() == "-" {
            let text = std::io::read_to_string(std::io::stdin()).context("reading stdin")?;
            read_documents(&text, "<stdin>", &mut loaded)?;
        } else if path.is_dir() {
            for file in manifest_files(path)? {
                let text = std::fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))?;
                read_documents(&text, &file.display().to_string(), &mut loaded)?;
            }
        } else {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            read_documents(&text, &path.display().to_string(), &mut loaded)?;
        }
    }
    Ok(loaded)
}

fn manifest_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading directory {}", dir.display()))?
        .map(|entry| Ok(entry?.path()))
        .collect::<Result<_>>()?;
    entries.sort();
    for entry in entries {
        if entry.is_dir() {
            files.extend(manifest_files(&entry)?);
        } else if matches!(
            entry.extension().and_then(|e| e.to_str()),
            Some("yaml" | "yml" | "json")
        ) {
            files.push(entry);
        }
    }
    Ok(files)
}

fn read_documents(text: &str, source: &str, loaded: &mut Loaded) -> Result<()> {
    for (index, document) in yaml_serde::Deserializer::from_str(text).enumerate() {
        let value = Value::deserialize(document)
            .with_context(|| format!("{source}: document {index} is not valid YAML"))?;
        ingest(value, source, loaded).with_context(|| format!("{source}: document {index}"))?;
    }
    Ok(())
}

fn ingest(value: Value, source: &str, loaded: &mut Loaded) -> Result<()> {
    // A `---` separator with nothing after it, or a file of only comments.
    if value.is_null() {
        return Ok(());
    }

    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .context("object has no `kind`")?
        .to_string();

    match kind.as_str() {
        "NetworkPolicy" => loaded
            .network_policies
            .push(from_value(value, &kind, source)?),
        "Pod" => loaded.pods.push(from_value(value, &kind, source)?),
        "Namespace" => loaded.namespaces.push(from_value(value, &kind, source)?),
        _ => {
            let Some(item_kind) = kind.strip_suffix("List") else {
                bail!("unsupported kind {kind:?} (expected NetworkPolicy, Pod or Namespace)");
            };
            let Some(Value::Sequence(items)) = value.get("items").cloned() else {
                bail!("{kind} has no `items` sequence");
            };
            for mut item in items {
                // Items of a typed list (`PodList`) carry no `kind` of their own.
                if !item_kind.is_empty()
                    && item.get("kind").is_none()
                    && let Value::Mapping(map) = &mut item
                {
                    map.insert("kind".into(), item_kind.into());
                }
                ingest(item, source, loaded)?;
            }
        }
    }
    Ok(())
}

fn from_value<T: serde::de::DeserializeOwned>(value: Value, kind: &str, source: &str) -> Result<T> {
    yaml_serde::from_value(value).with_context(|| format!("{source}: parsing a {kind}"))
}
