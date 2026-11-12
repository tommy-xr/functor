//! Built-ins a shell cannot perform. `functor build|run|develop vr|wasm`
//! warn at each one the game uses, so a gap in the headset or browser shell
//! shows up on the dev machine instead of as a silent no-op on the device.

use std::collections::HashSet;

use functor_lang::ir::ExprKind;
use functor_lang::project::Project;
use functor_lang::reuse::CodeIndex;

use super::asset_verify::{walk, Finding};
use crate::Environment;

/// `(built-in, what happens instead)` for each shell. Desktop performs all.
fn unsupported(environment: &Environment) -> &'static [(&'static str, &'static str)] {
    const NO_CONNECTIONS: &str = "fails as `Net.Error`: Quest has no connection transport yet";
    const NO_AUDIO: &str = "is silent: Quest has no audio host yet";
    const NO_ARGS: &str = "is always empty: this shell takes no --arg";
    match environment {
        Environment::Native => &[],
        Environment::Vr => &[
            ("Sub.connect", NO_CONNECTIONS),
            ("Sub.listen", NO_CONNECTIONS),
            ("Effect.play", NO_AUDIO),
            ("Effect.playAt", NO_AUDIO),
            (
                "Effect.playThen",
                "is silent and its message never arrives: Quest has no audio host yet",
            ),
            ("AudioScene.create", NO_AUDIO),
            ("Launch.args", NO_ARGS),
        ],
        Environment::Wasm => &[
            ("Sub.listen", "never opens on a top-level page: the browser is a client only"),
            ("Effect.playThen", "plays, but its message never arrives in the browser"),
            ("Launch.args", NO_ARGS),
        ],
    }
}

/// One finding per unsupported built-in reachable from `roots` (a role's
/// entry points), at its first use. Code only another role reaches, such as
/// a server's `Sub.listen` beside its client, is not charged to this one.
pub(crate) fn check(project: &Project, roots: &[&str], environment: &Environment) -> Vec<Finding> {
    let table = unsupported(environment);
    let index = CodeIndex::new(&project.module);
    let reached: HashSet<String> = roots.iter().filter_map(|name| index.deps_of(name)).flatten().collect();
    // Per table row: (first use, uses).
    let mut found: Vec<Option<(functor_lang::Span, usize)>> = vec![None; table.len()];
    for def in project.module.defs.iter().filter(|d| reached.contains(&d.name)) {
        walk(&def.value, &mut |expr| {
            let ExprKind::External(path) = &expr.kind else { return };
            let path = path.join(".");
            if let Some(i) = table.iter().position(|(name, _)| *name == path) {
                let entry = found[i].get_or_insert((expr.span, 0));
                if expr.span.start < entry.0.start {
                    entry.0 = expr.span;
                }
                entry.1 += 1;
            }
        });
    }
    table
        .iter()
        .zip(found)
        .filter_map(|((name, effect), found)| {
            let (span, uses) = found?;
            let more = match uses {
                1 => String::new(),
                n => format!(" ({n} uses)"),
            };
            Some(Finding {
                span,
                message: format!(
                    "`{name}` {effect} — `{}` cannot perform it{more}",
                    environment.as_str()
                ),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(src: &str) -> functor_lang::project::Project {
        functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()))
    }

    fn messages(src: &str, environment: Environment) -> Vec<String> {
        check(&project(src), &["a", "b", "c", "d"], &environment)
            .into_iter()
            .map(|f| f.message)
            .collect()
    }

    const NET: &str = "let a = Sub.connect(\"ws://h\", (e) => e)\n\
        let b = Sub.listen(\"0.0.0.0:9\", (e) => e)\n\
        let c = Sub.listen(\"0.0.0.0:8\", (e) => e)\n\
        let d = Effect.httpGet(\"http://h/\", (r) => r)";

    #[test]
    fn each_shell_flags_only_what_it_cannot_perform() {
        assert_eq!(messages(NET, Environment::Native), Vec::<String>::new());
        let vr = messages(NET, Environment::Vr);
        assert_eq!(vr.len(), 2, "{vr:?}");
        assert!(vr[0].starts_with("`Sub.connect` fails as `Net.Error`"), "{vr:?}");
        assert!(vr[1].starts_with("`Sub.listen`") && vr[1].ends_with("(2 uses)"), "{vr:?}");
        let wasm = messages(NET, Environment::Wasm);
        assert_eq!(wasm.len(), 1, "{wasm:?}");
        assert!(wasm[0].contains("`wasm` cannot perform it (2 uses)"), "{wasm:?}");
    }

    /// A sibling counts only through what the entry uses: `Helper`'s
    /// `Sub.connect` is reached, `server.fun`'s `Sub.listen` is not.
    #[test]
    fn only_what_the_entry_reaches_counts() {
        let dir = std::env::temp_dir().join(format!("shell-support-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.fun"), "let a = Helper.open()\n").unwrap();
        std::fs::write(dir.join("helper.fun"), "let open = () => Sub.connect(\"ws://h\", (e) => e)\n").unwrap();
        std::fs::write(dir.join("server.fun"), "let s = Sub.listen(\"0.0.0.0:9\", (e) => e)\n").unwrap();
        let project = functor_lang::project::load(&dir.join("game.fun"))
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let found: Vec<String> = check(&project, &["a"], &Environment::Vr).into_iter().map(|f| f.message).collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("`Sub.connect`"), "{found:?}");
    }

    /// Through the real prelude, as `build` loads it: its `.funi`
    /// signatures, not the unresolved-name path the other tests take.
    #[test]
    fn prelude_linked_builtins_are_found() {
        let dir = std::env::temp_dir().join(format!("shell-support-prelude-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.fun"), "let subscriptions = (m) => Sub.listen(\"0.0.0.0:9\", (e) => e)\n").unwrap();
        let project = functor_lang::project::load_with_bundled_modules(
            &dir.join("game.fun"),
            &std::collections::HashMap::new(),
            &functor_prelude::bundled_modules(),
        )
        .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let _ = std::fs::remove_dir_all(&dir);
        let found = check(&project, &["subscriptions"], &Environment::Vr);
        assert_eq!(found.len(), 1);
    }

    /// Roles sharing a file: the client's roots never reach the inline
    /// server module's `Sub.listen`.
    #[test]
    fn a_same_file_role_is_not_charged() {
        let project = project(
            "let tick = (m) => Sub.connect(\"ws://h\", (e) => e)\n\
             module Server {\n  let tick = (m) => Sub.listen(\"0.0.0.0:9\", (e) => e)\n}\n",
        );
        let client: Vec<String> = check(&project, &["tick"], &Environment::Wasm).into_iter().map(|f| f.message).collect();
        assert!(client.is_empty(), "{client:?}");
        let server = check(&project, &["Server.tick"], &Environment::Wasm);
        assert_eq!(server.len(), 1);
    }

    #[test]
    fn first_use_anchors_the_finding() {
        let project = project(
            "let early = Sub.listen(\"0.0.0.0:9\", (e) => e)\nlet late = Sub.listen(\"0.0.0.0:8\", (e) => e)",
        );
        let findings = check(&project, &["early", "late"], &Environment::Wasm);
        let (_, line, _) = project.sources.resolve(findings[0].span.start);
        assert_eq!(line, 1);
    }
}
