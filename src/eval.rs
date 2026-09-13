//! Evaluating a compiled policy set against the entity store.
//!
//! Every path here goes through type-aware partial evaluation, including the plain
//! `check`. A Pod whose `status.podIPs` we were never given has an unknown address,
//! so a rule that tests that address cannot be decided — and saying so is the
//! correct answer, not a limitation. See docs/concepts/partial-evaluation.md.

use anyhow::{Context as _, Result};
use cedar_policy::{
    Context, Decision, EntityTypeName, EntityUid, PartialEntities, PartialEntityUid,
    PartialRequest, PolicyId, PolicySet, RequestEnv, RestrictedExpression,
};
use cedar_policy_core::ast::Expr;

use crate::compile::Direction;
use crate::schema;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Verdict {
    Allow,
    Deny,
    /// Undecidable from what we know — the residuals say what is still missing.
    Unknown,
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Verdict::Allow => "ALLOW",
            Verdict::Deny => "DENY",
            Verdict::Unknown => "UNKNOWN",
        })
    }
}

pub struct Outcome {
    pub direction: Direction,
    pub verdict: Verdict,
    /// Where the decision came from, named as Kubernetes objects.
    pub reasons: Vec<String>,
    /// Residual policy text, when the verdict is `Unknown`.
    pub residuals: Vec<String>,
    /// Aligned with `residuals`: that policy's id and its bare condition when it is
    /// a permit — which is every policy this compiler emits. A symbolic discharger
    /// re-evaluates the condition under extra assumptions (see [`crate::symbolic`]).
    pub residual_conditions: Vec<(PolicyId, Option<Expr>)>,
    /// The request environment the outcome was evaluated in.
    pub env: RequestEnv,
}

/// Kubernetes requires both ends to agree, so the connection verdict is the
/// conjunction: `Allow` only if both directions allow, `Deny` if either denies,
/// and otherwise `Unknown`.
pub fn combine(outcomes: &[Outcome]) -> Verdict {
    if outcomes.iter().any(|o| o.verdict == Verdict::Deny) {
        Verdict::Deny
    } else if outcomes.iter().all(|o| o.verdict == Verdict::Allow) {
        Verdict::Allow
    } else {
        Verdict::Unknown
    }
}

/// Build the request context. `protocol` must be one of the `Protocol` enum members.
pub fn context(port: i64, protocol: &str) -> Result<Context> {
    let protocol: EntityUid = format!(r#"Protocol::"{protocol}""#)
        .parse()
        .with_context(|| format!("{protocol:?} is not a valid protocol"))?;
    Context::from_pairs([
        ("port".to_string(), RestrictedExpression::new_long(port)),
        (
            "protocol".to_string(),
            RestrictedExpression::new_entity_uid(protocol),
        ),
    ])
    .context("building the request context")
}

/// Evaluate one direction. `principal` is always the source and `resource` always
/// the destination; only the action changes. Either endpoint may be left symbolic,
/// and omitting `context` leaves the port and protocol unknown.
pub fn evaluate(
    policies: &PolicySet,
    entities: &PartialEntities,
    principal: PartialEntityUid,
    resource: PartialEntityUid,
    context: Option<&Context>,
    direction: Direction,
) -> Result<Outcome> {
    let action: EntityUid = format!(r#"Action::"{}""#, direction.action())
        .parse()
        .expect("action name is a literal");

    let env = RequestEnv::new(
        entity_type_name(&principal)?,
        action.clone(),
        entity_type_name(&resource)?,
    );

    let request = PartialRequest::new(principal, action, resource, context.cloned(), schema())
        .with_context(|| format!("building the {direction} request"))?;

    let response = policies
        .tpe(&request, entities, schema())
        .with_context(|| format!("evaluating {direction}"))?;

    let verdict = match response.decision() {
        Some(Decision::Allow) => Verdict::Allow,
        Some(Decision::Deny) => Verdict::Deny,
        None => Verdict::Unknown,
    };

    let mut reasons: Vec<String> = response
        .reason()
        .into_iter()
        .flatten()
        .map(|id| describe(policies, id))
        .collect();
    let mut residuals = Vec::new();
    let mut residual_conditions = Vec::new();
    if verdict == Verdict::Unknown {
        for id in response.residual_permits() {
            reasons.push(describe(policies, id));
        }
        for policy in response.residual_policies() {
            let Some(text) = policy.to_cedar() else {
                continue;
            };
            // The same policy's bare condition, for the symbolic discharger. `None`
            // would mean a non-permit residual, which this compiler never emits.
            let core_id = cedar_policy_core::ast::PolicyID::from_string(policy.id());
            let condition = response
                .as_ref()
                .get_residual_policy(&core_id)
                .map(|residual| Expr::from(residual.get_residual().as_ref().clone()));
            residual_conditions.push((policy.id().clone(), condition));
            residuals.push(text);
        }
    }
    reasons.sort();
    reasons.dedup();

    Ok(Outcome {
        direction,
        verdict,
        reasons,
        residuals,
        residual_conditions,
        env,
    })
}

/// Map a Cedar policy id back to the Kubernetes object it came from.
///
/// Note that Cedar assigns static policies the ids `policy0`, `policy1`, ... on
/// parse — an `@id` annotation does *not* become the policy id — so the mapping has
/// to go through the `@k8sPolicy`/`@k8sDirection`/`@k8sRule` annotations instead.
pub(crate) fn describe(policies: &PolicySet, id: &PolicyId) -> String {
    let Some(policy) = policies.policy(id) else {
        return id.to_string();
    };
    if policy.annotation("k8sKind") == Some("catch-all") {
        let direction = policy.annotation("k8sDirection").unwrap_or("?");
        return format!("catch-all {direction} (not isolated by any NetworkPolicy)");
    }
    match (
        policy.annotation("k8sPolicy"),
        policy.annotation("k8sDirection"),
        policy.annotation("k8sRule"),
    ) {
        (Some(object), Some(direction), Some(rule)) => format!("{object} {direction}[{rule}]"),
        _ => id.to_string(),
    }
}

/// The entity type of a possibly-symbolic uid, as the public name type.
fn entity_type_name(uid: &PartialEntityUid) -> Result<EntityTypeName> {
    let ty = uid.as_ref().ty.to_string();
    ty.parse()
        .with_context(|| format!("{ty:?} is not a valid entity type name"))
}
