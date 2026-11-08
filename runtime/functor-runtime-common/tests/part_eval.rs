//! `part_eval`: a part function evaluated headlessly with param records.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

use functor_runtime_common::part_eval::{parse_params, Evaluator, Params};

/// Counts each thread's live heap bytes, for the bounded-memory test (per
/// thread, so tests running alongside don't skew it).
struct Counting;

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
}

fn count(delta: isize) {
    let _ = LIVE.try_with(|live| live.set(live.get() + delta));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size() as isize);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count(-(layout.size() as isize));
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

// `part` stays a Solid (the default viewer and plain export use it).
const PLATE: &str = "\
let defaults = { w: 40.0, d: 30.0, h: 4.0, hole: 5.0 }
let plate = (p) =>
  Solid.box(p.w, p.d, p.h) |> Solid.subtract(Solid.cylinder(p.hole, p.h * 2.0))
let part = plate(defaults)
let block = (p) => Solid.box(p.w, p.d, p.h)
let notSolid = (p) => p.w
let reads = (p) => if File.exists(Path.of(\"x.txt\")) then Solid.box(p.w, 1.0, 1.0) else Solid.box(1.0, 1.0, 1.0)
";

/// A one-file project holding `src`, kept until the guard drops.
fn project(src: &str) -> (tempfile::TempDir, Evaluator) {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("part.fun");
    std::fs::write(&entry, src).unwrap();
    let evaluator = Evaluator::load(&entry).unwrap_or_else(|e| panic!("{e}"));
    (dir, evaluator)
}

fn params(json: &str) -> Params {
    parse_params(json).unwrap()
}

fn stl(mesh: &functor_solid::Mesh) -> Vec<u8> {
    let mut out = Vec::new();
    functor_solid::stl::write_binary(mesh, &mut out).unwrap();
    out
}

#[test]
fn identical_params_give_byte_identical_meshes() {
    let p = params(r#"{"w": 41, "d": 30, "h": 4, "hole": 5}"#);
    let (_dir, a) = project(PLATE);
    let (_dir2, b) = project(PLATE);
    let first = stl(&a.evaluate("plate", &p).unwrap());
    assert_eq!(first, stl(&a.evaluate("plate", &p).unwrap()));
    assert_eq!(first, stl(&b.evaluate("plate", &p).unwrap()));
}

/// `--params` equals editing the numbers into the source.
#[test]
fn params_match_an_edited_source() {
    let (_dir, evaluator) = project(PLATE);
    let mesh = evaluator
        .evaluate("plate", &params(r#"{"w": 41, "d": 30, "h": 4, "hole": 5}"#))
        .unwrap();
    let edited = "let part = Solid.box(41.0, 30.0, 4.0) |> Solid.subtract(Solid.cylinder(5.0, 8.0))";
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("part.fun");
    std::fs::write(&entry, edited).unwrap();
    let project = functor_lang::project::load_with_bundled_modules(
        &entry,
        &Default::default(),
        &functor_prelude::bundled_modules(),
    )
    .unwrap_or_else(|e| panic!("{}", e.render()));
    let session = functor_lang::Session::load(
        &project.module,
        &mut functor_runtime_common::functor_lang_prelude::FunctorHost,
    )
    .unwrap_or_else(|f| panic!("{}", f.error.message));
    let solid =
        functor_runtime_common::functor_lang_prelude::solid::solid_global(&session, "part").unwrap();
    assert_eq!(stl(&solid.mesh().unwrap()), stl(&mesh));
}

#[test]
fn a_param_changes_the_mesh() {
    let (_dir, evaluator) = project(PLATE);
    let volume = |w: f64| {
        evaluator
            .evaluate("block", &params(&format!(r#"{{"w": {w}, "d": 30, "h": 4}}"#)))
            .unwrap()
            .volume()
    };
    assert!((volume(40.0) - 4800.0).abs() < 1e-6);
    assert!((volume(50.0) - 6000.0).abs() < 1e-6);
}

#[test]
fn bad_params_are_clear_diagnostics() {
    let (_dir, evaluator) = project(PLATE);
    let error = |function: &str, json: &str| {
        evaluator
            .evaluate(function, &params(json))
            .unwrap_err()
            .to_string()
    };
    let missing = error("block", r#"{"w": 1, "d": 1}"#);
    assert!(missing.contains("part.fun:5:") && missing.contains("no field `h`"), "{missing}");
    let mut nan = params(r#"{"w": 1, "d": 1}"#);
    nan.insert("h".into(), f64::NAN);
    let nan = evaluator.evaluate("block", &nan).unwrap_err().to_string();
    assert_eq!(nan, "param `h` must be a finite number, got NaN");
    assert_eq!(
        error("notSolid", r#"{"w": 1}"#),
        "`notSolid` must return a Solid, got a number"
    );
    let reads = error("reads", r#"{"w": 1}"#);
    assert!(reads.contains("read file state through `File.exists`"), "{reads}");
    assert!(error("nope", "{}").contains("no top-level `let nope`"));

    let parse = |json: &str| parse_params(json).unwrap_err().to_string();
    assert!(parse("[1, 2]").starts_with("params must be a JSON object of numbers"));
    assert_eq!(parse(r#"{"w": "wide"}"#), "param `w` must be a number, got \"wide\"");
    assert!(parse("{").starts_with("params: "));
}

#[test]
fn a_check_error_fails_load() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("part.fun");
    std::fs::write(&entry, "let part = (p) => Solid.box(p.w)\n").unwrap();
    let error = Evaluator::load(&entry).err().unwrap().to_string();
    assert!(error.contains("part.fun:1:"), "{error}");
}

/// Evaluate and mesh `function` 10,000 times; assert the heap stays flat and
/// print throughput, split into evaluation and meshing.
fn ten_thousand(function: &str) {
    let (_dir, evaluator) = project(PLATE);
    let sample = |i: usize| params(&format!(r#"{{"w": {}, "d": 30, "h": 4, "hole": 5}}"#, 40 + i % 7));
    let (mut eval, mut mesh) = (Duration::ZERO, Duration::ZERO);
    let mut run = |range: std::ops::Range<usize>| {
        for i in range {
            let p = sample(i);
            let start = Instant::now();
            let solid = evaluator.solid(function, &p).unwrap();
            let meshed = Instant::now();
            assert!(!solid.mesh().unwrap().triangles.is_empty());
            eval += meshed - start;
            mesh += meshed.elapsed();
        }
    };
    run(0..100);
    let warm = LIVE.with(Cell::get);
    run(100..10_000);
    let grown = LIVE.with(Cell::get) - warm;
    let n = 10_000.0;
    println!(
        "10k `{function}`: {:.0} evals/s, eval {:.1} µs, mesh {:.1} µs, heap +{grown} B",
        n / (eval + mesh).as_secs_f64(),
        eval.as_secs_f64() * 1e6 / n,
        mesh.as_secs_f64() * 1e6 / n,
    );
    // A leak of even 100 B per evaluation would be ~1 MB.
    assert!(grown < 256 * 1024, "heap grew {grown} B over 9,900 evaluations");
}

#[test]
fn ten_thousand_evaluations_stay_bounded() {
    ten_thousand("block");
}

/// The boolean part: ~1 min unoptimized, so opt in with
/// `cargo test --release -p functor_runtime_common --test part_eval -- --ignored --nocapture`.
#[test]
#[ignore]
fn ten_thousand_boolean_evaluations_stay_bounded() {
    ten_thousand("plate");
}
