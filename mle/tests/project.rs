//! B8 part 1 verification (docs/mle.md): multi-file projects — file =
//! module, qualified-by-default access (values, constructors in expressions
//! AND patterns, type annotations), `open`, eager whole-program loading,
//! cycle refusal, protected namespaces, and cross-file hot-reload rebind.

use std::fs;
use std::path::{Path, PathBuf};

use mle::value::Value;
use mle::{RunOutcome, Tracing};

/// Write `files` into a fresh scratch directory and return it. The first
/// file is the entry.
struct Scratch {
    dir: PathBuf,
    entry: PathBuf,
}

impl Scratch {
    fn new(name: &str, files: &[(&str, &str)]) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("mle-project-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        for (file, src) in files {
            fs::write(dir.join(file), src).expect("write scratch file");
        }
        let entry = dir.join(files[0].0);
        Scratch { dir, entry }
    }

    fn load(&self) -> Result<mle::project::Project, mle::project::ProjectError> {
        mle::project::load(&self.entry)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Load a scratch project, expecting success.
fn load(name: &str, files: &[(&str, &str)]) -> mle::project::Project {
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
    let rendered = err.render();
    let prefix = format!("{}/", scratch.dir.display());
    rendered.replace(&prefix, "")
}

/// Run a scratch project's `main`.
fn run_main(name: &str, files: &[(&str, &str)]) -> Value {
    let project = load(name, files);
    let record = mle::run(&project.module, Tracing::Off).unwrap_or_else(|failure| {
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
        .join("game.mle");
    let project = mle::project::load(&entry).unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert!(diags.is_empty(), "fixture should check clean: {diags:?}");
    let record =
        mle::run(&project.module, Tracing::Off).unwrap_or_else(|f| panic!("{}", f.error.message));
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
                "game.mle",
                "let main = () =>\n\
                 match Util.wrap(4.0) with\n\
                 | Util.Wrapped(n) => n + Util.base\n",
            ),
            (
                "util.mle",
                "type Carton = | Wrapped(value: Float)\n\
                 let base = 10.0\n\
                 let wrap = (n: Float): Carton => Wrapped(n)\n",
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
                "game.mle",
                "let main = () =>\n\
                 [1.0, 2.0] |> List.map(Util.Wrapped) |> List.map(Util.unwrap) |> List.maximum\n",
            ),
            (
                "util.mle",
                "type Carton = | Wrapped(value: Float)\n\
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
            ("game.mle", "let base = 32.0\n"),
            ("util.mle", "let above = (x) => x + Game.base\n"),
        ],
    );
    let session = mle::Session::load(&project.module, &mut mle::NoHost)
        .unwrap_or_else(|f| panic!("session should load: {}", f.error.message));
    let result = session
        .call("Util.above", vec![Value::Number(10.0)], &mut mle::NoHost)
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
                "game.mle",
                "let start = Config.speed * 2.0\nlet main = () => start\n",
            ),
            ("config.mle", "let speed = 21.0\n"),
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
            ("game.mle", "let main = () => Util\n"),
            ("util.mle", "let x = 1.0\n"),
        ],
    );
    assert_eq!(
        err,
        "game.mle:1:18: unknown name `Util` — `Util` is a module; reference a member (`Util.name`)"
    );
}

#[test]
fn unknown_member_is_a_load_error() {
    let err = load_err(
        "unknown-member",
        &[
            ("game.mle", "let main = () => Util.nope(1.0)\n"),
            ("util.mle", "let x = 1.0\n"),
        ],
    );
    assert_eq!(err, "game.mle:1:18: module `Util` has no `nope`");
}

#[test]
fn unknown_member_type_is_a_load_error() {
    let err = load_err(
        "unknown-type",
        &[
            ("game.mle", "let f = (x: Util.Nope) => x\n"),
            ("util.mle", "let x = 1.0\n"),
        ],
    );
    assert_eq!(err, "game.mle:1:13: module `Util` has no type `Nope`");
}

#[test]
fn unknown_ctor_in_pattern_is_a_load_error() {
    let err = load_err(
        "unknown-pattern-ctor",
        &[
            (
                "game.mle",
                "let f = (x) => match x with | Util.Nope => 1.0\n",
            ),
            ("util.mle", "let x = 1.0\n"),
        ],
    );
    assert_eq!(
        err,
        "game.mle:1:31: module `Util` has no constructor `Nope`"
    );
}

/// A qualified name whose head is NOT a module stays the External seam
/// (builtins keep working; unknown ones stay runtime errors, as before).
#[test]
fn non_module_qualified_names_stay_external() {
    let value = run_main(
        "external-seam",
        &[
            ("game.mle", "let main = () => Math.clamp01(3.0) + Util.x\n"),
            ("util.mle", "let x = 1.0\n"),
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
                "game.mle",
                "open Util\n\
                 let grab = (c: Carton): Float => match c with | Wrapped(n) => n\n\
                 let main = () => grab(Wrapped(base))\n",
            ),
            (
                "util.mle",
                "type Carton = | Wrapped(value: Float)\nlet base = 42.0\n",
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
            ("game.mle", "open Util\nlet base = 1.0\n"),
            ("util.mle", "let base = 2.0\n"),
        ],
    );
    assert_eq!(
        err,
        "game.mle:1:1: open Util: `base` collides with this module's own `base` — qualify uses \
as `Util.base` instead of opening"
    );
}

#[test]
fn open_collision_between_opens_names_both_modules() {
    let err = load_err(
        "open-open-collision",
        &[
            ("game.mle", "open Alpha\nopen Beta\nlet main = () => 0.0\n"),
            ("alpha.mle", "let shared = 1.0\n"),
            ("beta.mle", "let shared = 2.0\n"),
        ],
    );
    assert_eq!(
        err,
        "game.mle:2:1: open Beta: `shared` is already in scope from `open Alpha` — qualify uses \
(`Alpha.shared` / `Beta.shared`)"
    );
}

#[test]
fn open_type_collision_is_an_error() {
    let err = load_err(
        "open-type-collision",
        &[
            (
                "game.mle",
                "open Util\ntype Carton = { x: Float }\nlet main = () => 0.0\n",
            ),
            ("util.mle", "type Carton = | Wrapped(value: Float)\n"),
        ],
    );
    assert_eq!(
        err,
        "game.mle:1:1: open Util: type `Carton` collides with this module's own `Carton` — \
qualify uses as `Util.Carton` instead of opening"
    );
}

#[test]
fn open_unknown_module_is_an_error() {
    let err = load_err(
        "open-unknown",
        &[("game.mle", "open Nowhere\nlet main = () => 0.0\n")],
    );
    assert_eq!(
        err,
        "game.mle:1:1: unknown module `Nowhere` — modules are the sibling `.mle` files next to \
the entry"
    );
}

#[test]
fn open_self_is_an_error() {
    let err = load_err(
        "open-self",
        &[("game.mle", "open Game\nlet main = () => 0.0\n")],
    );
    assert_eq!(
        err,
        "game.mle:1:1: `open Game` in module `Game` itself — a module's own names are already \
in scope"
    );
}

/// `open` is contextual: it stays a perfectly good binding name.
#[test]
fn open_remains_usable_as_a_name() {
    let value = run_main(
        "open-as-name",
        &[(
            "game.mle",
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
            ("game.mle", "let a = () => Util.b()\n"),
            ("util.mle", "let b = () => Game.a()\n"),
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
            ("game.mle", "open Util\nlet a = 1.0\n"),
            ("util.mle", "open Game\nlet b = 2.0\n"),
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
            ("game.mle", "let main = () => 0.0\n"),
            ("scene.mle", "let cube = 1.0\n"),
        ],
    );
    assert_eq!(
        err,
        "scene.mle:1:1: module name `Scene` (from scene.mle) collides with the builtin/prelude \
namespace `Scene` — rename the file"
    );
}

#[test]
fn non_identifier_file_stems_are_refused() {
    let err = load_err(
        "bad-stem",
        &[
            ("game.mle", "let main = () => 0.0\n"),
            ("my-utils.mle", "let x = 1.0\n"),
        ],
    );
    assert!(
        err.contains("cannot derive a module name from `my-utils.mle`"),
        "unexpected error: {err}"
    );
}

/// In single-file (non-project) lowering, `open` is an unknown module —
/// the honest answer for the LSP's per-file view too.
#[test]
fn open_outside_a_project_is_an_error() {
    let program = mle::parse("open Util\nlet x = 1.0\n").expect("parses");
    let err = mle::lower(program).expect_err("should not lower");
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
            ("game.mle", "let main = () => 0.0\n"),
            (
                "util.mle",
                "// an unreferenced module with a type error\nlet bad = (a: Float): Float => a + \"one\"\n",
            ),
        ],
    );
    let project = scratch.load().unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
    let rendered = project
        .sources
        .render(diags[0].span.start, &diags[0].message);
    let rendered = rendered.replace(&format!("{}/", scratch.dir.display()), "");
    assert_eq!(err_line(&rendered), "util.mle:2:36");
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
                "game.mle",
                "open Boxes\n\
                 let good = (b: Box<Float>): Float =>\n\
                 match b with | Full(v) => v + 1.0 | Empty => 0.0\n\
                 let bad = () => good(Full(\"nope\"))\n",
            ),
            ("boxes.mle", "type Box<v> = | Full(value: v) | Empty\n"),
        ],
    );
    let project = scratch.load().unwrap_or_else(|e| panic!("{}", e.render()));
    let diags = project.check();
    assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
    assert!(
        diags[0]
            .message
            .contains("expected Boxes.Box<Float>, got Boxes.Box<String>"),
        "unexpected diagnostic: {}",
        diags[0].message
    );
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
                "game.mle",
                "type Position = { x: Float, y: Float }
                 let p = { x: 1.0, y: 2.0 }
let main = () => p.x
",
            ),
            // Same field shape, never referenced, never opened.
            (
                "debug.mle",
                "type Point = { x: Float, y: Float }
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
                "game.mle",
                "open Vec
let f = () => { x: 1.0, y: 2.0 }
let bad = f() + 1.0
",
            ),
            (
                "vec.mle",
                "type V2 = { x: Float, y: Float }
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
                "game.mle",
                "let f = () => { x: 1.0, y: 2.0 }
let bad = f() + 1.0
",
            ),
            (
                "vec.mle",
                "type V2 = { x: Float, y: Float }
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
    let src = "type Shape = | Circle(radius: Float) | Point\n\
               let area = (s: Shape): Float =>\n\
               match s with | Circle(r) => 3.14 * r * r | Point => 0.0\n\
               let main = () => area(Circle(2.0))\n";
    let project = load("single-file", &[("game.mle", src)]);
    let plain = mle::lower(mle::parse(src).expect("parses")).expect("lowers");

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
            ("game.mle", game),
            ("util.mle", "let makeSpring = (k) => (x) => x * k\n"),
        ],
    );
    let record = mle::run(&old.module, Tracing::Off)
        .unwrap_or_else(|f| panic!("v1 runs: {}", f.error.message));
    let stored = match record.outcome {
        RunOutcome::Main(value) => value,
        RunOutcome::Bindings(_) => panic!("expected a closure"),
    };

    // Edit the SIBLING file: the spring gains a +1 offset.
    let new = load(
        "rebind-new",
        &[
            ("game.mle", game),
            ("util.mle", "let makeSpring = (k) => (x) => x * k + 1.0\n"),
        ],
    );
    let (rebound, report) = mle::rebind_value(&stored, &old.module, &new.module);
    assert_eq!(report.rebound, 1, "warnings: {:?}", report.warnings);

    let session = mle::Session::load(&new.module, &mut mle::NoHost)
        .unwrap_or_else(|f| panic!("v2 session: {}", f.error.message));
    let result = session
        .apply(
            rebound,
            vec![Value::Number(2.0)],
            "spring",
            &mut mle::NoHost,
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
                "game.mle",
                "let main = () => (Alpha.make(1.0), Beta.make(1.0))\n".to_string(),
            ),
            ("alpha.mle", format!("let make = (k) => (x) => {a_body}\n")),
            ("beta.mle", "let make = (k) => (x) => x - k\n".to_string()),
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
    let record = mle::run(&old.module, Tracing::Off)
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
    let session = mle::Session::load(&new.module, &mut mle::NoHost)
        .unwrap_or_else(|f| panic!("v2 session: {}", f.error.message));

    let (alpha, report) = mle::rebind_value(&pair[0], &old.module, &new.module);
    assert_eq!(report.rebound, 1, "warnings: {:?}", report.warnings);
    let result = session
        .apply(alpha, vec![Value::Number(2.0)], "alpha", &mut mle::NoHost)
        .expect("apply alpha");
    assert_eq!(number(&result), 12.0); // new body: 2 + 1*10

    let (beta, report) = mle::rebind_value(&pair[1], &old.module, &new.module);
    assert_eq!(report.rebound, 1, "warnings: {:?}", report.warnings);
    let result = session
        .apply(beta, vec![Value::Number(2.0)], "beta", &mut mle::NoHost)
        .expect("apply beta");
    assert_eq!(number(&result), 1.0); // beta unchanged: 2 - 1
}
