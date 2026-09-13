//! Compile Kubernetes NetworkPolicy objects into Cedar, and evaluate them.
//!
//! See docs/concepts/encoding.md for the encoding and its rationale.

#[cfg(feature = "cluster")]
pub mod cluster;
pub mod compile;
pub mod connect;
pub mod entities;
pub mod eval;
pub mod expr;
pub mod load;
pub mod peer;
pub mod port;
pub mod selector;
pub mod symbolic;

use std::sync::OnceLock;

use cedar_policy::Schema;

/// The bundled Cedar schema, also emitted by `np2cedar schema`.
pub const SCHEMA_SRC: &str = include_str!("../networkpolicy.cedarschema");

/// The parsed schema. Parsed once; a failure here is a bug in the bundled file.
pub fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let (schema, _warnings) =
            Schema::from_cedarschema_str(SCHEMA_SRC).expect("the bundled schema must parse");
        schema
    })
}
