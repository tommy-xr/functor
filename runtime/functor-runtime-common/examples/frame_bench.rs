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
//! same `Scene.*`/`Camera.*`/`Frame.*` host both shells use) and calls its
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
//! Per grid size: us/frame (min + median over the timed iterations), derived
//! us/cell (from the min — under background load the median inflates but the
//! min doesn't), and — the deterministic, future-gateable number — allocations and
//! bytes per `draw` via a counting `#[global_allocator]` local to this binary.
//! Wall time is noisy; alloc counts are exactly reproducible run-to-run.
//! Report-only: no thresholds, no CI gate (see the micro-suite README for why
//! raw perf thresholds flake on shared hardware).
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
use functor_runtime_common::functor_lang_prelude::FunctorHost;

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
    |> Scene.translate(0.0, -2.0, 0.0)
    |> Scene.emissiveTexture(gridTexture) in
  let sun =
    Scene.sphere()
    |> Scene.scale(16.0)
    |> Scene.translate(0.0, 9.0, 78.0)
    |> Scene.emissive(1.0, 0.82, 0.6) in
  let sky =
    Scene.quad()
    |> Scene.scaleXYZ(500.0, 280.0, 1.0)
    |> Scene.translate(0.0, 60.0, 84.0)
    |> Scene.emissiveTexture(skyTexture) in
  let scene = Scene.group([sky, sun, terrain]) in
  Frame.create(
    Camera.firstPerson(
      0.0, 5.0, -12.0,
      Angle.radians(0.0), Angle.radians(-0.05), Angle.degrees(70.0)),
    scene)
"#
    )
}

// --- The harness -----------------------------------------------------------

/// Warmup wall-clock before timing begins (caches / branch predictor).
const WARMUP: Duration = Duration::from_millis(300);
/// Timed-phase floor: keep drawing until this much wall-clock AND
/// [`MIN_ITERS`] frames have accumulated, so medians are stable.
const MIN_TIMED: Duration = Duration::from_millis(1500);
const MIN_ITERS: usize = 50;
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

/// Parse + lower + load the workload at `side` under the engine prelude.
fn load_session(side: u32) -> (functor_lang::Session, Value) {
    let src = workload(side);
    let module = functor_lang::lower(functor_lang::parse(&src).expect("workload parses"))
        .expect("workload lowers");
    let session = functor_lang::Session::load(&module, &mut FunctorHost)
        .unwrap_or_else(|f| panic!("workload load failed: {}", f.error.message));
    let model = session.global("init").expect("workload defines init");
    (session, model)
}

/// One `draw(model, tts)` frame. Fixed `tts` keeps the frame — and therefore
/// the alloc counts — byte-for-byte identical across iterations and runs.
fn draw_frame(session: &functor_lang::Session, model: &Value) {
    let value = session
        .call(
            "draw",
            vec![model.clone(), Value::Number(1.0)],
            &mut FunctorHost,
        )
        .unwrap_or_else(|e| panic!("draw failed: {}", e.message));
    black_box(value);
}

fn bench_size(side: u32) -> SizeResult {
    let (session, model) = load_session(side);

    // Warmup.
    let warm_start = Instant::now();
    while warm_start.elapsed() < WARMUP {
        draw_frame(&session, &model);
    }

    // Allocations per frame (deterministic; averaged only as a self-check).
    let count_before = ALLOC_COUNT.load(Relaxed);
    let bytes_before = ALLOC_BYTES.load(Relaxed);
    for _ in 0..ALLOC_FRAMES {
        draw_frame(&session, &model);
    }
    let allocs_per_frame = (ALLOC_COUNT.load(Relaxed) - count_before) / ALLOC_FRAMES;
    let bytes_per_frame = (ALLOC_BYTES.load(Relaxed) - bytes_before) / ALLOC_FRAMES;

    // Timed phase: per-frame wall time (frames are ms-scale, so per-call
    // Instant reads are far above clock resolution).
    let mut samples_ns: Vec<u128> = Vec::new();
    let timed_start = Instant::now();
    while timed_start.elapsed() < MIN_TIMED || samples_ns.len() < MIN_ITERS {
        let start = Instant::now();
        draw_frame(&session, &model);
        samples_ns.push(start.elapsed().as_nanos());
    }
    samples_ns.sort_unstable();

    let cells = side as u64 * side as u64;
    SizeResult {
        side,
        cells,
        min_us: samples_ns[0] as f64 / 1_000.0,
        median_us: samples_ns[samples_ns.len() / 2] as f64 / 1_000.0,
        allocs_per_frame,
        bytes_per_frame,
    }
}

/// `tick` is the identity in this workload, so this is pure entry-point call
/// overhead — reported once for completeness (it does not depend on the grid).
fn bench_tick(side: u32) -> f64 {
    let (session, model) = load_session(side);
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
    let sides: Vec<u32> = if args.is_empty() {
        vec![20, 40, 56]
    } else {
        args.iter()
            .map(|a| {
                let side: u32 = a.parse().unwrap_or(0);
                if side < 2 {
                    // Scene.heightmap needs at least a 2x2 grid.
                    eprintln!("frame_bench: expected grid sides (integers >= 2), got `{a}`");
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
            // Derived from MIN: under background load the median inflates but
            // the min doesn't, and per-cell cost is the A/B slope to trust.
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
