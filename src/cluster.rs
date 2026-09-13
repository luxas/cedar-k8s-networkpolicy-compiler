//! Reading live objects from a Kubernetes API server.
//!
//! Produces the same [`Loaded`] the file loader does, so everything downstream —
//! `compile`, `entities::build`, `eval` — is unchanged and already tested.
//!
//! Note this only ever *reads* NetworkPolicy objects. Whether the cluster's CNI
//! actually enforces them is a separate question that this tool does not ask.

use std::future::Future;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use k8s_openapi::api::core::v1::{Namespace, Pod};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::api::ListParams;
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Api, Client, Config, Resource};

use crate::load::Loaded;

/// How many objects to ask for per request. The server may return fewer.
const PAGE_SIZE: u32 = 500;

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// An explicit kubeconfig file. `None` uses `$KUBECONFIG`, then the default
    /// location, then the in-cluster service account.
    pub kubeconfig: Option<PathBuf>,
    /// kubeconfig context. `None` uses the current context, falling back to the
    /// in-cluster service account.
    pub context: Option<String>,
    /// Restricts the **pod** listing only. Empty means every namespace.
    pub namespaces: Vec<String>,
    /// Label selector applied to the pod listing.
    pub selector: Option<String>,
    /// Keep pods in a terminal phase, whose recorded address is stale.
    pub include_terminated: bool,
}

pub struct Fetched {
    pub loaded: Loaded,
    /// Things the caller should know but that are not errors.
    pub warnings: Vec<String>,
}

/// Fetch NetworkPolicies, Pods and Namespaces from the cluster.
///
/// Synchronous: the tokio runtime lives and dies inside this call, so the rest of
/// the crate stays free of async.
pub fn fetch(options: &Options) -> Result<Fetched> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting a tokio runtime")?;
    runtime.block_on(fetch_async(options))
}

async fn fetch_async(options: &Options) -> Result<Fetched> {
    let client = connect(options).await?;
    let mut warnings = Vec::new();

    // NetworkPolicies are always read cluster-wide, even when `--namespace` narrows
    // the pods. A policy in another namespace still isolates the pods it selects, so
    // reading a subset would silently turn "isolated" into "not isolated" — the one
    // direction in which being wrong lets traffic through unnoticed.
    let network_policies =
        list_all::<NetworkPolicy>(&Api::all(client.clone()), &ListParams::default())
            .await
            .context(
                "listing NetworkPolicies (cluster-wide read is required for a sound answer)",
            )?;

    // Namespaces likewise: `namespaceSelector` reads labels off namespaces nobody
    // asked about. This is the one degradable step -- `entities::build` synthesises a
    // namespace carrying the name label Kubernetes applies, which is enough unless a
    // policy selects on some other namespace label.
    let namespaces =
        match list_all::<Namespace>(&Api::all(client.clone()), &ListParams::default()).await {
            Ok(namespaces) => namespaces,
            Err(error) => {
                warnings.push(format!(
                    "could not list namespaces ({error}); falling back to synthesising them from \
                 pod metadata, so a namespaceSelector on any label other than \
                 kubernetes.io/metadata.name will not match"
                ));
                Vec::new()
            }
        };

    let mut params = ListParams::default();
    if let Some(selector) = &options.selector {
        params = params.labels(selector);
    }
    let mut pods = Vec::new();
    if options.namespaces.is_empty() {
        pods.extend(
            list_all::<Pod>(&Api::all(client.clone()), &params)
                .await
                .context("listing pods in all namespaces")?,
        );
    } else {
        for namespace in &options.namespaces {
            pods.extend(
                list_all::<Pod>(&Api::namespaced(client.clone(), namespace), &params)
                    .await
                    .with_context(|| format!("listing pods in namespace {namespace}"))?,
            );
        }
    }

    let (pods, filter_warnings) = filter_pods(pods, options.include_terminated);
    warnings.extend(filter_warnings);

    Ok(Fetched {
        loaded: Loaded {
            network_policies,
            pods,
            namespaces,
        },
        warnings,
    })
}

async fn connect(options: &Options) -> Result<Client> {
    let selection = KubeConfigOptions {
        context: options.context.clone(),
        ..Default::default()
    };
    let config = match (&options.kubeconfig, &options.context) {
        (Some(path), _) => {
            let kubeconfig = Kubeconfig::read_from(path)
                .with_context(|| format!("reading kubeconfig {}", path.display()))?;
            Config::from_custom_kubeconfig(kubeconfig, &selection)
                .await
                .with_context(|| format!("loading kubeconfig {}", path.display()))?
        }
        (None, Some(context)) => Config::from_kubeconfig(&selection)
            .await
            .with_context(|| format!("loading kubeconfig context {context:?}"))?,
        // `infer` tries the kubeconfig first, then the in-cluster service account, so
        // the same binary works from a laptop and from inside a pod.
        (None, None) => Config::infer()
            .await
            .context("inferring a cluster configuration (is KUBECONFIG set?)")?,
    };
    let server = config.cluster_url.to_string();
    Client::try_from(config).with_context(|| format!("building a client for {server}"))
}

/// List every page of a collection, following `continue` tokens.
async fn list_all<K>(api: &Api<K>, params: &ListParams) -> Result<Vec<K>>
where
    K: Resource + Clone + serde::de::DeserializeOwned + std::fmt::Debug,
{
    drain(|token| {
        let mut page = params.clone().limit(PAGE_SIZE);
        page.continue_token = token;
        async move {
            let list = api.list(&page).await?;
            Ok((list.items, list.metadata.continue_))
        }
    })
    .await
}

/// Follow `continue` tokens until the server stops handing them out.
///
/// Kept free of `kube` types so the loop that actually decides when to stop can be
/// tested without a cluster or an HTTP stack.
async fn drain<T, F, Fut>(mut page: F) -> Result<Vec<T>>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<(Vec<T>, Option<String>)>>,
{
    let mut all = Vec::new();
    let mut token = None;
    loop {
        let (items, next) = page(token).await?;
        all.extend(items);
        match next {
            // An empty token means the same as no token; some servers send one.
            None => return Ok(all),
            Some(next) if next.is_empty() => return Ok(all),
            Some(next) => token = Some(next),
        }
    }
}

/// Drop pods we should not reason about, and report what was dropped or is suspect.
fn filter_pods(pods: Vec<Pod>, include_terminated: bool) -> (Vec<Pod>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut terminated = Vec::new();
    let mut host_network = Vec::new();

    let kept: Vec<Pod> = pods
        .into_iter()
        .filter(|pod| {
            if is_terminal(pod) && !include_terminated {
                terminated.push(describe(pod));
                return false;
            }
            if is_host_network(pod) {
                host_network.push(describe(pod));
            }
            true
        })
        .collect();

    if !terminated.is_empty() {
        warnings.push(format!(
            "skipped {} pod(s) in a terminal phase, whose recorded address has most likely \
             been reassigned by now: {} (pass --include-terminated to keep them)",
            terminated.len(),
            terminated.join(", ")
        ));
    }
    if !host_network.is_empty() {
        warnings.push(format!(
            "{} pod(s) use hostNetwork, so their address is their node's and NetworkPolicy's \
             pod-level semantics do not apply to them the way this model assumes: {}",
            host_network.len(),
            host_network.join(", ")
        ));
    }
    (kept, warnings)
}

/// A pod that has run to completion or failed. Its address is gone; keeping it would
/// answer questions about traffic that cannot happen.
///
/// `Pending` is deliberately *not* terminal: such a pod has no address yet, which the
/// unknown-address handling already represents correctly as a residual.
fn is_terminal(pod: &Pod) -> bool {
    matches!(
        pod.status.as_ref().and_then(|s| s.phase.as_deref()),
        Some("Succeeded" | "Failed")
    )
}

fn is_host_network(pod: &Pod) -> bool {
    pod.spec.as_ref().and_then(|s| s.host_network) == Some(true)
}

fn describe(pod: &Pod) -> String {
    format!(
        "{}/{}",
        pod.metadata
            .namespace
            .as_deref()
            .unwrap_or("<no namespace>"),
        pod.metadata.name.as_deref().unwrap_or("<no name>")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use k8s_openapi::api::core::v1::{PodSpec, PodStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::cell::RefCell;

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// What one canned page yields: some items, and the token for the next page.
    type Page = Result<(Vec<u32>, Option<String>)>;
    /// The tokens `drain` asked for, in order.
    type Asked = &'static RefCell<Vec<Option<String>>>;

    /// Serve a canned sequence of pages, recording the tokens we were asked with.
    fn paged(
        pages: Vec<(Vec<u32>, Option<&'static str>)>,
    ) -> (
        impl FnMut(Option<String>) -> std::future::Ready<Page>,
        Asked,
    ) {
        let seen: &'static RefCell<Vec<Option<String>>> =
            Box::leak(Box::new(RefCell::new(Vec::new())));
        let mut pages = pages.into_iter();
        let f = move |token: Option<String>| {
            seen.borrow_mut().push(token);
            let (items, next) = pages.next().expect("asked for more pages than were canned");
            std::future::ready(Ok((items, next.map(str::to_string))))
        };
        (f, seen)
    }

    #[test]
    fn drain_follows_the_continue_chain_in_order() {
        let (pages, seen) = paged(vec![
            (vec![1, 2], Some("t1")),
            (vec![3, 4], Some("t2")),
            (vec![5], None),
        ]);
        assert_eq!(block_on(drain(pages)).unwrap(), vec![1, 2, 3, 4, 5]);
        assert_eq!(
            *seen.borrow(),
            vec![None, Some("t1".to_string()), Some("t2".to_string())]
        );
    }

    #[test]
    fn drain_stops_on_a_single_untokened_page() {
        let (pages, seen) = paged(vec![(vec![1], None)]);
        assert_eq!(block_on(drain(pages)).unwrap(), vec![1]);
        assert_eq!(seen.borrow().len(), 1);
    }

    #[test]
    fn drain_treats_an_empty_token_as_the_end() {
        // Some API servers send "" rather than omitting the field. Following it would
        // request the same page forever.
        let (pages, _) = paged(vec![(vec![1], Some(""))]);
        assert_eq!(block_on(drain(pages)).unwrap(), vec![1]);
    }

    #[test]
    fn drain_keeps_going_through_an_empty_middle_page() {
        let (pages, _) = paged(vec![
            (vec![1], Some("t1")),
            (vec![], Some("t2")),
            (vec![2], None),
        ]);
        assert_eq!(block_on(drain(pages)).unwrap(), vec![1, 2]);
    }

    #[test]
    fn drain_propagates_an_error_rather_than_returning_short() {
        let mut call = 0;
        let result = block_on(drain(|_| {
            call += 1;
            let outcome = if call == 1 {
                Ok((vec![1], Some("t1".to_string())))
            } else {
                Err(anyhow!("the server hung up"))
            };
            std::future::ready(outcome)
        }));
        assert!(
            result.is_err(),
            "a mid-chain failure must not look like a complete listing"
        );
    }

    fn pod(name: &str, phase: Option<&str>, host_network: Option<bool>) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                host_network,
                ..Default::default()
            }),
            status: phase.map(|phase| PodStatus {
                phase: Some(phase.into()),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn only_succeeded_and_failed_count_as_terminal() {
        assert!(is_terminal(&pod("a", Some("Succeeded"), None)));
        assert!(is_terminal(&pod("a", Some("Failed"), None)));
        assert!(!is_terminal(&pod("a", Some("Running"), None)));
        // Pending pods have no address yet, which is *unknown*, not absent.
        assert!(!is_terminal(&pod("a", Some("Pending"), None)));
        assert!(!is_terminal(&pod("a", None, None)));
    }

    #[test]
    fn host_network_is_detected_and_defaults_to_false() {
        assert!(is_host_network(&pod("a", None, Some(true))));
        assert!(!is_host_network(&pod("a", None, Some(false))));
        assert!(!is_host_network(&pod("a", None, None)));
    }

    #[test]
    fn terminal_pods_are_dropped_unless_asked_for() {
        let pods = vec![
            pod("running", Some("Running"), None),
            pod("done", Some("Succeeded"), None),
        ];

        let (kept, warnings) = filter_pods(pods.clone(), false);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].metadata.name.as_deref(), Some("running"));
        assert!(warnings.iter().any(|w| w.contains("default/done")));

        let (kept, warnings) = filter_pods(pods, true);
        assert_eq!(kept.len(), 2);
        assert!(warnings.is_empty());
    }

    #[test]
    fn host_network_pods_are_kept_but_flagged() {
        let (kept, warnings) =
            filter_pods(vec![pod("node-agent", Some("Running"), Some(true))], false);
        assert_eq!(kept.len(), 1, "a hostNetwork pod is still a real endpoint");
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("hostNetwork") && w.contains("default/node-agent"))
        );
    }
}
