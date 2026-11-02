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
            "Ui",
            "type view = host\nimpure[ui]\nlet button : (string, float) => view\n",
        ),
        BundledModule::interface(
            "Job",
            "let run : (string, ('i) => 'o, 'i, ('r) => 'msg) => Effect.t\n",
        ),
        BundledModule::interface(
            "Effect",
            "type t = host\nlet none : () => t\nlet log : (float) => t\n\
             let after : (() => bool) => t\nlet batch : (List<t>) => t\n",
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
    assert_eq!(inferred.path("viaRecord", ReadClass::File), ["api.has", "File.exists"]);
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
         let fx = Effect.after(() => File.exists(\"d\"))\n\
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

/// A local lambda counts where it's called, not where it's named.
#[test]
fn a_local_lambda_counts_only_when_called() {
    let (_, inferred) = load(&[(
        "game.fun",
        "let idle = (m) => let f = (x) => File.exists(x) in m\n\
         let busy = (m) => let f = (x) => File.exists(x) in f(\"a\")\n",
    )]);
    assert!(classes(&inferred, "idle").is_empty());
    assert_eq!(classes(&inferred, "busy"), ["file"]);
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

/// [`functor_lang::reads::phase_errors`] for the plain entry contract, each
/// as `line: message`.
fn phase(src: &str) -> Vec<String> {
    let (project, _) = load(&[("game.fun", src)]);
    let (_, types) = project.check_with_types();
    let entry = functor_lang::reads::Entry {
        hooks: &["tick", "draw", "ui"],
        init: "init",
        config: "config",
    };
    functor_lang::reads::phase_errors(&project.module, &types, &entry)
        .into_iter()
        .map(|error| {
            let (_, line, _) = project.sources.resolve(error.span.start);
            format!("{line}: {}", error.message)
        })
        .collect()
}

#[test]
fn a_file_read_reached_from_a_hook_is_an_error() {
    let errors = phase(
        "let load = (p) => File.size(p)\n\
         let clip = (p) => load(p) * 2.0\n\
         let level = clip(\"level.bin\")\n\
         let tick = (m) => m + clip(\"walk.anim\") + level\n",
    );
    assert_eq!(
        errors,
        ["4: `tick` reads project files (tick → clip → load → File.size), but files are read \
only at load, so the answer isn't pinned for replay — read it in a top-level value and use \
that value in `tick`"]
    );
}

#[test]
fn physics_and_ui_reads_at_load_are_errors() {
    let errors = phase(
        "let eye = (t) => Physics.height(t) + 1.0\n\
         let start = eye(\"player\")\n\
         let menu = Ui.button(\"go\", 1.0)\n\
         let tick = (m) => m + eye(\"player\")\n",
    );
    assert_eq!(
        errors,
        [
            "2: `start` reads the live physics world at load (start → eye → Physics.height), \
but no world has been stepped yet — read it in a hook (`tick`, `draw`, …), or compute it \
from data",
            "3: `menu` builds UI handlers at load (menu → Ui.button), but their slots belong \
to one frame — build the view in `ui`",
        ]
    );
}

/// Fable H1: a lambda passed to a call may run now; one stored in a record,
/// list or variant only runs when called later. File data a value holds is
/// already read, so a hook using it reads nothing.
#[test]
fn only_lambdas_that_may_run_count_at_load() {
    let errors = phase(
        "type Viewer = { tick: (float) => float, size: float }\n\
         let viewer: Viewer = { tick: (m) => m + Physics.height(\"a\"), size: File.size(\"v\") }\n\
         let held = [(m) => Physics.height(\"b\")]\n\
         type Box = | Boxed(f: (float) => float)\n\
         let boxed = Boxed((m) => Physics.height(\"c\"))\n\
         let mapped = List.map((t) => Physics.height(t), [\"d\"])\n\
         let local = let f = (t) => Physics.height(t) in f(\"e\")\n\
         let tick = (m) => viewer.tick(m) + viewer.size\n",
    );
    assert_eq!(errors.len(), 2, "{errors:#?}");
    assert!(errors[0].starts_with("6: `mapped` reads the live physics world"), "{errors:#?}");
    assert!(errors[1].starts_with("7: `local` reads the live physics world"), "{errors:#?}");
}

#[test]
fn init_and_config_read_nothing() {
    let errors = phase(
        "let level = File.size(\"level.bin\")\n\
         let init = { size: level, probe: File.size(\"x\") }\n\
         let config = if File.exists(\"y\") then 1.0 else 0.0\n",
    );
    assert_eq!(
        errors,
        [
            "2: `init` can't read project files (init → File.size): read them in a top-level \
value (`let level = …`) and use that value in `init`",
            "3: `config` can't read project files (config → File.exists): it's read first, to \
learn which paths are external — read them in a top-level value `config` doesn't use",
        ]
    );
}

#[test]
fn memo_builders_read_nothing() {
    let errors = phase(
        "let build = (d) => if Physics.height(\"a\") > d then Scene.cube() else Scene.cube()\n\
         let pure = (d) => Scene.cube()\n\
         let draw = (m) => Scene.group([\n\
           Scene.memo(\"a\", m, build),\n\
           Scene.memo(\"b\", m, (d) => if File.exists(\"c\") then Scene.cube() else Scene.cube()),\n\
           Scene.memo(\"c\", m, pure)])\n",
    );
    assert_eq!(
        errors,
        [
            "4: a Scene.memo builder can't read the live physics world (builder → build → \
Physics.height): deps can't capture it, so the cached result would freeze — make that read \
outside the memo and pass the values the builder needs in deps",
            // The memo error stands for the hook's file error at the same read.
            "5: a Scene.memo builder can't read project files (builder → File.exists): deps \
can't capture it, so the cached result would freeze — read the file in a top-level value and \
use that value in the builder",
        ]
    );
}

#[test]
fn unit_constructors_and_operators_read_nothing() {
    let errors = phase(
        "type len = | Len(m: float)\n\
         let meters = (x): len => Len(x * File.size(\"scale\"))\n\
         let add = (a: len, b: len): len => Len(Physics.height(\"a\"))\n\
         unit u = meters\n\
         unit u (+) = add\n",
    );
    assert_eq!(
        errors,
        [
            "4: the `u` unit can't read project files (u → meters → File.size): it runs \
wherever a `1u` literal appears, so it must be pure",
            "5: the `u` unit's (+) can't read the live physics world ((+) → add → \
Physics.height): it runs wherever (+) meets a `u` value, so it must be pure",
        ]
    );
}

#[test]
fn expects_may_read_files_but_not_physics() {
    let errors = phase(
        "expect File.size(\"a\") > 0.0\n\
         expect Physics.height(\"b\") > 0.0\n",
    );
    assert_eq!(
        errors,
        ["2: this `expect` can't read the live physics world (expect → Physics.height): \
expects run at load, outside any frame"]
    );
}

/// A value built by a function holds only the code the function returns: a
/// file read made while building it (`mesh`) ran at load.
#[test]
fn a_built_value_holds_only_its_results_code() {
    let errors = phase(
        "type Viewer = { tick: (float) => float, mesh: float }\n\
         let make = (p): Viewer => { tick: (m) => m + Physics.height(p), mesh: File.size(p) }\n\
         let viewer = make(\"v.bin\")\n\
         let tick = (m) => viewer.tick(m) + viewer.mesh\n\
         let draw = (m) => make(\"w.bin\").tick(m)\n",
    );
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert!(
        errors[0].starts_with("5: `draw` reads project files (draw → make → File.size)"),
        "{errors:#?}"
    );
}

/// Code that only runs while a value is built, or that a value holds but no
/// one calls, isn't charged where the value is used.
#[test]
fn reads_that_cannot_run_are_not_errors() {
    let errors = phase(
        "type Viewer = { tick: (float) => float, size: float, present: bool }\n\
         let shown = (x) => x\n\
         let viewer: Viewer = { tick: (m) => m + Physics.height(\"a\"), size: 1.0, present: true }\n\
         let size = shown(viewer.size)\n\
         let make = (p): Viewer =>\n\
           let probe = () => File.exists(p) in { tick: (m) => m, size: 0.0, present: probe() }\n\
         let built = make(\"x\")\n\
         let tick = (m) => let unused = () => File.exists(\"y\") in built.tick(m)\n",
    );
    assert!(errors.is_empty(), "{errors:#?}");
}

/// Calling one field of a record value runs that field only; a lambda handed
/// to a call that keeps it (an `Effect.t`, a record of handlers) only runs
/// later.
#[test]
fn stored_code_is_charged_where_it_runs() {
    let errors = phase(
        "type Viewer = { tick: (float) => float, reload: () => bool, start: () => float }\n\
         type Probe = { label: string, read: () => float }\n\
         let viewer: Viewer = {\n\
           tick: (m) => m + 1.0,\n\
           reload: () => File.exists(\"v.bin\"),\n\
           start: () => Physics.height(\"p\") }\n\
         let fresh = viewer.reload()\n\
         let probe = (label: string, read: () => float): Probe => { label: label, read: read }\n\
         let probes = [probe(\"x\", () => Physics.height(\"p\"))]\n\
         let later = Effect.after(() => Physics.height(\"p\") > 0.0)\n\
         let tick = (m) => viewer.tick(m) + List.length(probes)\n\
         let draw = (m) => Scene.memo(\"k\", m, viewer.tick)\n\
         let init = viewer.start()\n",
    );
    // Only `init` runs a physics read at load.
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert!(
        errors[0].starts_with(
            "13: `init` reads the live physics world at load (init → viewer.start → Physics.height)"
        ),
        "{errors:#?}"
    );
}

/// An alias of a record value, a local factory and a discarded match
/// scrutinee hold no more code than they return.
#[test]
fn returned_code_follows_aliases_and_locals() {
    let errors = phase(
        "type Api = { step: (float) => float, read: () => bool }\n\
         type Box = | Boxed(read: () => bool)\n\
         let api: Api = { step: (m) => m, read: () => File.exists(\"x\") }\n\
         let alias = api\n\
         let saved = let make = () => let present = File.exists(\"y\") in (m: float) => m in make()\n\
         let boxed = match Boxed(() => File.exists(\"z\")) with | Boxed(r) => (m: float) => m\n\
         let tick = (m) => alias.step(m) + saved(m) + boxed(m)\n",
    );
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn a_job_function_may_read_files_but_not_live_state() {
    let errors = phase(
        "type Msg = | Done(r: float)\n\
         let size = (p) => File.size(p)\n\
         let fine = (p) => size(p)\n\
         let live = (p) => Physics.height(p)\n\
         let tick = (m) => (m, Effect.batch([Job.run(\"a\", fine, \"x\", Done), Job.run(\"b\", live, \"x\", Done)]))\n",
    );
    assert_eq!(
        errors,
        ["5: a `Job.run` function can't read the live physics world (job → live → Physics.height): it \
runs on a worker, off the frame — read it in a hook and put it in the input"]
    );
}
