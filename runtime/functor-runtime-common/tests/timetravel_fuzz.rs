//! Time-travel fuzzer over the examples and fixtures: the harness is
//! [`functor_runtime_common::timetravel::fuzz`] (also `functor fuzz`).
//!
//! Short budget by default. A long run:
//!
//! ```sh
//! TIMETRAVEL_FUZZ_SEEDS=20 TIMETRAVEL_FUZZ_OPS=400 \
//!   cargo test -p functor_runtime_common --test timetravel_fuzz -- --nocapture
//! ```
//!
//! `TIMETRAVEL_FUZZ_GAME=<name>` limits the run to one game; a failure prints
//! its shrunk op sequence. `TIMETRAVEL_FUZZ_DUMP=<dir>` writes the recorded and
//! restored world JSON (`was.json` / `now.json`) on a world-bytes mismatch.

use functor_runtime_common::timetravel::fuzz::{self, format_ops, Spec};
use functor_runtime_common::Key;

// ── Games ───────────────────────────────────────────────────────────────

fn example(dir: &str) -> Vec<(String, String)> {
    let root = format!("{}/../../examples/{dir}", env!("CARGO_MANIFEST_DIR"));
    let read = |name: &str| std::fs::read_to_string(format!("{root}/{name}")).unwrap();
    let entry = "game.fun";
    let mut sources = vec![(entry.to_string(), read(entry))];
    let mut siblings: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".fun") && n != entry)
        .collect();
    siblings.sort();
    for name in siblings {
        let src = read(&name);
        sources.push((name, src));
    }
    sources
}

/// Dynamic bodies beside a `Physics.memo` static group, a `Scene.memo`
/// draw, `Sub.every`, and `update`: the memo caches under time travel.
const MEMO_GAME: &str = r#"
let tileCount = 12.0
let gravity = Vec3.make(0.0, -9.81, 0.0)
let tile = (i) => Physics.fixed(Physics.tag(Text.concat("t", Text.fromFloat(i))), Physics.box(1.0, 0.2, 1.0))
  |> Physics.at(Vec3.make(Math.mod(i, 4.0) * 1.1 - 2.0, 0.0, Math.floor(i / 4.0) * 1.1 - 1.5))
let tiles = (n) => List.map(tile, List.range(n))
let ballTag = (i) => Physics.tag(Text.concat("b", Text.fromFloat(i)))
let ball = (drop, i) => Physics.dynamic(ballTag(i), Physics.sphere(0.25))
  |> Physics.at(Vec3.make(Math.sin(i * 3.1 + drop) * 1.5, 2.0 + i * 0.6, Math.cos(i * 1.7 + drop)))
  |> Physics.restitution(0.5)
type Msg = | Beat
let init = { drop: 0.0, layout: 0.0, beats: 0.0 }
let physics = (model) =>
  Physics.scene(gravity, [Physics.memo("tiles", model.layout + tileCount, tiles), ball(model.drop, 0.0), ball(model.drop, 1.0), ball(model.drop, 2.0)])
let input = (model, key, isDown) =>
  match key with
  | Key.Space => if isDown then { model with drop: model.drop + 1.0 } else model
  | Key.L => if isDown then { model with layout: Math.mod(model.layout + 1.0, 3.0) } else model
  | Key.K => if isDown then model else (model, Physics.applyImpulse(ballTag(0.0), Vec3.make(0.5, 4.0, 0.0)))
  | _ => model
let subscriptions = (model) => Sub.every(Time.seconds(0.25), Beat)
let update = (model, msg) =>
  match msg with
  | Beat => { model with beats: model.beats + 1.0 }
let tick = (model, dt, tts) => model
let floor = (n) => Scene.group(List.range(n) |> List.map((i) => Scene.cube() |> Scene.translate(Vec3.make(i, 0.0, 0.0))))
let draw = (model, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 6.0, -8.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.group([
      Scene.memo("floor", model.layout + tileCount, floor),
      Scene.sphere() |> Physics.transformed(ballTag(0.0)),
      Scene.sphere() |> Physics.transformed(ballTag(1.0)),
      Scene.sphere() |> Physics.transformed(ballTag(2.0)),
      Scene.cube() |> Scene.translate(Vec3.make(model.beats * 0.01, 0.0, 0.0)),
    ]))
"#;

/// A closure stored in the model: a reload can't carry such history, so it
/// restarts history at the current frame (an unsafe reload). The two closures
/// differ in parameter name so the model's `Display` tells them apart.
const CLOSURE_GAME: &str = r#"
let ballTag = Physics.tag("ball")
let slow = (x) => x + 1.0
let fast = (y) => y + 2.0
let init = { step: slow, n: 0.0 }
let input = (model, key, isDown) =>
  match key with
  | Key.F => if isDown then { model with step: fast } else { model with step: slow }
  | Key.K => if isDown then model else (model, Physics.applyImpulse(ballTag, Vec3.make(0.0, 3.0, 0.0)))
  | _ => model
let tick = (model, dt, tts) => { model with n: model.step(model.n) }
let physics = (model) =>
  Physics.scene(Vec3.make(0.0, -9.81, 0.0), [
    Physics.fixed(Physics.tag("ground"), Physics.box(8.0, 0.2, 8.0)),
    Physics.dynamic(ballTag, Physics.sphere(0.3)) |> Physics.at(Vec3.make(0.0, 2.0, 0.0)) |> Physics.restitution(0.4)])
let draw = (model, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 3.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.group([Scene.sphere() |> Physics.transformed(ballTag),
                 Scene.cube() |> Scene.translate(Vec3.make(Math.mod(model.n, 7.0), 0.0, 0.0))]))
"#;

/// `Effect.random` from `tick` and from input: every frame logs answers.
const RANDOM_GAME: &str = r#"
type Msg = | Rolled(r: float)
let scale = 1.0
let init = { r: 0.0, sum: 0.0 }
let input = (model, key, isDown) =>
  match key with
  | Key.Space => if isDown then (model, Effect.random(Rolled)) else model
  | _ => model
let update = (model, msg) =>
  match msg with
  | Rolled(r) => { model with r: r, sum: model.sum + r * scale }
let tick = (model, dt, tts) => (model, Effect.random(Rolled))
let draw = (model, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 3.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.translate(Vec3.make(model.r, 0.0, 0.0)))
"#;

/// `Effect.now` from `tick` and from input: the wall clock, per frame.
const NOW_GAME: &str = r#"
type Msg = | Stamped(t: float)
let offset = 0.0
let init = { t: 0.0, stamps: 0.0 }
let input = (model, key, isDown) =>
  match key with
  | Key.T => if isDown then (model, Effect.now(Stamped)) else model
  | _ => model
let update = (model, msg) =>
  match msg with
  | Stamped(t) => { model with t: t + offset, stamps: model.stamps + 1.0 }
let tick = (model, dt, tts) => (model, Effect.now(Stamped))
let draw = (model, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 3.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.translate(Vec3.make(Math.mod(model.t, 7.0), 0.0, 0.0)))
"#;

/// Saves on even frames, loads on odd ones: a fork's first load reads what
/// the store held THEN, not what the live tail saved since.
const SAVE_GAME: &str = r#"
let step = 1.0
let init = { n: 0.0, loaded: 0.0 }
let update = (model, msg) =>
  match msg with
  | Option.Some(v) => { model with loaded: v }
  | Option.None => model
let tick = (model, dt, tts) =>
  if Math.mod(model.n, 2.0) == 0.0
  then ({ model with n: model.n + step }, Persistence.save("fuzz", model.n))
  else ({ model with n: model.n + step }, Persistence.load("fuzz", (o) => o))
let draw = (model, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 3.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.translate(Vec3.make(Math.mod(model.loaded, 7.0), 0.0, 0.0)))
"#;

/// `text`, rewritten to `template` with 0.3, 0.6 or 0.9 for `{}`.
fn knob(text: &str, template: &str) -> Option<(String, Vec<String>)> {
    let values = ["0.3", "0.6", "0.9"].map(|v| template.replace("{}", v));
    Some((text.to_string(), values.to_vec()))
}

fn fixture(src: &str) -> Vec<(String, String)> {
    vec![("game.fun".to_string(), src.to_string())]
}

fn specs() -> Vec<Spec> {
    use Key::*;
    vec![
        Spec {
            keys: vec![W, A, S, D, Space, Left, Right],
            xr: true,
            knob: knob("let walkSpeed = 3.0", "let walkSpeed = {}"),
            ..Spec::new("character", example("character"))
        },
        Spec {
            keys: vec![Space, K, R],
            knob: knob("|> Physics.restitution(0.55))", "|> Physics.restitution({}))"),
            ..Spec::new("physics", example("physics"))
        },
        Spec {
            keys: vec![Left, Right, A, D, Space, Up, W],
            knob: knob("let runSpeed = 8.0", "let runSpeed = {}"),
            ..Spec::new("platformer", example("platformer"))
        },
        Spec {
            knob: knob("let spacing = 1.2", "let spacing = {}"),
            ..Spec::new("crowd", {
                // A small crowd keeps per-frame draws cheap; the skinned path is
                // the same at any count.
                let mut s = example("crowd");
                s[0].1 = s[0].1.replace("let count = 100.0", "let count = 4.0");
                s
            })
        },
        Spec {
            keys: vec![W, A, S, D, Up, Down],
            mouse: true,
            knob: knob("let walkSpeed = 6.0", "let walkSpeed = {}"),
            ..Spec::new("night-terrain", example("night-terrain"))
        },
        Spec {
            keys: vec![F, K],
            restarts: Some(true),
            knob: knob("|> Physics.restitution(0.4)", "|> Physics.restitution({})"),
            ..Spec::new("closure", fixture(CLOSURE_GAME))
        },
        Spec {
            keys: vec![Space, L, K],
            knob: knob("|> Physics.restitution(0.5)", "|> Physics.restitution({})"),
            ..Spec::new("memo", fixture(MEMO_GAME))
        },
        Spec {
            keys: vec![Space],
            knob: knob("let scale = 1.0", "let scale = {}"),
            ..Spec::new("random", fixture(RANDOM_GAME))
        },
        Spec {
            keys: vec![T],
            knob: knob("let offset = 0.0", "let offset = {}"),
            ..Spec::new("now", fixture(NOW_GAME))
        },
        Spec {
            knob: knob("let step = 1.0", "let step = {}"),
            ..Spec::new("save-load", fixture(SAVE_GAME))
        },
    ]
}

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn fuzz(game: &str, default_seeds: u64, default_ops: u64) {
    if let Ok(only) = std::env::var("TIMETRAVEL_FUZZ_GAME") {
        if only != game {
            return;
        }
    }
    let spec = specs().into_iter().find(|s| s.name == game).expect("spec");
    // `Persistence.save` writes a real slot: keep it out of the repo. One
    // root for the process, as the store is process-global.
    static SAVES: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let saves = SAVES.get_or_init(|| tempfile::tempdir().expect("save root"));
    functor_runtime_common::storage::set_project_root(saves.path().to_path_buf());
    let seeds = env("TIMETRAVEL_FUZZ_SEEDS", default_seeds);
    let ops = env("TIMETRAVEL_FUZZ_OPS", default_ops) as usize;
    let first = env("TIMETRAVEL_FUZZ_SEED0", 1);
    let (verified, found) = fuzz::run(&spec, first..first + seeds, ops);
    let failures: Vec<String> = found
        .iter()
        .map(|f| {
            format!(
                "{game} seed {}: {}\n  repro ({} ops): {}",
                f.seed,
                f.failure,
                f.repro.len(),
                format_ops(&f.repro)
            )
        })
        .collect();
    eprintln!("{game}: {seeds} seeds x {ops} ops: {verified:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn fuzz_character() {
    fuzz("character", 2, 60);
}

#[test]
fn fuzz_physics() {
    fuzz("physics", 2, 60);
}

#[test]
fn fuzz_platformer() {
    fuzz("platformer", 2, 60);
}

#[test]
fn fuzz_crowd() {
    fuzz("crowd", 2, 60);
}

#[test]
fn fuzz_night_terrain() {
    fuzz("night-terrain", 2, 60);
}

#[test]
fn fuzz_closure() {
    fuzz("closure", 2, 60);
}

#[test]
fn fuzz_memo() {
    fuzz("memo", 2, 60);
}

#[test]
fn fuzz_random() {
    fuzz("random", 2, 60);
}

#[test]
fn fuzz_now() {
    fuzz("now", 2, 60);
}

#[test]
fn fuzz_save_load() {
    fuzz("save-load", 2, 60);
}
