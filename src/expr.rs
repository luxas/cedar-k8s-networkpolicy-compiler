//! A small expression AST for the Cedar conditions we generate.
//!
//! Rendering to text — rather than building [`cedar_policy::pst`] nodes — is a
//! deliberate but temporary choice: the PST carries no comments or trivia, and the
//! inline `// spec.podSelector` annotations are most of what makes the generated
//! `.cedar` file readable by a human reviewing a diff. See docs/README.md, planned item 1.
//!
//! The constructors simplify as they build, so we never emit `true && true && ...`.

/// A Cedar boolean expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A literal `true` or `false`.
    Bool(bool),
    /// A leaf condition, already rendered, e.g. `principal.labels.hasTag("app")`.
    Atom(String),
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
    /// `inner`, preceded by a `// text` line explaining where it came from.
    Commented(String, Box<Expr>),
}

impl Expr {
    pub fn atom(s: impl Into<String>) -> Self {
        Expr::Atom(s.into())
    }

    /// Conjunction, flattening nested `And`s and absorbing constants.
    pub fn and(parts: impl IntoIterator<Item = Expr>) -> Self {
        let mut flat: Vec<Expr> = Vec::new();
        for part in parts {
            match part {
                Expr::Bool(true) => {}
                Expr::Bool(false) => return Expr::Bool(false),
                // A `Commented` is never flattened: the comment groups its children.
                Expr::And(inner) => flat.extend(inner),
                other => flat.push(other),
            }
        }
        match flat.len() {
            0 => Expr::Bool(true),
            1 => flat.remove(0),
            _ => Expr::And(flat),
        }
    }

    /// Disjunction, flattening nested `Or`s and absorbing constants.
    pub fn or(parts: impl IntoIterator<Item = Expr>) -> Self {
        let mut flat: Vec<Expr> = Vec::new();
        for part in parts {
            match part {
                Expr::Bool(false) => {}
                Expr::Bool(true) => return Expr::Bool(true),
                Expr::Or(inner) => flat.extend(inner),
                other => flat.push(other),
            }
        }
        match flat.len() {
            0 => Expr::Bool(false),
            1 => flat.remove(0),
            _ => Expr::Or(flat),
        }
    }

    pub fn negate(self) -> Self {
        match self {
            Expr::Bool(b) => Expr::Bool(!b),
            Expr::Not(inner) => *inner,
            other => Expr::Not(Box::new(other)),
        }
    }

    /// Attach an explanatory comment. Dropped when `self` has already simplified to
    /// a constant, since the constant is about to be absorbed by its parent anyway.
    pub fn commented(self, text: impl Into<String>) -> Self {
        match self {
            Expr::Bool(_) => self,
            other => Expr::Commented(text.into(), Box::new(other)),
        }
    }

    pub fn is_const(&self) -> bool {
        matches!(self, Expr::Bool(_))
    }

    /// Render the expression. The first line is emitted unindented (the caller has
    /// already positioned it); every following line is indented by `indent` spaces.
    pub fn render(&self, indent: usize) -> String {
        let mut out = String::new();
        let mut expr = self;
        let pad = " ".repeat(indent);
        while let Expr::Commented(text, inner) = expr {
            out.push_str("// ");
            out.push_str(text);
            out.push('\n');
            out.push_str(&pad);
            expr = inner;
        }
        expr.write(&mut out, indent);
        out
    }

    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Expr::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Expr::Atom(a) => out.push_str(a),
            Expr::Commented(..) => out.push_str(&self.render(indent)),
            // `!` binds tighter than `&&`, so a leaf needs no parentheses; anything
            // compound gets a block, and never a doubled one.
            Expr::Not(inner) => match inner.as_ref() {
                Expr::Atom(a) => {
                    out.push('!');
                    out.push_str(a);
                }
                other => {
                    out.push_str("!(\n");
                    out.push_str(&" ".repeat(indent + 2));
                    other.write(out, indent + 2);
                    out.push('\n');
                    out.push_str(&" ".repeat(indent));
                    out.push(')');
                }
            },
            Expr::And(parts) => write_seq(out, indent, parts, "&&"),
            Expr::Or(parts) => write_seq(out, indent, parts, "||"),
        }
    }
}

fn write_seq(out: &mut String, indent: usize, parts: &[Expr], op: &str) {
    let pad = " ".repeat(indent);
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(' ');
            out.push_str(op);
            out.push('\n');
            out.push_str(&pad);
        }
        write_operand(out, indent, part);
    }
}

/// Write one operand of an `And`/`Or`: comments hoisted above, compound operands
/// wrapped in a parenthesised block so the grouping is unambiguous on sight.
fn write_operand(out: &mut String, indent: usize, part: &Expr) {
    let pad = " ".repeat(indent);
    let mut expr = part;
    while let Expr::Commented(text, inner) = expr {
        out.push_str("// ");
        out.push_str(text);
        out.push('\n');
        out.push_str(&pad);
        expr = inner;
    }
    if matches!(expr, Expr::And(_) | Expr::Or(_)) {
        out.push_str("(\n");
        out.push_str(&" ".repeat(indent + 2));
        expr.write(out, indent + 2);
        out.push('\n');
        out.push_str(&pad);
        out.push(')');
    } else {
        expr.write(out, indent);
    }
}

/// Render `s` as a Cedar string literal, escapes included.
pub fn string_lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Render an entity UID literal, e.g. `Namespace::"default"`.
pub fn entity_uid(ty: &str, id: &str) -> String {
    format!("{ty}::{}", string_lit(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn and_absorbs_constants() {
        assert_eq!(
            Expr::and([Expr::Bool(true), Expr::Bool(true)]),
            Expr::Bool(true)
        );
        assert_eq!(
            Expr::and([Expr::atom("a"), Expr::Bool(true)]),
            Expr::atom("a")
        );
        assert_eq!(
            Expr::and([Expr::atom("a"), Expr::Bool(false)]),
            Expr::Bool(false)
        );
    }

    #[test]
    fn or_absorbs_constants() {
        assert_eq!(Expr::or([]), Expr::Bool(false));
        assert_eq!(
            Expr::or([Expr::atom("a"), Expr::Bool(true)]),
            Expr::Bool(true)
        );
    }

    #[test]
    fn and_flattens_but_comments_group() {
        let nested = Expr::and([
            Expr::atom("a"),
            Expr::and([Expr::atom("b"), Expr::atom("c")]),
        ]);
        assert_eq!(
            nested,
            Expr::And(vec![Expr::atom("a"), Expr::atom("b"), Expr::atom("c")])
        );

        let grouped = Expr::and([
            Expr::atom("a"),
            Expr::and([Expr::atom("b"), Expr::atom("c")]).commented("grouped"),
        ]);
        assert!(matches!(&grouped, Expr::And(parts) if parts.len() == 2));
    }

    #[test]
    fn comment_on_constant_is_dropped() {
        assert_eq!(Expr::Bool(true).commented("gone"), Expr::Bool(true));
    }

    #[test]
    fn renders_nested_disjunction() {
        let e = Expr::and([
            Expr::atom("a"),
            Expr::or([Expr::atom("b"), Expr::atom("c")]).commented("from"),
        ]);
        assert_eq!(e.render(2), "a &&\n  // from\n  (\n    b ||\n    c\n  )");
    }

    #[test]
    fn escapes_string_literals() {
        assert_eq!(string_lit(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(entity_uid("Pod", "default/web"), r#"Pod::"default/web""#);
    }
}
