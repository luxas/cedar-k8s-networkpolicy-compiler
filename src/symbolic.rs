//! Discharging address residuals with the symbolic evaluator.
//!
//! An unknown pod address is not arbitrary: it lies in the cluster's pod CIDR.
//! `--pod-cidr` turns that fact into assumptions for
//! `cedar_policy_symcc::evaluator::Evaluator`, which asks an SMT solver (cvc5)
//! whether each residual permit can still be true. A rule whose ipBlock cannot
//! intersect the pod CIDR is pruned — a policy allowing egress only to the public
//! internet never applies to pod-to-pod traffic — and when that settles every
//! residual, the `Unknown` verdict collapses to a definite one.

use anyhow::{Context as _, Result, bail, ensure};
use cedar_policy::{Expression, PartialEntities, PolicySet};
use cedar_policy_core::ast::Expr;
use cedar_policy_core::tpe::residual::EvaluationOutcome;
use cedar_policy_symcc::CedarSymCompiler;
use cedar_policy_symcc::evaluator::{EvaluationError, Evaluator};
use cedar_policy_symcc::solver::LocalSolver;

use crate::eval::{Outcome, Verdict, describe};
use crate::schema;

/// Re-evaluates `Unknown` outcomes under the assumption that every Pod address
/// lies within the given pod CIDRs.
pub struct Discharger {
    /// The evaluator is async end to end; the runtime lives here so the rest of
    /// the crate stays synchronous, as in `cluster::fetch`.
    runtime: tokio::runtime::Runtime,
    evaluator: Evaluator<LocalSolver>,
    pod_cidrs: Vec<String>,
}

/// A current-thread runtime and a cvc5-backed symbolic compiler, together:
/// `LocalSolver::cvc5` spawns the process through tokio, which needs the
/// runtime's reactor even from synchronous code, so the two only make sense
/// as a pair.
pub(crate) fn solver_bootstrap() -> Result<(tokio::runtime::Runtime, CedarSymCompiler<LocalSolver>)>
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting a tokio runtime")?;
    let solver = {
        let _guard = runtime.enter();
        LocalSolver::cvc5().context("spawning cvc5 — install it, or point $CVC5 at it")?
    };
    let compiler = CedarSymCompiler::new(solver).context("initialising the symbolic compiler")?;
    Ok((runtime, compiler))
}

/// Check every CIDR with [`parse_cidr`]. clap does this for the CLI, but the
/// strings end up inside generated Cedar text, so the library entry points
/// must not trust their callers.
pub(crate) fn check_cidrs(pod_cidrs: &[String]) -> Result<()> {
    for cidr in pod_cidrs {
        parse_cidr(cidr).map_err(|reason| anyhow::anyhow!("pod CIDR {cidr:?}: {reason}"))?;
    }
    Ok(())
}

impl Discharger {
    /// Spawn cvc5 and register what is known: the entity store, and — for every
    /// address the store leaves unknown, which is always a pod's — that it lies
    /// within one of `pod_cidrs`.
    pub fn new(entities: &PartialEntities, pod_cidrs: &[String]) -> Result<Self> {
        ensure!(!pod_cidrs.is_empty(), "at least one pod CIDR is required");
        check_cidrs(pod_cidrs)?;
        let (runtime, compiler) = solver_bootstrap()?;
        let mut evaluator =
            Evaluator::new(compiler, schema()).context("initialising the symbolic evaluator")?;

        // Open-world: entities listed here have their known parts pinned; anything
        // else — the symbolic peer of `reachable`, say — stays unconstrained.
        evaluator.assume_partial_entities(entities.as_ref().clone());
        for entity in entities.as_ref().entities() {
            let uid = entity.uid();
            if uid.entity_type().to_string() == "IpAddr" && entity.attrs().is_none() {
                evaluator.assume_expr(membership(&format!("{uid}.addr"), pod_cidrs)?);
            }
        }
        Ok(Self {
            runtime,
            evaluator,
            pod_cidrs: pod_cidrs.to_vec(),
        })
    }

    /// Refine one `Unknown` outcome in place. Residual permits that can never be
    /// true are pruned (their descriptions are returned); if one is *always* true
    /// the verdict collapses to `Allow`, and if none survives, to `Deny`.
    ///
    /// `symbolic_resource` additionally assumes the resource's address lies in the
    /// pod CIDRs — for `reachable`'s symbolic Pod peer only. It must never be set
    /// when the resource is an `IpEndpoint`, whose address is not a pod's.
    pub fn refine(
        &mut self,
        policies: &PolicySet,
        outcome: &mut Outcome,
        symbolic_resource: bool,
    ) -> Result<Vec<String>> {
        if outcome.verdict != Verdict::Unknown {
            return Ok(Vec::new());
        }
        let extras: Vec<Expr> = if symbolic_resource {
            vec![membership("resource.ip.addr", &self.pod_cidrs)?]
        } else {
            Vec::new()
        };

        let mut kept_texts = Vec::new();
        let mut kept_conditions = Vec::new();
        let mut pruned = Vec::new();
        let mut always = None; // description of a permit that is definitely true

        for ((id, condition), text) in outcome.residual_conditions.iter().zip(&outcome.residuals) {
            let keep = |texts: &mut Vec<String>, conditions: &mut Vec<_>| {
                texts.push(text.clone());
                conditions.push((id.clone(), condition.clone()));
            };
            let Some(condition) = condition else {
                keep(&mut kept_texts, &mut kept_conditions);
                continue;
            };
            let evaluated = self.runtime.block_on(self.evaluator.evaluate(
                condition,
                &outcome.env,
                extras.iter().cloned(),
            ));
            let evaluated = match evaluated {
                Ok(evaluated) => evaluated,
                Err(EvaluationError::UnsatisfiableAssumptions) => bail!(
                    "no pod address can satisfy --pod-cidr {}; it contradicts the \
                     addresses the entity store already pins",
                    self.pod_cidrs.join(", ")
                ),
                Err(
                    error @ (EvaluationError::NotWellTyped { .. }
                    | EvaluationError::AssumptionNotWellTyped { .. }),
                ) => {
                    // A folded residual can in principle stop typechecking on its
                    // own (a stripped `hasTag` guard, an error node). Keeping it
                    // unknown is sound; collapsing would not be.
                    eprintln!(
                        "warning: {} is kept unknown, it cannot be evaluated symbolically: {error}",
                        describe(policies, id)
                    );
                    keep(&mut kept_texts, &mut kept_conditions);
                    continue;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("evaluating {}", describe(policies, id)));
                }
            };

            let outcomes = evaluated.data().outcomes();
            let can_be_true = outcomes.contains(&EvaluationOutcome::True);
            if can_be_true && outcomes.len().get() == 1 {
                always.get_or_insert_with(|| describe(policies, id));
            }
            if can_be_true {
                keep(&mut kept_texts, &mut kept_conditions);
            } else {
                // Never true — on every admissible completion this permit does not
                // apply, so it cannot contribute to the verdict.
                pruned.push(describe(policies, id));
            }
        }

        let cidrs = self.pod_cidrs.join(", ");
        if let Some(rule) = always {
            outcome.verdict = Verdict::Allow;
            outcome
                .reasons
                .push(format!("{rule} allows every pod address in {cidrs}"));
            outcome.residuals.clear();
            outcome.residual_conditions.clear();
        } else if kept_texts.is_empty() {
            // All permits — the residuals here, the concretely-false ones TPE
            // already discarded — are out; with no forbids, that is a deny.
            outcome.verdict = Verdict::Deny;
            outcome.reasons.push(format!(
                "no residual rule can match a pod address in {cidrs}"
            ));
            outcome.residuals.clear();
            outcome.residual_conditions.clear();
        } else {
            outcome.residuals = kept_texts;
            outcome.residual_conditions = kept_conditions;
        }
        Ok(pruned)
    }
}

/// `subject.isInRange(ip("cidr1")) || ...` over every CIDR, as a bare
/// expression.
pub(crate) fn membership(subject: &str, cidrs: &[String]) -> Result<Expr> {
    let text = cidrs
        .iter()
        .map(|cidr| format!(r#"{subject}.isInRange(ip("{cidr}"))"#))
        .collect::<Vec<_>>()
        .join(" || ");
    let expression: Expression = text
        .parse()
        .with_context(|| format!("building the CIDR assumption {text:?}"))?;
    Ok(expression.as_ref().clone())
}

/// A syntactic CIDR check, for clap: `address/prefix` with a prefix that fits the
/// address family. Returns the input unchanged; it stays a string all the way to
/// the `ip()` call in [`membership`].
pub fn parse_cidr(s: &str) -> std::result::Result<String, String> {
    let Some((address, prefix)) = s.split_once('/') else {
        return Err(format!("{s:?} is not in address/prefix form"));
    };
    let address: std::net::IpAddr = address
        .parse()
        .map_err(|_| format!("{address:?} is not an IP address"))?;
    let bits = if address.is_ipv4() { 32u8 } else { 128 };
    match prefix.parse::<u8>() {
        Ok(p) if p <= bits => Ok(s.to_string()),
        _ => Err(format!(
            "{prefix:?} is not a prefix length of at most {bits}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_cidr;

    #[test]
    fn cidr_syntax_is_checked_up_front() {
        assert!(parse_cidr("10.244.0.0/16").is_ok());
        assert!(parse_cidr("fd00::/8").is_ok());
        assert!(parse_cidr("10.244.0.0").is_err(), "no prefix");
        assert!(parse_cidr("banana/8").is_err(), "not an address");
        assert!(parse_cidr("10.0.0.0/33").is_err(), "prefix too long for v4");
        assert!(parse_cidr("fd00::/129").is_err(), "prefix too long for v6");
        assert!(parse_cidr("10.0.0.0/x").is_err(), "prefix not a number");
    }
}
