//! Golden tests over the generated `.cedar` text and entity store.
//!
//! Review and accept changes with `cargo insta review`, or take them wholesale with
//! `INSTA_FORCE_UPDATE=1 cargo test`.

mod common;

macro_rules! snapshot_fixtures {
    ($($test:ident => $dir:literal),* $(,)?) => {
        $(
            #[test]
            fn $test() {
                let fixture = common::fixture($dir);
                insta::assert_snapshot!(concat!($dir, ".cedar"), fixture.source);
                insta::assert_snapshot!(
                    concat!($dir, ".entities"),
                    serde_json::to_string_pretty(&fixture.entities_json).unwrap()
                );
            }
        )*
    };
}

snapshot_fixtures! {
    additive => "additive",
    allow_same_ns => "allow-same-ns",
    cross_namespace => "cross-namespace",
    default_deny_all => "default-deny-all",
    egress_defaulting => "egress-defaulting",
    ipblock_egress => "ipblock-egress",
    match_expressions => "match-expressions",
    named_port => "named-port",
    namespace_selector => "namespace-selector",
    port_range => "port-range",
    protocol_enum => "protocol-enum",
    unknown_pod_ip => "unknown-pod-ip",
}
