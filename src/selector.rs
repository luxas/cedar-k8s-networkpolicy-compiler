//! `LabelSelector` -> Cedar condition.

use anyhow::{Result, bail};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;

use crate::expr::{Expr, string_lit};

/// Translate a `LabelSelector` into a condition over the label map named by
/// `labels` (e.g. `principal.labels` or `resource.namespace.labels`).
///
/// A `None` selector and an empty selector both mean "match everything", per the
/// Kubernetes API contract for the fields we use it on.
///
/// Every `getTag` is guarded by a `hasTag`: Cedar's `getTag` *errors* on a missing
/// tag rather than returning a default, and `&&` short-circuits.
pub fn selector_expr(selector: Option<&LabelSelector>, labels: &str) -> Result<Expr> {
    let Some(selector) = selector else {
        return Ok(Expr::Bool(true));
    };

    let mut parts = Vec::new();

    for (key, value) in selector.match_labels.iter().flatten() {
        parts.push(Expr::and([
            has_tag(labels, key),
            tag_eq(labels, key, value),
        ]));
    }

    for req in selector.match_expressions.iter().flatten() {
        let values = req.values.as_deref().unwrap_or_default();
        let part = match req.operator.as_str() {
            "Exists" => has_tag(labels, &req.key),
            "DoesNotExist" => has_tag(labels, &req.key).negate(),
            "In" => {
                if values.is_empty() {
                    bail!(
                        "matchExpressions[key={}]: `In` requires a non-empty `values`",
                        req.key
                    );
                }
                Expr::and([has_tag(labels, &req.key), tag_in(labels, &req.key, values)])
            }
            // `NotIn` matches when the key is absent, so negate the whole
            // has-and-is-one-of conjunction rather than just the membership test.
            "NotIn" => {
                if values.is_empty() {
                    bail!(
                        "matchExpressions[key={}]: `NotIn` requires a non-empty `values`",
                        req.key
                    );
                }
                Expr::and([has_tag(labels, &req.key), tag_in(labels, &req.key, values)]).negate()
            }
            other => bail!(
                "matchExpressions[key={}]: unknown operator {other:?} \
                 (expected In, NotIn, Exists or DoesNotExist)",
                req.key
            ),
        };
        parts.push(part);
    }

    Ok(Expr::and(parts))
}

fn has_tag(labels: &str, key: &str) -> Expr {
    Expr::atom(format!("{labels}.hasTag({})", string_lit(key)))
}

fn tag_eq(labels: &str, key: &str, value: &str) -> Expr {
    Expr::atom(format!(
        "{labels}.getTag({}) == {}",
        string_lit(key),
        string_lit(value)
    ))
}

fn tag_in(labels: &str, key: &str, values: &[String]) -> Expr {
    let list = values
        .iter()
        .map(|v| string_lit(v))
        .collect::<Vec<_>>()
        .join(", ");
    Expr::atom(format!(
        "[{list}].contains({labels}.getTag({}))",
        string_lit(key)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelectorRequirement;

    fn req(key: &str, op: &str, values: &[&str]) -> LabelSelectorRequirement {
        LabelSelectorRequirement {
            key: key.into(),
            operator: op.into(),
            values: if values.is_empty() {
                None
            } else {
                Some(values.iter().map(|v| v.to_string()).collect())
            },
        }
    }

    #[test]
    fn absent_and_empty_selectors_match_everything() {
        assert_eq!(
            selector_expr(None, "principal.labels").unwrap(),
            Expr::Bool(true)
        );
        assert_eq!(
            selector_expr(Some(&LabelSelector::default()), "principal.labels").unwrap(),
            Expr::Bool(true)
        );
    }

    #[test]
    fn match_labels_guards_get_tag() {
        let sel = LabelSelector {
            match_labels: Some([("app".to_string(), "web".to_string())].into()),
            ..Default::default()
        };
        assert_eq!(
            selector_expr(Some(&sel), "principal.labels")
                .unwrap()
                .render(0),
            "principal.labels.hasTag(\"app\") &&\nprincipal.labels.getTag(\"app\") == \"web\""
        );
    }

    #[test]
    fn not_in_matches_an_absent_key() {
        let sel = LabelSelector {
            match_expressions: Some(vec![req("tier", "NotIn", &["db"])]),
            ..Default::default()
        };
        let rendered = selector_expr(Some(&sel), "principal.labels")
            .unwrap()
            .render(0);
        assert!(
            rendered.starts_with("!("),
            "expected a negated conjunction, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("!(\n  ("),
            "the negation should not double its parentheses"
        );
        assert!(rendered.contains("hasTag(\"tier\")"));
    }

    #[test]
    fn rejects_unknown_operator_and_empty_values() {
        let bad_op = LabelSelector {
            match_expressions: Some(vec![req("k", "Gt", &["1"])]),
            ..Default::default()
        };
        assert!(selector_expr(Some(&bad_op), "principal.labels").is_err());

        let empty_values = LabelSelector {
            match_expressions: Some(vec![req("k", "In", &[])]),
            ..Default::default()
        };
        assert!(selector_expr(Some(&empty_values), "principal.labels").is_err());
    }
}
