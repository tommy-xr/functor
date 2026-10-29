//! Drift test for the TypeScript mirror of the debug-protocol wire types:
//! renders every `ts-rs` declaration reachable from the roots below into
//! `tools/functor-sdk/src/generated/protocol.ts` and fails if the committed
//! file differs. Regenerate with `UPDATE_TS=1`.

use std::collections::BTreeMap;

use ts_rs::{Config, TypeVisitor, TS};

use crate::debug_protocol::{InputCommand, RuntimeState};
use crate::Camera;

const OUT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/functor-sdk/src/generated/protocol.ts"
);

const HEADER: &str = "\
// Generated from the Rust debug-protocol types by ts-rs. Do not edit.
// Regenerate: UPDATE_TS=1 cargo test -p functor_runtime_common --features ts ts_bindings
";

/// Collects each named declaration once, keyed by TS name.
struct Decls<'a> {
    cfg: &'a Config,
    out: BTreeMap<String, String>,
}

impl TypeVisitor for Decls<'_> {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        // Only declarable types (derived or shadowed) have an output path.
        if T::output_path().is_none() {
            return;
        }
        let name = T::ident(self.cfg);
        let decl = format!(
            "{}export {}",
            T::docs().unwrap_or_default(),
            T::decl(self.cfg)
        );
        // A repeat is fine (a self-reference, or a serde shadow like
        // `JsonValue`); a different shape under one name would drop one.
        if let Some(seen) = self.out.insert(name.clone(), decl.clone()) {
            assert_eq!(seen, decl, "two different types export as `{name}`");
            return;
        }
        T::visit_dependencies(self);
    }
}

fn render() -> String {
    // The JSON carries u64 counters as plain numbers, not bigints.
    let cfg = Config::new().with_large_int("number");
    let mut decls = Decls {
        cfg: &cfg,
        out: BTreeMap::new(),
    };
    decls.visit::<RuntimeState>();
    decls.visit::<InputCommand>();
    decls.visit::<Camera>();
    let body: Vec<String> = decls.out.into_values().collect();
    format!("{HEADER}\n{}\n", body.join("\n\n"))
}

#[test]
fn ts_bindings_match_committed_file() {
    let fresh = render();
    if std::env::var_os("UPDATE_TS").is_some() {
        std::fs::create_dir_all(std::path::Path::new(OUT).parent().unwrap()).unwrap();
        std::fs::write(OUT, &fresh).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(OUT).unwrap_or_default();
    assert!(
        committed == fresh,
        "{OUT} is stale — regenerate with \
         UPDATE_TS=1 cargo test -p functor_runtime_common --features ts ts_bindings"
    );
}
