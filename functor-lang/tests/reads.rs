//! Read-class inference (`functor_lang::reads::infer`): which `impure[…]`
//! classes each top-level def can reach, report-only.

use functor_lang::project::{BundledModule, Project};
use functor_lang::reads::{infer, Inferred, ReadClass};
use std::path::PathBuf;

/// Stand-ins for the engine prelude's impure built-ins.
fn prelude() -> Vec<BundledModule> {
    vec![
        BundledModule::interface(
            "File",
            "impure[file]\nlet exists : (string) => bool\n\
             impure[file]\nlet size : (string) => float\n",
        ),
        BundledModule::interface(
            "Physics",
            "impure[physics]\nlet height : (string) => float\nlet gravity : () => float\n",
        ),
        BundledModule::interface(
            "Effect",
            "type t = host\nlet none : () => t\nlet log : (float) => t\n",
        ),
    ]
}

fn load(files: &[(&str, &str)]) -> (Project, Inferred) {
    let sources = files
        .iter()
        .map(|(path, src)| (PathBuf::from(path), src.to_string()))
        .collect();
    let project = functor_lang::project::load_sources_with_bundled_modules(sources, &prelude())
        .unwrap_or_else(|e| panic!("{}", e.render()));
    let (errors, types) = project.check_with_types();
    assert!(errors.is_empty(), "{errors:?}");
    let inferred = infer(&project.module, &types);
    (project, inferred)
}

fn classes(inferred: &Inferred, def: &str) -> Vec<&'static str> {
    inferred.classes(def).into_iter().map(ReadClass::name).collect()
}

#[test]
fn direct_and_transitive_calls_reach_the_builtin() {
    let (_, inferred) = load(&[(
        "game.fun",
        "let has = (p) => File.exists(p)\n\
         let check = (p) => has(p)\n\
         let outer = (p) => if check(p) then Physics.height(\"a\") else 0.0\n\
         let calm = (x) => x + Physics.gravity()\n",
    )]);
    assert_eq!(classes(&inferred, "has"), ["file"]);
    assert_eq!(inferred.path("has", ReadClass::File), ["File.exists"]);
    assert_eq!(inferred.path("check", ReadClass::File), ["has", "File.exists"]);
    assert_eq!(classes(&inferred, "outer"), ["file", "physics"]);
    assert_eq!(
        inferred.path("outer", ReadClass::File),
        ["check", "has", "File.exists"]
    );
    // A pure built-in in an impure module reads nothing.
    assert!(classes(&inferred, "calm").is_empty());
}

#[test]
fn mutual_recursion_reaches_a_fixed_point() {
    let (_, inferred) = load(&[(
        "game.fun",
        "let a = (n) => if n > 0.0 then b(n - 1.0) else File.exists(\"x\")\n\
         let b = (n) => a(n)\n",
    )]);
    assert_eq!(classes(&inferred, "a"), ["file"]);
    assert_eq!(classes(&inferred, "b"), ["file"]);
    assert_eq!(inferred.path("b", ReadClass::File), ["a", "File.exists"]);
}

#[test]
fn calls_across_modules_carry_their_classes() {
    let (_, inferred) = load(&[
        (
            "game.fun",
            "let level = World.load(\"level.bin\")\n\
             let reload = (p) => World.load(p)\n\
             let tick = (m) => m + level\n",
        ),
        ("world.fun", "let load = (p) => File.size(p) * 2.0\n"),
    ]);
    assert_eq!(classes(&inferred, "World.load"), ["file"]);
    assert_eq!(inferred.path("reload", ReadClass::File), ["World.load", "File.size"]);
    assert_eq!(classes(&inferred, "level"), ["file"]);
    // `level` is a float: already read at load, so using it reads nothing.
    assert!(classes(&inferred, "tick").is_empty());
}

#[test]
fn a_function_passed_or_stored_is_charged_where_it_is_named() {
    let (_, inferred) = load(&[(
        "game.fun",
        "type Api = { has: (string) => bool }\n\
         let apply = (f, x) => f(x)\n\
         let viaArg = (p) => apply(File.exists, p)\n\
         let reader = File.exists\n\
         let viaValue = (p) => reader(p)\n\
         let api: Api = { has: (p) => File.exists(p) }\n\
         let viaRecord = (p) => api.has(p)\n",
    )]);
    // The higher-order function itself is pure; its caller pays.
    assert!(classes(&inferred, "apply").is_empty());
    assert_eq!(inferred.path("viaArg", ReadClass::File), ["File.exists"]);
    // A stored function (a partial, a record field) keeps spreading.
    assert_eq!(classes(&inferred, "reader"), ["file"]);
    assert_eq!(inferred.path("viaValue", ReadClass::File), ["reader", "File.exists"]);
    assert_eq!(inferred.path("viaRecord", ReadClass::File), ["api", "File.exists"]);
}

#[test]
fn closure_free_values_stop_the_spread() {
    let (_, inferred) = load(&[(
        "game.fun",
        "type Level = { size: float, tags: List<string> }\n\
         type Tree =\n  | Leaf\n  | Node(left: Tree, right: Tree, size: float)\n\
         type Lazy =\n  | Done(size: float)\n  | Later(next: (float) => Lazy)\n\
         let level: Level = { size: File.size(\"a\"), tags: [] }\n\
         let tree = Node(Leaf, Leaf, File.size(\"b\"))\n\
         let lazy = Later((x) => Done(File.size(\"c\")))\n\
         let fx = if File.exists(\"d\") then Effect.none() else Effect.log(1.0)\n\
         let pair = (File.size(\"e\"), Physics.height(\"f\"))\n\
         let draw = (m) => level.size + m\n\
         let walk = (t) => match tree with | Node(l, r, s) => s | Leaf => 0.0\n\
         let force = (m) => match lazy with | Later(f) => f(m) | Done(s) => Done(s)\n\
         let effects = (m) => fx\n\
         let both = (m) => pair\n",
    )]);
    for value in ["level", "tree", "lazy", "fx"] {
        assert_eq!(classes(&inferred, value), ["file"], "{value}");
    }
    // Records, recursive variants and tuples of data stop it.
    assert!(classes(&inferred, "draw").is_empty());
    assert!(classes(&inferred, "walk").is_empty());
    assert!(classes(&inferred, "both").is_empty());
    // A variant holding a function, and an `Effect.t`, don't.
    assert_eq!(inferred.path("force", ReadClass::File), ["lazy", "File.size"]);
    assert_eq!(inferred.path("effects", ReadClass::File), ["fx", "File.exists"]);
}

/// A value whose type is still generic (`Result.t<float, 'e>`, `List<'a>`)
/// is data too: nothing but `unknown` can put a function there.
#[test]
fn generic_data_values_stop_the_spread() {
    let (_, inferred) = load(&[(
        "game.fun",
        "let size = if File.exists(\"a\") then Result.Ok(File.size(\"a\")) else Result.Ok(0.0)\n\
         let none = if File.exists(\"b\") then [] else []\n\
         let tick = (m) => match size with | Result.Ok(s) => s + m | Result.Error(e) => m\n\
         let count = (m) => List.length(none) + m\n",
    )]);
    assert_eq!(classes(&inferred, "size"), ["file"]);
    assert!(classes(&inferred, "tick").is_empty());
    assert!(classes(&inferred, "count").is_empty());
}

/// Every name counts, called or not: a local lambda that is never called
/// still charges its def (an over-approximation PR 4 may narrow).
#[test]
fn a_named_but_uncalled_local_lambda_still_counts() {
    let (_, inferred) = load(&[(
        "game.fun",
        "let tick = (m) => let f = (x) => File.exists(x) in m\n",
    )]);
    assert_eq!(classes(&inferred, "tick"), ["file"]);
}

/// The pass is sound only modulo `unknown`: erasing a function's type hides
/// it. The run-time `note_read` is the authority for exactly this reason.
#[test]
fn unknown_erasure_is_a_known_miss() {
    let (_, inferred) = load(&[(
        "game.fun",
        "let erase = (x: unknown): float => x\n\
         let hidden = erase(File.exists)\n\
         let invoke = (f: unknown) => f(\"flag\")\n\
         let observed = (m) => invoke(hidden)\n",
    )]);
    assert_eq!(classes(&inferred, "hidden"), ["file"]);
    assert!(classes(&inferred, "observed").is_empty());
}

#[test]
fn hover_shows_a_defs_inferred_classes() {
    let src = "let has = (p) => File.exists(p)\n\
               let size = File.size(\"a\")\n\
               let tick = (m) => if has(\"x\") then size else m\n\
               let calm = (m) => m + size\n";
    let (project, _) = load(&[("game.fun", src)]);
    let (_, types) = project.check_with_types();
    let base = project
        .sources
        .file_by_path(std::path::Path::new("game.fun"))
        .expect("the entry")
        .base;
    let hover = |needle: &str| {
        let offset = base + src.find(needle).expect("in source") + 1;
        functor_lang::hover::hover_text(&project.module, &types, offset).map(|(_, text)| text)
    };
    assert_eq!(
        hover("tick =").as_deref(),
        Some("impure[file]\ntick : (float) => float")
    );
    assert_eq!(hover("size =").as_deref(), Some("impure[file]\nsize : float"));
    assert_eq!(hover("calm =").as_deref(), Some("calm : (float) => float"));
    // A reference to a def leads with its header too.
    assert_eq!(
        hover("has(\"x\")").as_deref(),
        Some("impure[file]\nhas : (string) => bool")
    );
}
