//! `frame_bench` — a headless MACRO benchmark of the interpreter's real
//! per-frame cost under the engine prelude.
//!
//! # Why this exists (vs the other two numbers)
//!
//! - The `functor-lang bench` micro-suite (`functor-lang/benches/README.md`) times language
//!   micro-ops under the plain prelude. It is the right tool for isolating a
//!   language change, but a derived estimate from it has twice misjudged what
//!   a real game pays per frame.
//! - The windowed runtime's `draw_us` telemetry inflates ~2x on sub-saturated
//!   scenes (vsync idle time plus DVFS downclocking between frames), so it
//!   cannot be trusted for perf work either.
//!
//! This harness is the honest middle: it loads a game-shaped program under the
//! REAL engine prelude ([`functor_runtime_common::functor_lang_prelude::FunctorHost`] — the
//! same `Scene.*`/`Camera3D.*`/`Frame.*` host both shells use) and calls its
//! `draw` back-to-back in a tight loop at full clock. No GL, no window, no
//! GPU — pure CPU, runnable by an agent or CI box.
//!
//! # The workload
//!
//! A hermetic, embedded `.fun` program modeled on `examples/synthwave/game.fun`'s
//! draw: a `List.grid` heightmap whose per-cell closure does a few `Math.sin`
//! calls, plus typical scene construction (sphere + quad + group + camera).
//! It is deliberately NOT the live example file — the example can change under
//! the bench. Parameterized by grid side; default sizes 20x20 / 40x40 / 56x56
//! (400 / 1600 / 3136 cells; 40x40 is synthwave's shipped resolution).
//!
//! # What it reports
//!
//! Per grid size: us/frame (min + median over a FIXED number of timed frames,
//! so every run and both sides of an A/B draw from the same sample count),
//! derived us/cell (from the min — under background load the median inflates
//! far more), and — the deterministic, future-gateable number — allocations
//! and bytes per frame via a counting `#[global_allocator]` local to this
//! binary. Wall time is noisy; alloc counts are exactly reproducible
//! run-to-run. A "frame" here mirrors the producers' render path: the `draw`
//! call plus `Frame` extraction and the retained `last_frame` clone the shells
//! perform (their GL rendering of that frame is out of scope by design).
//! Report-only: no thresholds, no CI gate (see the micro-suite README for why
//! raw perf thresholds flake on shared hardware).
//!
//! The counting allocator stays enabled during timing; its two relaxed
//! fetch-adds per allocation cost a few percent on this alloc-heavy workload,
//! identically on both sides of an A/B.
//!
//! # Run it
//!
//! ```sh
//! cargo run -q --release -p functor_runtime_common --example frame_bench
//! cargo run -q --release -p functor_runtime_common --example frame_bench -- 40 80   # grid sides
//! ```
//!
//! Always `--release` — a debug interpreter is many times slower and not
//! representative (the binary prints a loud warning if you forget). To A/B a
//! change, run on the base ref and on your branch on the same machine and
//! compare (2-3 runs each side); the alloc columns are exact, the time columns
//! carry a few percent of noise.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use functor_lang::Value;
use functor_runtime_common::functor_lang_prelude::{frame_value, FunctorHost};
use functor_runtime_common::Frame;

// --- Counting allocator (this binary only) --------------------------------
//
// Counts every allocation and its size on top of the system allocator.
// Relaxed atomics: the bench is single-threaded and only reads deltas between
// two points on the same thread. `realloc`/`alloc_zeroed` delegate to `System`
// (NOT the default alloc+copy fallback) so timing behavior matches a normal
// build; each counts as one allocation of the new size.

static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Relaxed);
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Relaxed);
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Relaxed);
        ALLOC_BYTES.fetch_add(new_size as u64, Relaxed);
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

// --- The workload ----------------------------------------------------------

/// The synthwave-shaped frame, with the grid side formatted in. Modeled on
/// `examples/synthwave/game.fun` (hermetic copy — see the module docs): the
/// same resolution-independent height field, texture bindings, and scene
/// shape (terrain heightmap + sun sphere + sky quad + first-person camera).
fn workload(side: u32) -> String {
    format!(
        r#"
let rows = {side}.0
let cols = {side}.0
let refRes = 80.0
let rowScale = refRes / rows
let colScale = refRes / cols
let terrainSize = 160.0
let scrollSpeed = 4.0

let terrainHeight = (phase: float, r: float, c: float): float =>
  let z = r * rowScale + phase in
  let x = c * colScale in
  Math.sin(z * 0.35) * 1.6
    + Math.sin(z * 0.16 + x * 0.10) * 1.1
    + Math.sin(x * 0.22) * 0.5
    + 2.0

let gridTexture = Texture.file("grid-neon.png")
let skyTexture = Texture.file("sky.png")

let init = 0.0

let tick = (m: float, dt: float, tts: float) => m

let draw = (m: float, tts: float) =>
  let phase = tts * scrollSpeed in
  let terrain =
    Scene.heightmap(List.grid((r, c) => terrainHeight(phase, r, c), rows, cols))
    |> Scene.scaleXYZ(terrainSize, 1.0, terrainSize)
    |> Scene.translate(Vec3.make(0.0, -2.0, 0.0))
    |> Scene.emissiveTexture(gridTexture) in
  let sun =
    Scene.sphere()
    |> Scene.scale(16.0)
    |> Scene.translate(Vec3.make(0.0, 9.0, 78.0))
    |> Scene.emissive(Color.rgb(1.0, 0.82, 0.6)) in
  let sky =
    Scene.quad()
    |> Scene.scaleXYZ(500.0, 280.0, 1.0)
    |> Scene.translate(Vec3.make(0.0, 60.0, 84.0))
    |> Scene.emissiveTexture(skyTexture) in
  let scene = Scene.group([sky, sun, terrain]) in
  Frame.create(
    Camera3D.firstPerson(
      Vec3.make(0.0, 5.0, -12.0),
      Angle.radians(0.0), Angle.radians(-0.05), Angle.degrees(70.0)),
    scene)
"#
    )
}

/// A text-bearing 2D frame: `lines` rows of `columns` characters drawn with
/// `Sprite.text`, plus a measured panel behind them, in a `Frame.create2D`
/// sprite pass.
///
/// This is a NEW workload, not a variant of the one above — text draws no 3D
/// scene, so the two tables are not comparable to each other. Its purpose is to
/// price glyph expansion: every visible glyph lowers to its own quad + material
/// during `Frame.create2D`, so `allocs/frame` here scales with glyph count and
/// is the number to watch when that expansion is optimized (batching is the
/// named follow-up in `docs/2d-presentation.md`).
fn text_workload(columns: u32, lines: u32) -> String {
    // A fixed, repeating ASCII line — content is irrelevant to cost as long as
    // every character is a visible glyph, which is the worst case.
    let line: String = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
        .chars()
        .cycle()
        .take(columns as usize)
        .collect();
    let rows: Vec<String> = (0..lines)
        .map(|row| {
            format!(
                "    Sprite.text(Color.rgb(0.8, 0.9, 1.0), 0.6, \"{line}\") \
                 |> Sprite.moveY({}.0 - {row}.0 * 0.7),",
                lines / 2
            )
        })
        .collect();
    format!(
        r#"
let caption = "{line}"

let init = 0.0

let tick = (m: float, dt: float, tts: float) => m

let draw = (m: float, tts: float) =>
  let box = Sprite.measure(0.6, caption) in
  Sprite.group([
    Sprite.rectangle(Color.rgb(0.08, 0.05, 0.16), box.width + 1.0, box.height + 1.0),
{}
  ])
  |> Frame.create2D(Camera2D.create(64.0, 36.0))
"#,
        rows.join("\n")
    )
}

/// A filled-shape 2D frame: `count` circles, convex polygons, and lines in a
/// `Frame.create2D` sprite pass.
///
/// Like the text workload this is a NEW workload, priced per shape. What it
/// measures is the CPU side of lowering: a circle expands to a 32-point unit ring
/// during `Frame.create2D`, which is where its allocations come from. The GPU
/// side (one draw call per shape, and the vertex re-upload that same-vertex-count
/// polygons share) is invisible here — see the note the `shapes` header prints.
fn shape_workload(count: u32) -> String {
    let shapes: Vec<String> = (0..count)
        .map(|i| {
            let t = i as f32 * 0.37;
            match i % 3 {
                0 => format!(
                    "    Sprite.circle(Color.rgb(1.0, 0.4, 0.8), {:.2}) \
                     |> Sprite.move({:.2}, {:.2}),",
                    0.4 + (i % 5) as f32 * 0.2,
                    t.cos() * 12.0,
                    t.sin() * 7.0
                ),
                1 => format!(
                    "    Sprite.polygon(Color.rgb(0.6, 1.0, 0.45), [\
                     {{ x: {:.2}, y: {:.2} }}, {{ x: {:.2}, y: {:.2} }}, \
                     {{ x: {:.2}, y: {:.2} }}]),",
                    t.cos() * 12.0,
                    t.sin() * 7.0,
                    t.cos() * 12.0 + 1.4,
                    t.sin() * 7.0,
                    t.cos() * 12.0,
                    t.sin() * 7.0 + 1.1
                ),
                _ => format!(
                    "    Sprite.line(Color.rgb(0.35, 0.95, 1.0), 0.12, \
                     {{ x: {:.2}, y: {:.2} }}, {{ x: {:.2}, y: {:.2} }}),",
                    t.cos() * 12.0,
                    t.sin() * 7.0,
                    t.cos() * 12.0 + 2.0,
                    t.sin() * 7.0 + 1.5
                ),
            }
        })
        .collect();
    format!(
        r#"
let init = 0.0

let tick = (m: float, dt: float, tts: float) => m

let draw = (m: float, tts: float) =>
  Sprite.group([
{}
  ])
  |> Frame.create2D(Camera2D.create(32.0, 18.0))
"#,
        shapes.join("\n")
    )
}

// --- The harness -----------------------------------------------------------

/// Warmup wall-clock before timing begins (caches / branch predictor).
const WARMUP: Duration = Duration::from_millis(300);
/// Timed frames per size. FIXED (not a wall-clock budget) so every run — and
/// both sides of an A/B — draws from the same number of samples; a time
/// budget would hand faster runs more chances at an anomalously low min.
const SAMPLES: usize = 200;
/// Frames the alloc counters are averaged over (they are deterministic, so
/// this only guards against a miscount, not noise).
const ALLOC_FRAMES: u64 = 5;

struct SizeResult {
    side: u32,
    cells: u64,
    min_us: f64,
    median_us: f64,
    allocs_per_frame: u64,
    bytes_per_frame: u64,
}

/// Parse + lower + load a workload under the engine prelude.
fn load_source(src: &str) -> (functor_lang::Session, Value) {
    let module = functor_lang::lower(functor_lang::parse(src).expect("workload parses"))
        .expect("workload lowers");
    let session = functor_lang::Session::load(&module, &mut FunctorHost)
        .unwrap_or_else(|f| panic!("workload load failed: {}", f.error.message));
    let model = session.global("init").expect("workload defines init");
    (session, model)
}

/// One `draw(model, tts)` frame, mirroring the producers' render path: call
/// `draw`, extract the `Frame`, and clone it into a retained slot exactly as
/// the shells' `last_frame` does — so extraction + clone cost is part of the
/// measured frame, and a non-Frame return fails loudly instead of being timed
/// as garbage. Fixed `tts` keeps the frame — and therefore the alloc counts —
/// byte-for-byte identical across iterations and runs.
fn draw_frame(session: &functor_lang::Session, model: &Value, last_frame: &mut Option<Frame>) {
    let value = session
        .call(
            "draw",
            vec![model.clone(), Value::Number(1.0)],
            &mut FunctorHost,
        )
        .unwrap_or_else(|e| panic!("draw failed: {}", e.message));
    let frame = frame_value(&value).unwrap_or_else(|| {
        panic!(
            "draw must return Frame.create(camera, scene), got {}",
            value.kind_name()
        )
    });
    *last_frame = Some(frame.clone());
    black_box(last_frame);
}

/// The measured columns for one workload source: wall time (min + median over
/// `SAMPLES` frames) and the deterministic alloc counters.
struct Measured {
    min_us: f64,
    median_us: f64,
    allocs_per_frame: u64,
    bytes_per_frame: u64,
}

fn bench_source(src: &str) -> Measured {
    let (session, model) = load_source(src);
    let mut last_frame: Option<Frame> = None;

    // Warmup.
    let warm_start = Instant::now();
    while warm_start.elapsed() < WARMUP {
        draw_frame(&session, &model, &mut last_frame);
    }

    // Allocations per frame (deterministic; averaged only as a self-check).
    let count_before = ALLOC_COUNT.load(Relaxed);
    let bytes_before = ALLOC_BYTES.load(Relaxed);
    for _ in 0..ALLOC_FRAMES {
        draw_frame(&session, &model, &mut last_frame);
    }
    let allocs_per_frame = (ALLOC_COUNT.load(Relaxed) - count_before) / ALLOC_FRAMES;
    let bytes_per_frame = (ALLOC_BYTES.load(Relaxed) - bytes_before) / ALLOC_FRAMES;

    // Timed phase: per-frame wall time over a fixed sample count (frames are
    // ms-scale, so per-call Instant reads are far above clock resolution).
    let mut samples_ns: Vec<u128> = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        draw_frame(&session, &model, &mut last_frame);
        samples_ns.push(start.elapsed().as_nanos());
    }
    samples_ns.sort_unstable();

    Measured {
        min_us: samples_ns[0] as f64 / 1_000.0,
        median_us: samples_ns[samples_ns.len() / 2] as f64 / 1_000.0,
        allocs_per_frame,
        bytes_per_frame,
    }
}

fn bench_size(side: u32) -> SizeResult {
    let m = bench_source(&workload(side));
    SizeResult {
        side,
        cells: side as u64 * side as u64,
        min_us: m.min_us,
        median_us: m.median_us,
        allocs_per_frame: m.allocs_per_frame,
        bytes_per_frame: m.bytes_per_frame,
    }
}

/// `tick` is the identity in this workload, so this is pure entry-point call
/// overhead — reported once for completeness (it does not depend on the grid).
fn bench_tick(side: u32) -> f64 {
    let (session, model) = load_source(&workload(side));
    let call = |n: u64| {
        let start = Instant::now();
        for _ in 0..n {
            let value = session
                .call(
                    "tick",
                    vec![model.clone(), Value::Number(0.016), Value::Number(1.0)],
                    &mut FunctorHost,
                )
                .unwrap_or_else(|e| panic!("tick failed: {}", e.message));
            black_box(value);
        }
        start.elapsed()
    };
    call(1_000); // warmup
    call(10_000).as_nanos() as f64 / 10_000.0 / 1_000.0 // us/call
}

/// The 2D text table: cost as a function of glyphs on screen. Reported
/// per-glyph as well as per-frame, since glyph count is the thing a game
/// controls.
fn bench_text() {
    println!("frame_bench: headless per-frame cost under the engine prelude (no GL)");
    println!("workload: Sprite.text 2D pass — a measured panel plus N lines of text");
    println!("NOTE: a NEW workload, not comparable to the synthwave table; it draws no 3D scene.");
    println!("NOTE: CPU only. This cannot see the GPU cost of one draw call per glyph.");
    println!();
    println!(
        "{:>7} {:>9} {:>15} {:>15} {:>10} {:>13} {:>12}",
        "glyphs",
        "layout",
        "us/frame(min)",
        "us/frame(med)",
        "us/glyph",
        "allocs/frame",
        "bytes/frame"
    );
    // 20 glyphs ~ a score line; 400 ~ a dense HUD; 1500 ~ a full screen of text.
    for (columns, lines) in [(20, 1), (40, 10), (60, 25)] {
        let glyphs = columns as u64 * lines as u64;
        let m = bench_source(&text_workload(columns, lines));
        println!(
            "{:>7} {:>9} {:>15.1} {:>15.1} {:>10.3} {:>13} {:>12}",
            glyphs,
            format!("{columns}x{lines}"),
            m.min_us,
            m.median_us,
            m.min_us / glyphs as f64,
            m.allocs_per_frame,
            m.bytes_per_frame,
        );
    }
}

/// The filled-shape table: cost as a function of shapes on screen.
fn bench_shapes() {
    println!("frame_bench: headless per-frame cost under the engine prelude (no GL)");
    println!("workload: Sprite.circle / polygon / line in a 2D pass (equal thirds)");
    println!("NOTE: a NEW workload, not comparable to the synthwave table; it draws no 3D scene.");
    println!("NOTE: CPU only — one draw call per shape, and the vertex re-upload that");
    println!("      same-vertex-count polygons share, are both invisible here.");
    println!();
    println!(
        "{:>7} {:>15} {:>15} {:>10} {:>13} {:>12}",
        "shapes", "us/frame(min)", "us/frame(med)", "us/shape", "allocs/frame", "bytes/frame"
    );
    for count in [12, 120, 600] {
        let m = bench_source(&shape_workload(count));
        println!(
            "{:>7} {:>15.1} {:>15.1} {:>10.3} {:>13} {:>12}",
            count,
            m.min_us,
            m.median_us,
            m.min_us / count as f64,
            m.allocs_per_frame,
            m.bytes_per_frame,
        );
    }
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("========================================================================");
        eprintln!("WARNING: debug build — the interpreter is many times slower than");
        eprintln!("release and these numbers are NOT representative. Re-run with:");
        eprintln!("  cargo run -q --release -p functor_runtime_common --example frame_bench");
        eprintln!("========================================================================");
    }

    // Optional args: grid sides (NxN). Default 20 / 40 / 56 = 400 / 1600 /
    // 3136 cells; 40 is examples/synthwave's shipped resolution.
    let args: Vec<String> = std::env::args().skip(1).collect();

    // `text` selects the 2D text workload instead (a separate table — the two
    // are not comparable to each other).
    if args.first().is_some_and(|a| a == "text") {
        bench_text();
        return;
    }
    if args.first().is_some_and(|a| a == "shapes") {
        bench_shapes();
        return;
    }

    let sides: Vec<u32> = if args.is_empty() {
        vec![20, 40, 56]
    } else {
        args.iter()
            .map(|a| {
                let side: u32 = a.parse().unwrap_or(0);
                // Scene.heightmap needs at least 2x2; List.grid caps total
                // cells at 1,000,000, so the largest square side is 1000.
                if !(2..=1000).contains(&side) {
                    eprintln!("frame_bench: expected grid sides in 2..=1000, got `{a}`");
                    std::process::exit(2);
                }
                side
            })
            .collect()
    };

    println!("frame_bench: headless per-frame cost under the engine prelude (no GL)");
    println!("workload: synthwave-shaped draw (modeled on examples/synthwave/game.fun)");
    println!();
    println!(
        "{:>7} {:>7} {:>15} {:>15} {:>9} {:>13} {:>12}",
        "cells", "grid", "us/frame(min)", "us/frame(med)", "us/cell", "allocs/frame", "bytes/frame"
    );
    for &side in &sides {
        let r = bench_size(side);
        println!(
            "{:>7} {:>7} {:>15.1} {:>15.1} {:>9.2} {:>13} {:>12}",
            r.cells,
            format!("{}x{}", r.side, r.side),
            r.min_us,
            r.median_us,
            // Derived from MIN: under background load the median inflates far
            // more, and per-cell cost is the A/B slope to trust.
            r.min_us / r.cells as f64,
            r.allocs_per_frame,
            r.bytes_per_frame,
        );
    }
    println!();
    println!(
        "tick (identity model pass-through): {:.2} us/call",
        bench_tick(sides[0])
    );
}
