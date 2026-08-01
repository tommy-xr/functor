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
use functor_lang::eval::{builtin_name, ALL_BUILTINS, BUILTIN_NAMESPACES};
use functor_lang::parse_interface;
use functor_lang::project::stdlib_documentation_modules;
use functor_lang::types::builtin_signature;
use std::collections::BTreeMap;

/// The `let name : Type` lines a documentation interface declares, by member
/// name, exactly as written in the source. A repeated member is a bug in the
/// file (the generator would publish it twice), so it panics rather than
/// letting the later one overwrite the earlier.
fn declared_signatures(source: &str) -> BTreeMap<String, String> {
    let program = parse_interface(source).expect("a documentation interface parses");
    let mut declared = BTreeMap::new();
    for item in program.items {
        let Item::Sig(decl) = item else { continue };
        let text = source
            .get(decl.span.start..decl.span.end)
            .expect("signature spans are char boundaries")
            .trim()
            .to_string();
        assert!(
            declared.insert(decl.name.clone(), text).is_none(),
            "`{}` is declared twice in one documentation interface",
            decl.name
        );
    }
    declared
}

/// Drop a module's own qualifier from a rendered type, the way its source
/// writes it: inside `Random`, `builtin_signature` says `Random.Seed` where
/// the file says `Seed`. Only a qualifier at an identifier boundary counts, so
/// an unrelated `NotRandom.t` is left alone.
fn unqualify(text: &str, module: &str) -> String {
    let qualifier = format!("{module}.");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(&qualifier) {
        let boundary = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '.');
        out.push_str(&rest[..at]);
        if boundary {
            rest = &rest[at + qualifier.len()..];
        } else {
            out.push_str(&qualifier);
            rest = &rest[at + qualifier.len()..];
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn unqualify_only_strips_at_an_identifier_boundary() {
    assert_eq!(
        unqualify("(Random.Seed) => Random.Seed", "Random"),
        "(Seed) => Seed"
    );
    assert_eq!(
        unqualify("(NotRandom.t) => t", "Random"),
        "(NotRandom.t) => t"
    );
    assert_eq!(
        unqualify("(List<'a>) => Option.t<'a>", "List"),
        "(List<'a>) => Option.t<'a>"
    );
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
                let signature = unqualify(&builtin_signature(*b).to_string(), namespace);
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
/// (`documentation_source` panics for an undocumented one, so this is what
/// keeps the loop above from silently iterating over nothing.)
#[test]
fn every_builtin_namespace_has_a_documentation_module() {
    let documented: Vec<&str> = stdlib_documentation_modules()
        .iter()
        .map(|module| module.name())
        .collect();
    for namespace in BUILTIN_NAMESPACES {
        assert!(
            documented.contains(namespace),
            "builtin namespace `{namespace}` is missing from stdlib_documentation_modules()"
        );
    }
}
