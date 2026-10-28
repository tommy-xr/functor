//! `physics_memo_bench` — what a static physics world costs per frame and in
//! history, declared flat vs through `Physics.memo`.
//!
//! The workload is N fixed boxes plus one falling ball, declared by a real
//! `physics` hook under the engine prelude and driven through
//! `SteppedPhysics` (hydrate, record, reconcile, step, keyframes) — the path
//! the shells run. Per variant it reports:
//!
//! - **hook**: the interpreter evaluating `physics(model)`;
//! - **drive**: `SteppedPhysics::advance` (reconcile + rapier step + the
//!   recorder, keyframes amortized);
//! - **compared**: declarations reconcile diffed per frame (exact);
//! - **history**: heap the recorder (command log + keyframes) frees when
//!   dropped after `FRAMES` frames, via a counting `#[global_allocator]`
//!   local to this binary. Shared group `Arc`s still held by the world and
//!   the memo cache count as refcounts only.
//!
//! It asserts the memo claim: an unchanged group costs reconcile 0 per-body
//! compares, and the hook costs the same at 1,500 bodies as at 15. Timings are
//! desktop; the Quest estimate is ×4.
//!
//! ```sh
//! cargo run -q --release -p functor_runtime_common --example physics_memo_bench
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI64, Ordering::Relaxed};
use std::time::Instant;

use functor_lang::Value;
use functor_runtime_common::functor_lang_prelude::{physics_scene_value, FunctorHost};
use functor_runtime_common::physics::{
    remove_world, with_world, SteppedPhysics, DEFAULT_WORLD, FIXED_DT,
};

/// Live heap bytes: allocations minus frees.
static LIVE: AtomicI64 = AtomicI64::new(0);

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as i64, Relaxed);
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as i64, Relaxed);
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(new_size as i64 - layout.size() as i64, Relaxed);
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as i64, Relaxed);
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Past the recorder's 900-frame window, so history is at its steady size.
const FRAMES: u32 = 1000;

const SOURCE: &str = "
let tile = (i) => Physics.fixed(Physics.tag(Text.concat(\"s\", Text.fromFloat(i))), Physics.box(1.0, 1.0, 1.0))
  |> Physics.at(Vec3.make(Math.mod(i, 30.0) - 15.0, 0.0, (i - Math.mod(i, 30.0)) / 30.0 - 25.0))
let statics = (n) => List.map(tile, List.range(n))
let ball = Physics.dynamic(Physics.tag(\"ball\"), Physics.sphere(0.25)) |> Physics.at(Vec3.make(0.5, 2.0, 0.5))
let gravity = Vec3.make(0.0, -9.81, 0.0)
let flat = (n) => Physics.scene(gravity, List.append(statics(n), [ball]))
let memo = (n) => Physics.scene(gravity, [Physics.memo(\"statics\", n, statics), ball])
";

struct Run {
    hook_ms: f64,
    drive_ms: f64,
    compared: usize,
    history_mb: f64,
}

/// `FRAMES` frames of `hook(n)` through a fresh recorded world.
fn run(hook: &str, n: usize) -> Run {
    let module = functor_lang::lower(functor_lang::parse(SOURCE).expect("parse")).expect("lower");
    let session = functor_lang::Session::load(&module, &mut FunctorHost)
        .unwrap_or_else(|f| panic!("load: {}", f.error.message));
    remove_world(DEFAULT_WORLD);
    let mut physics = SteppedPhysics::new();
    let declare = |session: &functor_lang::Session| {
        session
            .call(hook, vec![Value::Number(n as f64)], &mut FunctorHost)
            .unwrap_or_else(|e| panic!("{hook}: {}", e.message))
    };

    let value = declare(&session);
    physics.advance(physics_scene_value(&value).unwrap(), FIXED_DT);
    drop(value);

    let (mut hook_s, mut drive_s, mut compared) = (0.0, 0.0, 0);
    for _ in 1..FRAMES {
        let start = Instant::now();
        let value = declare(&session);
        let declared = Instant::now();
        physics.advance(physics_scene_value(&value).unwrap(), FIXED_DT);
        let advanced = Instant::now();
        hook_s += (declared - start).as_secs_f64();
        drive_s += (advanced - declared).as_secs_f64();
        compared += with_world(DEFAULT_WORLD, |w| w.reconcile_stats().compared).unwrap();
    }
    let live_before = LIVE.load(Relaxed);
    drop(physics);
    let history = live_before - LIVE.load(Relaxed);
    remove_world(DEFAULT_WORLD);
    let frames = (FRAMES - 1) as f64;
    Run {
        hook_ms: hook_s * 1e3 / frames,
        drive_ms: drive_s * 1e3 / frames,
        compared: compared / (FRAMES - 1) as usize,
        history_mb: history as f64 / 1e6,
    }
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("WARNING: debug build — run with --release for representative numbers");
    }
    println!("{FRAMES} frames, 1 dynamic ball + N fixed boxes; desktop ms/frame (Quest est. ×4)");
    println!(
        "{:<12} {:>6} {:>16} {:>16} {:>9} {:>12}",
        "variant", "N", "hook", "drive", "compared", "history MB"
    );
    let mut results = Vec::new();
    for hook in ["flat", "memo"] {
        for n in [15, 1500] {
            let r = run(hook, n);
            println!(
                "{:<12} {:>6} {:>7.3} ({:>6.3}) {:>7.3} ({:>6.3}) {:>9} {:>12.1}",
                hook,
                n,
                r.hook_ms,
                r.hook_ms * 4.0,
                r.drive_ms,
                r.drive_ms * 4.0,
                r.compared,
                r.history_mb
            );
            results.push(r);
        }
    }
    let (memo_small, memo_large) = (&results[2], &results[3]);
    assert_eq!(
        memo_large.compared, 1,
        "an unchanged group must skip its bodies"
    );
    assert!(
        memo_large.hook_ms < memo_small.hook_ms * 3.0 + 0.01,
        "the memo hook grows with the group: {:.4} ms at 15, {:.4} at 1,500",
        memo_small.hook_ms,
        memo_large.hook_ms
    );
    println!("ok: an unchanged 1,500-body group reconciles 0 bodies and the hook is O(1)");
}
