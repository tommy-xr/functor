//! The Functor prelude for MLE — Track C1 of `docs/mle.md`.
//!
//! A [`mle::Host`] giving MLE programs the engine vocabulary: scene
//! constructors, transforms, a camera, and frame assembly, all producing the
//! exact protocol types this crate already speaks ([`Scene3D`], [`Camera`],
//! [`Frame`] — see [`crate::protocol`]). Engine values cross into MLE as
//! opaque [`mle::Value::HostData`]; MLE code composes them and hands back a
//! `Frame`, which the shells extract with [`frame_value`].
//!
//! # Vocabulary
//!
//! ```text
//! Scene.cube() / sphere() / cylinder() / quad() / plane()   -> Scene
//! Scene.model(path)                                         -> Scene
//!   (a glTF file loaded by the shells' asset pipeline; the path is
//!    relative to the game dir, exactly as F#'s `Model.file` — a missing
//!    file logs an error and renders as the empty fallback asset)
//! Scene.heightmap([[height, …], …])                         -> Scene
//!   (a subdivided XZ grid displaced by per-vertex heights — a list of ROWS,
//!    each an equal-length list of numbers, at least 2x2; spans the unit
//!    square, so size it with Scene.scale. Sample a height function with
//!    List builtins: List.range(rows) |> List.map((r) => List.range(cols)
//!    |> List.map((c) => f(r, c))) — F#'s `heightmapFn`, in user space)
//! Scene.group([scene, …])                                   -> Scene
//! Scene.color(scene, r, g, b)                               -> Scene
//! Scene.translate(scene, x, y, z)                           -> Scene
//! Scene.rotateX/rotateY/rotateZ(scene, angle)               -> Scene
//! Angle.degrees(n) / Angle.radians(n)                       -> Angle
//!   (rotations and camera angles take Angle VALUES, never bare numbers —
//!    degree/radian confusion is unrepresentable)
//! Scene.scale(scene, k)                                     -> Scene
//! Texture.file(path)                                        -> Texture
//!   (an image loaded by the shells' asset pipeline, path relative to the
//!    game dir — F#'s `Texture.file`; declared once and passed as a VALUE,
//!    the Angle rule applied to assets)
//! Scene.litTexture(scene, texture)                          -> Scene
//!   (a diffuse-lit textured surface — F#'s `Material.litTexture`)
//! Scene.emissiveTexture(scene, texture)                     -> Scene
//!   (a self-lit textured surface, fullbright — F#'s `Material.emissiveTexture`)
//! Camera.lookAt(ex, ey, ez, tx, ty, tz)                     -> Camera
//!   (up is +Y; vertical fov pinned at 45°, near/far at protocol defaults)
//! Frame.create(camera, scene)                               -> Frame
//! RenderTarget.named(id)                                    -> RenderTarget
//! RenderTarget.sized(target, w, h)                          -> RenderTarget
//!   (a named offscreen texture, 512x512 unless sized; declare ONCE, use the
//!    value at both sites — the writer and the reader — so writer/reader id
//!    typos are unrepresentable, the Angle rule applied to identity)
//! Frame.withRenderTarget(frame, target, targetFrame)        -> Frame
//!   (the writer: targetFrame — its own camera/scene/lights — is rendered
//!    into the target before frame's main pass; a scene sampling its own
//!    target sees last frame's image)
//! Scene.screen(scene, target)                               -> Scene
//!   (the reader: an emissive "screen" surface showing the target's texture;
//!    an id no frame declares shows magenta and warns once)
//! Fog.linear(near, far, r, g, b) / Fog.exp(density, r, g, b) -> Fog
//! Frame.withFog(frame, fog)                                  -> Frame
//!   (frame-level distance fog on every forward material, emissive included —
//!    fog occludes glow; the fog color is also the pass's clear color)
//! Ui.text(s) / Ui.textColor(r, g, b, s)                     -> View
//! Ui.column([view, …])                                      -> View
//! Ui.panel(view, anchor)                                    -> View
//! Ui.topLeft()                                              -> Anchor
//!   (the optional `ui = (model) => …` hook's tree — hello's HUD shape:
//!    text lines stacked in a column, pinned to a screen corner. Only the
//!    corner the port needed exists; the rest arrive with a port that
//!    needs them. `Ui.panel` takes the view FIRST, so it pipes.)
//! Time.seconds(n) / Time.millis(n)                          -> Duration
//!   (like Angle: timing functions take Duration VALUES, never bare
//!    numbers — seconds/milliseconds confusion is unrepresentable)
//! Sub.every(duration, msg) / Sub.none() / Sub.batch([sub,…]) -> Sub
//!   (the game's `subscriptions` returns one of these; fired messages are
//!    folded through `update` — see the producers' drain seam)
//! Effect.none() / Effect.batch([fx,…])                       -> Effect
//! Effect.now(tagger) / Effect.random(tagger)                 -> Effect
//!   (one-shot commands: `update`/`tick` may return `(model, effect)`
//!    tuples; the producer performs each effect via its EffectRunner —
//!    real, fake, or replay — and folds `tagger(result)` back through
//!    `update`, draining to a fixed point. Taggers run same-frame, so no
//!    closure ever outlives its session.)
//!
//! Physics.box(w, h, d) / sphere(r) / capsule(hh, r)         -> Shape
//! Physics.dynamic/kinematic/fixed(tag, shape)               -> Body
//! Physics.at/velocity(body, x, y, z)                        -> Body
//! Physics.mass/friction/restitution(body, n)                -> Body
//! Physics.sensor(body)                                      -> Body
//! Physics.scene(gx, gy, gz, [body, …])                      -> PhysicsScene
//! Physics.position(tag)                                     -> {x, y, z}
//! Physics.transformed(scene, tag)                           -> Scene
//! Physics.applyImpulse/applyForce/setVelocity/teleport(tag, x, y, z)
//!                                                           -> Effect
//! ```
//!
//! The `Physics.*` reads target the singleton world the shell reconciles and
//! steps each frame from the game's optional `physics` hook (see the desktop
//! `MleGame` driver + docs/physics.md). MLE is interpreted in the shell's own
//! process, so these are direct reads of live world state — the seam the
//! dylib producers can't have.
//!
//! Scene-consuming functions take the scene FIRST, so they compose with
//! `|>` (the piped value is prepended — see `mle`'s lowering docs):
//! `Scene.cube() |> Scene.color(1.0, 0.0, 0.0) |> Scene.translate(2.0, 0.0, 0.0)`.
//!
//! # Transform semantics (deliberate — see the Milestone-0 quirks)
//!
//! Every transform **wraps its scene in a fresh `Group` node** carrying the
//! transform, rather than multiplying it onto the node's own `xform`. Two
//! renderer quirks make this the only composition that behaves the way the
//! source reads:
//!
//! - `Material` nodes ignore their own `xform` in `Scene3D::render`, so a
//!   transform applied directly to `Scene.color(…)`'s node would be silently
//!   dropped. A `Group` wrapper is always honored.
//! - `Scene3D::transform` right-multiplies (`self.xform * xform`), making
//!   `translate(rotateY(x))` apply the translation *first*. Wrapping makes
//!   each transform a parent node instead, so the outer call is applied last
//!   in world space: `Scene.translate(Scene.rotateY(x, r), …)` rotates in
//!   place, *then* moves — the order the source reads.

use cgmath::Matrix4;
use mle::value::HostData;
use mle::{Host, RunError, Span, Value};
use std::rc::Rc;

use crate::fog::Fog;
use crate::math::Angle;
use crate::ui::{self, View};
use crate::physics;
use crate::render_target::RenderTargetDescriptor;
use crate::scene3d::{MaterialDescription, ModelDescription, ModelHandle, TextureDescription};
use crate::{Camera, Frame, Light, Scene3D, SceneObject, Shape};

/// A [`Scene3D`] as an opaque MLE value.
pub struct MleScene(pub Scene3D);

/// A [`Camera`] as an opaque MLE value.
pub struct MleCamera(pub Camera);

/// A [`Frame`] as an opaque MLE value — what an MLE `draw` returns.
pub struct MleFrame(pub Frame);

/// A [`Light`] as an opaque MLE value.
pub struct MleLight(pub Light);

/// A duration as an opaque MLE value — `Time.seconds(…)`/`Time.millis(…)`.
/// Timing functions accept ONLY this, never a bare number, making
/// seconds/milliseconds confusion unrepresentable (the Angle rule, applied
/// to time). Stored canonically in seconds.
pub struct MleDuration(pub f64);

/// A subscription tree as an opaque MLE value — what the game's
/// `subscriptions(model)` returns. The declarative dual of effects: a
/// standing "while the model looks like this, listen to these".
///
/// `Every` is deliberately stateless (mirroring the F# runtime's
/// `Sub.crossedBoundary`): it fires when an integer multiple of its period
/// lies in `(prevTts, tts]` — a pure function of the global clock. No
/// per-timer identity, no frame-to-frame diffing, and it survives a hot
/// reload for free. A non-positive period never fires.
///
/// Precision: `tts` is f32-sourced (the protocol's `FrameTime`), so with a
/// period that has no exact binary representation a boundary can land one
/// frame late — but the floor comparison is monotone and telescoping over
/// adjacent frame windows, so a firing is never lost or doubled (and the F#
/// executor computes on the same f32s: exact parity).
#[derive(Clone)]
pub enum SubTree {
    None,
    Every { period_seconds: f64, msg: Value },
    Batch(Vec<SubTree>),
    /// Collision events from the physics step (docs/physics.md Phase 5):
    /// the tagger receives `{started, a, b, sensor}` for every contact that
    /// began/ended this frame, delivered post-step through `update` (the
    /// same point deferred queries answer).
    PhysicsEvents { tagger: Value },
}

pub struct MleSub(pub SubTree);

/// A one-shot effect as an opaque MLE value — what `update`/`tick` may
/// return beside the model (`(model, effect)`). The imperative dual of
/// [`SubTree`]: "do this once, then hand my tagger the result". Performed
/// by the producer through an [`EffectRunner`] (real / fake / replay), so
/// the same program is testable and replayable — the B6 broker contract.
#[derive(Clone)]
pub enum EffectTree {
    None,
    /// Wall-clock time: the tagger gets seconds since the Unix epoch.
    Now {
        tagger: Value,
    },
    /// A uniform float in [0, 1).
    Random {
        tagger: Value,
    },
    Batch(Vec<EffectTree>),
    /// A fire-and-forget physics command (docs/physics.md Phase 3): no
    /// tagger — performing it queues the command on the singleton world,
    /// applied at the next stepped frame's first substep (after reconcile).
    /// The game observes the outcome through the physics reads, not a
    /// message.
    Physics(physics::PhysicsCommand),
    /// A physics query (docs/physics.md Phase 4). Unlike `Now`/`Random`,
    /// queries are **deferred**: the pre-step drain holds them and the
    /// driver performs them right after the frame's physics step, so the
    /// tagger's record answers against the FRESH world ("commands apply at
    /// the step; queries answer after it").
    Raycast {
        origin: [f32; 3],
        dir: [f32; 3],
        max_dist: f32,
        tagger: Value,
    },
}

pub struct MleEffect(pub EffectTree);

/// Performs effects. `Real` asks the world; `Fake` gives fixed values
/// (tests); `Replay` feeds back a recorded [`EffectLog`] — same program,
/// three worlds, one contract (docs/mle.md B6).
pub trait EffectRunner {
    fn now(&mut self) -> f64;
    fn random(&mut self) -> f64;
    /// The raycast result record (`{hit, x, y, z, nx, ny, nz, distance,
    /// tag}` — `hit: false` with zeroed fields for a miss). `Real` asks the
    /// singleton physics world; `Fake`/`Replay` return canned/recorded
    /// records — physics queries are testable without a world at all.
    fn raycast(&mut self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> EffectValue;
}

/// The tagger-facing record for a raycast result. Always the full shape —
/// `hit: Bool` discriminates — so field access typechecks on both arms.
pub fn ray_result_value(hit: Option<physics::RayHit>) -> EffectValue {
    let (h, p, n, d, tag) = match hit {
        Some(hit) => (true, hit.position, hit.normal, hit.distance, hit.tag),
        None => (false, [0.0; 3], [0.0; 3], 0.0, String::new()),
    };
    EffectValue::Record(vec![
        ("hit".to_string(), EffectValue::Bool(h)),
        ("x".to_string(), EffectValue::Number(p[0] as f64)),
        ("y".to_string(), EffectValue::Number(p[1] as f64)),
        ("z".to_string(), EffectValue::Number(p[2] as f64)),
        ("nx".to_string(), EffectValue::Number(n[0] as f64)),
        ("ny".to_string(), EffectValue::Number(n[1] as f64)),
        ("nz".to_string(), EffectValue::Number(n[2] as f64)),
        ("distance".to_string(), EffectValue::Number(d as f64)),
        ("tag".to_string(), EffectValue::Text(tag)),
    ])
}

/// The structured effect log keeps this many most-recent records — enforced
/// INSIDE the drain, so the bound holds even mid-frame.
pub const EFFECT_LOG_CAP: usize = 256;

/// A performed effect's structured result: the serializable plain-data
/// subset of [`Value`] — no closures, no host data — which is what makes
/// results loggable, replayable, and fakeable. `now`/`random` results are
/// `Number`s; structured effects (physics raycasts, …) carry `Record`s.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EffectValue {
    Number(f64),
    Bool(bool),
    Text(String),
    List(Vec<EffectValue>),
    /// Field order is construction order (deterministic, like MLE records).
    Record(Vec<(String, EffectValue)>),
}

impl EffectValue {
    /// The MLE value handed to the effect's tagger.
    pub fn to_mle(&self) -> Value {
        match self {
            EffectValue::Number(n) => Value::Number(*n),
            EffectValue::Bool(b) => Value::Bool(*b),
            EffectValue::Text(s) => Value::String(Rc::from(s.as_str())),
            EffectValue::List(items) => {
                Value::List(Rc::new(items.iter().map(EffectValue::to_mle).collect()))
            }
            EffectValue::Record(fields) => Value::Record(Rc::new(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_mle()))
                    .collect(),
            )),
        }
    }
}

impl From<f64> for EffectValue {
    fn from(n: f64) -> EffectValue {
        EffectValue::Number(n)
    }
}

/// One performed effect: what kind, and what value the tagger received —
/// the structured effect log (LLM-readable, and the input to replay).
#[derive(Clone, Debug, PartialEq)]
pub struct EffectRecord {
    pub kind: &'static str,
    pub value: EffectValue,
}

pub type EffectLog = Vec<EffectRecord>;

/// The real world: system clock, xorshift PRNG seeded from it (no new
/// dependency; game-quality randomness, not crypto).
pub struct RealEffects {
    rng_state: u64,
}

/// Epoch seconds from the platform clock. `std::time::SystemTime` is
/// unimplemented on `wasm32-unknown-unknown` (it panics), so the browser
/// build asks `Date.now()` instead.
fn epoch_seconds() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() / 1000.0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }
}

impl RealEffects {
    #[allow(clippy::new_without_default)]
    pub fn new() -> RealEffects {
        let seed = (epoch_seconds() * 1e9) as u64;
        RealEffects {
            rng_state: seed | 1,
        }
    }
}

impl EffectRunner for RealEffects {
    fn now(&mut self) -> f64 {
        epoch_seconds()
    }
    fn raycast(&mut self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> EffectValue {
        ray_result_value(
            physics::with_world(physics::DEFAULT_WORLD, |w| w.raycast(origin, dir, max_dist))
                .flatten(),
        )
    }
    fn random(&mut self) -> f64 {
        // xorshift64*; map the top 53 bits into [0, 1).
        let mut x = self.rng_state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng_state = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Fixed answers, for tests: `now` is a constant, `random` cycles a
/// sequence.
pub struct FakeEffects {
    pub now: f64,
    pub randoms: Vec<f64>,
    next: usize,
    /// Canned raycast results, cycled like `randoms`; empty = every ray
    /// misses. Test physics-query handling with no world at all.
    pub ray_hits: Vec<EffectValue>,
    next_ray: usize,
}

impl FakeEffects {
    pub fn new(now: f64, randoms: Vec<f64>) -> FakeEffects {
        FakeEffects {
            now,
            randoms,
            next: 0,
            ray_hits: Vec::new(),
            next_ray: 0,
        }
    }

    pub fn with_ray_hits(mut self, ray_hits: Vec<EffectValue>) -> FakeEffects {
        self.ray_hits = ray_hits;
        self
    }
}

impl EffectRunner for FakeEffects {
    fn now(&mut self) -> f64 {
        self.now
    }
    fn random(&mut self) -> f64 {
        let v = self.randoms[self.next % self.randoms.len()];
        self.next += 1;
        v
    }
    fn raycast(&mut self, _origin: [f32; 3], _dir: [f32; 3], _max_dist: f32) -> EffectValue {
        if self.ray_hits.is_empty() {
            return ray_result_value(None);
        }
        let v = self.ray_hits[self.next_ray % self.ray_hits.len()].clone();
        self.next_ray += 1;
        v
    }
}

/// Replays a recorded log in order. A kind mismatch means the program
/// diverged from the recording — fail loud with the position.
pub struct ReplayEffects {
    log: EffectLog,
    next: usize,
}

impl ReplayEffects {
    pub fn new(log: EffectLog) -> ReplayEffects {
        ReplayEffects { log, next: 0 }
    }
    fn take(&mut self, kind: &'static str) -> EffectValue {
        let record = self
            .log
            .get(self.next)
            .unwrap_or_else(|| panic!("replay log exhausted at effect {}", self.next));
        assert_eq!(
            record.kind, kind,
            "replay diverged at effect {}: recorded {}, program asked for {kind}",
            self.next, record.kind
        );
        self.next += 1;
        record.value.clone()
    }

    fn take_number(&mut self, kind: &'static str) -> f64 {
        match self.take(kind) {
            EffectValue::Number(n) => n,
            other => panic!(
                "replay diverged at effect {}: recorded a non-number {other:?} for {kind}",
                self.next - 1
            ),
        }
    }
}

impl EffectRunner for ReplayEffects {
    fn now(&mut self) -> f64 {
        self.take_number("now")
    }
    fn random(&mut self) -> f64 {
        self.take_number("random")
    }
    fn raycast(&mut self, _origin: [f32; 3], _dir: [f32; 3], _max_dist: f32) -> EffectValue {
        self.take("physics.raycast")
    }
}

/// An [`Angle`] as an opaque MLE value — `Angle.degrees(…)`/`Angle.radians(…)`.
/// Rotation/camera functions accept ONLY this, never a bare number, making
/// degree/radian confusion unrepresentable (the F# side's `Math.Angle`
/// discipline, carried across the boundary).
pub struct MleAngle(pub Angle);

/// A [`RenderTargetDescriptor`] as an opaque MLE value — declared once via
/// `RenderTarget.named` and used at both sites: the writer
/// (`Frame.withRenderTarget`) and the reader (`Scene.screen`). Both accept
/// ONLY this, never a bare string, so a writer/reader id typo is
/// unrepresentable (the Angle rule, applied to identity).
pub struct MleRenderTarget(pub RenderTargetDescriptor);

impl HostData for MleRenderTarget {
    fn type_name(&self) -> &'static str {
        "RenderTarget"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A [`TextureDescription`] as an opaque MLE value — `Texture.file(…)`.
/// Texture-material functions (`Scene.litTexture` / `Scene.emissiveTexture`)
/// accept ONLY this, never a bare path string, so a texture is declared once
/// and passed as a value (the Angle rule, applied to assets).
pub struct MleTexture(pub TextureDescription);

impl HostData for MleTexture {
    fn type_name(&self) -> &'static str {
        "Texture"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A [`View`] as an opaque MLE value — what the optional `ui(model)` entry
/// point returns (`Ui.text` / `Ui.column` / `Ui.panel`). The shells lower it
/// to the shared egui text overlay, exactly as the F# `ui` hook's tree.
pub struct MleView(pub View);

impl HostData for MleView {
    fn type_name(&self) -> &'static str {
        "View"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A [`ui::Anchor`] as an opaque MLE value — `Ui.topLeft()`. `Ui.panel`
/// accepts ONLY this (the Angle rule, applied to screen corners).
pub struct MleUiAnchor(pub ui::Anchor);

impl HostData for MleUiAnchor {
    fn type_name(&self) -> &'static str {
        "Anchor"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A [`Fog`] as an opaque MLE value — `Fog.linear(…)`/`Fog.exp(…)`.
/// `Frame.withFog` accepts ONLY this (the Angle rule).
pub struct MleFog(pub Fog);

impl HostData for MleFog {
    fn type_name(&self) -> &'static str {
        "Fog"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for MleDuration {
    fn type_name(&self) -> &'static str {
        "Duration"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for MleSub {
    fn type_name(&self) -> &'static str {
        "Sub"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for MleEffect {
    fn type_name(&self) -> &'static str {
        "Effect"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for MleAngle {
    fn type_name(&self) -> &'static str {
        "Angle"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A [`physics::Shape`] as an opaque MLE value.
pub struct MleShape(pub physics::Shape);

/// A declared [`physics::Body`] as an opaque MLE value.
pub struct MleBody(pub physics::Body);

/// A [`physics::PhysicsScene`] as an opaque MLE value — what an MLE `physics`
/// hook returns.
pub struct MlePhysicsScene(pub physics::PhysicsScene);

impl HostData for MleShape {
    fn type_name(&self) -> &'static str {
        "Shape"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for MleBody {
    fn type_name(&self) -> &'static str {
        "Body"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for MlePhysicsScene {
    fn type_name(&self) -> &'static str {
        "PhysicsScene"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for MleLight {
    fn type_name(&self) -> &'static str {
        "Light"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for MleScene {
    fn type_name(&self) -> &'static str {
        "Scene"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for MleCamera {
    fn type_name(&self) -> &'static str {
        "Camera"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for MleFrame {
    fn type_name(&self) -> &'static str {
        "Frame"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Extract the [`Frame`] from an MLE value (an `Frame.create` result), for
/// the shells' render loop.
pub fn frame_value(value: &Value) -> Option<&Frame> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<MleFrame>().map(|f| &f.0),
        _ => None,
    }
}

/// Extract the [`View`] from an MLE value (a `Ui.*` result), for the shells'
/// overlay pass — the `ui` hook's [`frame_value`].
pub fn view_value(value: &Value) -> Option<&View> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<MleView>().map(|v| &v.0),
        _ => None,
    }
}

/// Extract the [`physics::PhysicsScene`] from an MLE value (a `Physics.scene`
/// result), for the shells' physics drive.
pub fn physics_scene_value(value: &Value) -> Option<&physics::PhysicsScene> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<MlePhysicsScene>()
            .map(|s| &s.0),
        _ => None,
    }
}

/// The prelude host. Stateless; construct one per interpreter session.
pub struct FunctorHost;

const PATHS: &[&str] = &[
    "Scene.cube",
    "Scene.sphere",
    "Scene.cylinder",
    "Scene.quad",
    "Scene.plane",
    "Scene.model",
    "Scene.heightmap",
    "Scene.group",
    "Scene.color",
    "Scene.translate",
    "Scene.rotateX",
    "Scene.rotateY",
    "Scene.rotateZ",
    "Scene.scale",
    "Scene.lit",
    "Scene.emissive",
    "Scene.litTexture",
    "Scene.emissiveTexture",
    "Texture.file",
    "Angle.degrees",
    "Angle.radians",
    "Camera.lookAt",
    "Camera.firstPerson",
    "Light.ambient",
    "Light.directional",
    "Light.point",
    "Light.castShadows",
    "Frame.create",
    "Frame.createLit",
    "Frame.withRenderTarget",
    "Frame.withFog",
    "Fog.linear",
    "Fog.exp",
    "RenderTarget.named",
    "RenderTarget.sized",
    "Scene.screen",
    "Ui.text",
    "Ui.textColor",
    "Ui.column",
    "Ui.panel",
    "Ui.topLeft",
    "Time.seconds",
    "Time.millis",
    "Sub.none",
    "Sub.every",
    "Sub.batch",
    "Effect.none",
    "Effect.now",
    "Effect.random",
    "Effect.batch",
    "Physics.box",
    "Physics.sphere",
    "Physics.capsule",
    "Physics.dynamic",
    "Physics.kinematic",
    "Physics.fixed",
    "Physics.at",
    "Physics.velocity",
    "Physics.mass",
    "Physics.friction",
    "Physics.restitution",
    "Physics.sensor",
    "Physics.scene",
    "Physics.position",
    "Physics.transformed",
    "Physics.applyImpulse",
    "Physics.applyForce",
    "Physics.setVelocity",
    "Physics.teleport",
    "Physics.raycast",
    "Physics.events",
];

impl Host for FunctorHost {
    fn provides(&self, path: &str) -> bool {
        PATHS.contains(&path)
    }

    fn call(&mut self, path: &str, args: Vec<Value>, span: Span) -> Result<Value, RunError> {
        let err = |message: String| Err(RunError { message, span });
        let usage = |sig: &str| {
            Err(RunError {
                message: format!("usage: {sig}"),
                span,
            })
        };
        match path {
            // Constructors take no arguments — reject any, so a guessed
            // `Scene.cube(size)` fails loud instead of silently ignoring it.
            "Scene.cube" | "Scene.sphere" | "Scene.cylinder" | "Scene.quad" | "Scene.plane" => {
                if !args.is_empty() {
                    return usage(&format!("{path}()"));
                }
                scene_value(match path {
                    "Scene.cube" => Scene3D::cube(),
                    "Scene.sphere" => Scene3D::sphere(),
                    "Scene.cylinder" => Scene3D::cylinder(),
                    "Scene.quad" => Scene3D::quad(),
                    _ => Scene3D::plane(),
                })
            }
            // A glTF model by file path (relative to the game dir), the MLE
            // face of F#'s `Model.file |> Graphics.Scene3D.model`. Loading is
            // the shells' asset pipeline, exactly as for the dylib producers;
            // a missing file logs an error and renders as the empty fallback.
            "Scene.model" => match args.as_slice() {
                [Value::String(path)] if !path.is_empty() => {
                    scene_value(Scene3D::model(ModelDescription {
                        handle: ModelHandle::File(path.to_string()),
                        overrides: Vec::new(),
                    }))
                }
                _ => usage(
                    "Scene.model(\"file.glb\") — a non-empty glTF path relative to \
the game dir",
                ),
            },
            // A subdivided XZ grid displaced by per-vertex heights — the MLE
            // face of F#'s `Scene3D.heightmap` (the protocol `Heightmap`
            // shape). The surface is a list of ROWS (each an equal-length
            // list of heights): MLE has no floor/mod to build F#'s flat
            // row-major array, but nested `List.range |> List.map` builds
            // rows naturally — and the host flattens row-major, so the wire
            // data is exactly what the F# side emits. Heights are sampled in
            // user space (F#'s `heightmapFn`, minus the host trampoline the
            // prelude can't offer: it has no session handle to apply an MLE
            // closure).
            "Scene.heightmap" => match args.as_slice() {
                [Value::List(rows)] => {
                    let usage_msg = "Scene.heightmap([[height, …], …]) — a list of at \
least 2 rows, each an equal-length list of at least 2 numbers";
                    if rows.len() < 2 {
                        return usage(usage_msg);
                    }
                    let mut cols: Option<usize> = None;
                    let mut heights = Vec::new();
                    for (r, row) in rows.iter().enumerate() {
                        let Value::List(row) = row else {
                            return err(format!(
                                "Scene.heightmap rows must be lists of numbers, got {}",
                                row.kind_name()
                            ));
                        };
                        match cols {
                            None => {
                                if row.len() < 2 {
                                    return usage(usage_msg);
                                }
                                cols = Some(row.len());
                                heights.reserve(rows.len() * row.len());
                            }
                            Some(cols) if row.len() != cols => {
                                return err(format!(
                                    "Scene.heightmap rows must all have the same length: \
row 0 has {cols} heights, row {r} has {}",
                                    row.len()
                                ))
                            }
                            Some(_) => {}
                        }
                        for h in row.iter() {
                            heights.push(num(h, span)? as f32);
                        }
                    }
                    scene_value(Scene3D {
                        obj: SceneObject::Geometry(Shape::Heightmap {
                            rows: rows.len() as u32,
                            cols: cols.unwrap_or(0) as u32,
                            heights,
                        }),
                        xform: Matrix4::from_scale(1.0),
                    })
                }
                _ => usage(
                    "Scene.heightmap([[height, …], …]) — a list of at \
least 2 rows, each an equal-length list of at least 2 numbers",
                ),
            },
            "Scene.group" => match args.as_slice() {
                [Value::List(items)] => {
                    let mut scenes = Vec::with_capacity(items.len());
                    for item in items.iter() {
                        match scene_of(item) {
                            Some(scene) => scenes.push(scene.clone()),
                            None => {
                                return err(format!(
                                    "Scene.group items must be Scenes, got {}",
                                    item.kind_name()
                                ))
                            }
                        }
                    }
                    scene_value(group(scenes, Matrix4::from_scale(1.0)))
                }
                _ => usage("Scene.group([scene, …])"),
            },
            // Scene first, so they pipe: `Scene.cube() |> Scene.lit(r, g, b)`.
            "Scene.lit" | "Scene.emissive" => match args.as_slice() {
                [scene, r, g, b] => {
                    let (r, g, b) = (
                        num(r, span)? as f32,
                        num(g, span)? as f32,
                        num(b, span)? as f32,
                    );
                    let Some(scene) = scene_of(scene) else {
                        return usage(&format!("{path}(scene, r, g, b)"));
                    };
                    let material = if path == "Scene.lit" {
                        MaterialDescription::lit(r, g, b, 1.0)
                    } else {
                        MaterialDescription::emissive(r, g, b, 1.0)
                    };
                    scene_value(Scene3D {
                        obj: SceneObject::Material(material, vec![scene.clone()]),
                        xform: Matrix4::from_scale(1.0),
                    })
                }
                _ => usage(&format!("{path}(scene, r, g, b)")),
            },
            // An image texture by file path (relative to the game dir), the
            // MLE face of F#'s `Texture.file`. Loading is the shells' asset
            // pipeline; a missing file logs an error and renders as the
            // fallback texture.
            "Texture.file" => match args.as_slice() {
                [Value::String(path)] if !path.is_empty() => Ok(host(MleTexture(
                    TextureDescription::File(path.to_string()),
                ))),
                _ => usage(
                    "Texture.file(\"file.png\") — a non-empty image path relative to \
the game dir",
                ),
            },
            // Scene first, so they pipe:
            // `Scene.plane() |> Scene.litTexture(Texture.file("dirt.png"))`.
            // The F# pair `Material.litTexture` / `Material.emissiveTexture`:
            // lit is shaded by the frame's lights (white albedo tint),
            // emissive renders fullbright (neon signage).
            "Scene.litTexture" | "Scene.emissiveTexture" => match args.as_slice() {
                [scene, texture] => {
                    let Some(scene) = scene_of(scene) else {
                        return usage(&format!("{path}(scene, texture)"));
                    };
                    let texture = texture_of(texture, path, span)?;
                    let material = if path == "Scene.litTexture" {
                        MaterialDescription::lit_texture(texture.clone())
                    } else {
                        MaterialDescription::emissive_texture(texture.clone())
                    };
                    scene_value(Scene3D {
                        obj: SceneObject::Material(material, vec![scene.clone()]),
                        xform: Matrix4::from_scale(1.0),
                    })
                }
                _ => usage(&format!("{path}(scene, texture)")),
            },
            // Scene first, so it pipes: `Scene.cube() |> Scene.color(r, g, b)`.
            "Scene.color" => match args.as_slice() {
                [scene, r, g, b] => {
                    let (r, g, b) = (num(r, span)?, num(g, span)?, num(b, span)?);
                    let Some(scene) = scene_of(scene) else {
                        return usage("Scene.color(scene, r, g, b)");
                    };
                    scene_value(Scene3D {
                        obj: SceneObject::Material(
                            MaterialDescription::color(r as f32, g as f32, b as f32, 1.0),
                            vec![scene.clone()],
                        ),
                        xform: Matrix4::from_scale(1.0),
                    })
                }
                _ => usage("Scene.color(scene, r, g, b)"),
            },
            "Scene.translate" => match args.as_slice() {
                [scene, x, y, z] => {
                    let xform = Matrix4::from_translation(cgmath::vec3(
                        num(x, span)? as f32,
                        num(y, span)? as f32,
                        num(z, span)? as f32,
                    ));
                    wrap_transform(scene, xform, "Scene.translate(scene, x, y, z)", span)
                }
                _ => usage("Scene.translate(scene, x, y, z)"),
            },
            "Angle.degrees" => match args.as_slice() {
                [n] => Ok(host(MleAngle(Angle::from_degrees(num(n, span)? as f32)))),
                _ => usage("Angle.degrees(n)"),
            },
            "Angle.radians" => match args.as_slice() {
                [n] => Ok(host(MleAngle(Angle::from_radians(num(n, span)? as f32)))),
                _ => usage("Angle.radians(n)"),
            },
            "Scene.rotateX" | "Scene.rotateY" | "Scene.rotateZ" => match args.as_slice() {
                [scene, angle] => {
                    let angle: cgmath::Rad<f32> = angle_of(angle, path, span)?.into();
                    let xform = match path {
                        "Scene.rotateX" => Matrix4::from_angle_x(angle),
                        "Scene.rotateY" => Matrix4::from_angle_y(angle),
                        _ => Matrix4::from_angle_z(angle),
                    };
                    wrap_transform(scene, xform, &format!("{path}(scene, angle)"), span)
                }
                _ => return usage(&format!("{path}(scene, angle)")),
            },
            "Scene.scale" => match args.as_slice() {
                [scene, k] => {
                    let xform = Matrix4::from_scale(num(k, span)? as f32);
                    wrap_transform(scene, xform, "Scene.scale(scene, k)", span)
                }
                _ => usage("Scene.scale(scene, k)"),
            },
            "Camera.lookAt" => match args.as_slice() {
                [ex, ey, ez, tx, ty, tz] => Ok(host(MleCamera(Camera::look_at(
                    [
                        num(ex, span)? as f32,
                        num(ey, span)? as f32,
                        num(ez, span)? as f32,
                    ],
                    [
                        num(tx, span)? as f32,
                        num(ty, span)? as f32,
                        num(tz, span)? as f32,
                    ],
                    [0.0, 1.0, 0.0],
                    Angle::from_degrees(45.0),
                )))),
                _ => usage("Camera.lookAt(ex, ey, ez, tx, ty, tz)"),
            },
            "Camera.firstPerson" => match args.as_slice() {
                [ex, ey, ez, yaw, pitch, fov] => Ok(host(MleCamera(Camera::first_person(
                    [
                        num(ex, span)? as f32,
                        num(ey, span)? as f32,
                        num(ez, span)? as f32,
                    ],
                    angle_of(yaw, "Camera.firstPerson yaw", span)?,
                    angle_of(pitch, "Camera.firstPerson pitch", span)?,
                    angle_of(fov, "Camera.firstPerson fov", span)?,
                )))),
                _ => usage(
                    "Camera.firstPerson(ex, ey, ez, yaw, pitch, fov) — Angle values \
(Angle.degrees/Angle.radians)",
                ),
            },
            "Light.ambient" => match args.as_slice() {
                [r, g, b] => Ok(host(MleLight(Light::ambient(
                    num(r, span)? as f32,
                    num(g, span)? as f32,
                    num(b, span)? as f32,
                )))),
                _ => usage("Light.ambient(r, g, b)"),
            },
            "Light.directional" => match args.as_slice() {
                [dx, dy, dz, r, g, b, intensity] => Ok(host(MleLight(Light::directional(
                    num(dx, span)? as f32,
                    num(dy, span)? as f32,
                    num(dz, span)? as f32,
                    num(r, span)? as f32,
                    num(g, span)? as f32,
                    num(b, span)? as f32,
                    num(intensity, span)? as f32,
                )))),
                _ => usage("Light.directional(dx, dy, dz, r, g, b, intensity)"),
            },
            "Light.point" => match args.as_slice() {
                [px, py, pz, r, g, b, intensity, range] => Ok(host(MleLight(Light::point(
                    num(px, span)? as f32,
                    num(py, span)? as f32,
                    num(pz, span)? as f32,
                    num(r, span)? as f32,
                    num(g, span)? as f32,
                    num(b, span)? as f32,
                    num(intensity, span)? as f32,
                    num(range, span)? as f32,
                )))),
                _ => usage("Light.point(px, py, pz, r, g, b, intensity, range)"),
            },
            // Light first, so it pipes: `Light.directional(…) |> Light.castShadows`.
            "Light.castShadows" => match args.as_slice() {
                [light] => match light_of(light) {
                    Some(inner) => Ok(host(MleLight(inner.clone().cast_shadows()))),
                    None => usage("Light.castShadows(light)"),
                },
                _ => usage("Light.castShadows(light)"),
            },
            "Frame.createLit" => match args.as_slice() {
                [camera, scene, Value::List(lights)] => {
                    let (Value::HostData(cam), Some(scene)) = (camera, scene_of(scene)) else {
                        return usage("Frame.createLit(camera, scene, [light, …])");
                    };
                    let Some(camera) = cam.as_any().downcast_ref::<MleCamera>() else {
                        return usage("Frame.createLit(camera, scene, [light, …])");
                    };
                    let mut lit = Vec::with_capacity(lights.len());
                    for light in lights.iter() {
                        match light_of(light) {
                            Some(inner) => lit.push(inner.clone()),
                            None => {
                                return err(format!(
                                    "Frame.createLit lights must be Lights, got {}",
                                    light.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(host(MleFrame(Frame {
                        camera: camera.0.clone(),
                        scene: scene.clone(),
                        lights: lit,
                        render_targets: vec![],
                        fog: None,
                        skybox: None,
                    })))
                }
                _ => usage("Frame.createLit(camera, scene, [light, …])"),
            },
            // ── Physics (docs/physics.md; the declarative surface) ─────────
            // Shapes are values, bodies are tag + shape + piped attributes,
            // and the optional game hook `physics = (model) => Physics.scene(…)`
            // declares the world each frame.
            "Physics.box" => match args.as_slice() {
                [w, h, d] => Ok(host(MleShape(physics::Shape::Cuboid {
                    extents: [
                        positive_num(w, span, "Physics.box width")? as f32,
                        positive_num(h, span, "Physics.box height")? as f32,
                        positive_num(d, span, "Physics.box depth")? as f32,
                    ],
                }))),
                _ => usage("Physics.box(width, height, depth)"),
            },
            "Physics.sphere" => match args.as_slice() {
                [r] => Ok(host(MleShape(physics::Shape::Sphere {
                    radius: positive_num(r, span, "Physics.sphere radius")? as f32,
                }))),
                _ => usage("Physics.sphere(radius)"),
            },
            "Physics.capsule" => match args.as_slice() {
                [half_height, r] => Ok(host(MleShape(physics::Shape::Capsule {
                    half_height: positive_num(half_height, span, "Physics.capsule halfHeight")?
                        as f32,
                    radius: positive_num(r, span, "Physics.capsule radius")? as f32,
                }))),
                _ => usage("Physics.capsule(halfHeight, radius)"),
            },
            "Physics.dynamic" | "Physics.kinematic" | "Physics.fixed" => match args.as_slice() {
                [Value::String(tag), shape] => match shape_of(shape) {
                    Some(shape) => {
                        let tag = tag.to_string();
                        let shape = shape.clone();
                        Ok(host(MleBody(match path {
                            "Physics.dynamic" => physics::Body::dynamic(tag, shape),
                            "Physics.kinematic" => physics::Body::kinematic(tag, shape),
                            _ => physics::Body::fixed(tag, shape),
                        })))
                    }
                    None => usage(&format!("{path}(tag, shape)")),
                },
                _ => usage(&format!("{path}(tag, shape)")),
            },
            // Body first, so they pipe:
            // `Physics.dynamic("crate", Physics.box(1.0, 1.0, 1.0)) |> Physics.at(0.0, 5.0, 0.0)`.
            "Physics.at" | "Physics.velocity" => match args.as_slice() {
                [body, x, y, z] => match body_of(body) {
                    Some(inner) => {
                        let v = [
                            num(x, span)? as f32,
                            num(y, span)? as f32,
                            num(z, span)? as f32,
                        ];
                        Ok(host(MleBody(if path == "Physics.at" {
                            inner.clone().at(v)
                        } else {
                            inner.clone().with_velocity(v)
                        })))
                    }
                    None => usage(&format!("{path}(body, x, y, z)")),
                },
                _ => usage(&format!("{path}(body, x, y, z)")),
            },
            "Physics.mass" | "Physics.friction" | "Physics.restitution" => match args.as_slice() {
                [body, n] => match body_of(body) {
                    Some(inner) => {
                        let n = match path {
                            "Physics.mass" => positive_num(n, span, "Physics.mass")?,
                            _ => non_negative_num(n, span, path)?,
                        } as f32;
                        Ok(host(MleBody(match path {
                            "Physics.mass" => inner.clone().with_mass(n),
                            "Physics.friction" => inner.clone().with_friction(n),
                            _ => inner.clone().with_restitution(n),
                        })))
                    }
                    None => usage(&format!("{path}(body, n)")),
                },
                _ => usage(&format!("{path}(body, n)")),
            },
            "Physics.sensor" => match args.as_slice() {
                [body] => match body_of(body) {
                    Some(inner) => Ok(host(MleBody(inner.clone().as_sensor()))),
                    None => usage("Physics.sensor(body)"),
                },
                _ => usage("Physics.sensor(body)"),
            },
            "Physics.scene" => match args.as_slice() {
                [gx, gy, gz, Value::List(items)] => {
                    let gravity = [
                        num(gx, span)? as f32,
                        num(gy, span)? as f32,
                        num(gz, span)? as f32,
                    ];
                    let mut bodies = Vec::with_capacity(items.len());
                    for item in items.iter() {
                        match body_of(item) {
                            Some(body) => bodies.push(body.clone()),
                            None => {
                                return err(format!(
                                    "Physics.scene bodies must be Bodies, got {}",
                                    item.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(host(MlePhysicsScene(physics::PhysicsScene::create(
                        gravity, bodies,
                    ))))
                }
                _ => usage("Physics.scene(gx, gy, gz, [body, …])"),
            },
            // Reads of the LIVE stepped world (the singleton, world 0). MLE
            // runs in the same process as the world the shell steps, so these
            // are direct reads — no boundary, no copy (the dylib producers
            // can't do this; MLE can). A tag that isn't in the world is a
            // loud spanned error — declare the body before reading. (An
            // Option-shaped variant return could come now that B5 match
            // exists, but loud-by-default is right for the common case.)
            "Physics.position" => match args.as_slice() {
                [Value::String(tag)] => match live_transform(tag) {
                    Some((pos, _)) => Ok(Value::Record(Rc::new(vec![
                        ("x".to_string(), Value::Number(pos[0] as f64)),
                        ("y".to_string(), Value::Number(pos[1] as f64)),
                        ("z".to_string(), Value::Number(pos[2] as f64)),
                    ]))),
                    None => err(no_body(tag)),
                },
                _ => usage("Physics.position(tag)"),
            },
            // Scene first, so it pipes: the way MLE draws a physics body —
            // `Scene.cube() |> Scene.lit(…) |> Physics.transformed("crate-1")`
            // places the visual at the body's live pose (position + rotation).
            // Command EFFECTS (docs/physics.md Phase 3): fire-and-forget,
            // returned beside the model like any effect —
            // `(model, Physics.applyImpulse("ball", 0.0, 5.0, 0.0))`.
            // Performing one queues it on the singleton world; it applies at
            // the next stepped frame's first substep, AFTER reconcile — so a
            // body declared and commanded in the same frame works.
            "Physics.applyImpulse" | "Physics.applyForce" | "Physics.setVelocity"
            | "Physics.teleport" => match args.as_slice() {
                [Value::String(tag), x, y, z] => {
                    let tag = tag.to_string();
                    let v = [
                        num(x, span)? as f32,
                        num(y, span)? as f32,
                        num(z, span)? as f32,
                    ];
                    let command = match path {
                        "Physics.applyImpulse" => {
                            physics::PhysicsCommand::ApplyImpulse { tag, impulse: v }
                        }
                        "Physics.applyForce" => {
                            physics::PhysicsCommand::ApplyForce { tag, force: v }
                        }
                        "Physics.setVelocity" => {
                            physics::PhysicsCommand::SetVelocity { tag, velocity: v }
                        }
                        _ => physics::PhysicsCommand::Teleport { tag, position: v },
                    };
                    Ok(host(MleEffect(EffectTree::Physics(command))))
                }
                _ => usage(&format!("{path}(tag, x, y, z)")),
            },
            // Query EFFECT (docs/physics.md Phase 4): deferred until after
            // the frame's physics step, then the tagger receives the result
            // record `{hit, x, y, z, nx, ny, nz, distance, tag}` (hit: false
            // with zeroed fields for a miss) — fresh, same-frame.
            "Physics.raycast" => match args.as_slice() {
                [ox, oy, oz, dx, dy, dz, max_dist, tagger @ (Value::Closure(_) | Value::Ctor { .. })] =>
                {
                    let origin = [
                        num(ox, span)? as f32,
                        num(oy, span)? as f32,
                        num(oz, span)? as f32,
                    ];
                    let dir = [
                        num(dx, span)? as f32,
                        num(dy, span)? as f32,
                        num(dz, span)? as f32,
                    ];
                    if dir == [0.0, 0.0, 0.0] {
                        return err("Physics.raycast: the direction must not be zero".to_string());
                    }
                    let max_dist =
                        positive_num(max_dist, span, "Physics.raycast maxDist")? as f32;
                    Ok(host(MleEffect(EffectTree::Raycast {
                        origin,
                        dir,
                        max_dist,
                        tagger: tagger.clone(),
                    })))
                }
                [_, _, _, _, _, _, _, other] => err(format!(
                    "Physics.raycast(ox, oy, oz, dx, dy, dz, maxDist, tagger): the tagger \
                     must be a function of the result record, got {}",
                    other.kind_name()
                )),
                _ => usage("Physics.raycast(ox, oy, oz, dx, dy, dz, maxDist, tagger)"),
            },
            // Collision-event SUB (docs/physics.md Phase 5): what
            // `subscriptions` returns (alone or in Sub.batch). The tagger
            // receives {started, a, b, sensor} per contact begin/end,
            // post-step (like query answers).
            "Physics.events" => match args.as_slice() {
                [tagger @ (Value::Closure(_) | Value::Ctor { .. })] => Ok(host(MleSub(
                    SubTree::PhysicsEvents {
                        tagger: tagger.clone(),
                    },
                ))),
                [other] => err(format!(
                    "Physics.events(tagger): the tagger must be a function of the event record, got {}",
                    other.kind_name()
                )),
                _ => usage("Physics.events(tagger)"),
            },
            "Physics.transformed" => match args.as_slice() {
                [scene, Value::String(tag)] => {
                    let Some(inner) = scene_of(scene) else {
                        return usage("Physics.transformed(scene, tag)");
                    };
                    match live_transform(tag) {
                        Some((pos, rot)) => {
                            // cgmath's Quaternion::new is scalar-FIRST (w, x, y, z).
                            let rotation = cgmath::Quaternion::new(rot[3], rot[0], rot[1], rot[2]);
                            let xform =
                                Matrix4::from_translation(cgmath::vec3(pos[0], pos[1], pos[2]))
                                    * Matrix4::from(rotation);
                            scene_value(group(vec![inner.clone()], xform))
                        }
                        None => err(no_body(tag)),
                    }
                }
                _ => usage("Physics.transformed(scene, tag)"),
            },
            "RenderTarget.named" => match args.as_slice() {
                [Value::String(name)] if !name.is_empty() => Ok(host(MleRenderTarget(
                    RenderTargetDescriptor::new(name.to_string()),
                ))),
                _ => usage(
                    "RenderTarget.named(\"id\") — a non-empty name; 512x512 unless \
piped through RenderTarget.sized",
                ),
            },
            // Target first, so it pipes:
            // `RenderTarget.named("x") |> RenderTarget.sized(256.0, 256.0)`.
            "RenderTarget.sized" => match args.as_slice() {
                [target, w, h] => {
                    let inner = target_of(target, "RenderTarget.sized", span)?;
                    let w = positive_num(w, span, "RenderTarget.sized width")?;
                    let h = positive_num(h, span, "RenderTarget.sized height")?;
                    Ok(host(MleRenderTarget(
                        inner.clone().sized(w as f32, h as f32),
                    )))
                }
                _ => usage("RenderTarget.sized(target, width, height)"),
            },
            // Frame first, so it pipes:
            // `Frame.createLit(…) |> Frame.withRenderTarget(feed, feedFrame)`.
            "Frame.withRenderTarget" => match args.as_slice() {
                [frame, target, target_frame] => {
                    let (Some(outer), Some(inner)) =
                        (frame_value(frame), frame_value(target_frame))
                    else {
                        return usage(
                            "Frame.withRenderTarget(frame, target, targetFrame) — targetFrame \
is a Frame.create/createLit(…) rendered into the target each frame, before \
frame's main pass",
                        );
                    };
                    let target = target_of(target, "Frame.withRenderTarget", span)?;
                    Ok(host(MleFrame(Frame::with_render_target(
                        outer.clone(),
                        target.clone(),
                        inner.clone(),
                    ))))
                }
                _ => usage("Frame.withRenderTarget(frame, target, targetFrame)"),
            },
            "Fog.linear" => match args.as_slice() {
                [near, far, r, g, b] => {
                    let near = non_negative_num(near, span, "Fog.linear near")?;
                    let far = positive_num(far, span, "Fog.linear far")?;
                    if far <= near {
                        return err(format!(
                            "Fog.linear: far ({far}) must be greater than near ({near})"
                        ));
                    }
                    Ok(host(MleFog(Fog::linear(
                        near as f32,
                        far as f32,
                        num(r, span)? as f32,
                        num(g, span)? as f32,
                        num(b, span)? as f32,
                    ))))
                }
                _ => usage("Fog.linear(near, far, r, g, b)"),
            },
            "Fog.exp" => match args.as_slice() {
                [density, r, g, b] => Ok(host(MleFog(Fog::exp(
                    positive_num(density, span, "Fog.exp density")? as f32,
                    num(r, span)? as f32,
                    num(g, span)? as f32,
                    num(b, span)? as f32,
                )))),
                _ => usage("Fog.exp(density, r, g, b)"),
            },
            // Frame first, so it pipes: `Frame.createLit(…) |> Frame.withFog(fog)`.
            "Frame.withFog" => match args.as_slice() {
                [frame, fog] => {
                    let Some(inner) = frame_value(frame) else {
                        return usage("Frame.withFog(frame, fog)");
                    };
                    let fog = fog_of(fog, "Frame.withFog", span)?;
                    Ok(host(MleFrame(Frame::with_fog(inner.clone(), fog.clone()))))
                }
                _ => usage("Frame.withFog(frame, fog)"),
            },
            // Scene first, so it pipes: `Scene.quad() |> Scene.screen(feed)` —
            // an emissive (fullbright, screens glow) surface showing the
            // target's texture. A target no frame declares shows magenta.
            "Scene.screen" => match args.as_slice() {
                [scene, target] => {
                    let Some(scene) = scene_of(scene) else {
                        return usage("Scene.screen(scene, target)");
                    };
                    let target = target_of(target, "Scene.screen", span)?;
                    scene_value(Scene3D {
                        obj: SceneObject::Material(
                            MaterialDescription::emissive_texture(
                                TextureDescription::render_target(target.clone()),
                            ),
                            vec![scene.clone()],
                        ),
                        xform: Matrix4::from_scale(1.0),
                    })
                }
                _ => usage("Scene.screen(scene, target)"),
            },
            "Frame.create" => match args.as_slice() {
                [camera, scene] => {
                    let (Value::HostData(cam), Some(scene)) = (camera, scene_of(scene)) else {
                        return usage("Frame.create(camera, scene)");
                    };
                    let Some(camera) = cam.as_any().downcast_ref::<MleCamera>() else {
                        return usage("Frame.create(camera, scene)");
                    };
                    Ok(host(MleFrame(Frame::new(camera.0.clone(), scene.clone()))))
                }
                _ => usage("Frame.create(camera, scene)"),
            },
            // ── Ui (the optional `ui = (model) => …` hook) ─────────────────
            // hello's HUD vocabulary: text lines (white or colored), stacked
            // in a column, pinned to a screen corner. The shells lower the
            // View through the same egui overlay as the F# `ui` hook.
            "Ui.text" => match args.as_slice() {
                [Value::String(s)] => Ok(host(MleView(View::Text {
                    text: s.to_string(),
                    color: [255, 255, 255],
                    font: None,
                }))),
                _ => usage("Ui.text(\"…\")"),
            },
            "Ui.textColor" => match args.as_slice() {
                [r, g, b, Value::String(s)] => Ok(host(MleView(View::Text {
                    text: s.to_string(),
                    color: ui::rgb_u8(
                        num(r, span)? as f32,
                        num(g, span)? as f32,
                        num(b, span)? as f32,
                    ),
                    font: None,
                }))),
                _ => usage("Ui.textColor(r, g, b, \"…\")"),
            },
            "Ui.column" => match args.as_slice() {
                [Value::List(items)] => {
                    let mut views = Vec::with_capacity(items.len());
                    for item in items.iter() {
                        match view_value(item) {
                            Some(view) => views.push(view.clone()),
                            None => {
                                return err(format!(
                                    "Ui.column items must be Views, got {}",
                                    item.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(host(MleView(View::Column(views))))
                }
                _ => usage("Ui.column([view, …])"),
            },
            // View first, so it pipes:
            // `Ui.column([…]) |> Ui.panel(Ui.topLeft())`. Only the corner
            // hello's HUD needed exists; the other three arrive with a port
            // that needs them.
            "Ui.panel" => match args.as_slice() {
                [view, anchor] => {
                    let Some(view) = view_value(view) else {
                        return usage("Ui.panel(view, anchor)");
                    };
                    let anchor = ui_anchor_of(anchor, span)?;
                    Ok(host(MleView(View::Panel {
                        anchor,
                        child: Box::new(view.clone()),
                    })))
                }
                _ => usage("Ui.panel(view, anchor)"),
            },
            "Ui.topLeft" => match args.as_slice() {
                [] => Ok(host(MleUiAnchor(ui::Anchor::TopLeft))),
                _ => usage("Ui.topLeft()"),
            },
            "Time.seconds" => match args.as_slice() {
                [n] => Ok(host(MleDuration(num(n, span)?))),
                _ => usage("Time.seconds(n)"),
            },
            "Time.millis" => match args.as_slice() {
                [n] => Ok(host(MleDuration(num(n, span)? / 1000.0))),
                _ => usage("Time.millis(n)"),
            },
            "Sub.none" => match args.as_slice() {
                [] => Ok(host(MleSub(SubTree::None))),
                _ => usage("Sub.none()"),
            },
            // The msg is any MLE value (typically an ADT variant), held by
            // the host and handed back verbatim when the timer fires.
            "Sub.every" => match args.as_slice() {
                [duration, msg] => Ok(host(MleSub(SubTree::Every {
                    period_seconds: duration_of(duration, "Sub.every", span)?,
                    msg: msg.clone(),
                }))),
                _ => usage("Sub.every(duration, msg)"),
            },
            "Effect.none" => match args.as_slice() {
                [] => Ok(host(MleEffect(EffectTree::None))),
                _ => usage("Effect.none()"),
            },
            // The tagger is an MLE function value, applied by the producer
            // with the performed result; validated as callable here so a
            // `Effect.now(3.0)` fails at construction, not mid-drain.
            "Effect.now" | "Effect.random" => match args.as_slice() {
                [tagger @ (Value::Closure(_) | Value::Ctor { .. })] => {
                    let tree = if path == "Effect.now" {
                        EffectTree::Now {
                            tagger: tagger.clone(),
                        }
                    } else {
                        EffectTree::Random {
                            tagger: tagger.clone(),
                        }
                    };
                    Ok(host(MleEffect(tree)))
                }
                [other] => err(format!(
                    "{path}(tagger): the tagger must be a function of the result, got {}",
                    other.kind_name()
                )),
                _ => usage(&format!("{path}(tagger)")),
            },
            "Effect.batch" => match args.as_slice() {
                [Value::List(items)] => {
                    let mut fx = Vec::with_capacity(items.len());
                    for item in items.iter() {
                        match effect_of(item) {
                            Some(effect) => fx.push(effect.0.clone()),
                            None => {
                                return err(format!(
                                    "Effect.batch items must be Effects, got {}",
                                    item.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(host(MleEffect(EffectTree::Batch(fx))))
                }
                _ => usage("Effect.batch([effect, …])"),
            },
            "Sub.batch" => match args.as_slice() {
                [Value::List(items)] => {
                    let mut subs = Vec::with_capacity(items.len());
                    for item in items.iter() {
                        match sub_of(item) {
                            Some(sub) => subs.push(sub.0.clone()),
                            None => {
                                return err(format!(
                                    "Sub.batch items must be Subs, got {}",
                                    item.kind_name()
                                ))
                            }
                        }
                    }
                    Ok(host(MleSub(SubTree::Batch(subs))))
                }
                _ => usage("Sub.batch([sub, …])"),
            },
            _ => err(format!("internal: unregistered prelude path `{path}`")),
        }
    }
}

/// Protocol scalars must be finite f32s: NaN/inf (which MLE numbers permit —
/// IEEE division) and f64s beyond f32 range are spanned errors here rather
/// than non-finite matrices inside the renderer.
fn num(value: &Value, span: Span) -> Result<f64, RunError> {
    match value {
        Value::Number(n) if (*n as f32).is_finite() => Ok(*n),
        Value::Number(n) => Err(RunError {
            message: format!("expected a finite number, got {n}"),
            span,
        }),
        other => Err(RunError {
            message: format!("expected a number, got {}", other.kind_name()),
            span,
        }),
    }
}

/// Extract an [`Angle`] — rotation/camera functions accept ONLY Angle values,
/// so a bare number gets a teaching error instead of a silent unit guess.
fn angle_of(value: &Value, what: &str, span: Span) -> Result<Angle, RunError> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<MleAngle>()
            .map(|a| a.0)
            .ok_or_else(|| RunError {
                message: format!("{what}: expected an Angle, got {}", value.kind_name()),
                span,
            }),
        Value::Number(_) => Err(RunError {
            message: format!(
                "{what}: expected an Angle, got a bare number — say which unit: \
Angle.degrees(…) or Angle.radians(…)"
            ),
            span,
        }),
        other => Err(RunError {
            message: format!("{what}: expected an Angle, got {}", other.kind_name()),
            span,
        }),
    }
}

/// Extract a duration in seconds — timing functions accept ONLY Duration
/// values, so a bare number gets a teaching error instead of a silent unit
/// guess (the [`angle_of`] rule, applied to time).
fn duration_of(value: &Value, what: &str, span: Span) -> Result<f64, RunError> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<MleDuration>()
            .map(|d| d.0)
            .ok_or_else(|| RunError {
                message: format!("{what}: expected a Duration, got {}", value.kind_name()),
                span,
            }),
        Value::Number(_) => Err(RunError {
            message: format!(
                "{what}: expected a Duration, got a bare number — say which unit: \
Time.seconds(…) or Time.millis(…)"
            ),
            span,
        }),
        other => Err(RunError {
            message: format!("{what}: expected a Duration, got {}", other.kind_name()),
            span,
        }),
    }
}

fn sub_of(value: &Value) -> Option<&MleSub> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<MleSub>(),
        _ => None,
    }
}

/// The messages a subscription tree fires in the frame interval
/// `(prev_tts, tts]`, in declaration order. `subs` is the raw value the
/// game's `subscriptions(model)` returned — a non-Sub is an error string the
/// producer reports like any other per-frame error.
pub fn sub_messages_for_frame(subs: &Value, prev_tts: f64, tts: f64) -> Result<Vec<Value>, String> {
    let Some(sub) = sub_of(subs) else {
        return Err(format!(
            "subscriptions must return a Sub (Sub.every / Sub.none / Sub.batch), got {}",
            subs.kind_name()
        ));
    };
    let mut msgs = Vec::new();
    collect_fired(&sub.0, prev_tts, tts, &mut msgs);
    Ok(msgs)
}

fn collect_fired(sub: &SubTree, prev_tts: f64, tts: f64, msgs: &mut Vec<Value>) {
    match sub {
        SubTree::None => {}
        SubTree::Every {
            period_seconds: p,
            msg,
        } => {
            // An integer multiple of the period lies in (prev_tts, tts] —
            // the F# runtime's `Sub.crossedBoundary`, verbatim.
            if *p > 0.0 && (tts / p).floor() > (prev_tts / p).floor() {
                msgs.push(msg.clone());
            }
        }
        SubTree::Batch(items) => {
            for item in items.iter() {
                collect_fired(item, prev_tts, tts, msgs);
            }
        }
        // Event subs fire from the physics step, not the time grid — the
        // drivers collect their taggers via `physics_event_taggers`.
        SubTree::PhysicsEvents { .. } => {}
    }
}

/// The `Physics.events` taggers in a subscription tree, in declaration
/// order — the drivers apply each to every event record from this frame's
/// physics step and fold the messages through `update`, post-step. A
/// non-Sub value yields the same error `sub_messages_for_frame` reports.
pub fn physics_event_taggers(subs: &Value) -> Result<Vec<Value>, String> {
    let Some(sub) = sub_of(subs) else {
        return Err(format!(
            "subscriptions must return a Sub (Sub.every / Sub.none / Sub.batch), got {}",
            subs.kind_name()
        ));
    };
    let mut taggers = Vec::new();
    collect_event_taggers(&sub.0, &mut taggers);
    Ok(taggers)
}

fn collect_event_taggers(sub: &SubTree, taggers: &mut Vec<Value>) {
    match sub {
        SubTree::None | SubTree::Every { .. } => {}
        SubTree::Batch(items) => {
            for item in items.iter() {
                collect_event_taggers(item, taggers);
            }
        }
        SubTree::PhysicsEvents { tagger } => taggers.push(tagger.clone()),
    }
}

/// The tagger-facing record for one collision event.
pub fn physics_event_value(event: &physics::PhysicsEvent) -> Value {
    Value::Record(Rc::new(vec![
        ("started".to_string(), Value::Bool(event.started)),
        ("a".to_string(), Value::String(Rc::from(event.a.as_str()))),
        ("b".to_string(), Value::String(Rc::from(event.b.as_str()))),
        ("sensor".to_string(), Value::Bool(event.sensor)),
    ]))
}

/// Deliver this frame's collision events to the game: every tagger from the
/// current `subscriptions(model)` × every event, folded through `update` at
/// the post-step point (chained effects drain post-step: further queries
/// answer immediately, further commands queue for the next step).
pub fn deliver_physics_events(
    session: &mle::Session,
    model: &mut Value,
    taggers: &[Value],
    events: &[physics::PhysicsEvent],
    runner: &mut dyn EffectRunner,
    log: &mut EffectLog,
    report: &mut dyn FnMut(String),
) {
    // Events outer: the frame's causal contact sequence is the primary fold
    // order (each event reaches every tagger before the next event).
    for event in events {
        for tagger in taggers {
            let msg = match session.apply(
                tagger.clone(),
                vec![physics_event_value(event)],
                "Physics.events tagger",
                &mut FunctorHost,
            ) {
                Ok(msg) => msg,
                Err(e) => {
                    report(format!("[mle] Physics.events tagger error: {}", e.message));
                    continue;
                }
            };
            match session.call("update", vec![model.clone(), msg], &mut FunctorHost) {
                Ok(returned) => {
                    let (next_model, more) = split_model_effect(returned);
                    *model = next_model;
                    if let Some(more) = more {
                        drain_mode(session, model, vec![more], runner, log, report, None);
                    }
                }
                Err(e) => report(format!("[mle] update error: {}", e.message)),
            }
        }
    }
}

/// Physical dimensions (shape extents, radii, mass) must be strictly
/// positive: Rapier accepts a negative radius and silently builds a
/// degenerate collider that misbehaves far from the declaration — so reject
/// it loud at the boundary.
fn positive_num(value: &Value, span: Span, what: &str) -> Result<f64, RunError> {
    let n = num(value, span)?;
    if n > 0.0 {
        Ok(n)
    } else {
        Err(RunError {
            message: format!("{what} must be positive, got {n}"),
            span,
        })
    }
}

/// Friction/restitution are coefficients: zero is meaningful, negative is not.
fn non_negative_num(value: &Value, span: Span, what: &str) -> Result<f64, RunError> {
    let n = num(value, span)?;
    if n >= 0.0 {
        Ok(n)
    } else {
        Err(RunError {
            message: format!("{what} must not be negative, got {n}"),
            span,
        })
    }
}

fn effect_of(value: &Value) -> Option<&MleEffect> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<MleEffect>(),
        _ => None,
    }
}

/// Split an entry point's return into (model, effect). The pair contract:
/// a 2-tuple whose SECOND element is an Effect value means "model plus
/// commands" — anything else is just the model. Effects are COMMANDS, not
/// data: one stored inside the model would make this sniff ambiguous
/// (`(0.0, Effect.none())` as a model would be mis-split), so producers
/// refuse an Effect inside `init` at load and warn if one appears in an
/// adopted model ([`contains_effect`]).
pub fn split_model_effect(value: Value) -> (Value, Option<EffectTree>) {
    if let Value::Tuple(items) = &value {
        if items.len() == 2 {
            if let Some(effect) = effect_of(&items[1]) {
                let tree = effect.0.clone();
                return (items[0].clone(), Some(tree));
            }
        }
    }
    (value, None)
}

/// Does an Effect value lurk anywhere inside `value`? Effects are commands,
/// not model data — see [`split_model_effect`]. Early-exits; containers
/// only (host values are opaque and cannot hold MLE values).
pub fn contains_effect(value: &Value) -> bool {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<MleEffect>().is_some(),
        Value::Tuple(items) | Value::List(items) => items.iter().any(contains_effect),
        Value::Record(fields) => fields.iter().any(|(_, v)| contains_effect(v)),
        Value::Variant { args, .. } => args.iter().any(contains_effect),
        _ => false,
    }
}

/// Perform `first` and drain to a fixed point: each performed effect's
/// tagger result is folded through `update`, whose return may carry MORE
/// effects — capped so a self-sustaining chain cannot hang the frame (the
/// F# executor's `maxEffectsPerFrame` discipline). Every performed effect
/// is appended to `log` (the structured effect log; replay's input).
/// Errors report through `report` (deduped by the producer) and drop that
/// effect — one bad tagger must not stall the rest.
///
/// Physics QUERIES are not performed here: they come back in the returned
/// list for the driver to hold until after the frame's physics step, then
/// hand to [`perform_deferred_queries`] — so their taggers answer against
/// the fresh world ("commands apply at the step; queries answer after it").
#[must_use = "deferred physics queries must be performed after the step"]
pub fn drain_effects(
    session: &mle::Session,
    model: &mut Value,
    first: EffectTree,
    runner: &mut dyn EffectRunner,
    log: &mut EffectLog,
    report: &mut dyn FnMut(String),
) -> Vec<EffectTree> {
    let mut deferred = Vec::new();
    drain_mode(
        session,
        model,
        vec![first],
        runner,
        log,
        report,
        Some(&mut deferred),
    );
    deferred
}

/// Does this effect tree produce MESSAGES (tagger results that must fold
/// through `update`)? Tagger-less trees — physics commands, `Effect.none`,
/// batches thereof — drain fine without an `update` hook; the producers'
/// "effects returned but there is no update" guard must only fire for trees
/// that actually need one. (The guard predates tagger-less effects; gating
/// on it unconditionally silently dropped physics commands from
/// update-less games.)
pub fn needs_update(tree: &EffectTree) -> bool {
    match tree {
        EffectTree::None | EffectTree::Physics(_) => false,
        EffectTree::Now { .. } | EffectTree::Random { .. } | EffectTree::Raycast { .. } => true,
        EffectTree::Batch(items) => items.iter().any(needs_update),
    }
}

/// The post-step half of the query story: perform queries deferred by
/// [`drain_effects`], folding their tagger messages through `update`.
/// Chained effects drain to the same fixed point — further queries answer
/// immediately (the world already stepped this frame); further commands
/// queue for the next frame's step, as always.
pub fn perform_deferred_queries(
    session: &mle::Session,
    model: &mut Value,
    deferred: Vec<EffectTree>,
    runner: &mut dyn EffectRunner,
    log: &mut EffectLog,
    report: &mut dyn FnMut(String),
) {
    if deferred.is_empty() {
        return;
    }
    drain_mode(session, model, deferred, runner, log, report, None);
}

fn drain_mode(
    session: &mle::Session,
    model: &mut Value,
    queue: Vec<EffectTree>,
    runner: &mut dyn EffectRunner,
    log: &mut EffectLog,
    report: &mut dyn FnMut(String),
    // `Some` = pre-step drain: hold queries here instead of performing.
    // `None` = post-step drain: answer queries now.
    mut defer_queries: Option<&mut Vec<EffectTree>>,
) {
    // Each drain invocation gets its own cap, so a frame that defers queries
    // is bounded by 2×MAX (pre-step + post-step) — the cap's job is to bound
    // runaway chains, not to meter exactly N.
    const MAX_EFFECTS_PER_FRAME: usize = 1000;
    let mut queue: Vec<EffectTree> = queue;
    queue.reverse(); // treat the input list as front-of-queue, in order
    // Counts every DEQUEUED node (None and Batch structure included), and is
    // checked BEFORE the runner performs anything — so a runaway chain can't
    // consume unbounded frame time through structural nodes, and the capped
    // effect is never half-performed (no runner state advanced, nothing
    // logged, no replay-log entry consumed).
    let mut processed = 0usize;
    while let Some(tree) = queue.pop() {
        processed += 1;
        if processed > MAX_EFFECTS_PER_FRAME {
            report(format!(
                "[mle] effect drain hit the per-frame cap ({MAX_EFFECTS_PER_FRAME}); \
dropping the rest"
            ));
            return;
        }
        let (kind, value, tagger) = match tree {
            EffectTree::None => continue,
            EffectTree::Batch(items) => {
                // Preserve declaration order against the LIFO queue.
                for item in items.into_iter().rev() {
                    queue.push(item);
                }
                continue;
            }
            EffectTree::Physics(command) => {
                // Fire-and-forget: queue on the singleton world, log the kind
                // and the target tag (there is no result value to feed a
                // tagger), continue. Not routed through the runner — commands
                // are per-frame *inputs* recorded by the physics Timeline,
                // not environment reads that replay must fake.
                let kind = match &command {
                    physics::PhysicsCommand::ApplyImpulse { .. } => "physics.applyImpulse",
                    physics::PhysicsCommand::ApplyForce { .. } => "physics.applyForce",
                    physics::PhysicsCommand::SetVelocity { .. } => "physics.setVelocity",
                    physics::PhysicsCommand::Teleport { .. } => "physics.teleport",
                };
                let tag = command.tag_and_kind().0.to_string();
                physics::with_world(physics::DEFAULT_WORLD, |w| w.queue_command(command));
                log.push(EffectRecord {
                    kind,
                    value: EffectValue::Text(tag),
                });
                if log.len() > EFFECT_LOG_CAP {
                    log.remove(0);
                }
                continue;
            }
            EffectTree::Raycast {
                origin,
                dir,
                max_dist,
                tagger,
            } => match defer_queries.as_deref_mut() {
                Some(deferred) => {
                    // Pre-step: hold the query for the post-step drain (not
                    // performed, not logged — it hasn't happened yet).
                    deferred.push(EffectTree::Raycast {
                        origin,
                        dir,
                        max_dist,
                        tagger,
                    });
                    continue;
                }
                None => (
                    "physics.raycast",
                    runner.raycast(origin, dir, max_dist),
                    tagger,
                ),
            },
            EffectTree::Now { tagger } => ("now", runner.now().into(), tagger),
            EffectTree::Random { tagger } => ("random", runner.random().into(), tagger),
        };
        let value: EffectValue = value;
        let mle_value = value.to_mle();
        log.push(EffectRecord { kind, value });
        if log.len() > EFFECT_LOG_CAP {
            log.remove(0);
        }
        let msg = match session.apply(
            tagger,
            vec![mle_value],
            &format!("Effect.{kind} tagger"),
            &mut FunctorHost,
        ) {
            Ok(msg) => msg,
            Err(e) => {
                report(format!("[mle] Effect.{kind} tagger error: {}", e.message));
                continue;
            }
        };
        match session.call("update", vec![model.clone(), msg], &mut FunctorHost) {
            Ok(returned) => {
                let (next_model, more) = split_model_effect(returned);
                *model = next_model;
                if let Some(more) = more {
                    queue.push(more);
                }
            }
            Err(e) => report(format!("[mle] update error: {}", e.message)),
        }
    }
}

fn light_of(value: &Value) -> Option<&Light> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<MleLight>().map(|l| &l.0),
        _ => None,
    }
}

fn scene_of(value: &Value) -> Option<&Scene3D> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<MleScene>().map(|s| &s.0),
        _ => None,
    }
}

/// Extract a [`RenderTargetDescriptor`] — both use sites accept ONLY the
/// branded value, so the predictable mistake (a bare id string) gets a
/// teaching error pointing at `RenderTarget.named` instead of a generic
/// usage line (the [`angle_of`] rule, applied to identity).
fn target_of<'a>(
    value: &'a Value,
    what: &str,
    span: Span,
) -> Result<&'a RenderTargetDescriptor, RunError> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<MleRenderTarget>()
            .map(|t| &t.0)
            .ok_or_else(|| RunError {
                message: format!("{what}: expected a RenderTarget, got {}", value.kind_name()),
                span,
            }),
        Value::String(_) => Err(RunError {
            message: format!(
                "{what}: expected a RenderTarget, got a bare string — declare it once \
with RenderTarget.named(\"…\") and pass that value at both sites"
            ),
            span,
        }),
        other => Err(RunError {
            message: format!("{what}: expected a RenderTarget, got {}", other.kind_name()),
            span,
        }),
    }
}

/// Extract a [`ui::Anchor`] — `Ui.panel` accepts ONLY the branded value, so
/// a guessed corner name (a bare string) gets a teaching error pointing at
/// the anchor constructor (the [`angle_of`] rule, applied to corners).
fn ui_anchor_of(value: &Value, span: Span) -> Result<ui::Anchor, RunError> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<MleUiAnchor>()
            .map(|a| a.0)
            .ok_or_else(|| RunError {
                message: format!("Ui.panel: expected an Anchor, got {}", value.kind_name()),
                span,
            }),
        Value::String(_) => Err(RunError {
            message: "Ui.panel: expected an Anchor, got a bare string — pin a corner \
with Ui.topLeft()"
                .to_string(),
            span,
        }),
        other => Err(RunError {
            message: format!("Ui.panel: expected an Anchor, got {}", other.kind_name()),
            span,
        }),
    }
}

/// Extract a [`TextureDescription`] — texture materials accept ONLY the
/// branded value, so the predictable mistake (a bare path string) gets a
/// teaching error pointing at `Texture.file` (the [`angle_of`] rule, applied
/// to assets).
fn texture_of<'a>(
    value: &'a Value,
    what: &str,
    span: Span,
) -> Result<&'a TextureDescription, RunError> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<MleTexture>()
            .map(|t| &t.0)
            .ok_or_else(|| RunError {
                message: format!("{what}: expected a Texture, got {}", value.kind_name()),
                span,
            }),
        Value::String(_) => Err(RunError {
            message: format!(
                "{what}: expected a Texture, got a bare string — build one with \
Texture.file(\"…\") and pass that value"
            ),
            span,
        }),
        other => Err(RunError {
            message: format!("{what}: expected a Texture, got {}", other.kind_name()),
            span,
        }),
    }
}

/// Extract a [`Fog`] — `Frame.withFog` accepts ONLY the branded value, so
/// the predictable mistake (a bare number where a Fog belongs) gets a
/// teaching error pointing at the constructors (the [`angle_of`] rule).
fn fog_of<'a>(value: &'a Value, what: &str, span: Span) -> Result<&'a Fog, RunError> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<MleFog>()
            .map(|f| &f.0)
            .ok_or_else(|| RunError {
                message: format!("{what}: expected a Fog, got {}", value.kind_name()),
                span,
            }),
        Value::Number(_) => Err(RunError {
            message: format!(
                "{what}: expected a Fog, got a bare number — build one with \
Fog.linear(near, far, r, g, b) or Fog.exp(density, r, g, b)"
            ),
            span,
        }),
        other => Err(RunError {
            message: format!("{what}: expected a Fog, got {}", other.kind_name()),
            span,
        }),
    }
}

fn shape_of(value: &Value) -> Option<&physics::Shape> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<MleShape>().map(|s| &s.0),
        _ => None,
    }
}

fn body_of(value: &Value) -> Option<&physics::Body> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<MleBody>().map(|b| &b.0),
        _ => None,
    }
}

/// Live pose of a body in the singleton world (the world the shell steps —
/// same process, same crate statics as this prelude).
fn live_transform(tag: &str) -> Option<([f32; 3], [f32; 4])> {
    physics::with_world(physics::DEFAULT_WORLD, |w| w.body_transform(tag)).flatten()
}

fn no_body(tag: &str) -> String {
    format!(
        "no body tagged \"{tag}\" in the physics world (bodies exist after the \
         frame's `physics` declaration has been reconciled and stepped)"
    )
}

fn host(data: impl HostData + 'static) -> Value {
    Value::HostData(Rc::new(data))
}

fn scene_value(scene: Scene3D) -> Result<Value, RunError> {
    Ok(host(MleScene(scene)))
}

/// A `Group` wrapper carrying `xform` — the transform representation the
/// prelude uses everywhere (see the module doc for why).
fn group(scenes: Vec<Scene3D>, xform: Matrix4<f32>) -> Scene3D {
    Scene3D {
        obj: SceneObject::Group(scenes),
        xform,
    }
}

fn wrap_transform(
    scene: &Value,
    xform: Matrix4<f32>,
    sig: &str,
    span: Span,
) -> Result<Value, RunError> {
    match scene_of(scene) {
        Some(inner) => scene_value(group(vec![inner.clone()], xform)),
        None => Err(RunError {
            message: format!("usage: {sig}"),
            span,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mle::Tracing;

    /// Evaluate an MLE `main` under the prelude and return its value.
    fn eval(src: &str) -> Value {
        let module = mle::lower(mle::parse(src).expect("parse")).expect("lower");
        let record = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("run failed: {}", f.error.message));
        match record.outcome {
            mle::RunOutcome::Main(value) => value,
            _ => panic!("expected a main result"),
        }
    }

    fn frame_of(src: &str) -> Frame {
        let value = eval(src);
        frame_value(&value)
            .expect("main should return a Frame")
            .clone()
    }

    // The C1 verify criterion (docs/mle.md): an .mle snippet emits exactly
    // the protocol data the shells consume — pinned as the serialized wire
    // form the protocol tests use.
    #[test]
    fn mle_snippet_emits_protocol_frame() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())",
        );
        let json = serde_json::to_string(&frame).expect("serialize");
        // Camera: eye/target as given, up +Y, 45° fov, protocol defaults.
        assert!(json.contains(r#""eye":[0.0,2.0,-6.0]"#), "json: {json}");
        assert!(json.contains(r#""fov_radians":0.7853982"#), "json: {json}");
        assert!(
            json.contains(r#""obj":{"Geometry":"Cube"}"#),
            "json: {json}"
        );
        // And the whole thing round-trips through the protocol.
        let back: Frame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // Milestone-0 quirk 1: the renderer drops a Material node's own xform, so
    // a transform applied over Scene.color must survive via a Group wrapper.
    #[test]
    fn transform_over_material_is_not_dropped() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(\n\
               Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0),\n\
               Scene.cube() |> Scene.color(1.0, 0.0, 0.0) |> Scene.translate(2.0, 0.0, 0.0))",
        );
        // Outermost node: a Group carrying the translation…
        let SceneObject::Group(children) = &frame.scene.obj else {
            panic!("expected a transform Group, got {:?}", frame.scene.obj);
        };
        assert_eq!(
            frame.scene.xform.w.x, 2.0,
            "translation must be on the wrapper"
        );
        // …whose child is the Material node (its own xform untouched/identity).
        assert!(matches!(children[0].obj, SceneObject::Material(..)));
    }

    // Milestone-0 quirk 2: wrapping makes the OUTER transform apply last in
    // world space — translate(rotateY(cube)) rotates in place then moves,
    // the order the source reads (not the right-multiply surprise).
    #[test]
    fn outer_transform_applies_after_inner() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(\n\
               Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0),\n\
               Scene.translate(Scene.rotateY(Scene.cube(), Angle.degrees(90.0)), 3.0, 0.0, 0.0))",
        );
        // World composition for nested Groups is parent-first:
        // world = T * R, so a cube corner rotates about the cube's own origin
        // and the whole thing lands at x = 3.
        let t = frame.scene.xform; // outer wrapper: translation
        let SceneObject::Group(children) = &frame.scene.obj else {
            panic!("expected outer Group");
        };
        let r = children[0].xform; // inner wrapper: rotation
        assert_eq!(t.w.x, 3.0);
        assert!((r.x.z - (-1.0)).abs() < 1e-5, "inner is the Y-rotation");
        // The composed transform maps the origin to (3, 0, 0): rotation
        // happened in place, translation after.
        let composed = t * r;
        let origin = composed * cgmath::vec4(0.0f32, 0.0, 0.0, 1.0);
        assert!((origin.x - 3.0).abs() < 1e-5);
    }

    // The mle-hello shape: a List.map-built group of colored cubes.
    #[test]
    fn mapped_group_builds_n_children() {
        let frame = frame_of(
            "let cubeAt = (i) => Scene.cube() |> Scene.color(1.0, 0.5, 0.2) |> Scene.translate(i, 0.0, 0.0)\n\
             let main = () =>\n\
             Frame.create(\n\
               Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0),\n\
               Scene.group([0.0, 1.0, 2.0] |> List.map(cubeAt)))",
        );
        let SceneObject::Group(children) = &frame.scene.obj else {
            panic!("expected group");
        };
        assert_eq!(children.len(), 3);
        // Each child is a translate-wrapper Group at x = i.
        for (i, child) in children.iter().enumerate() {
            assert_eq!(child.xform.w.x, i as f32);
        }
    }

    // The E1 hello shape: Scene.model emits a protocol Model node carrying
    // the file handle, transformable like any scene (per-model scale, then
    // placement — the glTF-lineup composition).
    #[test]
    fn scene_model_emits_a_model_node() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(\n\
               Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0),\n\
               Scene.model(\"shark.glb\") |> Scene.scale(0.002) |> Scene.translate(3.0, 1.0, 3.0))",
        );
        // Outermost: the translate wrapper; inside it, the scale wrapper;
        // inside that, the Model node itself.
        let SceneObject::Group(children) = &frame.scene.obj else {
            panic!("expected translate wrapper, got {:?}", frame.scene.obj);
        };
        let SceneObject::Group(inner) = &children[0].obj else {
            panic!("expected scale wrapper, got {:?}", children[0].obj);
        };
        let SceneObject::Model(model) = &inner[0].obj else {
            panic!("expected a Model node, got {:?}", inner[0].obj);
        };
        let ModelHandle::File(path) = &model.handle;
        assert_eq!(path, "shark.glb");
        assert!(model.overrides.is_empty());
        // And the whole thing speaks the protocol.
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(json.contains("shark.glb"), "json: {json}");
        let back: Frame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // Scene.model teaches its usage: a bare number or an empty path is a
    // spanned error, not a silently-empty scene.
    #[test]
    fn model_requires_a_nonempty_path_string() {
        for src in [
            "let main = () => Scene.model(42.0)",
            "let main = () => Scene.model(\"\")",
        ] {
            let module = mle::lower(mle::parse(src).unwrap()).unwrap();
            let failure = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
                .err()
                .expect("should fail");
            assert_eq!(
                failure.error.message,
                "usage: Scene.model(\"file.glb\") — a non-empty glTF path relative to \
the game dir"
            );
        }
    }

    // The lit pipeline: materials, all three light kinds, shadow flag, and
    // firstPerson camera flow into a protocol Frame with lights.
    #[test]
    fn lit_frame_carries_lights_and_materials() {
        let frame = frame_of(
            "let main = () =>
             Frame.createLit(
               Camera.firstPerson(0.0, 3.5, -8.0, Angle.radians(0.0), Angle.radians(-0.3), Angle.degrees(60.0)),
               Scene.group([
                 Scene.plane() |> Scene.scale(24.0) |> Scene.lit(0.6, 0.6, 0.62),
                 Scene.sphere() |> Scene.emissive(1.0, 0.3, 0.25),
               ]),
               [
                 Light.ambient(0.1, 0.1, 0.13),
                 Light.directional(0.5, -1.0, 0.35, 1.0, 0.98, 0.95, 0.85) |> Light.castShadows,
                 Light.point(1.0, 2.2, 0.0, 1.0, 0.3, 0.25, 1.4, 4.0),
               ])",
        );
        assert_eq!(frame.lights.len(), 3);
        assert!(frame.lights[1].casts_shadows(), "directional casts shadows");
        // firstPerson: 60° fov, eye as given.
        assert!((frame.camera.fov_radians - 60.0_f32.to_radians()).abs() < 1e-5);
        assert_eq!(frame.camera.eye, [0.0, 3.5, -8.0]);
        // The scene serializes through the protocol (Lit/Emissive materials).
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(json.contains("Lit"), "json: {json}");
        assert!(json.contains("Emissive"), "json: {json}");
    }

    // Host errors are spanned MLE runtime errors, not panics.
    #[test]
    fn prelude_errors_are_spanned() {
        let module = mle::lower(
            mle::parse("let main = () => Scene.color(Scene.cube(), 1.0, \"x\", 0.0)").unwrap(),
        )
        .unwrap();
        let failure = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(failure.error.message, "expected a number, got a string");
    }

    // [units, tier 1] rotations/camera angles refuse bare numbers with a
    // teaching error — degree/radian confusion is unrepresentable.
    #[test]
    fn bare_numbers_are_not_angles() {
        let module =
            mle::lower(mle::parse("let main = () => Scene.cube() |> Scene.rotateY(1.57)").unwrap())
                .unwrap();
        let failure = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(
            failure.error.message,
            "Scene.rotateY: expected an Angle, got a bare number — say which unit: \
Angle.degrees(…) or Angle.radians(…)"
        );
    }

    // Degrees and radians agree where they should: 90° == τ/4 rad.
    #[test]
    fn degrees_and_radians_agree() {
        let deg = frame_of(
            "let main = () => Frame.create(Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0), \
Scene.cube() |> Scene.rotateY(Angle.degrees(90.0)))",
        );
        let rad = frame_of(
            "let main = () => Frame.create(Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0), \
Scene.cube() |> Scene.rotateY(Angle.radians(1.5707964)))",
        );
        assert_eq!(
            serde_json::to_string(&deg.scene).unwrap(),
            serde_json::to_string(&rad.scene).unwrap()
        );
    }

    // A value the prelude doesn't serve still errors as unknown.
    #[test]
    fn unknown_externals_still_error() {
        let module =
            mle::lower(mle::parse("let main = () => Scene.frobnicate()").unwrap()).unwrap();
        let failure = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(failure.error.message, "unknown external `Scene.frobnicate`");
    }

    // [AGREED review pin] every advertised path must dispatch to a real arm:
    // garbage args must produce a usage/type error, never the
    // `internal: unregistered prelude path` fallback or `unknown external`.
    #[test]
    fn every_advertised_path_dispatches() {
        let mut host = FunctorHost;
        for path in PATHS {
            let result = host.call(path, vec![Value::Bool(true)], mle::Span::new(0, 0));
            let message = result.err().expect("garbage args should error").message;
            assert!(
                !message.starts_with("internal:"),
                "`{path}` fell through to the internal fallback: {message}"
            );
        }
    }

    // The render-target vocabulary end to end: a branded target declared once
    // (named + sized), used at the reader (Scene.screen) and the writer
    // (Frame.withRenderTarget) — the frame carries the pass, the scene carries
    // the texture reference, and the whole thing speaks the protocol.
    #[test]
    fn mle_snippet_declares_a_render_target_frame() {
        let frame = frame_of(
            "let feed = RenderTarget.named(\"security\") |> RenderTarget.sized(256.0, 128.0)\n\
             let main = () =>\n\
             Frame.createLit(\n\
               Camera.lookAt(0.0, 2.0, -8.0, 0.0, 1.0, 0.0),\n\
               Scene.group([\n\
                 Scene.plane() |> Scene.lit(0.6, 0.6, 0.6),\n\
                 Scene.quad() |> Scene.screen(feed),\n\
               ]),\n\
               [Light.ambient(0.1, 0.1, 0.1)])\n\
             |> Frame.withRenderTarget(feed, Frame.createLit(\n\
                  Camera.lookAt(0.0, 4.0, -6.0, 0.0, 0.5, 0.0),\n\
                  Scene.cube() |> Scene.lit(0.8, 0.2, 0.2),\n\
                  [Light.ambient(0.2, 0.2, 0.2)]))",
        );
        assert_eq!(frame.render_targets.len(), 1);
        let pass = &frame.render_targets[0];
        assert_eq!(pass.target.id, "security");
        assert_eq!((pass.target.width, pass.target.height), (256, 128));
        assert_eq!(pass.frame.lights.len(), 1);
        assert!(pass.frame.render_targets.is_empty());
        // The reader's wire shape: the screen material samples the target by id.
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(json.contains(r#""RenderTarget":"security""#), "json: {json}");
        // And the whole frame round-trips through the protocol.
        let back: Frame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // Scene.screen is an emissive (fullbright) white surface over the target's
    // texture — screens glow, unaffected by scene lighting.
    #[test]
    fn screen_material_is_emissive_white_over_the_target_texture() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(\n\
               Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0),\n\
               Scene.quad() |> Scene.screen(RenderTarget.named(\"feed\")))",
        );
        let json = serde_json::to_string(&frame.scene).expect("serialize");
        assert!(
            json.contains(r#""Emissive":{"color":[1.0,1.0,1.0,1.0],"texture":{"RenderTarget":"feed"}}"#),
            "json: {json}"
        );
    }

    // [units, tier 1 — the Angle rule applied to identity] both sites accept
    // ONLY the branded RenderTarget value; a bare string / wrong value is a
    // spanned usage error, so writer/reader id typos are unrepresentable.
    #[test]
    fn bare_strings_are_not_render_targets() {
        let fail = |src: &str| {
            let module = mle::lower(mle::parse(src).unwrap()).unwrap();
            mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
                .err()
                .expect("should fail")
                .error
                .message
        };
        assert_eq!(
            fail("let main = () => Scene.quad() |> Scene.screen(\"security\")"),
            "Scene.screen: expected a RenderTarget, got a bare string — declare it once \
with RenderTarget.named(\"…\") and pass that value at both sites"
        );
        assert_eq!(
            fail(
                "let main = () => Frame.create(Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0), \
Scene.cube()) |> Frame.withRenderTarget(\"feed\", Frame.create(\
Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0), Scene.cube()))"
            ),
            "Frame.withRenderTarget: expected a RenderTarget, got a bare string — declare \
it once with RenderTarget.named(\"…\") and pass that value at both sites"
        );
        assert_eq!(
            fail(
                "let main = () => RenderTarget.named(\"x\") |> RenderTarget.sized(-1.0, 4.0)"
            ),
            "RenderTarget.sized width must be positive, got -1"
        );
        assert_eq!(
            fail("let main = () => RenderTarget.named(3.0)"),
            "usage: RenderTarget.named(\"id\") — a non-empty name; 512x512 unless \
piped through RenderTarget.sized"
        );
    }

    // The fog vocabulary: a branded Fog on the frame, round-tripping the
    // protocol wire shape.
    #[test]
    fn mle_snippet_declares_fog() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(Camera.lookAt(0.0, 2.0, -8.0, 0.0, 1.0, 0.0), Scene.cube())\n\
             |> Frame.withFog(Fog.linear(4.0, 30.0, 0.5, 0.6, 0.7))",
        );
        assert_eq!(frame.fog, Some(Fog::linear(4.0, 30.0, 0.5, 0.6, 0.7)));
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(json.contains(r#""Linear""#), "json: {json}");
        let back: Frame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // [units, tier 1 — the Angle rule] Frame.withFog accepts ONLY the branded
    // value; a bare number gets the teaching error.
    #[test]
    fn bare_numbers_are_not_fog() {
        let fail = |src: &str| {
            let module = mle::lower(mle::parse(src).unwrap()).unwrap();
            mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
                .err()
                .expect("should fail")
                .error
                .message
        };
        assert_eq!(
            fail(
                "let main = () => Frame.create(Camera.lookAt(0.0, 0.0, -5.0, 0.0, 0.0, 0.0), \
Scene.cube()) |> Frame.withFog(0.5)"
            ),
            "Frame.withFog: expected a Fog, got a bare number — build one with \
Fog.linear(near, far, r, g, b) or Fog.exp(density, r, g, b)"
        );
        // Degenerate parameters are teaching errors at construction, not
        // silent bad renders.
        assert_eq!(
            fail("let main = () => Fog.linear(10.0, 5.0, 0.5, 0.5, 0.5)"),
            "Fog.linear: far (5) must be greater than near (10)"
        );
        assert_eq!(
            fail("let main = () => Fog.exp(-1.0, 0.5, 0.5, 0.5)"),
            "Fog.exp density must be positive, got -1"
        );
    }

    // The physics vocabulary: an MLE snippet declares a PhysicsScene the
    // shells can hand to `World::reconcile` — bodies, attributes, gravity.
    #[test]
    fn mle_snippet_declares_a_physics_scene() {
        let value = eval(
            "let crate1 = Physics.dynamic(\"crate-1\", Physics.box(1.0, 1.0, 1.0))\n\
             |> Physics.at(0.0, 5.0, 0.0)\n\
             |> Physics.velocity(1.0, 0.0, 0.0)\n\
             |> Physics.mass(2.0)\n\
             |> Physics.restitution(0.5)\n\
             let main = () => Physics.scene(0.0, -9.81, 0.0, [\n\
               Physics.fixed(\"ground\", Physics.box(20.0, 0.2, 20.0)),\n\
               crate1,\n\
               Physics.kinematic(\"door\", Physics.capsule(1.0, 0.3)) |> Physics.sensor,\n\
             ])",
        );
        let scene = physics_scene_value(&value).expect("a PhysicsScene");
        assert_eq!(scene.gravity, [0.0, -9.81, 0.0]);
        assert_eq!(scene.bodies.len(), 3);
        assert_eq!(scene.bodies[0].tag, "ground");
        assert_eq!(scene.bodies[1].position, [0.0, 5.0, 0.0]);
        assert_eq!(scene.bodies[1].velocity, [1.0, 0.0, 0.0]);
        assert_eq!(scene.bodies[1].mass, Some(2.0));
        assert_eq!(scene.bodies[1].restitution, 0.5);
        assert!(scene.bodies[2].sensor);
    }

    // End to end across the seam: reconcile + step the singleton world the
    // way the MleGame driver does, then read it back from MLE — the in-process
    // live read that is the whole point of the MLE surface.
    #[test]
    fn physics_reads_see_the_stepped_world() {
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        let declare = eval(
            "let main = () => Physics.scene(0.0, -9.81, 0.0, [\n\
               Physics.dynamic(\"ball\", Physics.sphere(0.5)) |> Physics.at(0.0, 5.0, 0.0)])",
        );
        let scene = physics_scene_value(&declare)
            .expect("a PhysicsScene")
            .clone();
        crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| {
            w.reconcile(&scene);
            for _ in 0..30 {
                w.step_fixed();
            }
        });

        // Physics.position sees the fallen ball…
        let pos = eval("let main = () => Physics.position(\"ball\")");
        let Value::Record(fields) = &pos else {
            panic!("expected a record, got {}", pos.kind_name());
        };
        let y = fields
            .iter()
            .find(|(k, _)| k == "y")
            .and_then(|(_, v)| match v {
                Value::Number(n) => Some(*n),
                _ => None,
            })
            .expect("y field");
        assert!(y < 5.0, "ball should have fallen, y = {y}");

        // …and Physics.transformed places a scene node at the live pose.
        let drawn = eval("let main = () => Scene.sphere() |> Physics.transformed(\"ball\")");
        let scene3d = scene_of(&drawn).expect("a Scene");
        assert!((scene3d.xform.w.y as f64 - y).abs() < 1e-6);

        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
    }

    // Degenerate physical dimensions are boundary errors — Rapier would
    // silently build a broken collider, and MLE can't branch to notice.
    #[test]
    fn non_positive_dimensions_are_rejected() {
        for (src, needle) in [
            (
                "Physics.sphere(-0.5)",
                "Physics.sphere radius must be positive",
            ),
            (
                "Physics.box(1.0, 0.0, 1.0)",
                "Physics.box height must be positive",
            ),
            (
                "Physics.dynamic(\"x\", Physics.sphere(0.5)) |> Physics.mass(0.0)",
                "Physics.mass must be positive",
            ),
            (
                "Physics.dynamic(\"x\", Physics.sphere(0.5)) |> Physics.friction(-1.0)",
                "Physics.friction must not be negative",
            ),
        ] {
            let module =
                mle::lower(mle::parse(&format!("let main = () => {src}")).unwrap()).unwrap();
            let failure = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
                .err()
                .unwrap_or_else(|| panic!("`{src}` should fail"));
            assert!(
                failure.error.message.contains(needle),
                "`{src}`: got {}",
                failure.error.message
            );
        }
    }

    // The Phase 3 command path end to end: an entry point returns
    // (model, Physics.applyImpulse(…)); draining queues the command on the
    // singleton world; the next step applies it.
    #[test]
    fn physics_command_effects_queue_and_apply() {
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        let effect = eval(
            "let main = () => Physics.applyImpulse(\"ball\", 2.0, 0.0, 0.0)",
        );
        let Value::HostData(data) = &effect else {
            panic!("expected an Effect");
        };
        let tree = &data.as_any().downcast_ref::<MleEffect>().expect("Effect").0;

        // Drain it the way the producer does (no session/update involvement —
        // physics effects are tagger-less).
        let module = mle::lower(mle::parse("let update = (m, msg) => m").unwrap()).unwrap();
        let session = mle::Session::load(&module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("load failed: {}", f.error.message));
        let mut model = Value::Number(0.0);
        let mut log = EffectLog::new();
        let mut runner = FakeEffects::new(0.0, vec![]);
        let deferred = drain_effects(
            &session,
            &mut model,
            tree.clone(),
            &mut runner,
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
        );
        assert!(deferred.is_empty(), "nothing should defer here");
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].kind, "physics.applyImpulse");
        // The structured log records the TARGET, not a filler number.
        assert_eq!(log[0].value, EffectValue::Text("ball".to_string()));

        // The command sits queued on world 0; a declared body + step applies it.
        crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| {
            w.reconcile(&crate::physics::PhysicsScene::create(
                [0.0, 0.0, 0.0],
                vec![crate::physics::Body::dynamic(
                    "ball".to_string(),
                    crate::physics::Shape::Sphere { radius: 0.5 },
                )],
            ));
            w.step_frame(1.0 / 60.0);
            let v = w.body_velocity("ball").unwrap();
            assert!(v[0] > 0.0, "impulse not applied: {v:?}");
            assert!(w.take_command_warnings().is_empty());
        });
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
    }

    // A missing tag is a loud spanned error, not a sentinel value.
    #[test]
    fn physics_read_of_unknown_tag_is_a_spanned_error() {
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        let module =
            mle::lower(mle::parse("let main = () => Physics.position(\"ghost\")").unwrap())
                .unwrap();
        let failure = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert!(
            failure.error.message.contains("no body tagged \"ghost\""),
            "got: {}",
            failure.error.message
        );
    }

    // `main` bound to a host function errors like a builtin, not a value.
    #[test]
    fn main_bound_to_host_fn_errors() {
        let module = mle::lower(mle::parse("let main = Scene.cube").unwrap()).unwrap();
        let failure = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(
            failure.error.message,
            "`main` must take no parameters to be runnable"
        );
    }

    // MLE permits non-finite numbers (IEEE division); the protocol boundary
    // does not — they become spanned errors, not NaN matrices.
    #[test]
    fn non_finite_numbers_are_rejected_at_the_boundary() {
        let module = mle::lower(
            mle::parse("let main = () => Scene.translate(Scene.cube(), 1.0 / 0.0, 0.0, 0.0)")
                .unwrap(),
        )
        .unwrap();
        let failure = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(failure.error.message, "expected a finite number, got inf");
    }

    /// The messages fired by `main`'s sub tree over `(prev, tts]`, as display
    /// strings ("Pulse", "Beat(2)").
    fn fired(src: &str, prev: f64, tts: f64) -> Vec<String> {
        sub_messages_for_frame(&eval(src), prev, tts)
            .expect("a Sub tree")
            .iter()
            .map(|m| m.to_string())
            .collect()
    }

    const PULSE: &str = "type Msg = | Pulse\n\
         let main = () => Sub.every(Time.seconds(1.0), Pulse)";

    /// `Every` fires exactly when an integer multiple of the period lies in
    /// `(prev, tts]` — boundary inclusive on the right, and a long frame that
    /// skips several boundaries still fires ONCE (floor comparison, exactly
    /// the F# `Sub.crossedBoundary`).
    #[test]
    fn every_fires_on_the_global_time_grid() {
        assert!(fired(PULSE, 0.5, 0.9).is_empty());
        assert_eq!(fired(PULSE, 0.9, 1.0), ["Pulse"]);
        assert!(fired(PULSE, 1.0, 1.5).is_empty());
        assert_eq!(fired(PULSE, 2.9, 4.1), ["Pulse"]);
    }

    /// `Time.millis(500)` and `Time.seconds(0.5)` are the same duration.
    #[test]
    fn millis_and_seconds_agree() {
        let millis = "type Msg = | Pulse\n\
             let main = () => Sub.every(Time.millis(500.0), Pulse)";
        assert_eq!(fired(millis, 0.9, 1.0), ["Pulse"]);
        assert!(fired(millis, 1.0, 1.4).is_empty());
    }

    /// Batches fire in declaration order; `Sub.none` fires nothing; the msg
    /// crosses back verbatim (here, a parameterful variant).
    #[test]
    fn batch_collects_in_declaration_order() {
        let src = "type Msg = | Fast(n: Float) | Slow\n\
             let main = () => Sub.batch([\n\
               Sub.every(Time.seconds(2.0), Slow),\n\
               Sub.none(),\n\
               Sub.every(Time.millis(500.0), Fast(2.0)),\n\
             ])";
        assert_eq!(fired(src, 1.9, 2.0), ["Slow", "Fast(2)"]);
        assert_eq!(fired(src, 2.0, 2.5), ["Fast(2)"]);
    }

    /// Walking a non-binary period over noisy f32-sourced frame times, the
    /// firing count telescopes exactly: fires = boundary count of the whole
    /// span, no message lost or doubled, no matter where the frame edges
    /// land. (Timing may jitter by a frame; the COUNT is exact.)
    #[test]
    fn firing_count_telescopes_over_noisy_frames() {
        let sub = eval(
            "type Msg = | Pulse\n\
             let main = () => Sub.every(Time.millis(300.0), Pulse)",
        );
        // f32-truncated, uneven frame edges (dt ~16.7ms), like the runner's.
        let mut edges: Vec<f64> = Vec::new();
        let mut t: f32 = 0.5;
        for i in 0..600 {
            t += 0.0167 + (i % 7) as f32 * 0.0011;
            edges.push(t as f64);
        }
        let mut fires = 0;
        for pair in edges.windows(2) {
            fires += sub_messages_for_frame(&sub, pair[0], pair[1])
                .expect("a Sub tree")
                .len();
        }
        let (first, last) = (edges[0], edges[edges.len() - 1]);
        let expected = ((last / 0.3).floor() - (first / 0.3).floor()) as usize;
        assert_eq!(fires, expected, "over ({first}, {last}]");
    }

    /// The B6 broker contract: the same program under fake and replay
    /// runners produces the SAME model — the fake run's structured log IS
    /// replay's input. (Real differs only in the values the world supplies.)
    #[test]
    fn same_program_under_fake_and_replay() {
        let src = "type Msg = | Rolled(n: Float) | Stamped(t: Float)\n\
             let update = (m, msg) =>\n\
               match msg with\n\
               | Rolled(n) => ({ m with rolls: m.rolls + n }, Effect.now(Stamped))\n\
               | Stamped(t) => { m with at: t }\n\
             let roll = (m) => (m, Effect.random(Rolled))";
        let module = mle::lower(mle::parse(src).expect("parse")).expect("lower");
        let session = match mle::Session::load(&module, &mut FunctorHost) {
            Ok(session) => session,
            Err(failure) => panic!("load failed: {}", failure.error.message),
        };
        let init = || {
            Value::Record(std::rc::Rc::new(vec![
                ("rolls".to_string(), Value::Number(0.0)),
                ("at".to_string(), Value::Number(0.0)),
            ]))
        };
        let run = |runner: &mut dyn EffectRunner| {
            let mut model = init();
            let mut log = EffectLog::new();
            let returned = session
                .call("roll", vec![model.clone()], &mut FunctorHost)
                .expect("roll");
            let (m, fx) = split_model_effect(returned);
            model = m;
            let deferred = drain_effects(
                &session,
                &mut model,
                fx.expect("an effect"),
                runner,
                &mut log,
                &mut |msg| panic!("unexpected report: {msg}"),
            );
            assert!(deferred.is_empty(), "nothing should defer here");
            (model.to_string(), log)
        };
        // Fake world: random = 0.25, now = 99.5 — exact arithmetic.
        let (fake_model, fake_log) = run(&mut FakeEffects::new(99.5, vec![0.25]));
        assert_eq!(fake_model, "{ rolls: 0.25, at: 99.5 }");
        assert_eq!(
            fake_log,
            vec![
                EffectRecord {
                    kind: "random",
                    value: EffectValue::Number(0.25)
                },
                EffectRecord {
                    kind: "now",
                    value: EffectValue::Number(99.5)
                },
            ]
        );
        // Replay the log: same model, no world consulted.
        let (replay_model, replay_log) = run(&mut ReplayEffects::new(fake_log.clone()));
        assert_eq!(replay_model, fake_model);
        assert_eq!(replay_log, fake_log);
        // The real world: same SHAPE (both effects performed, chain drained),
        // world-supplied values.
        let (real_model, real_log) = run(&mut RealEffects::new());
        assert_eq!(real_log.len(), 2);
        assert_eq!(real_log[0].kind, "random");
        let EffectValue::Number(r0) = real_log[0].value else {
            panic!("random should log a number");
        };
        assert!((0.0..1.0).contains(&r0));
        assert_eq!(real_log[1].kind, "now");
        assert!(real_model.starts_with("{ rolls: 0."));
    }

    /// The Phase 4 query path end to end: a raycast effect DEFERS through the
    /// pre-step drain, then answers post-step against the live world, its
    /// record folding through `update` — and the fake/replay runners answer
    /// without a world at all.
    #[test]
    fn raycast_effects_defer_then_answer_post_step() {
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        // `update` swaps the model for the hit record (tagger = a closure, so
        // the record itself is the message).
        let src = "let update = (m, msg) => msg\n\
                   let main = () => Physics.raycast(0.0, 5.0, 0.0, 0.0, -1.0, 0.0, 100.0, (hit) => hit)";
        let module = mle::lower(mle::parse(src).unwrap()).unwrap();
        let session = mle::Session::load(&module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("load failed: {}", f.error.message));
        let record = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("run failed: {}", f.error.message));
        let effect = match record.outcome {
            mle::RunOutcome::Main(value) => value,
            _ => panic!("expected main"),
        };
        let Value::HostData(data) = &effect else {
            panic!("expected an Effect")
        };
        let tree = data.as_any().downcast_ref::<MleEffect>().unwrap().0.clone();
        let EffectTree::Raycast { tagger, .. } = &tree else {
            panic!("expected a raycast effect");
        };
        let tagger = tagger.clone();

        // A settled body for the ray to hit.
        crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| {
            w.reconcile(&crate::physics::PhysicsScene::create(
                [0.0, 0.0, 0.0],
                vec![crate::physics::Body::fixed(
                    "slab".to_string(),
                    crate::physics::Shape::Cuboid {
                        extents: [4.0, 1.0, 4.0],
                    },
                )],
            ));
            w.step_fixed();
        });

        let mut model = Value::Number(0.0);
        let mut log = EffectLog::new();
        let mut runner = RealEffects::new();
        let mut fail = |m: String| panic!("unexpected report: {m}");

        // Pre-step drain: DEFERRED — nothing performed, nothing logged.
        let deferred = drain_effects(&session, &mut model, tree, &mut runner, &mut log, &mut fail);
        assert_eq!(deferred.len(), 1);
        assert!(log.is_empty());
        assert!(matches!(model, Value::Number(_)), "model must be untouched");

        // Post-step drain: performed against the live world.
        perform_deferred_queries(&session, &mut model, deferred, &mut runner, &mut log, &mut fail);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].kind, "physics.raycast");
        let Value::Record(fields) = &model else {
            panic!("update should have received the hit record");
        };
        let field = |name: &str| {
            fields
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert!(matches!(field("hit"), Value::Bool(true)));
        assert!(matches!(field("tag"), Value::String(s) if &*s == "slab"));
        assert!(matches!(field("y"), Value::Number(n) if (n - 0.5).abs() < 1e-4));

        // Replay: the recorded log answers with no world consulted.
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        let mut replay_model = Value::Number(0.0);
        let mut replay_log = EffectLog::new();
        let mut replay = ReplayEffects::new(log.clone());
        let deferred = drain_effects(
            &session,
            &mut replay_model,
            EffectTree::Raycast {
                origin: [0.0, 5.0, 0.0],
                dir: [0.0, -1.0, 0.0],
                max_dist: 100.0,
                tagger: tagger.clone(),
            },
            &mut replay,
            &mut replay_log,
            &mut fail,
        );
        perform_deferred_queries(
            &session,
            &mut replay_model,
            deferred,
            &mut replay,
            &mut replay_log,
            &mut fail,
        );
        assert_eq!(replay_log, log, "replay must reproduce the log");
        assert_eq!(replay_model.to_string(), model.to_string());

        // Fake: canned hits, no world.
        let mut fake = FakeEffects::new(0.0, vec![]).with_ray_hits(vec![ray_result_value(None)]);
        let miss = fake.raycast([0.0; 3], [0.0, -1.0, 0.0], 10.0);
        let EffectValue::Record(f) = &miss else {
            panic!()
        };
        assert!(f.iter().any(|(k, v)| k == "hit" && *v == EffectValue::Bool(false)));
    }

    /// The Phase 5 event path end to end: a `Physics.events` sub's tagger
    /// receives contact records, folding through `update` post-step.
    #[test]
    fn physics_events_flow_to_update() {
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        let src = "let subscriptions = (m) => Physics.events((e) => e)\n\
                   let update = (m, msg) =>\n\
                     { contacts: m.contacts + 1.0, a: msg.a, b: msg.b, began: msg.started }";
        let module = mle::lower(mle::parse(src).unwrap()).unwrap();
        let session = mle::Session::load(&module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("load failed: {}", f.error.message));

        // Taggers come from the game's subscriptions value.
        let subs = session
            .apply(
                session.global("subscriptions").unwrap(),
                vec![Value::Number(0.0)],
                "subscriptions",
                &mut FunctorHost,
            )
            .unwrap_or_else(|e| panic!("subs failed: {}", e.message));
        let taggers = physics_event_taggers(&subs).expect("a Sub");
        assert_eq!(taggers.len(), 1);

        // Drive a real collision.
        let event = crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| {
            w.reconcile(&crate::physics::PhysicsScene::create(
                [0.0, -9.81, 0.0],
                vec![
                    crate::physics::Body::fixed(
                        "slab".to_string(),
                        crate::physics::Shape::Cuboid {
                            extents: [8.0, 0.4, 8.0],
                        },
                    ),
                    crate::physics::Body::dynamic(
                        "ball".to_string(),
                        crate::physics::Shape::Sphere { radius: 0.5 },
                    )
                    .at([0.0, 2.0, 0.0]),
                ],
            ));
            let mut found = None;
            for _ in 0..180 {
                w.step_frame(1.0 / 60.0);
                if let Some(e) = w.take_events().into_iter().find(|e| e.started) {
                    found = Some(e);
                    break;
                }
            }
            found
        })
        .flatten()
        .expect("a contact");

        let mut model = Value::Record(Rc::new(vec![(
            "contacts".to_string(),
            Value::Number(0.0),
        )]));
        let mut log = EffectLog::new();
        let mut runner = FakeEffects::new(0.0, vec![]);
        deliver_physics_events(
            &session,
            &mut model,
            &taggers,
            &[event],
            &mut runner,
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
        );
        let Value::Record(fields) = &model else {
            panic!("update should have run");
        };
        let get = |k: &str| fields.iter().find(|(n, _)| n == k).unwrap().1.clone();
        assert!(matches!(get("contacts"), Value::Number(n) if n == 1.0));
        assert!(matches!(get("began"), Value::Bool(true)));
        let (a, b) = (get("a").to_string(), get("b").to_string());
        assert!(a.contains("slab") || b.contains("slab"), "{a} {b}");
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
    }

    /// Tagger-less trees (physics commands) don't require an `update` hook;
    /// message-producing ones do — the drivers' drop-guard keys on this.
    #[test]
    fn needs_update_distinguishes_taggered_from_fire_and_forget() {
        let cmd = EffectTree::Physics(crate::physics::PhysicsCommand::ApplyImpulse {
            tag: "x".to_string(),
            impulse: [0.0; 3],
        });
        assert!(!needs_update(&EffectTree::None));
        assert!(!needs_update(&cmd));
        assert!(!needs_update(&EffectTree::Batch(vec![
            EffectTree::None,
            cmd.clone()
        ])));
        let tagged = EffectTree::Raycast {
            origin: [0.0; 3],
            dir: [0.0, -1.0, 0.0],
            max_dist: 1.0,
            tagger: Value::Number(0.0), // shape only; construction validates real taggers
        };
        assert!(needs_update(&tagged));
        assert!(needs_update(&EffectTree::Batch(vec![cmd, tagged])));
    }

    /// Structured effect values convert to the MLE values taggers receive,
    /// and round-trip through serde (the future disk-replay seam).
    #[test]
    fn effect_values_convert_and_serialize() {
        let value = EffectValue::Record(vec![
            ("hit".to_string(), EffectValue::Bool(true)),
            ("distance".to_string(), EffectValue::Number(4.25)),
            ("tag".to_string(), EffectValue::Text("crate-1".to_string())),
        ]);
        let mle = value.to_mle();
        let Value::Record(fields) = &mle else {
            panic!("expected a record, got {}", mle.kind_name());
        };
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].0, "hit");
        assert!(matches!(fields[0].1, Value::Bool(true)));
        assert!(matches!(fields[1].1, Value::Number(n) if n == 4.25));

        let json = serde_json::to_string(&value).expect("serialize");
        let back: EffectValue = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, value);
    }

    /// A diverged replay fails loud with the position, not silently wrong.
    #[test]
    #[should_panic(expected = "replay diverged at effect 0")]
    fn replay_divergence_fails_loud() {
        ReplayEffects::new(vec![EffectRecord {
            kind: "now",
            value: EffectValue::Number(1.0),
        }])
        .random();
    }

    /// Effect construction refuses non-function taggers, and batches refuse
    /// non-effects — construction-time teaching errors, not mid-drain ones.
    #[test]
    fn effect_construction_is_validated() {
        let module = mle::lower(mle::parse("let main = () => Effect.now(3.0)").unwrap()).unwrap();
        let failure = mle::run_with_host(&module, mle::Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(
            failure.error.message,
            "Effect.now(tagger): the tagger must be a function of the result, got a number"
        );
        let module =
            mle::lower(mle::parse("let main = () => Effect.batch([1.0])").unwrap()).unwrap();
        let failure = mle::run_with_host(&module, mle::Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(
            failure.error.message,
            "Effect.batch items must be Effects, got a number"
        );
    }

    /// The drain cap stops a self-sustaining effect chain instead of hanging
    /// the frame.
    #[test]
    fn effect_drain_cap_stops_runaway_chains() {
        let src = "type Msg = | Again(n: Float)\n\
             let again = (n) => Again(n)\n\
             let update = (m, msg) => (m + 1.0, Effect.random(Again))";
        let module = mle::lower(mle::parse(src).expect("parse")).expect("lower");
        let session = match mle::Session::load(&module, &mut FunctorHost) {
            Ok(session) => session,
            Err(failure) => panic!("load failed: {}", failure.error.message),
        };
        let mut model = Value::Number(0.0);
        let mut log = EffectLog::new();
        let mut reports = Vec::new();
        let deferred = drain_effects(
            &session,
            &mut model,
            EffectTree::Random {
                tagger: session.global("again").expect("tagger fn"),
            },
            &mut FakeEffects::new(0.0, vec![0.5]),
            &mut log,
            &mut |msg| reports.push(msg),
        );
        assert!(deferred.is_empty(), "nothing should defer here");
        // The log bound is enforced INSIDE the drain (not after), and the
        // cap fires before performing the over-limit effect.
        assert_eq!(log.len(), EFFECT_LOG_CAP, "log bounded mid-drain");
        assert!(reports.iter().any(|r| r.contains("per-frame cap")));
    }

    /// Structural nodes count toward the cap too — a giant batch of
    /// `Effect.none()` cannot consume unbounded frame time. [Codex M]
    #[test]
    fn structural_nodes_count_toward_the_cap() {
        let module =
            mle::lower(mle::parse("let noop = (m, msg) => m").expect("parse")).expect("lower");
        let session = match mle::Session::load(&module, &mut FunctorHost) {
            Ok(session) => session,
            Err(failure) => panic!("load failed: {}", failure.error.message),
        };
        let mut model = Value::Number(0.0);
        let mut log = EffectLog::new();
        let mut reports = Vec::new();
        let deferred = drain_effects(
            &session,
            &mut model,
            EffectTree::Batch(vec![EffectTree::None; 1500]),
            &mut FakeEffects::new(0.0, vec![0.5]),
            &mut log,
            &mut |msg| reports.push(msg),
        );
        assert!(deferred.is_empty(), "nothing should defer here");
        assert!(log.is_empty(), "nothing performed");
        assert!(reports.iter().any(|r| r.contains("per-frame cap")));
    }

    /// Effects are commands, not data — the deep scan that backs the
    /// producers' init rejection and adopted-model warning. [Codex H]
    #[test]
    fn contains_effect_finds_nested_effects() {
        let fx = eval("let main = () => Effect.none()");
        assert!(contains_effect(&fx));
        let nested = Value::Record(std::rc::Rc::new(vec![(
            "inner".to_string(),
            Value::List(std::rc::Rc::new(vec![Value::Tuple(std::rc::Rc::new(
                vec![Value::Number(1.0), fx],
            ))])),
        )]));
        assert!(contains_effect(&nested));
        assert!(!contains_effect(&Value::Number(1.0)));
    }

    /// Bare numbers are not durations — the Angle teaching error, for time.
    #[test]
    fn bare_numbers_are_not_durations() {
        let module = mle::lower(
            mle::parse("type Msg = | Pulse\nlet main = () => Sub.every(0.5, Pulse)").unwrap(),
        )
        .unwrap();
        let failure = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(
            failure.error.message,
            "Sub.every: expected a Duration, got a bare number — say which unit: \
Time.seconds(…) or Time.millis(…)"
        );
    }

    /// A `subscriptions` that returns something else is a reportable error,
    /// and non-Subs are rejected at `Sub.batch` construction.
    #[test]
    fn non_subs_are_rejected() {
        let err = sub_messages_for_frame(&Value::Number(1.0), 0.0, 1.0)
            .err()
            .expect("should fail");
        assert_eq!(
            err,
            "subscriptions must return a Sub (Sub.every / Sub.none / Sub.batch), got a number"
        );
        let module = mle::lower(mle::parse("let main = () => Sub.batch([1.0])").unwrap()).unwrap();
        let failure = mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(
            failure.error.message,
            "Sub.batch items must be Subs, got a number"
        );
    }

    fn run_fail(src: &str) -> String {
        let module = mle::lower(mle::parse(src).unwrap()).unwrap();
        mle::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail")
            .error
            .message
    }

    // The E1-gap heightmap: a list of rows flattens ROW-MAJOR into the
    // protocol `Heightmap` shape — the exact wire data F#'s
    // `Scene3D.heightmap rows cols heights` emits.
    #[test]
    fn heightmap_emits_the_protocol_shape() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(\n\
               Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0),\n\
               Scene.heightmap([[0.0, 1.0], [2.0, 3.0], [4.0, 5.0]]))",
        );
        let SceneObject::Geometry(Shape::Heightmap {
            rows,
            cols,
            heights,
        }) = &frame.scene.obj
        else {
            panic!("expected a Heightmap node, got {:?}", frame.scene.obj);
        };
        assert_eq!((*rows, *cols), (3, 2));
        assert_eq!(heights, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        // And the whole thing speaks the protocol.
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(json.contains(r#""Heightmap""#), "json: {json}");
        let back: Frame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // The hello ripple shape: rows sampled with nested List builtins — F#'s
    // `heightmapFn`, in user space.
    #[test]
    fn heightmap_rows_sample_with_list_builtins() {
        let frame = frame_of(
            "let ripple = (r, c) => 0.05 * (Math.sin(c * 0.5) + Math.cos(r * 0.5))\n\
             let main = () =>\n\
             Frame.create(\n\
               Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0),\n\
               Scene.heightmap(\n\
                 List.range(4.0) |> List.map((r) =>\n\
                   List.range(4.0) |> List.map((c) => ripple(r, c)))))",
        );
        let SceneObject::Geometry(Shape::Heightmap {
            rows,
            cols,
            heights,
        }) = &frame.scene.obj
        else {
            panic!("expected a Heightmap node, got {:?}", frame.scene.obj);
        };
        assert_eq!((*rows, *cols), (4, 4));
        // heights[r * cols + c] — row-major, r the outer index.
        let expected = (0.05f64 * ((2.0f64 * 0.5).sin() + (1.0f64 * 0.5).cos())) as f32;
        assert_eq!(heights[4 + 2], expected);
    }

    // Heightmap teaching errors: degenerate grids, ragged rows, and
    // non-number heights fail loud at construction, not as broken meshes.
    #[test]
    fn heightmap_teaches_its_usage() {
        let usage = "usage: Scene.heightmap([[height, …], …]) — a list of at \
least 2 rows, each an equal-length list of at least 2 numbers";
        assert_eq!(run_fail("let main = () => Scene.heightmap(3.0)"), usage);
        assert_eq!(
            run_fail("let main = () => Scene.heightmap([[0.0, 1.0]])"),
            usage
        );
        assert_eq!(
            run_fail("let main = () => Scene.heightmap([[0.0], [1.0]])"),
            usage
        );
        assert_eq!(
            run_fail("let main = () => Scene.heightmap([[0.0, 1.0], [2.0, 3.0, 4.0]])"),
            "Scene.heightmap rows must all have the same length: row 0 has 2 heights, \
row 1 has 3"
        );
        assert_eq!(
            run_fail("let main = () => Scene.heightmap([[0.0, 1.0], 2.0])"),
            "Scene.heightmap rows must be lists of numbers, got a number"
        );
        assert_eq!(
            run_fail("let main = () => Scene.heightmap([[0.0, 1.0], [2.0, \"x\"]])"),
            "expected a number, got a string"
        );
    }

    // The E1-gap texture materials: Texture.file over the asset pipeline,
    // lit (Lambert-shaded, white tint) and emissive (fullbright) — the
    // protocol wire shapes of F#'s Material.litTexture/emissiveTexture.
    #[test]
    fn texture_materials_emit_protocol_materials() {
        let frame = frame_of(
            "let dirt = Texture.file(\"dirt.png\")\n\
             let grid = Texture.file(\"grid.png\")\n\
             let main = () =>\n\
             Frame.create(\n\
               Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0),\n\
               Scene.group([\n\
                 Scene.plane() |> Scene.litTexture(dirt),\n\
                 Scene.quad() |> Scene.emissiveTexture(grid),\n\
               ]))",
        );
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(
            json.contains(
                r#""Lit":{"color":[1.0,1.0,1.0,1.0],"texture":{"File":"dirt.png"},"normal_map":null}"#
            ),
            "json: {json}"
        );
        assert!(
            json.contains(r#""Emissive":{"color":[1.0,1.0,1.0,1.0],"texture":{"File":"grid.png"}}"#),
            "json: {json}"
        );
        // And the whole thing round-trips through the protocol.
        let back: Frame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // [units, tier 1 — the Angle rule applied to assets] texture materials
    // accept ONLY the branded Texture value; a bare path string is a
    // teaching error pointing at Texture.file.
    #[test]
    fn bare_strings_are_not_textures() {
        assert_eq!(
            run_fail("let main = () => Scene.plane() |> Scene.litTexture(\"dirt.png\")"),
            "Scene.litTexture: expected a Texture, got a bare string — build one with \
Texture.file(\"…\") and pass that value"
        );
        assert_eq!(
            run_fail("let main = () => Scene.quad() |> Scene.emissiveTexture(3.0)"),
            "Scene.emissiveTexture: expected a Texture, got a number"
        );
        assert_eq!(
            run_fail("let main = () => Texture.file(\"\")"),
            "usage: Texture.file(\"file.png\") — a non-empty image path relative to \
the game dir"
        );
    }

    // The E1-gap ui hook vocabulary: hello's HUD shape — colored text lines
    // in a column pinned to a corner — builds the same protocol View tree
    // the F# Ui module emits (color quantization included).
    #[test]
    fn ui_snippet_builds_the_hud_view_tree() {
        let value = eval(
            "let main = () =>\n\
             Ui.column([\n\
               Ui.text(\"functor · hello\"),\n\
               Ui.textColor(1.0, 0.85, 0.4, \"eye  0.0 0.0 -5.0\"),\n\
             ]) |> Ui.panel(Ui.topLeft())",
        );
        let view = view_value(&value).expect("main should return a View");
        let json = serde_json::to_string(view).expect("serialize");
        assert_eq!(
            json,
            r#"{"Panel":{"anchor":"TopLeft","child":{"Column":[{"Text":{"text":"functor · hello","color":[255,255,255],"font":null}},{"Text":{"text":"eye  0.0 0.0 -5.0","color":[255,217,102],"font":null}}]}}}"#
        );
        // And it round-trips (the wasm boundary ships Views as JSON).
        let back: View = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // Ui teaching errors: non-View children and unbranded anchors fail loud.
    #[test]
    fn ui_teaches_its_usage() {
        assert_eq!(
            run_fail("let main = () => Ui.column([Ui.text(\"a\"), 3.0])"),
            "Ui.column items must be Views, got a number"
        );
        assert_eq!(
            run_fail("let main = () => Ui.text(\"a\") |> Ui.panel(\"topLeft\")"),
            "Ui.panel: expected an Anchor, got a bare string — pin a corner \
with Ui.topLeft()"
        );
        assert_eq!(
            run_fail("let main = () => Ui.textColor(1.0, \"x\", 0.0, \"a\")"),
            "expected a number, got a string"
        );
    }
}
