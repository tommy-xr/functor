//! Persistent `Map` bench: per-op cost at 1.5k / 6.8k entries, and the
//! memory a retained history of map versions holds. Exits non-zero when a
//! budget is missed (inserts > 1 µs, `hist-map` history > 35 MB).
//!
//! ```sh
//! cargo run -q --release -p functor_runtime_common --example map_bench
//! ```
//!
//! Always `--release`. Timings are min-of-5 with the same fold's no-op
//! floor subtracted; memory is live heap bytes (a counting allocator), so it
//! is deterministic and excludes the process baseline.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};
use std::time::Instant;

use functor_runtime_common::functor_lang_prelude::FunctorHost;

static LIVE: AtomicIsize = AtomicIsize::new(0);

struct LiveBytes;

unsafe impl GlobalAlloc for LiveBytes {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Relaxed);
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: LiveBytes = LiveBytes;

const OPS: usize = 20_000;
const INSERT_BUDGET_US: f64 = 1.0;
const HISTORY_BUDGET_MB: f64 = 35.0;

/// Per-op workloads: `OPS` folds of an insert / get / no-op over a table.
fn ops_source() -> String {
    let mut src = String::new();
    for n in [1500, 6800] {
        src += &format!(
            "let t{n} = Map.fromList(List.map((i) => (i, i), List.range({n}.0)))\n\
             let insert{n} = () => List.range({OPS}.0)\n\
               |> List.fold((m, k) => Map.insert(Math.mod(k * 7.0, {n}.0), k, m), t{n})\n\
             let get{n} = () => List.range({OPS}.0)\n\
               |> List.fold((a, k) => match Map.get(Math.mod(k * 7.0, {n}.0), t{n}) with\n\
                 | Option.Some(v) => a + v\n\
                 | Option.None => a, 0.0)\n\
             let floor{n} = () => List.range({OPS}.0)\n\
               |> List.fold((a, k) => a + Math.mod(k * 7.0, {n}.0), 0.0)\n\
             let values{n} = () => List.range({}.0)\n\
               |> List.fold((a, _) => a + List.length(Map.values(t{n})), 0.0)\n",
            OPS / n,
        );
    }
    src
}

/// The gap bench's `hist-map`: `n` records keyed by id, then 900 frames of
/// 20 replacing writes, every version kept (as the timeline ring does).
fn history_source(n: usize) -> String {
    format!(
        "type obj = {{ id: float, pos: Vec3.t, vel: Vec3.t, hp: float, flags: float, script: float }}\n\
         let initObj = (i: float): obj =>\n\
           {{ id: i, pos: Vec3.make(i, 0.0, 1.0), vel: Vec3.make(0.01, 0.0, 0.0), hp: 100.0, flags: 0.0, script: Math.mod(i, 5.0) }}\n\
         let w0 = Map.fromList(List.map((o: obj) => (o.id, o), List.map(initObj, List.range({n}.0))))\n\
         let hist = List.fold((h, k) => match List.head(h) with\n\
             | Option.Some(w) => List.append([List.fold((m, j) => Map.insert(Math.mod(k * 13.0 + j * 7.0, {n}.0), initObj(j), m), w, List.range(20.0))], h)\n\
             | Option.None => h, [w0], List.range(900.0))\n"
    )
}

/// Load `src` as a game: the engine's bundled modules plus the host.
fn load(src: &str) -> functor_lang::Session {
    let project = functor_lang::project::load_sources_with_bundled_modules(
        vec![(std::path::PathBuf::from("bench.fun"), src.to_string())],
        &functor_prelude::bundled_modules(),
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    match functor_lang::Session::load(&project.module, &mut FunctorHost) {
        Ok(session) => session,
        Err(failure) => panic!("load failed: {}", failure.error.message),
    }
}

/// Min-of-5 seconds for one call of `name`.
fn time(session: &functor_lang::Session, name: &str) -> f64 {
    (0..5)
        .map(|_| {
            let start = Instant::now();
            session
                .call(name, Vec::new(), &mut FunctorHost)
                .unwrap_or_else(|e| panic!("{name}: {}", e.message));
            start.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min)
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("WARNING: debug build — run with --release for real numbers");
    }
    let mut failed = false;
    let session = load(&ops_source());
    println!(
        "{:>7} {:>11} {:>11} {:>14}",
        "entries", "insert µs", "get µs", "values ns/entry"
    );
    for n in [1500, 6800] {
        let floor = time(&session, &format!("floor{n}"));
        let per_op = |name: &str| (time(&session, name) - floor) / OPS as f64 * 1e6;
        let (insert, get) = (per_op(&format!("insert{n}")), per_op(&format!("get{n}")));
        let walks = (OPS / n) as f64 * n as f64;
        let values = time(&session, &format!("values{n}")) / walks * 1e9;
        println!("{n:>7} {insert:>11.3} {get:>11.3} {values:>14.2}");
        if insert > INSERT_BUDGET_US {
            eprintln!("FAIL: insert at {n} entries {insert:.3} µs > {INSERT_BUDGET_US} µs");
            failed = true;
        }
    }
    drop(session);

    // Live bytes a session holds, less an empty session's.
    let retained = |src: &str| {
        let before = LIVE.load(Relaxed);
        let session = load(src);
        let bytes = LIVE.load(Relaxed) - before;
        drop(session);
        bytes
    };
    let empty = retained("let x = 1.0\n");
    for n in [1500, 6800] {
        let mb = (retained(&history_source(n)) - empty) as f64 / (1024.0 * 1024.0);
        println!("hist-map {n}: {mb:.1} MB retained (901 versions)");
        if n == 1500 && mb > HISTORY_BUDGET_MB {
            eprintln!("FAIL: hist-map history {mb:.1} MB > {HISTORY_BUDGET_MB} MB");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}
