//! The standard library's documentation sources must describe the standard
//! library that actually runs.
//!
//! The namespaces the interpreter implements in Rust (`List`, `Math`, …) have
//! no Functor Lang source of their own, so their documentation lives in
//! `stdlib/*.funi` interfaces that nothing links. These tests are what stops
//! those files from drifting: the member set and every signature are compared
//! against the registry the checker and the interpreter share, so a builtin
//! added, removed, or retyped without a documentation update fails here rather
//! than silently publishing a wrong reference.

use functor_lang::ast::Item;
use functor_lang::eval::{
    builtin, builtin_members, builtin_name, ALL_BUILTINS, BUILTIN_NAMESPACES,
};
use functor_lang::parse_interface;
use functor_lang::project::stdlib_documentation_modules;
use functor_lang::types::builtin_signature;
use std::collections::BTreeMap;

/// The `let name : Type` lines a documentation interface declares, by member
/// name, exactly as written in the source.
fn declared_signatures(source: &str) -> BTreeMap<String, String> {
    let program = parse_interface(source).expect("a documentation interface parses");
    program
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Sig(decl) => {
                let text = source
                    .get(decl.span.start..decl.span.end)
                    .expect("signature spans are char boundaries")
                    .trim()
                    .to_string();
                Some((decl.name, text))
            }
            _ => None,
        })
        .collect()
}

fn documentation_source(module: &str) -> String {
    stdlib_documentation_modules()
        .into_iter()
        .find(|candidate| candidate.name() == module)
        .unwrap_or_else(|| panic!("`{module}` has no documentation module"))
        .source()
        .to_string()
}

/// Every builtin namespace is documented, member for member, with the exact
/// signature the checker gives it. A module writes its OWN types unqualified
/// (`Seed`, not `Random.Seed`), so the expectation is qualified the same way.
#[test]
fn builtin_documentation_matches_the_registry() {
    for namespace in BUILTIN_NAMESPACES {
        let source = documentation_source(namespace);
        let expected: BTreeMap<String, String> = ALL_BUILTINS
            .iter()
            .filter_map(|b| {
                let member = builtin_name(*b).strip_prefix(&format!("{namespace}."))?;
                let signature = builtin_signature(*b)
                    .to_string()
                    .replace(&format!("{namespace}."), "");
                Some((member.to_string(), format!("let {member} : {signature}")))
            })
            .collect();
        assert_eq!(
            declared_signatures(&source),
            expected,
            "`{namespace}`'s documentation interface has drifted from the builtin registry \
             — update functor-lang/stdlib/{}.funi",
            namespace.to_lowercase()
        );
    }
}

/// The namespace list itself is covered: a NEW builtin namespace has to bring
/// documentation with it, rather than quietly missing from the reference.
#[test]
fn every_builtin_namespace_has_a_documentation_module() {
    let documented: Vec<String> = stdlib_documentation_modules()
        .iter()
        .map(|module| module.name().to_string())
        .collect();
    for namespace in BUILTIN_NAMESPACES {
        assert!(
            documented.contains(&namespace.to_string()),
            "builtin namespace `{namespace}` is missing from stdlib_documentation_modules()"
        );
        // And nothing documents a member the registry does not dispatch.
        for member in builtin_members(namespace) {
            assert!(
                builtin(&[namespace.to_string(), member.to_string()]).is_some(),
                "`{namespace}.{member}` is documented but does not dispatch"
            );
        }
    }
}

/// The Functor Lang-implemented modules are documented from the very source
/// that is linked into every project, so those cannot drift by construction —
/// this pins that they are all present.
#[test]
fn language_implemented_modules_are_documented() {
    let documented: Vec<String> = stdlib_documentation_modules()
        .iter()
        .map(|module| module.name().to_string())
        .collect();
    for module in ["Option", "Result", "Key", "Mouse"] {
        assert!(
            documented.contains(&module.to_string()),
            "`{module}` is missing from stdlib_documentation_modules()"
        );
    }
}
