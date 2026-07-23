//! B8 part 1 verification (docs/functor-lang.md): multi-file projects — file =
//! module, qualified-by-default access (values, constructors in expressions
//! AND patterns, type annotations), `open`, eager whole-program loading,
//! cycle refusal, protected namespaces, and cross-file hot-reload rebind.

use std::fs;
use std::path::{Path, PathBuf};

use functor_lang::value::Value;
use functor_lang::{RunOutcome, Tracing};

/// Write `files` into a fresh scratch directory and return it. The first
/// file is the entry.
struct Scratch {
    dir: PathBuf,
    entry: PathBuf,
}

impl Scratch {
    fn new(name: &str, files: &[(&str, &str)]) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("functor-lang-project-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        for (file, src) in files {
            fs::write(dir.join(file), src).expect("write scratch file");
        }
        let entry = dir.join(files[0].0);
        Scratch { dir, entry }
    }

    fn load(&self) -> Result<functor_lang::project::Project, functor_lang::project::ProjectError> {
        functor_lang::project::load(&self.entry)
    }

    /// Strip this scratch dir's prefix (with the platform's separator — `\`
    /// on Windows) from a rendered `path:line:col` diagnostic.
    fn strip_dir(&self, rendered: &str) -> String {
        let prefix = format!("{}{}", self.dir.display(), std::path::MAIN_SEPARATOR);
        rendered.replace(&prefix, "")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Load a scratch project, expecting success.
fn load(name: &str, files: &[(&str, &str)]) -> functor_lang::project::Project {
    let scratch = Scratch::new(name, files);
    scratch
        .load()
        .unwrap_or_else(|e| panic!("project should load: {}", e.render()))
}

/// Load a scratch project, expecting failure; returns the rendered error
/// (`path:line:col: message`) with the scratch dir prefix stripped.
fn load_err(name: &str, files: &[(&str, &str)]) -> String {
    let scratch = Scratch::new(name, files);
    let err = match scratch.load() {
        Err(err) => err,
        Ok(_) => panic!("project should fail to load"),
    };
    scratch.strip_dir(&err.render())
}

/// Run a scratch project's `main`.
fn run_main(name: &str, files: &[(&str, &str)]) -> Value {
    let project = load(name, files);
    let record = functor_lang::run(&project.module, Tracing::Off).unwrap_or_else(|failure| {
        panic!(
            "project should run: {}",
            project
                .sources
                .render(failure.error.span.start, &failure.error.message)
        )
    });
    match record.outcome {
        RunOutcome::Main(value) => value,
        RunOutcome::Bindings(_) => panic!("expected a main result"),
    }
}

fn number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => *n,
        other => panic!("expected a number, got {other}"),
    }
}

// ── The committed fixture ────────────────────────────────────────────────

/// `examples/project/` exercises the whole surface: `open`, qualified
/// values/ctors/types, generics across modules. Run + check must stay clean.
#[test]
fn fixture_runs_and_checks_clean() {
    let entry = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("project")
        .join("game.fun");
    let project = functor_lang::project::load(&entry).unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert!(diags.is_empty(), "fixture should check clean: {diags:?}");
    let record =
        functor_lang::run(&project.module, Tracing::Off).unwrap_or_else(|f| panic!("{}", f.error.message));
    match record.outcome {
        RunOutcome::Main(Value::Number(n)) => assert_eq!(n, 7.75),
        other => panic!(
            "expected main = 7.75, got {:?}",
            matches!(other, RunOutcome::Main(_))
        ),
    }
}

// ── Qualified access (no import) ─────────────────────────────────────────

#[test]
fn qualified_values_and_ctors_work_without_open() {
    let value = run_main(
        "qualified",
        &[
            (
                "game.fun",
                "let main = () =>\n\
                 match Util.wrap(4.0) with\n\
                 | Util.Wrapped(n) => n + Util.base\n",
            ),
            (
                "util.fun",
                "type Carton = | Wrapped(value: float)\n\
                 let base = 10.0\n\
                 let wrap = (n: float): Carton => Wrapped(n)\n",
            ),
        ],
    );
    assert_eq!(number(&value), 14.0);
}

/// Unapplied qualified constructors are first-class, like bare ones.
#[test]
fn qualified_ctor_is_first_class() {
    let value = run_main(
        "ctor-value",
        &[
            (
                "game.fun",
                "let main = () =>\n\
                 [1.0, 2.0] |> List.map(Util.Wrapped) |> List.map(Util.unwrap) |> List.maximum\n",
            ),
            (
                "util.fun",
                "type Carton = | Wrapped(value: float)\n\
                 let unwrap = (c) => match c with | Wrapped(n) => n\n",
            ),
        ],
    );
    assert_eq!(number(&value), 2.0);
}

/// Entry-module members referenced from a sibling resolve bare (the entry's
/// canonical names have no prefix) — legal as long as there is no cycle.
#[test]
fn sibling_may_reference_the_entry() {
    let project = load(
        "entry-ref",
        &[
            ("game.fun", "let base = 32.0\n"),
            ("util.fun", "let above = (x) => x + Game.base\n"),
        ],
    );
    let session = functor_lang::Session::load(&project.module, &mut functor_lang::NoHost)
        .unwrap_or_else(|f| panic!("session should load: {}", f.error.message));
    let result = session
        .call("Util.above", vec![Value::Number(10.0)], &mut functor_lang::NoHost)
        .expect("call should succeed");
    assert_eq!(number(&result), 42.0);
}

/// Eager loading in dependency order: the entry's top-level initializer may
/// demand a sibling's global.
#[test]
fn entry_initializer_may_demand_sibling_globals() {
    let value = run_main(
        "eager-order",
        &[
            (
                "game.fun",
                "let start = Config.speed * 2.0\nlet main = () => start\n",
            ),
            ("config.fun", "let speed = 21.0\n"),
        ],
    );
    assert_eq!(number(&value), 42.0);
}

/// A bare module name used as a value gets a teaching hint.
#[test]
fn bare_module_name_hints() {
    let err = load_err(
        "bare-module",
        &[
            ("game.fun", "let main = () => Util\n"),
            ("util.fun", "let x = 1.0\n"),
        ],
    );
    assert_eq!(
        err,
        "game.fun:1:18: unknown name `Util` — `Util` is a module; reference a member (`Util.name`)"
    );
}

#[test]
fn unknown_member_is_a_load_error() {
    let err = load_err(
        "unknown-member",
        &[
            ("game.fun", "let main = () => Util.nope(1.0)\n"),
            ("util.fun", "let x = 1.0\n"),
        ],
    );
    assert_eq!(err, "game.fun:1:18: module `Util` has no `nope`");
}

#[test]
fn unknown_member_type_is_a_load_error() {
    let err = load_err(
        "unknown-type",
        &[
            ("game.fun", "let f = (x: Util.Nope) => x\n"),
            ("util.fun", "let x = 1.0\n"),
        ],
    );
    assert_eq!(err, "game.fun:1:13: module `Util` has no type `Nope`");
}

#[test]
fn unknown_ctor_in_pattern_is_a_load_error() {
    let err = load_err(
        "unknown-pattern-ctor",
        &[
            (
                "game.fun",
                "let f = (x) => match x with | Util.Nope => 1.0\n",
            ),
            ("util.fun", "let x = 1.0\n"),
        ],
    );
    assert_eq!(
        err,
        "game.fun:1:31: module `Util` has no constructor `Nope`"
    );
}

/// A qualified name whose head is NOT a module stays the External seam
/// (builtins keep working; unknown ones stay runtime errors, as before).
#[test]
fn non_module_qualified_names_stay_external() {
    let value = run_main(
        "external-seam",
        &[
            ("game.fun", "let main = () => Math.clamp01(3.0) + Util.x\n"),
            ("util.fun", "let x = 1.0\n"),
        ],
    );
    assert_eq!(number(&value), 2.0);
}

// ── open ─────────────────────────────────────────────────────────────────

#[test]
fn open_brings_defs_ctors_and_types_into_scope() {
    let value = run_main(
        "open-basic",
        &[
            (
                "game.fun",
                "open Util\n\
                 let grab = (c: Carton): float => match c with | Wrapped(n) => n\n\
                 let main = () => grab(Wrapped(base))\n",
            ),
            (
                "util.fun",
                "type Carton = | Wrapped(value: float)\nlet base = 42.0\n",
            ),
        ],
    );
    assert_eq!(number(&value), 42.0);
}

#[test]
fn open_collision_with_own_name_names_both_sides() {
    let err = load_err(
        "open-own-collision",
        &[
            ("game.fun", "open Util\nlet base = 1.0\n"),
            ("util.fun", "let base = 2.0\n"),
        ],
    );
    assert_eq!(
        err,
        "game.fun:1:1: open Util: `base` collides with this module's own `base` — qualify uses \
as `Util.base` instead of opening"
    );
}

#[test]
fn open_collision_between_opens_names_both_modules() {
    let err = load_err(
        "open-open-collision",
        &[
            ("game.fun", "open Alpha\nopen Beta\nlet main = () => 0.0\n"),
            ("alpha.fun", "let shared = 1.0\n"),
            ("beta.fun", "let shared = 2.0\n"),
        ],
    );
    assert_eq!(
        err,
        "game.fun:2:1: open Beta: `shared` is already in scope from `open Alpha` — qualify uses \
(`Alpha.shared` / `Beta.shared`)"
    );
}

#[test]
fn open_type_collision_is_an_error() {
    let err = load_err(
        "open-type-collision",
        &[
            (
                "game.fun",
                "open Util\ntype Carton = { x: float }\nlet main = () => 0.0\n",
            ),
            ("util.fun", "type Carton = | Wrapped(value: float)\n"),
        ],
    );
    assert_eq!(
        err,
        "game.fun:1:1: open Util: type `Carton` collides with this module's own `Carton` — \
qualify uses as `Util.Carton` instead of opening"
    );
}

#[test]
fn open_unknown_module_is_an_error() {
    let err = load_err(
        "open-unknown",
        &[("game.fun", "open Nowhere\nlet main = () => 0.0\n")],
    );
    assert_eq!(
        err,
        "game.fun:1:1: unknown module `Nowhere` — modules are the sibling `.fun` files next to \
the entry"
    );
}

#[test]
fn open_self_is_an_error() {
    let err = load_err(
        "open-self",
        &[("game.fun", "open Game\nlet main = () => 0.0\n")],
    );
    assert_eq!(
        err,
        "game.fun:1:1: `open Game` in module `Game` itself — a module's own names are already \
in scope"
    );
}

/// `open` is contextual: it stays a perfectly good binding name.
#[test]
fn open_remains_usable_as_a_name() {
    let value = run_main(
        "open-as-name",
        &[(
            "game.fun",
            "let open = 40.0\nlet f = (open) => open + 2.0\nlet main = () => f(open)\n",
        )],
    );
    assert_eq!(number(&value), 42.0);
}

// ── Load-time refusals ───────────────────────────────────────────────────

#[test]
fn dependency_cycles_are_refused_with_the_path() {
    let err = load_err(
        "cycle",
        &[
            ("game.fun", "let a = () => Util.b()\n"),
            ("util.fun", "let b = () => Game.a()\n"),
        ],
    );
    assert!(
        err.contains("modules depend on each other in a cycle: Game → Util → Game"),
        "unexpected error: {err}"
    );
}

/// An `open` alone is a dependency edge — a cycle through it is refused
/// even if no opened name is used.
#[test]
fn open_counts_as_a_dependency_edge() {
    let err = load_err(
        "open-cycle",
        &[
            ("game.fun", "open Util\nlet a = 1.0\n"),
            ("util.fun", "open Game\nlet b = 2.0\n"),
        ],
    );
    assert!(
        err.contains("cycle: Game → Util → Game"),
        "unexpected error: {err}"
    );
}

#[test]
fn protected_namespace_module_names_are_refused() {
    let err = load_err(
        "protected",
        &[
            ("game.fun", "let main = () => 0.0\n"),
            ("scene.fun", "let cube = 1.0\n"),
        ],
    );
    assert_eq!(
        err,
        "scene.fun:1:1: module name `Scene` (from scene.fun) collides with the builtin/prelude \
namespace `Scene` — rename the file"
    );
}

/// `Debug` is protected (the `Debug.log` builtin's namespace), so a sibling
/// `debug.fun` can't shadow it — a load error names the collision.
#[test]
fn debug_namespace_module_name_is_refused() {
    let err = load_err(
        "protected-debug",
        &[
            ("game.fun", "let main = () => 0.0\n"),
            ("debug.fun", "let log = 1.0\n"),
        ],
    );
    assert_eq!(
        err,
        "debug.fun:1:1: module name `Debug` (from debug.fun) collides with the builtin/prelude \
namespace `Debug` — rename the file"
    );
}

#[test]
fn non_identifier_entry_stem_is_refused() {
    // A non-identifier SIBLING is now skipped (see
    // `non_identifier_fun_siblings_are_skipped`), but the ENTRY file — which is
    // always loaded — must still name a valid module.
    let err = load_err("bad-entry-stem", &[("my-utils.fun", "let x = 1.0\n")]);
    assert!(
        err.contains("cannot derive a module name from `my-utils.fun`"),
        "unexpected error: {err}"
    );
}

/// A SINGLE inline source (the wasm single-entry / docs "try it" path) whose
/// path is a non-identifier label — a `data:` URL — loads as module `Main`
/// rather than hard-erroring on module-name derivation. Multi-file projects
/// keep the loud errors (see `non_identifier_file_stems_are_refused`).
#[test]
fn single_source_with_non_identifier_path_loads_as_main() {
    let sources = vec![(
        PathBuf::from("data:text/plain;base64,bGV0IHg="),
        "let main = () => 0.0\n".to_string(),
    )];
    let project = functor_lang::project::load_sources_with_prelude(sources, &[])
        .unwrap_or_else(|e| panic!("single inline source should load: {}", e.render()));
    assert_eq!(project.entry, "Main");
}

/// A single `physics.fun` source (stem capitalizes to the protected `Physics`
/// namespace) also falls back to `Main` rather than colliding with the
/// builtin/prelude namespace the loader injects.
#[test]
fn single_source_with_protected_stem_loads_as_main() {
    let sources = vec![(
        PathBuf::from("physics.fun"),
        "let main = () => 0.0\n".to_string(),
    )];
    let project = functor_lang::project::load_sources_with_prelude(sources, &[])
        .unwrap_or_else(|e| panic!("single protected-stem source should load: {}", e.render()));
    assert_eq!(project.entry, "Main");
}

/// In single-file (non-project) lowering, `open` is an unknown module —
/// the honest answer for the LSP's per-file view too.
#[test]
fn open_outside_a_project_is_an_error() {
    let program = functor_lang::parse("open Util\nlet x = 1.0\n").expect("parses");
    let err = functor_lang::lower(program).expect_err("should not lower");
    assert!(
        err.message.contains("unknown module `Util`"),
        "unexpected error: {}",
        err.message
    );
}

// ── Whole-program checking + span rendering ──────────────────────────────

/// Sibling modules are checked even when unreferenced, and diagnostics
/// render against the sibling's own file and position.
#[test]
fn unreferenced_sibling_diagnostics_surface_with_their_file() {
    let scratch = Scratch::new(
        "sibling-diags",
        &[
            ("game.fun", "let main = () => 0.0\n"),
            (
                "util.fun",
                "// an unreferenced module with a type error\nlet bad = (a: float): float => a + \"one\"\n",
            ),
        ],
    );
    let project = scratch.load().unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
    let rendered = project
        .sources
        .render(diags[0].span.start, &diags[0].message);
    let rendered = scratch.strip_dir(&rendered);
    assert_eq!(err_line(&rendered), "util.fun:2:36");
}

fn err_line(rendered: &str) -> &str {
    rendered.rsplit_once(": ").map(|(pos, _)| pos).unwrap_or("")
}

/// Cross-module inference has teeth: a bad argument to a sibling's function
/// is a real diagnostic, generics included.
#[test]
fn cross_module_generics_check() {
    let scratch = Scratch::new(
        "cross-generics",
        &[
            (
                "game.fun",
                "open Boxes\n\
                 let good = (b: Box<float>): float =>\n\
                 match b with | Full(v) => v + 1.0 | Empty => 0.0\n\
                 let bad = () => good(Full(\"nope\"))\n",
            ),
            ("boxes.fun", "type Box<'v> = | Full(value: 'v) | Empty\n"),
        ],
    );
    let project = scratch.load().unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
    assert!(
        diags[0]
            .message
            .contains("expected Boxes.Box<float>, got Boxes.Box<string>"),
        "unexpected diagnostic: {}",
        diags[0].message
    );
}

/// Binding annotations in a NON-ENTRY module are canonicalized like the
/// entry's: the sibling's own bare type name resolves nominally, so a wrong
/// value is a real diagnostic (it used to silently resolve to Unknown), on
/// both top-level `let name: Type = …` and expression `let … in`.
#[test]
fn sibling_binding_annotations_are_enforced() {
    let scratch = Scratch::new(
        "sibling-binding-annot",
        &[
            ("game.fun", "let main = () => 0.0\n"),
            (
                "utils.fun",
                "type Shape = | Circle(radius: float) | Point\n\
                 let bad: Shape = 3.0\n\
                 let letIn = () =>\n\
                 let x: Shape = 4.0 in\n\
                 x\n",
            ),
        ],
    );
    let project = scratch.load().unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert_eq!(diags.len(), 2, "expected two diagnostics, got {diags:?}");
    for diag in &diags {
        assert!(
            diag.message.contains("expected Utils.Shape, got float"),
            "unexpected diagnostic: {}",
            diag.message
        );
    }
}

/// Qualified cross-module type names in a sibling's binding annotation
/// resolve too: a good value checks clean, a bad one is flagged with the
/// canonical name.
#[test]
fn qualified_types_in_sibling_binding_annotations() {
    let files: &[(&str, &str)] = &[
        ("game.fun", "let main = () => 0.0\n"),
        ("shapes.fun", "type Shape = | Circle(radius: float) | Point\n"),
        (
            "consumer.fun",
            "let good: Shapes.Shape = Shapes.Circle(1.0)\n\
             let bad: Shapes.Shape = 3.0\n",
        ),
    ];
    let scratch = Scratch::new("sibling-qualified-annot", files);
    let project = scratch.load().unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
    assert!(
        diags[0]
            .message
            .contains("expected Shapes.Shape, got float"),
        "unexpected diagnostic: {}",
        diags[0].message
    );
}

/// `open`ed type names in binding annotations resolve to their module's
/// canonical type (this held for params; binding annotations skipped it).
#[test]
fn opened_types_in_binding_annotations_are_enforced() {
    let scratch = Scratch::new(
        "opened-binding-annot",
        &[
            (
                "game.fun",
                "open Shapes\nlet bad: Shape = 3.0\nlet main = () => 0.0\n",
            ),
            ("shapes.fun", "type Shape = | Circle(radius: float) | Point\n"),
        ],
    );
    let project = scratch.load().unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
    assert!(
        diags[0]
            .message
            .contains("expected Shapes.Shape, got float"),
        "unexpected diagnostic: {}",
        diags[0].message
    );
}

/// Interface (`.funi`) types — the prelude's `Anim.t` shape — are enforced in
/// a SIBLING module's binding annotations: a host-made value checks clean, a
/// bare number is flagged.
#[test]
fn interface_types_in_sibling_binding_annotations() {
    let scratch = Scratch::new(
        "sibling-funi-annot",
        &[
            ("game.fun", "let main = () => 0.0\n"),
            (
                "widget.funi",
                "type Handle\nlet make : () => Handle\n",
            ),
            (
                "util.fun",
                "let good: Widget.Handle = Widget.make()\n\
                 let bad: Widget.Handle = 3.0\n",
            ),
        ],
    );
    let project = scratch.load().unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
    assert!(
        diags[0]
            .message
            .contains("expected Widget.Handle, got float"),
        "unexpected diagnostic: {}",
        diags[0].message
    );
}

/// Binding annotations get the same lowering-time validation as param
/// annotations: an unknown member of a KNOWN module is a load error.
#[test]
fn unknown_member_type_in_binding_annotation_is_a_load_error() {
    let err = load_err(
        "unknown-binding-annot-type",
        &[
            ("game.fun", "let x: Util.Nope = 1.0\n"),
            ("util.fun", "let x = 1.0\n"),
        ],
    );
    assert_eq!(err, "game.fun:1:8: module `Util` has no type `Nope`");
}

/// An UNREFERENCED sibling declaring a same-shaped record type must not
/// capture (or make ambiguous) a bare record literal elsewhere — literal
/// resolution is scoped to types visible unqualified where the literal is
/// written. [Codex High — B8 review]
#[test]
fn stray_sibling_type_does_not_capture_bare_literals() {
    let scratch = Scratch::new(
        "literal-scope",
        &[
            (
                "game.fun",
                "type Position = { x: float, y: float }
                 let p = { x: 1.0, y: 2.0 }
let main = () => p.x
",
            ),
            // Same field shape, never referenced, never opened.
            (
                "extra.fun",
                "type Point = { x: float, y: float }
",
            ),
        ],
    );
    let project = scratch.load().unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert!(
        diags.is_empty(),
        "the stray sibling type must not interfere: {diags:?}"
    );
}

/// `open`ed types ARE literal-visible: a bare literal matching an opened
/// type resolves to it nominally (proven by the resulting type error)…
#[test]
fn opened_types_are_literal_visible() {
    let scratch = Scratch::new(
        "literal-open",
        &[
            (
                "game.fun",
                "open Vec
let f = () => { x: 1.0, y: 2.0 }
let bad = f() + 1.0
",
            ),
            (
                "vec.fun",
                "type V2 = { x: float, y: float }
",
            ),
        ],
    );
    let project = scratch.load().unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
    assert!(
        diags[0].message.contains("Vec.V2"),
        "the literal should have resolved to Vec.V2: {}",
        diags[0].message
    );
}

/// …while the SAME program without the `open` stays gradual: the sibling's
/// type is not in scope unqualified, so the literal is anonymous data.
#[test]
fn unopened_sibling_literals_stay_gradual() {
    let scratch = Scratch::new(
        "literal-no-open",
        &[
            (
                "game.fun",
                "let f = () => { x: 1.0, y: 2.0 }
let bad = f() + 1.0
",
            ),
            (
                "vec.fun",
                "type V2 = { x: float, y: float }
",
            ),
        ],
    );
    let project = scratch.load().unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert!(
        diags.is_empty(),
        "an unopened sibling type must not resolve the literal: {diags:?}"
    );
}

/// A single-file project lowers byte-identically to plain single-file
/// lowering (the backward-compatibility pin: bare names, IDs from zero,
/// spans from zero).
#[test]
fn single_file_project_adds_only_the_core_modules() {
    // A project always includes the language-owned modules, so its merged IR
    // is plain lowering's defs/types plus those modules — nothing else changes.
    let src = "type Shape = | Circle(radius: float) | Point\n\
               let area = (s: Shape): float =>\n\
               match s with | Circle(r) => 3.14 * r * r | Point => 0.0\n\
               let main = () => area(Circle(2.0))\n";
    let project = load("single-file", &[("game.fun", src)]);
    let plain = functor_lang::lower(functor_lang::parse(src).expect("parses")).expect("lowers");

    let proj_defs: Vec<&str> = project.module.defs.iter().map(|d| d.name.as_str()).collect();
    for def in &plain.defs {
        assert!(
            proj_defs.contains(&def.name.as_str()),
            "entry def `{}` must survive into the project unchanged",
            def.name
        );
    }
    let proj_types: Vec<&str> = project.module.types.iter().map(|t| t.name.as_str()).collect();
    for ty in &plain.types {
        assert!(proj_types.contains(&ty.name.as_str()));
    }
    // The ONLY additions are the core modules': Net's canonicalized
    // `Net.NetEvent` / `Net.HttpResponse`, Random's abstract `Random.Seed`,
    // the input-key variant `Key.t`, and the stdlib's generic Option/Result.
    assert!(
        proj_types.contains(&"Net.NetEvent") && proj_types.contains(&"Net.HttpResponse"),
        "the built-in Net module must be injected: {proj_types:?}"
    );
    assert!(
        proj_types.contains(&"Random.Seed"),
        "the built-in Random module must be injected: {proj_types:?}"
    );
    assert!(
        proj_types.contains(&"Key.t"),
        "the built-in Key module must be injected: {proj_types:?}"
    );
    assert!(
        proj_types.contains(&"Option.t") && proj_types.contains(&"Result.t"),
        "the bundled stdlib modules must be injected: {proj_types:?}"
    );
    assert_eq!(
        proj_types.len(),
        plain.types.len() + 6,
        "no types beyond the entry's + Net(2) + Random + Key + Option + Result"
    );
}

#[test]
fn bundled_option_module_runs_and_checks() {
    let src = "let main = () =>\n\
               let kept = Option.Some(20.0)\n\
                 |> Option.map((n) => n + 1.0)\n\
                 |> Option.bind((n) => Option.Some(n * 2.0))\n\
                 |> Option.filter((n) => n > 40.0) in\n\
               let removed = kept |> Option.filter((n) => n > 100.0) in\n\
               let flags =\n\
                 if Option.isSome(kept) && Option.isNone(removed) then 1.0 else 0.0 in\n\
               Option.defaultValue(0.0, kept)\n\
                 + Option.defaultWith(() => 5.0, removed)\n\
                 + List.length(Option.toList(kept))\n\
                 + flags\n";
    let project = load("option-stdlib", &[("game.fun", src)]);
    let diags = project.check();
    assert!(diags.is_empty(), "Option API should check: {diags:?}");

    let record = functor_lang::run(&project.module, Tracing::Off)
        .unwrap_or_else(|f| panic!("Option API should run: {}", f.error.message));
    match record.outcome {
        RunOutcome::Main(Value::Number(n)) => assert_eq!(n, 49.0),
        _ => panic!("expected a numeric main result"),
    }

    let option = project
        .sources
        .files()
        .iter()
        .find(|file| file.module == "Option")
        .expect("Option source is mapped");
    assert_eq!(option.path, PathBuf::from("<stdlib>/Option.fun"));
    assert!(!option.interface);
}

#[test]
fn bundled_result_module_runs_and_checks() {
    let src = "let main = () =>\n\
               let good = Result.Ok(20.0)\n\
                 |> Result.map((n) => n + 1.0)\n\
                 |> Result.bind((n) => Result.Ok(n * 2.0)) in\n\
               let bad = Result.Error(\"no\")\n\
                 |> Result.map((n) => n + 1.0)\n\
                 |> Result.mapError((e) => Text.concat(e, \"!\")) in\n\
               let flags =\n\
                 if Result.isOk(good) && Result.isError(bad) then 1.0 else 0.0 in\n\
               let options =\n\
                 Option.defaultValue(0.0, Result.toOption(good))\n\
                   + (if Option.isNone(Result.toOption(bad)) then 1.0 else 0.0) in\n\
               Result.defaultValue(0.0, good)\n\
                 + Result.defaultWith((e) => if e == \"no!\" then 5.0 else 0.0, bad)\n\
                 + flags\n\
                 + options\n";
    let project = load("result-stdlib", &[("game.fun", src)]);
    let diags = project.check();
    assert!(diags.is_empty(), "Result API should check: {diags:?}");

    let record = functor_lang::run(&project.module, Tracing::Off)
        .unwrap_or_else(|f| panic!("Result API should run: {}", f.error.message));
    match record.outcome {
        RunOutcome::Main(Value::Number(n)) => assert_eq!(n, 91.0),
        _ => panic!("expected a numeric main result"),
    }
}

/// The built-in `Key` module: the `input` hook's key variant. Qualified
/// constructors work in patterns and expressions, keys compare structurally,
/// and a typo (`Key.Enterr`) is a load-time unknown-member error — the whole
/// point of retiring the string spelling.
#[test]
fn builtin_key_module_is_matchable() {
    let src = "let dir = (k: Key.t): float =>\n\
               match k with\n\
               | Key.Left => 0.0 - 1.0\n\
               | Key.Right => 1.0\n\
               | Key.Num0 => 0.5\n\
               | _ => 0.0\n\
               let main = () => (dir(Key.Left), dir(Key.Num0), Key.Space == Key.Space)\n";
    let project = load("key-module", &[("game.fun", src)]);
    assert!(project.check().is_empty(), "checks clean");
    let record = functor_lang::run(&project.module, Tracing::Off)
        .unwrap_or_else(|f| panic!("runs: {}", f.error.message));
    match record.outcome {
        RunOutcome::Main(value) => assert_eq!(value.to_string(), "(-1, 0.5, true)"),
        RunOutcome::Bindings(_) => panic!("expected main's value"),
    }

    let err = load_err(
        "key-module-typo",
        &[("game.fun", "let f = (k) => match k with | Key.Enterr => 1.0 | _ => 0.0\n")],
    );
    assert!(
        err.contains("module `Key` has no constructor `Enterr`"),
        "a key typo is a load error: {err}"
    );
}

/// The four `Random.*` builtins are typed from two places that must not
/// drift: the injected interface module's signatures (project loads — the
/// authoritative path) and `builtin_signature` (the fallback for bare
/// single-file checks with no project env). The same correct use must be
/// clean through both, and the same misuse must produce the SAME diagnostic.
#[test]
fn random_signatures_agree_between_interface_and_builtin_fallback() {
    let ok = "let ok = () =>\n\
              let (v, s) = Random.step(Random.seed(1.0)) in\n\
              let (w, s2) = Random.range(0.0, 1.0, s |> Random.fork(2.0)) in\n\
              (v + w, s2)\n";
    let bad = "let bad = () => Random.step(1.0)\n";

    // Signature path: a loaded project injects `<builtin>/Random.funi`.
    let via_project = |name: &str, src: &str| -> Vec<String> {
        load(name, &[("game.fun", src)])
            .check()
            .into_iter()
            .map(|d| d.message)
            .collect()
    };
    // Builtin-scheme path: bare parse + lower + check, no project env.
    let via_builtin = |src: &str| -> Vec<String> {
        let module = functor_lang::lower(functor_lang::parse(src).expect("parses")).expect("lowers");
        functor_lang::check(&module).into_iter().map(|d| d.message).collect()
    };

    assert_eq!(via_project("rand-parity-ok", ok), Vec::<String>::new());
    assert_eq!(via_builtin(ok), Vec::<String>::new());
    let project_diag = via_project("rand-parity-bad", bad);
    assert_eq!(project_diag, via_builtin(bad));
    assert_eq!(
        project_diag,
        vec!["argument 1 of `Random.step`: expected Random.Seed, got float".to_string()]
    );
}

// ── Cross-file hot-reload rebind ─────────────────────────────────────────

/// A model-stored closure created by a SIBLING module rebinds across a
/// reload: its stable id carries the module prefix (from the def name), so
/// the edited sibling body is adopted with the captured env carried over.
#[test]
fn stored_closures_from_siblings_rebind_across_reload() {
    let game = "let main = () => Util.makeSpring(3.0)\n";
    let old = load(
        "rebind-old",
        &[
            ("game.fun", game),
            ("util.fun", "let makeSpring = (k) => (x) => x * k\n"),
        ],
    );
    let record = functor_lang::run(&old.module, Tracing::Off)
        .unwrap_or_else(|f| panic!("v1 runs: {}", f.error.message));
    let stored = match record.outcome {
        RunOutcome::Main(value) => value,
        RunOutcome::Bindings(_) => panic!("expected a closure"),
    };

    // Edit the SIBLING file: the spring gains a +1 offset.
    let new = load(
        "rebind-new",
        &[
            ("game.fun", game),
            ("util.fun", "let makeSpring = (k) => (x) => x * k + 1.0\n"),
        ],
    );
    let (rebound, report) = functor_lang::rebind_value(&stored, &old.module, &new.module);
    assert_eq!(report.rebound, 1, "warnings: {:?}", report.warnings);

    let session = functor_lang::Session::load(&new.module, &mut functor_lang::NoHost)
        .unwrap_or_else(|f| panic!("v2 session: {}", f.error.message));
    let result = session
        .apply(
            rebound,
            vec![Value::Number(2.0)],
            "spring",
            &mut functor_lang::NoHost,
        )
        .expect("apply");
    // New body, old captured k: 2 * 3 + 1.
    assert_eq!(number(&result), 7.0);
}

/// Same-named defs in DIFFERENT modules stay distinct rebind identities:
/// editing one module's `make` must not confuse a closure from the other's.
#[test]
fn same_named_defs_in_different_modules_do_not_cross_rebind() {
    let files = |a_body: &str| {
        [
            (
                "game.fun",
                "let main = () => (Alpha.make(1.0), Beta.make(1.0))\n".to_string(),
            ),
            ("alpha.fun", format!("let make = (k) => (x) => {a_body}\n")),
            ("beta.fun", "let make = (k) => (x) => x - k\n".to_string()),
        ]
    };
    let old_files = files("x + k");
    let old = load(
        "twin-old",
        &old_files
            .iter()
            .map(|(n, s)| (*n, s.as_str()))
            .collect::<Vec<_>>(),
    );
    let record = functor_lang::run(&old.module, Tracing::Off)
        .unwrap_or_else(|f| panic!("v1 runs: {}", f.error.message));
    let RunOutcome::Main(Value::Tuple(pair)) = record.outcome else {
        panic!("expected a tuple of closures");
    };
    // Edit ONLY alpha's body.
    let new_files = files("x + k * 10.0");
    let new = load(
        "twin-new",
        &new_files
            .iter()
            .map(|(n, s)| (*n, s.as_str()))
            .collect::<Vec<_>>(),
    );
    let session = functor_lang::Session::load(&new.module, &mut functor_lang::NoHost)
        .unwrap_or_else(|f| panic!("v2 session: {}", f.error.message));

    let (alpha, report) = functor_lang::rebind_value(&pair[0], &old.module, &new.module);
    assert_eq!(report.rebound, 1, "warnings: {:?}", report.warnings);
    let result = session
        .apply(alpha, vec![Value::Number(2.0)], "alpha", &mut functor_lang::NoHost)
        .expect("apply alpha");
    assert_eq!(number(&result), 12.0); // new body: 2 + 1*10

    let (beta, report) = functor_lang::rebind_value(&pair[1], &old.module, &new.module);
    assert_eq!(report.rebound, 1, "warnings: {:?}", report.warnings);
    let result = session
        .apply(beta, vec![Value::Number(2.0)], "beta", &mut functor_lang::NoHost)
        .expect("apply beta");
    assert_eq!(number(&result), 1.0); // beta unchanged: 2 - 1
}

// --- Track D: project-aware LSP typed check (slice 1a) ---

/// `Project::check_with_types` types the whole program, so a def in the entry
/// that calls a sibling recovers a concrete signature — the cross-file
/// inference the LSP's hover/inlay/codelens build on. And a hint's
/// project-wide span resolves back to the file that owns it.
#[test]
fn project_typed_check_infers_across_files() {
    let project = load(
        "typed-xfile",
        &[
            // Entry calls Utils.double with no annotations anywhere.
            ("game.fun", "let apply = (n) => Utils.double(n)\n"),
            ("utils.fun", "let double = (x: float): float => x * 2.0\n"),
        ],
    );
    let (diags, types) = project.check_with_types();
    assert!(diags.is_empty(), "clean: {diags:?}");

    // The entry's `apply` is inferred `(float) => float` across the file
    // boundary (Utils.double is annotated float→float).
    let sigs: Vec<String> = functor_lang::codelens::signatures(&project.module, &types)
        .into_iter()
        .map(|s| s.title)
        .collect();
    assert!(
        sigs.contains(&"apply : (float) => float".to_string()),
        "signatures: {sigs:?}"
    );

    // The sibling's def is canonicalized to `Utils.double` in a project.
    // Its signature lens carries a project-wide span; it must resolve to
    // utils.fun, not the entry.
    let double = functor_lang::codelens::signatures(&project.module, &types)
        .into_iter()
        .find(|s| s.title.starts_with("Utils.double :"))
        .expect("Utils.double has a signature");
    let (file, _line, _col) = project.sources.resolve(double.span.start);
    assert_eq!(
        file.path.file_name().unwrap().to_str().unwrap(),
        "utils.fun",
        "double should resolve to its own file"
    );
}

/// `load_with_overrides` replaces a *sibling* file's on-disk source with an
/// in-memory buffer (the LSP editing a non-entry file), and `file_by_path`
/// maps a path back to its project base. Disk still holds the old sibling.
#[test]
fn overrides_replace_a_sibling_buffer() {
    let scratch = Scratch::new(
        "override-sibling",
        &[
            ("game.fun", "let apply = (n) => Utils.tripled(n)\n"),
            // On disk `utils.fun` has no `tripled` — loading from disk fails.
            ("utils.fun", "let double = (x: float): float => x * 2.0\n"),
        ],
    );
    // Disk-only load: `Utils.tripled` is unresolved (load or check fails).
    let disk_clean = match functor_lang::project::load(&scratch.entry) {
        Ok(project) => project.check().is_empty(),
        Err(_) => false,
    };
    assert!(!disk_clean, "disk load should not resolve Utils.tripled");

    // Override the sibling buffer with a version that defines `tripled`.
    let mut overrides = std::collections::HashMap::new();
    overrides.insert(
        scratch.dir.join("utils.fun"),
        "let tripled = (x: float): float => x * 3.0\n".to_string(),
    );
    let project = functor_lang::project::load_with_overrides(&scratch.entry, &overrides)
        .unwrap_or_else(|e| panic!("override load: {}", e.render()));
    assert!(project.check().is_empty(), "clean with override");

    // `file_by_path` maps the sibling path to its project file (which now
    // carries the overridden source).
    let file = project
        .sources
        .file_by_path(&scratch.dir.join("utils.fun"))
        .expect("utils.fun is a project file");
    assert!(file.src.contains("tripled"), "override source is in the map");
}

/// The shipped multi-file example (`examples/hello-cubes` = game.fun +
/// pieces.fun) must keep loading as a project and checking clean, so the
/// split sample can't silently bit-rot.
#[test]
fn shipped_hello_cubes_multifile_checks_clean() {
    let entry = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("examples")
        .join("hello-cubes")
        .join("game.fun");
    let project = functor_lang::project::load(&entry)
        .unwrap_or_else(|e| panic!("hello-cubes should load: {}", e.render()));
    assert!(
        project.sources.files().iter().any(|f| f.module == "Pieces"),
        "pieces.fun loads as module Pieces"
    );
    let diags = project.check();
    assert!(diags.is_empty(), "hello-cubes checks clean: {diags:?}");
}

// ── interface files (.funi) — funi slice 2d ──────────────────────────────

/// A `.funi` gives host-implemented values real types: a sibling `.fun`'s
/// `Widget.make()` / `Widget.size(h)` type against `widget.funi` and check
/// clean.
#[test]
fn interface_file_types_externals() {
    let project = load(
        "funi-basic",
        &[
            (
                "game.fun",
                "let build = (): Widget.Handle => Widget.make()\n\
                 let area = (h: Widget.Handle): float => Widget.size(h)",
            ),
            (
                "widget.funi",
                "type Handle\n\
                 let make : () => Handle\n\
                 let size : (Handle) => float",
            ),
        ],
    );
    let diags = project.check();
    assert!(diags.is_empty(), "should check clean: {diags:?}");
}

/// An interface signature is enforced — a wrong argument type is caught, with
/// the interface's nominal type in the message.
#[test]
fn interface_signature_mismatch_is_flagged() {
    let project = load(
        "funi-mismatch",
        &[
            ("game.fun", "let bad = (): float => Widget.size(3.0)"),
            (
                "widget.funi",
                "type Handle\nlet size : (Handle) => float",
            ),
        ],
    );
    let diags = project.check();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("expected Widget.Handle"),
        "unexpected: {}",
        diags[0].message
    );
}

/// `open` brings an interface's signatures (and types) unqualified.
#[test]
fn open_brings_interface_signatures() {
    let project = load(
        "funi-open",
        &[
            ("game.fun", "open Widget\nlet f = (): float => size(make())"),
            (
                "widget.funi",
                "type Handle\n\
                 let make : () => Handle\n\
                 let size : (Handle) => float",
            ),
        ],
    );
    let diags = project.check();
    assert!(diags.is_empty(), "should check clean: {diags:?}");
}

/// A body in a `.funi` is a load error with a clear message — interface files
/// declare, not define.
#[test]
fn body_in_interface_file_is_rejected() {
    let err = load_err(
        "funi-body",
        &[
            ("game.fun", "let main = () => 1.0"),
            ("widget.funi", "let make : float = 3.0"),
        ],
    );
    assert!(
        err.contains("declare signatures, not definitions"),
        "unexpected: {err}"
    );
}

/// An interface signature may reference a type in the module that USES it (a
/// host callback typed against the app's own `Model`). Interface modules have
/// no runtime initializers, so this is NOT a real cycle (funi 2d review).
#[test]
fn interface_signature_may_reference_a_consumer_type() {
    let project = load(
        "funi-consumer-type",
        &[
            (
                "game.fun",
                "type Model = { n: float }\n\
                 let sc = (m: Model): Widget.Handle => Widget.render(m)",
            ),
            (
                "widget.funi",
                "type Handle\nlet render : (Game.Model) => Handle",
            ),
        ],
    );
    let diags = project.check();
    assert!(diags.is_empty(), "no false cycle: {diags:?}");
}

/// An interface member still resolves as an External at runtime (host-backed),
/// so a `.fun`-only project keeps running unchanged.
#[test]
fn interface_member_lowers_to_external() {
    // The game references Widget.make but never calls it; run `main`, which is
    // pure — proving the .funi presence doesn't disturb evaluation.
    let value = run_main(
        "funi-runtime",
        &[
            (
                "game.fun",
                "let unused = (): Widget.Handle => Widget.make()\n\
                 let main = () => 40.0 + 2.0",
            ),
            ("widget.funi", "type Handle\nlet make : () => Handle"),
        ],
    );
    assert_eq!(number(&value), 42.0);
}

/// Injected prelude interface modules (funi 2e) give the checker real types
/// for host externals — exempt from the protected-namespace check, since they
/// OWN those namespaces. `Scene.*` types against the injected `Scene` module.
#[test]
fn injected_prelude_types_host_externals() {
    let scratch = Scratch::new(
        "prelude-inject",
        &[(
            "game.fun",
            "let ok = () => Scene.color(1.0, 0.0, 0.0, Scene.cube())\n\
             let bad = () => Scene.color(1.0, 0.0, 0.0, 3.0)",
        )],
    );
    let prelude = [(
        "Scene".to_string(),
        "type Node\n\
         let cube : () => Node\n\
         let color : (float, float, float, Node) => Node"
            .to_string(),
    )];
    let project =
        functor_lang::project::load_with_prelude(&scratch.entry, &Default::default(), &prelude)
            .unwrap_or_else(|e| panic!("prelude load: {}", e.render()));
    let diags = project.check();
    // Only the deliberate `3.0` misuse; `Scene.cube()` → `Scene.Node` is fine.
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("expected Scene.Node"),
        "unexpected: {}",
        diags[0].message
    );
}

// ── Bundled implementation modules ──────────────────────────────────────

/// A bundled `.fun` module is real executable language code, not an
/// interface-shaped host external.
#[test]
fn bundled_implementation_module_runs_and_checks() {
    let sources = vec![(
        PathBuf::from("game.fun"),
        "let main = () => Stdlib.double(21.0)\n".to_string(),
    )];
    let bundled = [functor_lang::project::BundledModule::implementation(
        "Stdlib",
        "let double = (n: float): float => n * 2.0\n",
    )];
    let project = functor_lang::project::load_sources_with_bundled_modules(sources, &bundled)
        .unwrap_or_else(|e| panic!("bundled module should load: {}", e.render()));

    let diags = project.check();
    assert!(diags.is_empty(), "bundled module should check: {diags:?}");
    let record = functor_lang::run(&project.module, Tracing::Off)
        .unwrap_or_else(|f| panic!("bundled module should run: {}", f.error.message));
    match record.outcome {
        RunOutcome::Main(Value::Number(n)) => assert_eq!(n, 42.0),
        _ => panic!("expected a numeric main result"),
    }

    let stdlib = project
        .sources
        .files()
        .iter()
        .find(|file| file.module == "Stdlib")
        .expect("bundled source is mapped");
    assert_eq!(stdlib.path, PathBuf::from("<stdlib>/Stdlib.fun"));
    assert!(!stdlib.interface);
}

/// Engine stdlib code can depend on an injected host interface: all bundled
/// modules share the normal export/dependency graph.
#[test]
fn bundled_implementation_can_depend_on_interface() {
    let sources = vec![(
        PathBuf::from("game.fun"),
        "let unused = () => Animator.pose()\nlet main = () => 0.0\n".to_string(),
    )];
    let bundled = [
        functor_lang::project::BundledModule::interface("Anim", "type t\nlet rest : () => t\n"),
        functor_lang::project::BundledModule::implementation(
            "Animator",
            "let pose = (): Anim.t => Anim.rest()\n",
        ),
    ];
    let project = functor_lang::project::load_sources_with_bundled_modules(sources, &bundled)
        .unwrap_or_else(|e| panic!("bundled dependency should load: {}", e.render()));
    let diags = project.check();
    assert!(
        diags.is_empty(),
        "implementation should typecheck against interface: {diags:?}"
    );
}

#[test]
fn stored_closure_from_bundled_module_rebinds_across_reload() {
    let sources = || {
        vec![(
            PathBuf::from("game.fun"),
            "let main = () => Stdlib.makeAdder(3.0)\n".to_string(),
        )]
    };
    let old_bundled = [functor_lang::project::BundledModule::implementation(
        "Stdlib",
        "let makeAdder = (n) => (x) => x + n\n",
    )];
    let old = functor_lang::project::load_sources_with_bundled_modules(sources(), &old_bundled)
        .unwrap_or_else(|e| panic!("old bundled module should load: {}", e.render()));
    let record = functor_lang::run(&old.module, Tracing::Off)
        .unwrap_or_else(|f| panic!("old bundled module should run: {}", f.error.message));
    let RunOutcome::Main(stored) = record.outcome else {
        panic!("expected a stored closure");
    };

    let new_bundled = [functor_lang::project::BundledModule::implementation(
        "Stdlib",
        "let makeAdder = (n) => (x) => x + n * 10.0\n",
    )];
    let new = functor_lang::project::load_sources_with_bundled_modules(sources(), &new_bundled)
        .unwrap_or_else(|e| panic!("new bundled module should load: {}", e.render()));
    let (rebound, report) = functor_lang::rebind_value(&stored, &old.module, &new.module);
    assert_eq!(report.rebound, 1, "warnings: {:?}", report.warnings);

    let session = functor_lang::Session::load(&new.module, &mut functor_lang::NoHost)
        .unwrap_or_else(|f| panic!("new bundled module should load: {}", f.error.message));
    let result = session
        .apply(
            rebound,
            vec![Value::Number(2.0)],
            "bundled closure",
            &mut functor_lang::NoHost,
        )
        .expect("rebound bundled closure should apply");
    assert_eq!(number(&result), 32.0);
}

#[test]
fn project_file_cannot_shadow_a_bundled_module() {
    let sources = vec![
        (
            PathBuf::from("game.fun"),
            "let main = () => 0.0\n".to_string(),
        ),
        (
            PathBuf::from("animator.fun"),
            "let pose = 1.0\n".to_string(),
        ),
    ];
    let bundled = [functor_lang::project::BundledModule::implementation(
        "Animator",
        "let pose = 2.0\n",
    )];
    let err = match functor_lang::project::load_sources_with_bundled_modules(sources, &bundled) {
        Err(err) => err.render(),
        Ok(_) => panic!("project shadowing should fail"),
    };
    assert_eq!(
        err,
        "animator.fun:1:1: module name `Animator` (from animator.fun) collides with the bundled \
module namespace `Animator` — rename the file"
    );
}

#[test]
fn single_file_named_like_a_bundled_module_uses_main_label() {
    let path = PathBuf::from("animator.fun");
    let bundled = [functor_lang::project::BundledModule::implementation(
        "Animator",
        "let pose = 2.0\n",
    )];
    let project = functor_lang::project::load_single_file_with_bundled_modules(
        &path,
        "let main = () => Animator.pose\n",
        &bundled,
    )
    .unwrap_or_else(|e| panic!("single-file label should not shadow: {}", e.render()));

    assert_eq!(project.entry, "Main");
    assert!(
        project.check().is_empty(),
        "bundled module should remain usable"
    );
}

#[test]
fn duplicate_bundled_module_descriptors_are_refused() {
    let sources = vec![(
        PathBuf::from("game.fun"),
        "let main = () => 0.0\n".to_string(),
    )];
    let bundled = [
        functor_lang::project::BundledModule::implementation("Shared", "let x = 1.0\n"),
        functor_lang::project::BundledModule::interface("Shared", "let x : float\n"),
    ];
    let err = match functor_lang::project::load_sources_with_bundled_modules(sources, &bundled) {
        Err(err) => err.render(),
        Ok(_) => panic!("duplicate bundled modules should fail"),
    };
    assert_eq!(
        err,
        "<prelude>/Shared.funi:1:1: bundled module `Shared` is already supplied by \
<stdlib>/Shared.fun"
    );
}

/// Editor temp files and other non-identifier `.fun` stems (`.#game.fun`,
/// `2d.fun`) are ignored by the loader, not treated as modules — so a stray
/// temp file next to `game.fun` can't break the load (or hot reload). Their
/// (deliberately broken) contents are never parsed.
#[test]
fn non_identifier_fun_siblings_are_skipped() {
    let project = load(
        "skip-temp-siblings",
        &[
            ("game.fun", "let main = 1.0\n"),
            (".#game.fun", "$$ not valid functor-lang $$\n"),
            ("2d.fun", "also completely broken !!!\n"),
        ],
    );
    let names: Vec<String> = project
        .sources
        .files()
        .iter()
        .filter_map(|f| f.path.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();
    // The entry loads; the built-in Net module is always injected.
    assert!(names.contains(&"game.fun".to_string()), "{names:?}");
    // Neither skipped sibling contributes a module.
    assert!(!names.iter().any(|n| n == ".#game.fun"), "{names:?}");
    assert!(!names.iter().any(|n| n == "2d.fun"), "{names:?}");
}

/// A well-named sibling still loads normally alongside a skipped temp file —
/// the filter drops only the non-identifier stems, not real modules.
#[test]
fn valid_sibling_loads_beside_a_skipped_temp_file() {
    let value = run_main(
        "valid-sibling-beside-temp",
        &[
            ("game.fun", "let main = Utils.answer\n"),
            ("utils.fun", "let answer = 42.0\n"),
            (".#utils.fun", "garbage that must be ignored\n"),
        ],
    );
    assert_eq!(number(&value), 42.0);
}
