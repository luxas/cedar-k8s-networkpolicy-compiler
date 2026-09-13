//! Synthesizing the implied `connect` policies: cedar-woodpecker's
//! privilege-escalation synthesis over one transition whose two sources are
//! the egress and ingress halves of the same connection.
//!
//! The schema's `connect` action has no compiled policies; its permits exist
//! only as this synthesis' output — every case in which a Pod may talk to a
//! Pod. The transition's condition identifies the three requests explicitly:
//! both intermediate resources are the connect request's destination, and both
//! intermediate contexts agree with the connect request's context, attribute
//! by attribute, the attribute list read from the schema itself. The equality
//! per attribute is load-bearing: woodpecker reifies each source request's
//! context as `context.context{i}`, and a constraint over an attribute with no
//! equality binding it is silently dropped as `true` by the elimination — an
//! over-approximation that would still print `[sound]`. Hence the guard that
//! all three actions declare identical context attributes.

use std::collections::BTreeSet;

use anyhow::{Context as _, Result, bail, ensure};
use cedar_policy::{ActionConstraint, EntityUid, Expression, PolicySet};
use cedar_policy_core::ast::Expr;
use cedar_woodpecker::escalate::Setup;
use cedar_woodpecker::expr::and_chain;
use cedar_woodpecker::{
    Budgets, Escalation, LoadedSchema, SourceEnv, Transition, escalate, evaluator_with,
    source_cubes,
};

use crate::SCHEMA_SRC;
use crate::symbolic::{check_cidrs, membership, solver_bootstrap};

/// Synthesize the `connect` permits implied by the compiled ingress/egress
/// policy set — over every possible pod, not one cluster's snapshot. Needs
/// cvc5. `pod_cidrs` (optional) additionally constrains both endpoints'
/// addresses, pruning rules that can only match the world outside the cluster.
pub fn synthesize(policies: &PolicySet, pod_cidrs: &[String]) -> Result<Vec<Escalation>> {
    check_cidrs(pod_cidrs)?;
    only_ingress_and_egress(policies)?;
    // The pipeline is async end to end; the runtime lives and dies here so the
    // callers stay synchronous.
    let (runtime, compiler) = solver_bootstrap()?;
    runtime.block_on(async {
        // Not crate::schema(): the schema extension works on the JSON form, so
        // the bundled source is parsed to both forms here.
        let loaded =
            LoadedSchema::parse(SCHEMA_SRC, false).context("loading the bundled schema")?;
        let mut evaluator = evaluator_with(compiler, &loaded.schema, &loaded.assumptions, None)
            .await
            .context("initialising the symbolic evaluator")?;
        let cubes = source_cubes(policies, &loaded.schema, &mut evaluator, Budgets::default())
            .await
            .context("splitting the policies into their explicit permissions")?;
        let transitions = [transition(pod_cidrs, &loaded.json)?];
        let setup = Setup {
            schema: &loaded.schema,
            schema_json: &loaded.json,
            assumptions: &loaded.assumptions,
            entities: None,
            budgets: Budgets::default(),
        };
        let (escalations, _compiler) =
            escalate(&cubes, &transitions, setup, evaluator.into_compiler())
                .await
                .context("synthesizing the connect policies")?;
        Ok(escalations)
    })
}

/// Reject a policy set that names any action beyond ingress and egress.
///
/// The synthesis treats `connect` as implied-only. An input policy that could
/// itself apply to `connect` — a previous synthesis' output fed back in, or an
/// unconstrained `action` scope — would be silently ignored by the
/// escalation's environment filter while the header still claimed
/// completeness, so the schema comment's "the compiler never emits policies
/// for this action" becomes a checked precondition here.
fn only_ingress_and_egress(policies: &PolicySet) -> Result<()> {
    let allowed: Vec<EntityUid> = ["ingress", "egress"]
        .map(|action| {
            format!(r#"Action::"{action}""#)
                .parse()
                .expect("the action names are literals")
        })
        .into();
    for policy in policies.policies() {
        let ok = match policy.action_constraint() {
            ActionConstraint::Eq(uid) => allowed.contains(&uid),
            ActionConstraint::In(uids) => uids.iter().all(|uid| allowed.contains(uid)),
            ActionConstraint::Any => false,
        };
        if !ok {
            bail!(
                "policy {} applies to actions other than ingress/egress; connect \
                 permits are synthesized output, never input",
                policy.id()
            );
        }
    }
    Ok(())
}

/// The one transition: holding egress and ingress for the same request grants
/// connect.
fn transition(pod_cidrs: &[String], schema_json: &serde_json::Value) -> Result<Transition> {
    // Source order is the packet's life: the source pod's egress rules let it
    // out (`context.*1`), the destination's ingress rules let it in
    // (`context.*2`). The principal — the connecting pod — is shared by all
    // three requests, and escalate itself pins `context.action{i}`.
    let sources = vec![
        SourceEnv {
            action: r#"Action::"egress""#.parse()?,
            resource: "Pod".parse()?,
        },
        SourceEnv {
            action: r#"Action::"ingress""#.parse()?,
            resource: "Pod".parse()?,
        },
    ];
    let target = SourceEnv {
        action: r#"Action::"connect""#.parse()?,
        resource: "Pod".parse()?,
    };

    // The context equalities are generated from the schema, not hand-listed,
    // and the shapes must match exactly: an attribute added to one action but
    // not equated here would be dropped, not carried (see the module doc).
    let attrs = context_attrs(schema_json, "connect")?;
    for action in ["ingress", "egress"] {
        let other = context_attrs(schema_json, action)?;
        ensure!(
            other == attrs,
            "the {action} and connect actions must declare identical context \
             attributes, got {other:?} vs {attrs:?}: an attribute without its \
             when-equality would be silently dropped by the elimination"
        );
    }

    // Both legs are about the connect request's destination, port and protocol.
    let mut conjuncts: Vec<Expr> = Vec::new();
    for i in 1..=sources.len() {
        conjuncts.push(parse_expr(&format!("context.resource{i} == resource"))?);
    }
    for i in 1..=sources.len() {
        for attr in &attrs {
            conjuncts.push(parse_expr(&format!(
                r#"context.context{i}["{attr}"] == context["{attr}"]"#
            ))?);
        }
    }
    if !pod_cidrs.is_empty() {
        // Both endpoints are Pods here, so constraining them to the pod CIDRs
        // is exactly right — an ipBlock rule that cannot intersect them can
        // never contribute a connect permit. This lives in the condition, not
        // in an evaluator assumption, which would wrongly constrain the
        // IpEndpoint request environments too.
        conjuncts.push(membership("principal.ip.addr", pod_cidrs)?);
        conjuncts.push(membership("resource.ip.addr", pod_cidrs)?);
    }
    let when = and_chain(conjuncts).expect("the resource equalities are always present");

    Ok(Transition {
        name: "connect".into(),
        principal: Some(vec!["Pod".parse()?]),
        sources,
        target,
        when,
    })
}

/// The context attribute names of `action`, from the schema's JSON form.
fn context_attrs(schema_json: &serde_json::Value, action: &str) -> Result<BTreeSet<String>> {
    let attrs: BTreeSet<String> = schema_json
        .get("")
        .and_then(|namespace| namespace.get("actions"))
        .and_then(|actions| actions.get(action))
        .and_then(|action| action.get("appliesTo"))
        .and_then(|applies_to| applies_to.get("context"))
        .and_then(|context| context.get("attributes"))
        .and_then(|attributes| attributes.as_object())
        .map(|attributes| attributes.keys().cloned().collect())
        .with_context(|| format!("the schema JSON has no inline context record for {action:?}"))?;
    for attr in &attrs {
        // The names are spliced into Cedar text as string literals.
        ensure!(
            !attr.contains(['"', '\\']),
            "context attribute {attr:?} cannot be quoted into a Cedar string"
        );
    }
    Ok(attrs)
}

/// Cedar text to a bare expression, the same route eval.rs takes.
fn parse_expr(text: &str) -> Result<Expr> {
    let expression: Expression = text
        .parse()
        .with_context(|| format!("building the transition conjunct {text:?}"))?;
    Ok(expression.as_ref().clone())
}
