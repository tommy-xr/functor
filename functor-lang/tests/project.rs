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
fn non_identifier_file_stems_are_refused() {
    let err = load_err(
        "bad-stem",
        &[
            ("game.fun", "let main = () => 0.0\n"),
            ("my-utils.fun", "let x = 1.0\n"),
        ],
    );
    assert!(
        err.contains("cannot derive a module name from `my-utils.fun`"),
        "unexpected error: {err}"
    );
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
fn single_file_project_adds_only_the_builtin_net_module() {
    // A project always includes the built-in `Net` prelude module (so any
    // game can `match ev with | Net.Connected(id) => …`), so its merged IR
    // is plain lowering's defs/types PLUS Net's — nothing else changes.
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
    // The ONLY additions are the Net module's (canonicalized `Net.NetEvent`
    // and `Net.HttpResponse`).
    assert!(
        proj_types.contains(&"Net.NetEvent") && proj_types.contains(&"Net.HttpResponse"),
        "the built-in Net module must be injected: {proj_types:?}"
    );
    assert_eq!(
        proj_types.len(),
        plain.types.len() + 2,
        "no types beyond the entry's + Net.NetEvent + Net.HttpResponse"
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
