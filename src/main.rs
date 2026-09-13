//! `np2cedar` — compile Kubernetes NetworkPolicies to Cedar, and query them.

use std::io::Write as _;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use cedar_policy::{EntityUid, PartialEntities, PartialEntityUid, PolicySet};
use clap::{ArgGroup, Args, Parser, Subcommand};

use k8s_networkpolicy_smt::compile::{Direction, compile};
use k8s_networkpolicy_smt::entities::{build, ip_endpoint_entities, pod_uid};
use k8s_networkpolicy_smt::eval::{Outcome, Verdict, combine, context, evaluate};
use k8s_networkpolicy_smt::load::{Loaded, load};
use k8s_networkpolicy_smt::{SCHEMA_SRC, schema};

#[derive(Parser)]
#[command(
    name = "np2cedar",
    about = "Compile Kubernetes NetworkPolicies to Cedar and evaluate connections",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Where to read Kubernetes objects from. Exactly one source must be chosen.
#[derive(Args)]
#[command(group = ArgGroup::new("source").required(true).multiple(false)
    .args(["filenames", "cluster"]))]
struct Source {
    /// Files, directories (recursed) or `-` for stdin.
    #[arg(short = 'f', long = "filename", num_args = 1..)]
    filenames: Vec<PathBuf>,
    /// Read live objects from a Kubernetes cluster.
    #[arg(long)]
    cluster: bool,
    #[command(flatten)]
    cluster_options: ClusterOptions,
    /// Write here instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

/// Where to get the policy set and entity store from. Either a pair of files
/// produced earlier, or objects to compile on the spot.
#[derive(Args)]
#[command(group = ArgGroup::new("store").required(true).multiple(false)
    .args(["policies", "filenames", "cluster"]))]
struct Store {
    /// A `.cedar` file produced by `np2cedar compile`. Needs --entities too.
    #[arg(long, requires = "entities")]
    policies: Option<PathBuf>,
    /// An entities file produced by `np2cedar entities`. Needs --policies too.
    #[arg(long, requires = "policies")]
    entities: Option<PathBuf>,
    /// Compile these files on the fly instead of using --policies/--entities.
    #[arg(short = 'f', long = "filename", num_args = 1..)]
    filenames: Vec<PathBuf>,
    /// Compile live cluster objects on the fly.
    #[arg(long)]
    cluster: bool,
    #[command(flatten)]
    cluster_options: ClusterOptions,
}

/// Applies wherever `--cluster` does. Registered even when the `cluster` feature is
/// off, so that using it then produces an actionable error rather than an unknown
/// flag, and so the argument graph is identical in both builds.
#[derive(Args, Clone, Default)]
struct ClusterOptions {
    /// An explicit kubeconfig file (default: $KUBECONFIG, then the usual place).
    #[arg(long)]
    kubeconfig: Option<PathBuf>,
    /// kubeconfig context (default: the current one, else the in-cluster account).
    #[arg(long)]
    context: Option<String>,
    /// Restrict the pods fetched. Policies are always read cluster-wide.
    #[arg(short = 'n', long = "namespace")]
    namespaces: Vec<String>,
    /// Label selector applied to the pod listing.
    #[arg(short = 'l', long)]
    selector: Option<String>,
    /// Keep Succeeded/Failed pods, whose recorded address is most likely stale.
    #[arg(long)]
    include_terminated: bool,
}

impl ClusterOptions {
    /// Reject cluster flags passed without `--cluster`.
    ///
    /// clap's `requires = "cluster"` cannot express this: `--cluster` is a `SetTrue`
    /// flag, which counts as *present* even when it was not passed because it has a
    /// default value, so the constraint never fires. Left to clap, a mistyped
    /// invocation would silently read files while looking like it queried a cluster.
    fn reject_unless_cluster(&self, cluster: bool) -> Result<()> {
        if cluster {
            return Ok(());
        }
        let stray: Vec<&str> = [
            self.kubeconfig.is_some().then_some("--kubeconfig"),
            self.context.is_some().then_some("--context"),
            (!self.namespaces.is_empty()).then_some("--namespace"),
            self.selector.is_some().then_some("--selector"),
            self.include_terminated.then_some("--include-terminated"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if stray.is_empty() {
            return Ok(());
        }
        bail!("{} only applies together with --cluster", stray.join(", "))
    }
}

#[cfg(feature = "cluster")]
fn fetch_cluster(options: &ClusterOptions) -> Result<Loaded> {
    let fetched =
        k8s_networkpolicy_smt::cluster::fetch(&k8s_networkpolicy_smt::cluster::Options {
            kubeconfig: options.kubeconfig.clone(),
            context: options.context.clone(),
            namespaces: options.namespaces.clone(),
            selector: options.selector.clone(),
            include_terminated: options.include_terminated,
        })?;
    for warning in &fetched.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(fetched.loaded)
}

#[cfg(not(feature = "cluster"))]
fn fetch_cluster(_options: &ClusterOptions) -> Result<Loaded> {
    bail!(
        "this binary was built without the `cluster` feature, so it cannot talk to a \
         Kubernetes API server; rebuild with `cargo build --features cluster`"
    )
}

impl Source {
    fn load(&self) -> Result<Loaded> {
        self.cluster_options.reject_unless_cluster(self.cluster)?;
        if self.cluster {
            fetch_cluster(&self.cluster_options)
        } else {
            load(&self.filenames)
        }
    }
}

impl Store {
    /// The policy set and entity store, however the caller chose to name them.
    fn resolve(&self) -> Result<(PolicySet, serde_json::Value)> {
        self.cluster_options.reject_unless_cluster(self.cluster)?;
        if let (Some(policies), Some(entities)) = (&self.policies, &self.entities) {
            return Ok((read_policies(policies)?, read_entity_json(entities)?));
        }
        let loaded = if self.cluster {
            fetch_cluster(&self.cluster_options)?
        } else {
            load(&self.filenames)?
        };
        let compiled = compile(&loaded.network_policies)?;
        let built = build(&loaded.pods, &loaded.namespaces)?;
        note_unknown_ips(&built.unknown_ips);
        Ok((compiled.policy_set, built.json))
    }
}

fn note_unknown_ips(unknown: &[String]) {
    if !unknown.is_empty() {
        eprintln!(
            "note: no status.podIPs for {}; their addresses are left unknown, so \
             ipBlock rules over them evaluate to a residual rather than a decision",
            unknown.join(", ")
        );
    }
}

#[derive(Subcommand)]
enum Command {
    /// Compile NetworkPolicies into a Cedar policy set.
    Compile(Source),
    /// Build the Cedar entity store from Pods and Namespaces.
    Entities(Source),
    /// Decide whether one endpoint may connect to another.
    Check {
        #[command(flatten)]
        store: Store,
        /// Source pod, as `namespace/name`.
        #[arg(long)]
        from: String,
        /// Destination pod (`namespace/name`) or a bare IP address.
        #[arg(long)]
        to: String,
        #[arg(long)]
        port: i64,
        #[arg(long, default_value = "TCP")]
        protocol: String,
    },
    /// Show what a pod may reach, as residual policies over a symbolic peer.
    Reachable {
        #[command(flatten)]
        store: Store,
        /// Source pod, as `namespace/name`.
        #[arg(long)]
        from: String,
        /// Leave unset to keep the port unknown as well.
        #[arg(long)]
        port: Option<i64>,
        #[arg(long, default_value = "TCP")]
        protocol: String,
    },
    /// Print the Cedar schema these tools compile against.
    Schema {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:?}");
            ExitCode::from(3)
        }
    }
}

fn run() -> Result<ExitCode> {
    match Cli::parse().command {
        Command::Compile(source) => {
            let loaded = source.load()?;
            let compiled = compile(&loaded.network_policies)?;
            emit(&source.output, compiled.source.as_bytes())?;
            Ok(ExitCode::SUCCESS)
        }

        Command::Entities(source) => {
            let loaded = source.load()?;
            let built = build(&loaded.pods, &loaded.namespaces)?;
            let mut text = serde_json::to_string_pretty(&built.json)?;
            text.push('\n');
            emit(&source.output, text.as_bytes())?;
            note_unknown_ips(&built.unknown_ips);
            Ok(ExitCode::SUCCESS)
        }

        Command::Check {
            store,
            from,
            to,
            port,
            protocol,
        } => {
            let (policies, mut entity_json) = store.resolve()?;

            let principal = pod_ref(&from)?;
            let resource = match to.parse::<IpAddr>() {
                // A bare address is an endpoint outside the cluster; synthesise it,
                // with its address known, since it is what the caller asked about.
                Ok(address) => {
                    let address = address.to_string();
                    if let serde_json::Value::Array(items) = &mut entity_json {
                        items.extend(ip_endpoint_entities(&address));
                    }
                    format!(r#"IpEndpoint::"{address}""#).parse()?
                }
                Err(_) => pod_ref(&to)?,
            };

            let entities = PartialEntities::from_json_value(entity_json, schema())
                .context("loading the entity store")?;
            let context = context(port, &protocol)?;

            let outcomes = Direction::ALL
                .into_iter()
                .map(|direction| {
                    evaluate(
                        &policies,
                        &entities,
                        PartialEntityUid::from_concrete(principal.clone()),
                        PartialEntityUid::from_concrete(resource.clone()),
                        Some(&context),
                        direction,
                    )
                })
                .collect::<Result<Vec<_>>>()?;

            for outcome in &outcomes {
                report(outcome, &from, &to, port, &protocol);
            }
            let verdict = combine(&outcomes);
            match verdict {
                Verdict::Allow => println!("=> ALLOWED"),
                Verdict::Deny => {
                    let blocked: Vec<String> = outcomes
                        .iter()
                        .filter(|o| o.verdict == Verdict::Deny)
                        .map(|o| o.direction.to_string())
                        .collect();
                    println!("=> DENIED (blocked on {})", blocked.join(" and "));
                }
                Verdict::Unknown => println!("=> UNKNOWN (see the residuals above)"),
            }
            Ok(exit_code(verdict))
        }

        Command::Reachable {
            store,
            from,
            port,
            protocol,
        } => {
            let (policies, entity_json) = store.resolve()?;
            let entities = PartialEntities::from_json_value(entity_json, schema())
                .context("loading the entity store")?;

            let principal = pod_ref(&from)?;
            let context = port.map(|port| context(port, &protocol)).transpose()?;
            // Any Pod, left symbolic: the residuals describe which ones qualify.
            let symbolic = PartialEntityUid::new("Pod".parse()?, None);

            for direction in Direction::ALL {
                let outcome = evaluate(
                    &policies,
                    &entities,
                    PartialEntityUid::from_concrete(principal.clone()),
                    symbolic.clone(),
                    context.as_ref(),
                    direction,
                )?;
                println!("# {direction} from {from} to any Pod: {}", outcome.verdict);
                for residual in &outcome.residuals {
                    println!("{residual}\n");
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Schema { output } => {
            emit(&output, SCHEMA_SRC.as_bytes())?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn report(outcome: &Outcome, from: &str, to: &str, port: i64, protocol: &str) {
    println!(
        "{:<8} {from} -> {to}  {protocol}:{port}  {}{}",
        outcome.direction.to_string(),
        outcome.verdict,
        if outcome.reasons.is_empty() {
            String::new()
        } else {
            format!("  ({})", outcome.reasons.join(", "))
        }
    );
    for residual in &outcome.residuals {
        for line in residual.lines() {
            println!("    {line}");
        }
    }
}

fn exit_code(verdict: Verdict) -> ExitCode {
    match verdict {
        Verdict::Allow => ExitCode::SUCCESS,
        Verdict::Deny => ExitCode::from(1),
        Verdict::Unknown => ExitCode::from(2),
    }
}

fn pod_ref(reference: &str) -> Result<EntityUid> {
    let Some((namespace, name)) = reference.split_once('/') else {
        bail!("{reference:?} is not a `namespace/name` pod reference");
    };
    if namespace.is_empty() || name.is_empty() || name.contains('/') {
        bail!("{reference:?} is not a `namespace/name` pod reference");
    }
    Ok(format!(r#"Pod::"{}""#, pod_uid(namespace, name)).parse()?)
}

fn read_policies(path: &Path) -> Result<PolicySet> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.parse()
        .with_context(|| format!("parsing {} as Cedar", path.display()))
}

fn read_entity_json(path: &Path) -> Result<serde_json::Value> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {} as JSON", path.display()))
}

fn emit(output: &Option<PathBuf>, bytes: &[u8]) -> Result<()> {
    match output {
        Some(path) if path.as_os_str() != "-" => {
            std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
        }
        _ => std::io::stdout()
            .write_all(bytes)
            .context("writing to stdout"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn the_argument_graph_is_well_formed() {
        // Catches the mistakes clap can only find at runtime: a group naming an
        // argument that does not exist, a duplicated id, a bad `requires`.
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Result<(), clap::Error> {
        Cli::try_parse_from(std::iter::once("np2cedar").chain(args.iter().copied())).map(|_| ())
    }

    #[test]
    fn exactly_one_source_is_required() {
        let query = ["check", "--from", "a/b", "--to", "c/d", "--port", "80"];
        let with = |extra: &[&str]| {
            let mut args = query.to_vec();
            args.extend_from_slice(extra);
            parse(&args)
        };

        assert!(with(&[]).is_err(), "a source must be named");
        assert!(with(&["-f", "manifests"]).is_ok());
        assert!(with(&["--cluster"]).is_ok());
        assert!(with(&["--policies", "p.cedar", "--entities", "e.json"]).is_ok());

        assert!(
            with(&["--cluster", "-f", "manifests"]).is_err(),
            "files and a cluster are different answers to the same question"
        );
        assert!(with(&["--cluster", "--policies", "p.cedar", "--entities", "e.json"]).is_err());
    }

    #[test]
    fn policies_and_entities_come_as_a_pair() {
        assert!(
            parse(&[
                "check",
                "--policies",
                "p.cedar",
                "--from",
                "a/b",
                "--to",
                "c/d",
                "--port",
                "80"
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "check",
                "--entities",
                "e.json",
                "--from",
                "a/b",
                "--to",
                "c/d",
                "--port",
                "80"
            ])
            .is_err()
        );
    }

    #[test]
    fn compile_and_entities_also_demand_a_source() {
        assert!(parse(&["compile"]).is_err());
        assert!(parse(&["compile", "-f", "manifests"]).is_ok());
        assert!(parse(&["compile", "--cluster"]).is_ok());
        assert!(parse(&["entities", "--cluster", "-n", "team-a"]).is_ok());
    }

    #[test]
    fn cluster_flags_are_rejected_without_cluster() {
        // clap's own `requires` cannot catch this, so the check is by hand and this
        // is what keeps it honest.
        let with_context = ClusterOptions {
            context: Some("prod".into()),
            ..Default::default()
        };
        assert!(with_context.reject_unless_cluster(true).is_ok());
        let error = with_context
            .reject_unless_cluster(false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("--context"), "{error}");

        let scoped = ClusterOptions {
            namespaces: vec!["team-a".into()],
            ..Default::default()
        };
        assert!(scoped.reject_unless_cluster(false).is_err());

        assert!(
            ClusterOptions::default()
                .reject_unless_cluster(false)
                .is_ok()
        );
    }
}
