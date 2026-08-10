//! A/B construction probe for `Scene.instanced` (NOT COMMITTED — scratch
//! evidence for the instancing PR, mirroring #673's probe shape).
//!
//! Both paths perform identical per-cell math; only scene construction
//! differs: the per-node pipeline builds N cube nodes (cube → lit → scale →
//! translate), the instanced path builds N `Instance.t` builder chains into
//! one `Scene.instanced` node. Headless, engine prelude, release only.
//!
//! ```sh
//! cargo run -q --release -p functor_runtime_common --example instancing_probe
//! ```

use std::hint::black_box;
use std::time::{Duration, Instant};

use functor_lang::Value;
use functor_runtime_common::functor_lang_prelude::{frame_value, FunctorHost};
use functor_runtime_common::Frame;

fn shared_header(side: u32) -> String {
    format!(
        r#"
let side = {side}.0

let height = (fx: float, fz: float, tts: float): float =>
  0.6 + 2.6 * (0.5 + 0.5 * Math.sin(fx * 9.0 + fz * 5.0 + tts * 1.5))

let init = 0.0
let tick = (m: float, dt: float, tts: float) => m
"#
    )
}

/// Path A: one scene node pipeline per cube.
fn per_node(side: u32) -> String {
    shared_header(side)
        + r#"
let cell = (ix, iz, tts) =>
  let fx = ix / side in
  let fz = iz / side in
  let h = height(fx, fz, tts) in
  Scene.cube()
    |> Scene.lit(Color.rgb(0.25 + 0.75 * fx, 0.3 + 0.4 * (h / 3.2), 0.25 + 0.75 * fz))
    |> Scene.scaleXYZ(0.8, h, 0.8)
    |> Scene.rotateY(Angle.degrees(10.0 * fx))
    |> Scene.translate(Vec3.make((ix - side / 2.0) * 1.4, h / 2.0, (iz - side / 2.0) * 1.4))

let draw = (m: float, tts: float) =>
  let cubes =
    List.range(side)
      |> List.concatMap((ix) => List.range(side) |> List.map((iz) => cell(ix, iz, tts))) in
  Frame.create(
    Camera3D.lookAt(Vec3.make(30.0, 24.0, 30.0), Vec3.make(0.0, 1.0, 0.0)),
    Scene.group(cubes))
"#
}

/// Path B: one `Instance.t` builder chain per cube into `Scene.instanced`.
fn instanced(side: u32) -> String {
    shared_header(side)
        + r#"
let cell = (ix, iz, tts) =>
  let fx = ix / side in
  let fz = iz / side in
  let h = height(fx, fz, tts) in
  Instance.at(Vec3.make((ix - side / 2.0) * 1.4, h / 2.0, (iz - side / 2.0) * 1.4))
    |> Instance.scaleXYZ(0.8, h, 0.8)
    |> Instance.rotateY(Angle.degrees(10.0 * fx))
    |> Instance.tint(Color.rgb(0.25 + 0.75 * fx, 0.3 + 0.4 * (h / 3.2), 0.25 + 0.75 * fz))

let draw = (m: float, tts: float) =>
  let copies =
    List.range(side)
      |> List.concatMap((ix) => List.range(side) |> List.map((iz) => cell(ix, iz, tts))) in
  Frame.create(
    Camera3D.lookAt(Vec3.make(30.0, 24.0, 30.0), Vec3.make(0.0, 1.0, 0.0)),
    Scene.cube() |> Scene.lit(Color.rgb(1.0, 1.0, 1.0)) |> Scene.instanced(copies))
"#
}

/// Path C: the flat `Instance.trs` fast path plus one tint call.
fn instanced_trs(side: u32) -> String {
    shared_header(side)
        + r#"
let cell = (ix, iz, tts) =>
  let fx = ix / side in
  let fz = iz / side in
  let h = height(fx, fz, tts) in
  Instance.trs(
      Vec3.make((ix - side / 2.0) * 1.4, h / 2.0, (iz - side / 2.0) * 1.4),
      Angle.degrees(10.0 * fx), 0.8, h, 0.8)
    |> Instance.tint(Color.rgb(0.25 + 0.75 * fx, 0.3 + 0.4 * (h / 3.2), 0.25 + 0.75 * fz))

let draw = (m: float, tts: float) =>
  let copies =
    List.range(side)
      |> List.concatMap((ix) => List.range(side) |> List.map((iz) => cell(ix, iz, tts))) in
  Frame.create(
    Camera3D.lookAt(Vec3.make(30.0, 24.0, 30.0), Vec3.make(0.0, 1.0, 0.0)),
    Scene.cube() |> Scene.lit(Color.rgb(1.0, 1.0, 1.0)) |> Scene.instanced(copies))
"#
}

const WARMUP: Duration = Duration::from_millis(300);
const SAMPLES: usize = 120;

fn load_source(src: &str) -> (functor_lang::Session, Value) {
    let module =
        functor_lang::lower(functor_lang::parse(src).expect("probe parses")).expect("probe lowers");
    let session = functor_lang::Session::load(&module, &mut FunctorHost)
        .unwrap_or_else(|f| panic!("probe load failed: {}", f.error.message));
    let model = session.global("init").expect("probe defines init");
    (session, model)
}

fn draw_frame(session: &functor_lang::Session, model: &Value, last_frame: &mut Option<Frame>) {
    let value = session
        .call(
            "draw",
            vec![model.clone(), Value::Number(1.0)],
            &mut FunctorHost,
        )
        .unwrap_or_else(|e| panic!("draw failed: {}", e.message));
    let frame = frame_value(&value)
        .unwrap_or_else(|| panic!("draw must return a Frame, got {}", value.kind_name()));
    *last_frame = Some(frame.clone());
    black_box(last_frame);
}

fn bench(src: &str) -> f64 {
    let (session, model) = load_source(src);
    let mut last_frame: Option<Frame> = None;
    let warm = Instant::now();
    while warm.elapsed() < WARMUP {
        draw_frame(&session, &model, &mut last_frame);
    }
    let mut samples: Vec<u128> = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        draw_frame(&session, &model, &mut last_frame);
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    samples[0] as f64 / 1_000_000.0
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("WARNING: debug build — rerun with --release");
    }
    println!(
        "{:>7} {:>18} {:>18} {:>9} {:>14} {:>9}",
        "cubes", "per-node ms", "builder ms", "red%", "trs ms", "red%"
    );
    for side in [32u32, 64, 96] {
        let a = bench(&per_node(side));
        let b = bench(&instanced(side));
        let c = bench(&instanced_trs(side));
        println!(
            "{:>7} {:>18.3} {:>18.3} {:>8.1}% {:>14.3} {:>8.1}%",
            side * side,
            a,
            b,
            (1.0 - b / a) * 100.0,
            c,
            (1.0 - c / a) * 100.0
        );
    }
}
