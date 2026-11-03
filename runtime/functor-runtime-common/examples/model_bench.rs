//! `model_bench` — the runtime's per-frame cost beyond the game's own `tick`,
//! by model size and writes per tick.
//!
//! The model is a `Map` of `size` records; `tick` rewrites `writes` of them.
//! Each case drives the embedded producer (the shared frame body both shells
//! run: subscriptions, `tick`, effect checks, history recording, `draw`) back
//! to back, past the 900-frame history window, and separately times the bare
//! interpreter `tick` call on the same model. `runtime` is the difference (an
//! empty-scene `draw` included): what the engine adds per frame. It should
//! track `writes`, not `size`.
//!
//! ```sh
//! cargo run -q --release -p functor_runtime_common --example model_bench
//! cargo run -q --release -p functor_runtime_common --example model_bench -- 7000:20 28000:0
//! ```
//!
//! Report-only. Medians over `FRAMES` frames; noise is a few µs.

use std::time::Instant;

use functor_lang::Value;
use functor_runtime_common::functor_lang_game_embedded::{FunctorLangEmbeddedGame, NativePlatform};
use functor_runtime_common::functor_lang_prelude::FunctorHost;
use functor_runtime_common::protocol::GameProducer;
use functor_runtime_common::FrameTime;

/// Past the recorder's 900-frame window, so history evicts as it records.
const WARMUP: usize = 1000;
const FRAMES: usize = 600;

fn source(size: usize, writes: usize) -> String {
    format!(
        r#"
type rec = {{ a: float, b: float, name: string }}
type model = {{ table: Map<float, rec>, step: float }}

let init: model = {{
  table: List.fold((m, i) => Map.insert(i, {{ a: i, b: 0.0, name: "r" }}, m), Map.empty(), List.range({size}.0)),
  step: 0.0,
}}

// An odd stride over the keys, so writes spread over the whole map.
let write = (step: float, t: Map<float, rec>, k: float) =>
  let id = Math.mod((step * {writes}.0 + k) * 2655.0, {size}.0) in
  Map.insert(id, {{ a: id, b: step, name: "w" }}, t)

let tick = (m: model, dt, tts) =>
  {{ table: List.fold((t, k) => write(m.step, t, k), m.table, List.range({writes}.0)), step: m.step + 1.0 }}

let draw = (m: model, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -4.0), Vec3.make(0.0, 0.0, 0.0)), Scene.group([]))
"#
    )
}

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.total_cmp(b));
    xs[xs.len() / 2]
}

/// Median µs of a full producer frame (`tick` + `render_at`).
fn producer_frame_us(src: &str) -> f64 {
    let mut game = FunctorLangEmbeddedGame::create(
        vec![("game.fun".to_string(), src.to_string())],
        Box::new(NativePlatform),
    )
    .unwrap_or_else(|e| panic!("load: {e}"));
    let dt = 1.0 / 60.0;
    let mut samples = Vec::with_capacity(FRAMES);
    for frame in 0..WARMUP + FRAMES {
        let time = FrameTime {
            dts: dt,
            tts: (frame + 1) as f32 * dt,
        };
        let start = Instant::now();
        game.tick(time);
        std::hint::black_box(game.render_at(time, 0.0));
        if frame >= WARMUP {
            samples.push(start.elapsed().as_secs_f64() * 1e6);
        }
    }
    median(samples)
}

/// Median µs of the bare interpreter `tick` call over the same model sequence.
fn tick_call_us(src: &str) -> f64 {
    let module = functor_lang::lower(functor_lang::parse(src).expect("parse")).expect("lower");
    let session = functor_lang::Session::load(&module, &mut FunctorHost)
        .unwrap_or_else(|f| panic!("load: {}", f.error.message));
    let mut model = session.global("init").expect("init");
    let mut samples = Vec::with_capacity(FRAMES);
    for frame in 0..WARMUP + FRAMES {
        let args = vec![model.clone(), Value::Number(1.0 / 60.0), Value::Number(frame as f64 / 60.0)];
        let start = Instant::now();
        model = session.call("tick", args, &mut FunctorHost).expect("tick");
        if frame >= WARMUP {
            samples.push(start.elapsed().as_secs_f64() * 1e6);
        }
    }
    median(samples)
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("model_bench: build with --release; debug timings are meaningless");
    }
    let cases: Vec<(usize, usize)> = match std::env::args().skip(1).collect::<Vec<_>>() {
        args if args.is_empty() => [1000, 7000, 28000]
            .iter()
            .flat_map(|&size| [0, 20, 200].map(|writes| (size, writes)))
            .collect(),
        args => args
            .iter()
            .map(|a| {
                let (s, w) = a.split_once(':').expect("SIZE:WRITES");
                (s.parse().expect("size"), w.parse().expect("writes"))
            })
            .collect(),
    };
    println!("{:>7} {:>6} {:>10} {:>10} {:>10}", "size", "writes", "tick µs", "frame µs", "runtime µs");
    for (size, writes) in cases {
        let src = source(size, writes);
        let tick = tick_call_us(&src);
        let frame = producer_frame_us(&src);
        println!("{size:>7} {writes:>6} {tick:>10.1} {frame:>10.1} {:>10.1}", frame - tick);
    }
}
