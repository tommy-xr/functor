//! The Functor prelude for Functor Lang — Track C1 of `docs/functor-lang.md`.
//!
//! A [`functor_lang::Host`] giving Functor Lang programs the engine vocabulary: scene
//! constructors, transforms, a camera, and frame assembly, all producing the
//! exact protocol types this crate already speaks ([`Scene3D`], [`Camera`],
//! [`Frame`] — see [`crate::protocol`]). Engine values cross into Functor Lang as
//! opaque [`functor_lang::Value::HostData`]; Functor Lang code composes them and hands back a
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
//! Scene.color(color, scene)                                 -> Scene
//! Scene.translate(Vec3.make(x, y, z), scene)                           -> Scene
//! Scene.rotateX/rotateY/rotateZ(angle, scene)               -> Scene
//! Angle.degrees(n) / Angle.radians(n)                       -> Angle
//!   (rotations and camera angles take Angle VALUES, never bare numbers —
//!    degree/radian confusion is unrepresentable)
//! Scene.scale(k, scene)                                     -> Scene
//! Texture.file(path)                                        -> Texture
//!   (an image loaded by the shells' asset pipeline, path relative to the
//!    game dir — F#'s `Texture.file`; declared once and passed as a VALUE,
//!    the Angle rule applied to assets)
//! Scene.litTexture(texture, scene)                          -> Scene
//!   (a diffuse-lit textured surface — F#'s `Material.litTexture`)
//! Scene.emissiveTexture(texture, scene)                     -> Scene
//!   (a self-lit textured surface, fullbright — F#'s `Material.emissiveTexture`)
//! Camera.lookAt(Vec3.make(ex, ey, ez), Vec3.make(tx, ty, tz))                     -> Camera
//!   (up is +Y; vertical fov pinned at 45°, near/far at protocol defaults)
//! Frame.create(camera, scene)                               -> Frame
//! RenderTarget.named(id)                                    -> RenderTarget
//! RenderTarget.sized(w, h, target)                          -> RenderTarget
//!   (a named offscreen texture, 512x512 unless sized; declare ONCE, use the
//!    value at both sites — the writer and the reader — so writer/reader id
//!    typos are unrepresentable, the Angle rule applied to identity)
//! Frame.withRenderTarget(target, targetFrame, frame)        -> Frame
//!   (the writer: targetFrame — its own camera/scene/lights — is rendered
//!    into the target before frame's main pass; a scene sampling its own
//!    target sees last frame's image)
//! Scene.screen(target, scene)                               -> Scene
//!   (the reader: an emissive "screen" surface showing the target's texture;
//!    an id no frame declares shows magenta and warns once)
//! Fog.linear(near, far, color) / Fog.exp(density, color)    -> Fog
//! Frame.withFog(fog, frame)                                  -> Frame
//!   (frame-level distance fog on every forward material, emissive included —
//!    fog occludes glow; the fog color is also the pass's clear color)
//! Frame.withClearColor(color, frame)                         -> Frame
//!   (explicit background clear color, overriding the fog-color default; it
//!    only paints the background, not fog blending)
//! Ui.text(s) / Ui.textColor(color, s)                       -> View
//! Ui.column([view, …]) / Ui.row([view, …])                  -> View
//! Ui.panel(anchor, view)                                    -> View
//! Ui.topLeft() / topRight() / bottomLeft() / bottomRight()  -> Anchor
//! Ui.center()                                               -> Anchor
//!   (pins a panel to the screen center — e.g. a menu column)
//! Ui.button(label, msg)                                     -> View
//! Ui.slider(min, max, value, tagger)                        -> View
//! Ui.textInput(value, tagger)                               -> View
//!   (the optional `ui = (model) => …` hook's tree. `Ui.panel` takes the
//!    view LAST, so it pipes. The interactive widgets fold through
//!    `update` — docs/ui-interaction.md: a button click delivers `msg`
//!    verbatim (the Sub.every shape); a slider drag / text edit applies
//!    `tagger` to the new value (the Effect.now shape).)
//! Skybox.files(px, nx, py, ny, pz, nz)                       -> Skybox
//! Frame.withSkybox(sky, frame)                               -> Frame
//!   (a cubemap sky drawn behind everything; while the six faces load the
//!    clear color shows, a failed face disables the sky with one warning;
//!    fog does not apply to the sky — it IS the horizon)
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
//! Physics.at/velocity(v, body)                        -> Body
//! Physics.mass/friction/restitution(n, body)                -> Body
//! Physics.sensor(body)                                      -> Body
//! Physics.scene(Vec3.make(gx, gy, gz), [body, …])                      -> PhysicsScene
//! Physics.position(tag)                                     -> {x, y, z}
//! Physics.transformed(tag, scene)                           -> Scene
//! Physics.applyImpulse/applyForce/setVelocity/teleport(tag, v)
//!                                                           -> Effect
//! ```
//!
//! The `Physics.*` reads target the singleton world the shell reconciles and
//! steps each frame from the game's optional `physics` hook (see the desktop
//! `FunctorLangGame` driver + docs/physics.md). Functor Lang is interpreted in the shell's own
//! process, so these are direct reads of live world state — the seam the
//! dylib producers can't have.
//!
//! Scene-consuming functions take the scene LAST, so they compose with
//! `|>` (the piped value is appended — thread-last; see `functor_lang`'s lowering docs):
//! `Scene.cube() |> Scene.color(Color.rgb(1.0, 0.0, 0.0)) |> Scene.translate(Vec3.make(2.0, 0.0, 0.0))`.
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
//!   in world space: `s |> Scene.rotateY(r) |> Scene.translate(Vec3.make(x, 0, 0))` rotates in
//!   place, *then* moves — the order the source reads.

use cgmath::Matrix4;
use functor_lang::value::HostData;
use functor_lang::{Host, RunError, Span, Value};
use std::rc::Rc;

use crate::anim::AnimExpr;
use crate::fog::Fog;
use crate::math::Angle;
use crate::physics;
use crate::render_target::RenderTargetDescriptor;
use crate::scene3d::{MaterialDescription, ModelDescription, ModelHandle, TextureDescription};
use crate::skybox::SkyboxDescription;
use crate::ui::{self, View};
use crate::{Camera, Frame, Light, Scene3D, SceneObject, Shape};

/// A [`Scene3D`] as an opaque Functor Lang value.
pub struct FunctorLangScene(pub Scene3D);

/// A [`Camera`] as an opaque Functor Lang value.
pub struct FunctorLangCamera(pub Camera);

/// A [`Frame`] as an opaque Functor Lang value — what a Functor Lang `draw` returns.
pub struct FunctorLangFrame(pub Frame);

/// A [`Light`] as an opaque Functor Lang value.
pub struct FunctorLangLight(pub Light);

/// A duration as an opaque Functor Lang value — `Time.seconds(…)`/`Time.millis(…)`.
/// Timing functions accept ONLY this, never a bare number, making
/// seconds/milliseconds confusion unrepresentable (the Angle rule, applied
/// to time). Stored canonically in seconds.
pub struct FunctorLangDuration(pub f64);

/// A subscription tree as an opaque Functor Lang value — what the game's
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
    Every {
        period_seconds: f64,
        msg: Value,
    },
    Batch(Vec<SubTree>),
    /// Collision events from the physics step (docs/physics.md Phase 5):
    /// the tagger receives `{started, a, b, sensor}` for every contact that
    /// began/ended this frame, delivered post-step through `update` (the
    /// same point deferred queries answer).
    PhysicsEvents {
        tagger: Value,
    },
    /// Asset-loading progress (the loading-screen seam): the tagger receives
    /// `{loaded, total, failed}` whenever the shell's snapshot changes,
    /// delivered with the frame's subscription messages through `update`.
    /// NOT fired on the time grid — the producer compares snapshots.
    Assets {
        tagger: Value,
    },
    /// A persistent client connection (`Sub.connect`) or server listener
    /// (`Sub.listen`), keyed by its endpoint url/addr. NOT fired on the time
    /// grid — the producer reconciles declared keys against the live set
    /// each frame (open/close) and routes inbound `Net.NetEvent`s to the
    /// matching key's tagger through `update` (the physics-events pattern,
    /// but routed BY KEY). `listen` is the server side.
    Connect {
        key: String,
        listen: bool,
        tagger: Value,
    },
}

pub struct FunctorLangSub(pub SubTree);

/// A one-shot effect as an opaque Functor Lang value — what `update`/`tick` may
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
    /// Send text on an open connection (`Effect.send(id, text)`). Tagger-less
    /// like a physics command: performing it queues a `ConnCommand::Send` for
    /// the shell's connection manager.
    Send {
        conn: f64,
        text: String,
    },
    /// An HTTP request (`Effect.httpGet`/`httpPost`). Unlike `Now`/`Random`
    /// (same-frame) or `Raycast` (same-frame deferred), the response lands
    /// FRAMES later: performing it mints a token, queues a
    /// `NetCommand::HttpRequest`, and registers `tagger` by token
    /// ([`register_http_tagger`]). When the shell pushes the response
    /// back, the producer's per-frame pump applies the tagger to a
    /// `Net.HttpResponse` and folds the message through `update`.
    Http {
        method: crate::net::HttpMethod,
        url: String,
        body: String,
        tagger: Value,
    },
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
    /// A fire-and-forget audio one-shot (`Effect.play`/`playAt`). Tagger-less
    /// like a physics command: performing it pushes an `AudioCommand::PlayOneShot`
    /// on the shell's audio queue. `position` is `Some` for a spatialized
    /// one-shot (`playAt`), `None` for a non-spatial bed (`play`).
    PlayAudio {
        sound: String,
        position: Option<[f32; 3]>,
    },
    /// A one-shot whose completion is reported back frames later
    /// (`Effect.playThen`). Like [`Http`], the request is fire-and-forget but
    /// the RESULT needs `update`: performing it mints a token, registers
    /// `message` by it ([`register_audio_completion`]), and queues a tokened
    /// one-shot. When the shell reports the sound finished
    /// (`audio_push_finished`), the producer delivers `message` VERBATIM
    /// through `update` — unlike `Http`, there is no tagger to apply (F#'s
    /// `playThen` takes a message value, not a function).
    PlayAudioThen {
        sound: String,
        message: Value,
    },
}

pub struct FunctorLangEffect(pub EffectTree);

/// Performs effects. `Real` asks the world; `Fake` gives fixed values
/// (tests); `Replay` feeds back a recorded [`EffectLog`] — same program,
/// three worlds, one contract (docs/functor-lang.md B6).
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

/// A raycast against the ACTIVE world (a world read, not an environment read)
/// — shared by the live and dry-run runners, so both answer against whatever
/// world is scoped (the live singleton, or a forward-step's throwaway world).
fn active_world_raycast(origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> EffectValue {
    ray_result_value(
        physics::with_world(physics::active_world(), |w| w.raycast(origin, dir, max_dist))
            .flatten(),
    )
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
    /// Field order is construction order (deterministic, like Functor Lang records).
    Record(Vec<(String, EffectValue)>),
    /// A variant value: a (canonical) constructor name and its positional
    /// args — how the host hands a game a prelude-declared ADT like
    /// `Net.Connected(id)` (see the built-in `Net` module).
    Variant(String, Vec<EffectValue>),
}

impl EffectValue {
    /// The Functor Lang value handed to the effect's tagger.
    pub fn to_functor_lang(&self) -> Value {
        match self {
            EffectValue::Number(n) => Value::Number(*n),
            EffectValue::Bool(b) => Value::Bool(*b),
            EffectValue::Text(s) => Value::String(Rc::from(s.as_str())),
            EffectValue::List(items) => {
                Value::List(Rc::new(items.iter().map(EffectValue::to_functor_lang).collect()))
            }
            EffectValue::Record(fields) => Value::Record(Rc::new(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_functor_lang()))
                    .collect(),
            )),
            EffectValue::Variant(ctor, args) => Value::Variant {
                ctor: Rc::from(ctor.as_str()),
                args: Rc::new(args.iter().map(EffectValue::to_functor_lang).collect()),
            },
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
        active_world_raycast(origin, dir, max_dist)
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

/// The dry-run forward-step's runner (docs/time-travel.md T6b): ENVIRONMENT
/// reads (`now`/`random`) are deterministic — the projection must not depend
/// on wall clock or entropy — but a raycast is a WORLD read, not an
/// environment read, so it answers against the ACTIVE world (the forward-step
/// scopes that to its throwaway projected world) exactly like the live runner.
pub struct DryRunEffects(FakeEffects);

impl DryRunEffects {
    #[allow(clippy::new_without_default)]
    pub fn new() -> DryRunEffects {
        DryRunEffects(FakeEffects::new(0.0, vec![0.0]))
    }
}

impl EffectRunner for DryRunEffects {
    fn now(&mut self) -> f64 {
        self.0.now()
    }
    fn random(&mut self) -> f64 {
        self.0.random()
    }
    fn raycast(&mut self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> EffectValue {
        active_world_raycast(origin, dir, max_dist)
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

/// An [`AnimExpr`] as an opaque Functor Lang value — `Anim.clip(…)`/`Anim.blend(…)`.
/// `Scene.animate` accepts ONLY this, never a bare clip-name string (the
/// Angle rule, applied to animations): the playhead/weights are explicit in
/// the value, so the pose stays a pure function of what the game derived.
pub struct FunctorLangAnim(pub AnimExpr);

impl HostData for FunctorLangAnim {
    fn type_name(&self) -> &'static str {
        "Anim"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// An [`Angle`] as an opaque Functor Lang value — `Angle.degrees(…)`/`Angle.radians(…)`.
/// Rotation/camera functions accept ONLY this, never a bare number, making
/// degree/radian confusion unrepresentable (the F# side's `Math.Angle`
/// discipline, carried across the boundary).
pub struct FunctorLangAngle(pub Angle);

/// An RGB color as an opaque Functor Lang value — made by `Color.rgb(r, g, b)`.
/// Material/light/fog/UI color parameters accept ONLY this, never three bare
/// floats, so channel swaps and argument miscounts are unrepresentable (the
/// Angle rule, applied to color).
pub struct FunctorLangColor(pub (f32, f32, f32));

/// A 3-component vector as an opaque Functor Lang value — made by
/// `Vec3.make(x, y, z)`. Position/direction parameters accept ONLY this,
/// never three bare floats, so arity slips and float-interleaving mistakes
/// are unrepresentable (the Angle rule, applied to space).
pub struct FunctorLangVec3(pub (f32, f32, f32));

/// A [`RenderTargetDescriptor`] as an opaque Functor Lang value — declared once via
/// `RenderTarget.named` and used at both sites: the writer
/// (`Frame.withRenderTarget`) and the reader (`Scene.screen`). Both accept
/// ONLY this, never a bare string, so a writer/reader id typo is
/// unrepresentable (the Angle rule, applied to identity).
pub struct FunctorLangRenderTarget(pub RenderTargetDescriptor);

impl HostData for FunctorLangRenderTarget {
    fn type_name(&self) -> &'static str {
        "RenderTarget"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A [`TextureDescription`] as an opaque Functor Lang value — `Texture.file(…)`.
/// Texture-material functions (`Scene.litTexture` / `Scene.emissiveTexture`)
/// accept ONLY this, never a bare path string, so a texture is declared once
/// and passed as a value (the Angle rule, applied to assets).
pub struct FunctorLangTexture(pub TextureDescription);

impl HostData for FunctorLangTexture {
    fn type_name(&self) -> &'static str {
        "Texture"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Which asset family a branded [`FunctorLangAsset`] locates. Consumers check
/// it, so a sound asset can't slip into `Scene.model` (the Angle rule,
/// applied per asset kind).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AssetKind {
    Model,
    Texture,
    Sound,
}

impl AssetKind {
    /// How errors name the kind: "a model asset".
    fn noun(self) -> &'static str {
        match self {
            AssetKind::Model => "model",
            AssetKind::Texture => "texture",
            AssetKind::Sound => "sound",
        }
    }

    /// The constructor a teaching error points at.
    fn constructor(self) -> &'static str {
        match self {
            AssetKind::Model => "Asset.model(…)",
            AssetKind::Texture => "Asset.texture(…)",
            AssetKind::Sound => "Asset.sound(…)",
        }
    }

    /// The example path in the constructor's usage message.
    fn example(self) -> &'static str {
        match self {
            AssetKind::Model => "file.glb",
            AssetKind::Texture => "file.png",
            AssetKind::Sound => "file.ogg",
        }
    }
}

/// A typed asset locator as an opaque Functor Lang value — made by
/// `Asset.model` / `Asset.texture` / `Asset.sound` (the typed-manifest front
/// door). Asset-consuming functions accept it alongside the bare path string
/// (the pre-manifest form, deprecated at B.6) and check the KIND, so a
/// wrong-kind asset is a teaching error at the call instead of a silent
/// fallback at draw.
struct FunctorLangAsset {
    kind: AssetKind,
    path: String,
}

impl HostData for FunctorLangAsset {
    fn type_name(&self) -> &'static str {
        "Asset"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // A kind tag + a path string — plain data, like Color/Vec3. Manifests
    // get stored in models (`init = { mesh: Assets.barrel }`), and that must
    // not invalidate hot-reload time-travel history.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

/// A [`View`] as an opaque Functor Lang value — what the optional `ui(model)` entry
/// point returns (`Ui.text` / `Ui.column` / `Ui.panel`). The shells lower it
/// to the shared egui text overlay, exactly as the F# `ui` hook's tree.
pub struct FunctorLangView(pub View);

impl HostData for FunctorLangView {
    fn type_name(&self) -> &'static str {
        "View"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A [`ui::Anchor`] as an opaque Functor Lang value — `Ui.topLeft()`. `Ui.panel`
/// accepts ONLY this (the Angle rule, applied to screen corners).
pub struct FunctorLangUiAnchor(pub ui::Anchor);

impl HostData for FunctorLangUiAnchor {
    fn type_name(&self) -> &'static str {
        "Anchor"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A [`Fog`] as an opaque Functor Lang value — `Fog.linear(…)`/`Fog.exp(…)`.
/// `Frame.withFog` accepts ONLY this (the Angle rule).
pub struct FunctorLangFog(pub Fog);

impl HostData for FunctorLangFog {
    fn type_name(&self) -> &'static str {
        "Fog"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A [`SkyboxDescription`] as an opaque Functor Lang value — `Skybox.files(…)`.
/// `Frame.withSkybox` accepts ONLY this (the Angle rule).
pub struct FunctorLangSkybox(pub SkyboxDescription);

impl HostData for FunctorLangSkybox {
    fn type_name(&self) -> &'static str {
        "Skybox"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for FunctorLangDuration {
    fn type_name(&self) -> &'static str {
        "Duration"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for FunctorLangSub {
    fn type_name(&self) -> &'static str {
        "Sub"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for FunctorLangEffect {
    fn type_name(&self) -> &'static str {
        "Effect"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for FunctorLangColor {
    fn type_name(&self) -> &'static str {
        "Color"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // Three module-independent f32s — no taggers, no closures. A Color
    // stored in the model must not invalidate hot-reload time-travel
    // history (colors were plain floats before the brand; this preserves
    // that reload behavior and keeps "name a palette once" model-friendly).
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

impl HostData for FunctorLangVec3 {
    fn type_name(&self) -> &'static str {
        "Vec3"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // Same plain-f32 reasoning as Color: a spawn point or velocity stored
    // in the model must not invalidate hot-reload time-travel history.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

impl HostData for FunctorLangAngle {
    fn type_name(&self) -> &'static str {
        "Angle"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A [`physics::Shape`] as an opaque Functor Lang value.
pub struct FunctorLangShape(pub physics::Shape);

/// A declared [`physics::Body`] as an opaque Functor Lang value.
pub struct FunctorLangBody(pub physics::Body);

/// A [`physics::PhysicsScene`] as an opaque Functor Lang value — what a Functor Lang `physics`
/// hook returns.
pub struct FunctorLangPhysicsScene(pub physics::PhysicsScene);

impl HostData for FunctorLangShape {
    fn type_name(&self) -> &'static str {
        "Shape"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for FunctorLangBody {
    fn type_name(&self) -> &'static str {
        "Body"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for FunctorLangPhysicsScene {
    fn type_name(&self) -> &'static str {
        "PhysicsScene"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for FunctorLangLight {
    fn type_name(&self) -> &'static str {
        "Light"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A continuous soundscape voice (`AudioSource.ambient`/`at`) as an opaque Functor Lang
/// value.
pub struct FunctorLangAudioSource(pub crate::audio::AudioSource);

/// The set of voices a Functor Lang `soundScape` returns (`AudioScene.create`/`empty`)
/// as an opaque Functor Lang value.
pub struct FunctorLangAudioScene(pub crate::audio::AudioScene);

impl HostData for FunctorLangAudioSource {
    fn type_name(&self) -> &'static str {
        "AudioSource"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for FunctorLangAudioScene {
    fn type_name(&self) -> &'static str {
        "AudioScene"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for FunctorLangScene {
    fn type_name(&self) -> &'static str {
        "Scene"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for FunctorLangCamera {
    fn type_name(&self) -> &'static str {
        "Camera"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl HostData for FunctorLangFrame {
    fn type_name(&self) -> &'static str {
        "Frame"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Extract the [`Frame`] from a Functor Lang value (an `Frame.create` result), for
/// the shells' render loop.
pub fn frame_value(value: &Value) -> Option<&Frame> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<FunctorLangFrame>().map(|f| &f.0),
        _ => None,
    }
}

/// Extract the [`View`] from a Functor Lang value (a `Ui.*` result), for the shells'
/// overlay pass — the `ui` hook's [`frame_value`].
pub fn view_value(value: &Value) -> Option<&View> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<FunctorLangView>().map(|v| &v.0),
        _ => None,
    }
}

/// Extract the [`physics::PhysicsScene`] from a Functor Lang value (a `Physics.scene`
/// result), for the shells' physics drive.
pub fn physics_scene_value(value: &Value) -> Option<&physics::PhysicsScene> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<FunctorLangPhysicsScene>()
            .map(|s| &s.0),
        _ => None,
    }
}

/// The prelude host. Stateless; construct one per interpreter session.
pub struct FunctorHost;

/// Route Functor Lang `Debug.log` traces into the runtime's region-aware event stream.
///
/// The `functor_lang` crate owns a process-wide trace sink (default: print to stdout, for
/// plain `functor-lang run`). The shell calls this once at producer startup to redirect
/// it into [`events::emit`] as a [`RuntimeEvent::FunctorLangTrace`], which the CLI maps
/// to an always-visible, region-aware `Event::Log` (above the live panel / a
/// structured ndjson log event) — see `docs/cli-output.md`. It is installed on
/// the process, NOT on any `Session`, so it survives hot-reload's `Session`
/// rebuild for free; re-calling it is idempotent (the closure is stateless).
pub fn install_debug_log_sink() {
    functor_lang::set_trace_sink(Box::new(|message| {
        crate::events::emit(crate::events::RuntimeEvent::FunctorLangTrace { message });
    }));
}

/// The typed external registry (see [`crate::host_registry`]) — the ONLY
/// dispatch: every prelude external is registered here. The drift test
/// asserts `.funi` signatures ≡ the registered paths (with matching arities).
pub(crate) fn registry() -> &'static crate::host_registry::Registry {
    static REGISTRY: std::sync::OnceLock<crate::host_registry::Registry> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut reg = crate::host_registry::Registry::default();
        register_branded_constructors(&mut reg);
        register_ui_anchors(&mut reg);
        register_scene(&mut reg);
        register_camera(&mut reg);
        register_light(&mut reg);
        register_frame(&mut reg);
        register_render_resources(&mut reg);
        register_physics(&mut reg);
        register_assets(&mut reg);
        register_anim(&mut reg);
        register_effects(&mut reg);
        register_subs(&mut reg);
        register_ui_widgets(&mut reg);
        register_audio(&mut reg);
        reg
    })
}

/// The branded-value constructors — the first module migrated off the legacy
/// match (the registry proof). Each is one registration: arity/usage errors
/// and argument conversion (with the per-type teaching errors) are derived.
fn register_branded_constructors(reg: &mut crate::host_registry::Registry) {
    reg.fn1("Angle.degrees", "Angle.degrees(n)", |n: f64| {
        FunctorLangAngle(Angle::from_degrees(n as f32))
    });
    reg.fn1("Angle.radians", "Angle.radians(n)", |n: f64| {
        FunctorLangAngle(Angle::from_radians(n as f32))
    });
    reg.fn1("Time.seconds", "Time.seconds(n)", FunctorLangDuration);
    reg.fn1("Time.millis", "Time.millis(n)", |n: f64| {
        FunctorLangDuration(n / 1000.0)
    });
    reg.fn3("Color.rgb", "Color.rgb(r, g, b)", |r: f64, g: f64, b: f64| {
        FunctorLangColor((r as f32, g as f32, b as f32))
    });
    reg.fn3("Vec3.make", "Vec3.make(x, y, z)", |x: f64, y: f64, z: f64| {
        FunctorLangVec3((x as f32, y as f32, z as f32))
    });
    // Identity at runtime — the tag brand is check-time only (physics.funi).
    // Rc<str> in and out: allocation-neutral in a per-frame physics hook.
    reg.fn1("Physics.tag", "Physics.tag(\"name\")", |name: std::rc::Rc<str>| {
        Value::String(name)
    });
    const RT_NAMED: &str = "RenderTarget.named(\"id\") — a non-empty name; 512x512 unless \
piped through RenderTarget.sized";
    reg.fn1("RenderTarget.named", RT_NAMED, |name: String| {
        if name.is_empty() {
            Err(format!("usage: {RT_NAMED}"))
        } else {
            Ok(FunctorLangRenderTarget(RenderTargetDescriptor::new(name)))
        }
    });
    reg.fn3(
        "Fog.linear",
        "Fog.linear(near, far, color)",
        |near: f64, far: f64, color: FunctorLangColor| {
            if near < 0.0 {
                return Err(format!("Fog.linear near must not be negative, got {near}"));
            }
            if far <= 0.0 {
                return Err(format!("Fog.linear far must be positive, got {far}"));
            }
            if far <= near {
                return Err(format!(
                    "Fog.linear: far ({far}) must be greater than near ({near})"
                ));
            }
            let (r, g, b) = color.0;
            Ok(FunctorLangFog(Fog::linear(near as f32, far as f32, r, g, b)))
        },
    );
    reg.fn2(
        "Fog.exp",
        "Fog.exp(density, color)",
        |density: f64, color: FunctorLangColor| {
            if density <= 0.0 {
                return Err(format!("Fog.exp density must be positive, got {density}"));
            }
            let (r, g, b) = color.0;
            Ok(FunctorLangFog(Fog::exp(density as f32, r, g, b)))
        },
    );
}

crate::host_returnable!(
    FunctorLangAngle,
    FunctorLangDuration,
    FunctorLangColor,
    FunctorLangVec3,
    FunctorLangRenderTarget,
    FunctorLangFog,
    FunctorLangUiAnchor,
    FunctorLangScene,
    FunctorLangCamera,
    FunctorLangFrame,
    FunctorLangLight,
    FunctorLangTexture,
    FunctorLangSkybox,
    FunctorLangShape,
    FunctorLangBody,
    FunctorLangPhysicsScene,
    FunctorLangAnim,
    FunctorLangEffect,
    FunctorLangSub,
    FunctorLangAsset,
    FunctorLangView,
    FunctorLangAudioSource,
    FunctorLangAudioScene,
);

fn register_ui_anchors(reg: &mut crate::host_registry::Registry) {
    reg.fn0("Ui.topLeft", "Ui.topLeft()", || FunctorLangUiAnchor(ui::Anchor::TopLeft));
    reg.fn0("Ui.topRight", "Ui.topRight()", || {
        FunctorLangUiAnchor(ui::Anchor::TopRight)
    });
    reg.fn0("Ui.bottomLeft", "Ui.bottomLeft()", || {
        FunctorLangUiAnchor(ui::Anchor::BottomLeft)
    });
    reg.fn0("Ui.bottomRight", "Ui.bottomRight()", || {
        FunctorLangUiAnchor(ui::Anchor::BottomRight)
    });
    reg.fn0("Ui.center", "Ui.center()", || FunctorLangUiAnchor(ui::Anchor::Center));
}

/// A material node over a scene — the shared shape of every Scene material
/// external (`Scene.color` / `lit` / `emissive` / the texture variants /
/// `Scene.screen`).
fn material_scene(material: MaterialDescription, scene: FunctorLangScene) -> FunctorLangScene {
    FunctorLangScene(Scene3D {
        obj: SceneObject::Material(material, vec![scene.0]),
        xform: Matrix4::from_scale(1.0),
    })
}

/// A `Group` wrapper carrying `xform` over one scene — the transform
/// representation the prelude uses everywhere. Wrapping makes the OUTER call
/// apply last in world space (translate(rotateY(cube)) rotates in place then
/// moves — the order the source reads).
fn transformed(scene: FunctorLangScene, xform: Matrix4<f32>) -> FunctorLangScene {
    FunctorLangScene(group(vec![scene.0], xform))
}

/// The Scene vocabulary — geometry constructors, assets, composition,
/// materials, and transforms. Materials/transforms are scene-LAST
/// (subject-last), so they pipe: `Scene.cube() |> Scene.lit(color)`.
fn register_scene(reg: &mut crate::host_registry::Registry) {
    // Primitive geometry: constructors take no arguments — the registry
    // rejects any with the usage error, so a guessed `Scene.cube(size)`
    // fails loud instead of silently ignoring it.
    reg.fn0("Scene.cube", "Scene.cube()", || FunctorLangScene(Scene3D::cube()));
    reg.fn0("Scene.sphere", "Scene.sphere()", || FunctorLangScene(Scene3D::sphere()));
    reg.fn0("Scene.cylinder", "Scene.cylinder()", || {
        FunctorLangScene(Scene3D::cylinder())
    });
    reg.fn0("Scene.quad", "Scene.quad()", || FunctorLangScene(Scene3D::quad()));
    reg.fn0("Scene.plane", "Scene.plane()", || FunctorLangScene(Scene3D::plane()));
    // A glTF model by file path (relative to the game dir), the Functor Lang
    // face of F#'s `Model.file |> Graphics.Scene3D.model`. Loading is the
    // shells' asset pipeline; a missing file logs an error and renders as
    // the empty fallback.
    const MODEL: &str = "Scene.model(\"file.glb\") — a non-empty glTF path relative to \
the game dir";
    reg.fn1("Scene.model", MODEL, |path: ModelPath| {
        FunctorLangScene(Scene3D::model(ModelDescription {
            handle: ModelHandle::File(path.0),
            overrides: Vec::new(),
            animation: None,
        }))
    });
    // A subdivided XZ grid displaced by per-vertex heights — the Functor Lang
    // face of F#'s `Scene3D.heightmap` (the protocol `Heightmap` shape). The
    // surface is a list of ROWS (each an equal-length list of heights):
    // Functor Lang has no floor/mod to build F#'s flat row-major array, but
    // nested `List.range |> List.map` builds rows naturally — and the host
    // flattens row-major, so the wire data is exactly what the F# side
    // emits. The shape is bespoke (rows of rows, degenerate-grid checks), so
    // it takes the raw `Value` and validates by hand.
    const HEIGHTMAP: &str = "Scene.heightmap([[height, …], …]) — a list of at \
least 2 rows, each an equal-length list of at least 2 numbers";
    reg.fn1("Scene.heightmap", HEIGHTMAP, |rows: Value| {
        let Value::List(rows) = rows else {
            return Err(format!("usage: {HEIGHTMAP}"));
        };
        if rows.len() < 2 {
            return Err(format!("usage: {HEIGHTMAP}"));
        }
        let mut cols: Option<usize> = None;
        let mut heights = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            let Value::List(row) = row else {
                return Err(format!(
                    "Scene.heightmap rows must be lists of numbers, got {}",
                    row.kind_name()
                ));
            };
            match cols {
                None => {
                    if row.len() < 2 {
                        return Err(format!("usage: {HEIGHTMAP}"));
                    }
                    cols = Some(row.len());
                    heights.reserve(rows.len() * row.len());
                }
                Some(cols) if row.len() != cols => {
                    return Err(format!(
                        "Scene.heightmap rows must all have the same length: \
row 0 has {cols} heights, row {r} has {}",
                        row.len()
                    ))
                }
                Some(_) => {}
            }
            for h in row.iter() {
                match h {
                    Value::Number(n) if (*n as f32).is_finite() => heights.push(*n as f32),
                    Value::Number(n) => {
                        return Err(format!("expected a finite number, got {n}"))
                    }
                    other => {
                        return Err(format!("expected a number, got {}", other.kind_name()))
                    }
                }
            }
        }
        Ok(FunctorLangScene(Scene3D {
            obj: SceneObject::Geometry(Shape::Heightmap {
                rows: rows.len() as u32,
                cols: cols.unwrap_or(0) as u32,
                heights,
            }),
            xform: Matrix4::from_scale(1.0),
        }))
    });
    reg.fn1(
        "Scene.group",
        "Scene.group([scene, …])",
        |scenes: Vec<FunctorLangScene>| {
            FunctorLangScene(group(
                scenes.into_iter().map(|s| s.0).collect(),
                Matrix4::from_scale(1.0),
            ))
        },
    );
    reg.fn2(
        "Scene.color",
        "Scene.color(color, scene)",
        |color: FunctorLangColor, scene: FunctorLangScene| {
            let (r, g, b) = color.0;
            material_scene(MaterialDescription::color(r, g, b, 1.0), scene)
        },
    );
    // lit is shaded by the frame's lights; emissive renders fullbright.
    fn lit_or_emissive(
        emissive: bool,
    ) -> impl Fn(FunctorLangColor, FunctorLangScene) -> FunctorLangScene {
        move |color, scene| {
            let (r, g, b) = color.0;
            let material = if emissive {
                MaterialDescription::emissive(r, g, b, 1.0)
            } else {
                MaterialDescription::lit(r, g, b, 1.0)
            };
            material_scene(material, scene)
        }
    }
    reg.fn2("Scene.lit", "Scene.lit(color, scene)", lit_or_emissive(false));
    reg.fn2(
        "Scene.emissive",
        "Scene.emissive(color, scene)",
        lit_or_emissive(true),
    );
    // The F# pair `Material.litTexture` / `Material.emissiveTexture`: lit is
    // shaded by the frame's lights (white albedo tint), emissive renders
    // fullbright (neon signage).
    fn textured(
        emissive: bool,
    ) -> impl Fn(FunctorLangTexture, FunctorLangScene) -> FunctorLangScene {
        move |texture, scene| {
            let material = if emissive {
                MaterialDescription::emissive_texture(texture.0)
            } else {
                MaterialDescription::lit_texture(texture.0)
            };
            material_scene(material, scene)
        }
    }
    reg.fn2(
        "Scene.litTexture",
        "Scene.litTexture(texture, scene)",
        textured(false),
    );
    reg.fn2(
        "Scene.emissiveTexture",
        "Scene.emissiveTexture(texture, scene)",
        textured(true),
    );
    // Lit material with a tangent-space normal map perturbing the surface
    // normal (F#'s `Material.litNormalMapped`), so the lights and specular
    // play across the bumps. The color is the albedo tint; the normal map is
    // a Texture value (alpha fixed at 1.0).
    reg.fn3(
        "Scene.litNormalMapped",
        "Scene.litNormalMapped(color, normalMap, scene)",
        |color: FunctorLangColor, normal: FunctorLangTexture, scene: FunctorLangScene| {
            let (r, g, b) = color.0;
            material_scene(
                MaterialDescription::lit_normal_mapped(r, g, b, 1.0, normal.0),
                scene,
            )
        },
    );
    // Scene LAST, so it pipes: `Scene.quad() |> Scene.screen(feed)` — an
    // emissive (fullbright, screens glow) surface showing the target's
    // texture. A target no frame declares shows magenta.
    reg.fn2(
        "Scene.screen",
        "Scene.screen(target, scene)",
        |target: FunctorLangRenderTarget, scene: FunctorLangScene| {
            material_scene(
                MaterialDescription::emissive_texture(TextureDescription::render_target(
                    target.0,
                )),
                scene,
            )
        },
    );
    // Attach an animation expression to the Model node(s) in a scene
    // (scene-last, so it pipes right after `Scene.model`). Without it a
    // skinned model keeps the zero-config default: its first clip auto-plays
    // on the game clock.
    reg.fn2(
        "Scene.animate",
        "Scene.animate(anim, scene) — e.g. scene |> Scene.animate(Anim.clip(\"walk\", tts))",
        |anim: FunctorLangAnim, scene: FunctorLangScene| {
            FunctorLangScene(scene.0.with_animation(anim.0))
        },
    );
    // Transforms (scene-last). Each wraps the scene in a Group carrying the
    // matrix, so the OUTER call applies last in world space.
    reg.fn2(
        "Scene.translate",
        "Scene.translate(v, scene)",
        |v: FunctorLangVec3, scene: FunctorLangScene| {
            let (x, y, z) = v.0;
            transformed(scene, Matrix4::from_translation(cgmath::vec3(x, y, z)))
        },
    );
    reg.fn2(
        "Scene.scale",
        "Scene.scale(k, scene)",
        |k: f64, scene: FunctorLangScene| transformed(scene, Matrix4::from_scale(k as f32)),
    );
    // Non-uniform scale (the F# `Transform.scaleX/Y/Z`): stretch each axis
    // independently — e.g. a wide, short backdrop quad, or a heightmap sized
    // in XZ without inflating its Y heights.
    reg.fn4(
        "Scene.scaleXYZ",
        "Scene.scaleXYZ(x, y, z, scene)",
        |x: f64, y: f64, z: f64, scene: FunctorLangScene| {
            transformed(
                scene,
                Matrix4::from_nonuniform_scale(x as f32, y as f32, z as f32),
            )
        },
    );
    reg.fn2(
        "Scene.rotateX",
        "Scene.rotateX(angle, scene)",
        |angle: FunctorLangAngle, scene: FunctorLangScene| {
            let angle: cgmath::Rad<f32> = angle.0.into();
            transformed(scene, Matrix4::from_angle_x(angle))
        },
    );
    reg.fn2(
        "Scene.rotateY",
        "Scene.rotateY(angle, scene)",
        |angle: FunctorLangAngle, scene: FunctorLangScene| {
            let angle: cgmath::Rad<f32> = angle.0.into();
            transformed(scene, Matrix4::from_angle_y(angle))
        },
    );
    reg.fn2(
        "Scene.rotateZ",
        "Scene.rotateZ(angle, scene)",
        |angle: FunctorLangAngle, scene: FunctorLangScene| {
            let angle: cgmath::Rad<f32> = angle.0.into();
            transformed(scene, Matrix4::from_angle_z(angle))
        },
    );
}

fn register_camera(reg: &mut crate::host_registry::Registry) {
    // Eye + target (Y-up, right-handed), a fixed 45° fov.
    reg.fn2(
        "Camera.lookAt",
        "Camera.lookAt(eye, target) — Vec3 values (Vec3.make)",
        |eye: FunctorLangVec3, target: FunctorLangVec3| {
            let (ex, ey, ez) = eye.0;
            let (tx, ty, tz) = target.0;
            FunctorLangCamera(Camera::look_at(
                [ex, ey, ez],
                [tx, ty, tz],
                [0.0, 1.0, 0.0],
                Angle::from_degrees(45.0),
            ))
        },
    );
    // Eye + yaw/pitch/fov Angles: yaw = 0 / pitch = 0 looks down +Z.
    reg.fn4(
        "Camera.firstPerson",
        "Camera.firstPerson(eye, yaw, pitch, fov) — a Vec3 eye and Angle values \
(Angle.degrees/Angle.radians)",
        |eye: FunctorLangVec3, yaw: FunctorLangAngle, pitch: FunctorLangAngle, fov: FunctorLangAngle| {
            let (ex, ey, ez) = eye.0;
            FunctorLangCamera(Camera::first_person([ex, ey, ez], yaw.0, pitch.0, fov.0))
        },
    );
}

fn register_light(reg: &mut crate::host_registry::Registry) {
    reg.fn1(
        "Light.ambient",
        "Light.ambient(color)",
        |color: FunctorLangColor| {
            let (r, g, b) = color.0;
            FunctorLangLight(Light::ambient(r, g, b))
        },
    );
    reg.fn3(
        "Light.directional",
        "Light.directional(dir, color, intensity)",
        |dir: FunctorLangVec3, color: FunctorLangColor, intensity: f64| {
            let (dx, dy, dz) = dir.0;
            let (r, g, b) = color.0;
            FunctorLangLight(Light::directional(dx, dy, dz, r, g, b, intensity as f32))
        },
    );
    reg.fn4(
        "Light.point",
        "Light.point(pos, color, intensity, range)",
        |pos: FunctorLangVec3, color: FunctorLangColor, intensity: f64, range: f64| {
            let (px, py, pz) = pos.0;
            let (r, g, b) = color.0;
            FunctorLangLight(Light::point(
                px,
                py,
                pz,
                r,
                g,
                b,
                intensity as f32,
                range as f32,
            ))
        },
    );
    // A cone of light from `pos` aimed along `dir`, soft-edged at
    // `coneAngle` (an Angle from the axis) with falloff to `range`.
    // Shadow-casting when piped through `Light.castShadows`.
    reg.fn6(
        "Light.spot",
        "Light.spot(pos, dir, color, intensity, range, coneAngle)",
        |pos: FunctorLangVec3,
         dir: FunctorLangVec3,
         color: FunctorLangColor,
         intensity: f64,
         range: f64,
         cone: FunctorLangAngle| {
            let cone: cgmath::Rad<f32> = cone.0.into();
            let (px, py, pz) = pos.0;
            let (dx, dy, dz) = dir.0;
            let (r, g, b) = color.0;
            FunctorLangLight(Light::spot(
                px,
                py,
                pz,
                dx,
                dy,
                dz,
                r,
                g,
                b,
                intensity as f32,
                range as f32,
                cone.0,
            ))
        },
    );
    // Light first, so it pipes: `Light.directional(…) |> Light.castShadows`.
    reg.fn1(
        "Light.castShadows",
        "Light.castShadows(light)",
        |light: FunctorLangLight| FunctorLangLight(light.0.cast_shadows()),
    );
}

fn register_frame(reg: &mut crate::host_registry::Registry) {
    reg.fn2(
        "Frame.create",
        "Frame.create(camera, scene)",
        |camera: FunctorLangCamera, scene: FunctorLangScene| {
            FunctorLangFrame(Frame::new(camera.0, scene.0))
        },
    );
    reg.fn3(
        "Frame.createLit",
        "Frame.createLit(camera, scene, [light, …])",
        |camera: FunctorLangCamera, scene: FunctorLangScene, lights: Vec<FunctorLangLight>| {
            FunctorLangFrame(Frame {
                camera: camera.0,
                scene: scene.0,
                lights: lights.into_iter().map(|l| l.0).collect(),
                render_targets: vec![],
                fog: None,
                skybox: None,
                clear_color: None,
            })
        },
    );
    // Frame LAST (subject-last), so they pipe:
    // `Frame.createLit(…) |> Frame.withRenderTarget(feed, feedFrame)`.
    reg.fn3(
        "Frame.withRenderTarget",
        "Frame.withRenderTarget(target, targetFrame, frame) — targetFrame \
is a Frame.create/createLit(…) rendered into the target each frame, before \
frame's main pass",
        |target: FunctorLangRenderTarget, target_frame: FunctorLangFrame, frame: FunctorLangFrame| {
            FunctorLangFrame(Frame::with_render_target(frame.0, target.0, target_frame.0))
        },
    );
    reg.fn2(
        "Frame.withFog",
        "Frame.withFog(fog, frame)",
        |fog: FunctorLangFog, frame: FunctorLangFrame| {
            FunctorLangFrame(Frame::with_fog(frame.0, fog.0))
        },
    );
    reg.fn2(
        "Frame.withSkybox",
        "Frame.withSkybox(skybox, frame)",
        |sky: FunctorLangSkybox, frame: FunctorLangFrame| {
            FunctorLangFrame(Frame::with_skybox(frame.0, sky.0))
        },
    );
    // Sets the background clear color explicitly, overriding the fog-color
    // default.
    reg.fn2(
        "Frame.withClearColor",
        "Frame.withClearColor(color, frame)",
        |color: FunctorLangColor, frame: FunctorLangFrame| {
            let (r, g, b) = color.0;
            FunctorLangFrame(Frame::with_clear_color(frame.0, r, g, b))
        },
    );
}

/// The render resources the Scene/Frame vocabulary consumes: image textures,
/// render-target sizing, and skyboxes.
fn register_render_resources(reg: &mut crate::host_registry::Registry) {
    // An image texture by file path (relative to the game dir), the Functor Lang
    // face of F#'s `Texture.file`. Loading is the shells' asset pipeline; a
    // missing file logs an error and renders as the fallback texture.
    const TEXTURE_FILE: &str = "Texture.file(\"file.png\") — a non-empty image path \
relative to the game dir";
    reg.fn1("Texture.file", TEXTURE_FILE, |path: String| {
        if path.is_empty() {
            return Err(format!("usage: {TEXTURE_FILE}"));
        }
        Ok(FunctorLangTexture(TextureDescription::File(path)))
    });
    // Target LAST (subject-last), so it pipes:
    // `RenderTarget.named("x") |> RenderTarget.sized(256.0, 256.0)`.
    reg.fn3(
        "RenderTarget.sized",
        "RenderTarget.sized(width, height, target)",
        |w: f64, h: f64, target: FunctorLangRenderTarget| {
            if w <= 0.0 {
                return Err(format!("RenderTarget.sized width must be positive, got {w}"));
            }
            if h <= 0.0 {
                return Err(format!(
                    "RenderTarget.sized height must be positive, got {h}"
                ));
            }
            Ok(FunctorLangRenderTarget(target.0.sized(w as f32, h as f32)))
        },
    );
    const SKYBOX_FILES: &str = "Skybox.files(px, nx, py, ny, pz, nz) — six non-empty face \
paths (+X, -X, +Y, -Y, +Z, -Z)";
    reg.fn6(
        "Skybox.files",
        SKYBOX_FILES,
        |px: String, nx: String, py: String, ny: String, pz: String, nz: String| {
            if [&px, &nx, &py, &ny, &pz, &nz].iter().any(|s| s.is_empty()) {
                return Err(format!("usage: {SKYBOX_FILES}"));
            }
            Ok(FunctorLangSkybox(SkyboxDescription::new(px, nx, py, ny, pz, nz)))
        },
    );
}

/// The Physics vocabulary (docs/physics.md; the declarative surface): shapes
/// are values, bodies are tag + shape + piped attributes, the optional game
/// hook `physics = (model) => Physics.scene(…)` declares the world each
/// frame, and the command/query/event APIs are effects and subs.
///
/// `Physics.tag` (registered with the branded constructors) is the body
/// identity — check-time only, so at runtime a tag IS its string: the tag
/// parameters here take the string directly.
fn register_physics(reg: &mut crate::host_registry::Registry) {
    // Shapes: dimensions are strictly positive (Rapier accepts a negative
    // radius and silently builds a degenerate collider that misbehaves far
    // from the declaration — reject it loud at the boundary).
    reg.fn3(
        "Physics.box",
        "Physics.box(width, height, depth)",
        |w: f64, h: f64, d: f64| {
            Ok(FunctorLangShape(physics::Shape::Cuboid {
                extents: [
                    positive(w, "Physics.box width")? as f32,
                    positive(h, "Physics.box height")? as f32,
                    positive(d, "Physics.box depth")? as f32,
                ],
            }))
        },
    );
    reg.fn1("Physics.sphere", "Physics.sphere(radius)", |r: f64| {
        Ok(FunctorLangShape(physics::Shape::Sphere {
            radius: positive(r, "Physics.sphere radius")? as f32,
        }))
    });
    reg.fn2(
        "Physics.capsule",
        "Physics.capsule(halfHeight, radius)",
        |half_height: f64, r: f64| {
            Ok(FunctorLangShape(physics::Shape::Capsule {
                half_height: positive(half_height, "Physics.capsule halfHeight")? as f32,
                radius: positive(r, "Physics.capsule radius")? as f32,
            }))
        },
    );
    // Bodies (tag, shape). The tag brand is erased at runtime, so it arrives
    // as the plain string (Rc<str>, allocation-light like `Physics.tag`).
    fn body_ctor(
        make: fn(String, physics::Shape) -> physics::Body,
    ) -> impl Fn(std::rc::Rc<str>, FunctorLangShape) -> FunctorLangBody {
        move |tag, shape| FunctorLangBody(make(tag.to_string(), shape.0))
    }
    reg.fn2(
        "Physics.dynamic",
        "Physics.dynamic(tag, shape)",
        body_ctor(physics::Body::dynamic),
    );
    reg.fn2(
        "Physics.kinematic",
        "Physics.kinematic(tag, shape)",
        body_ctor(physics::Body::kinematic),
    );
    reg.fn2(
        "Physics.fixed",
        "Physics.fixed(tag, shape)",
        body_ctor(physics::Body::fixed),
    );
    // Body LAST (subject-last), so they pipe:
    // `Physics.dynamic(crateTag, Physics.box(1.0, 1.0, 1.0)) |> Physics.at(Vec3.make(0.0, 5.0, 0.0))`.
    reg.fn2(
        "Physics.at",
        "Physics.at(v, body)",
        |v: FunctorLangVec3, body: FunctorLangBody| {
            let (x, y, z) = v.0;
            FunctorLangBody(body.0.at([x, y, z]))
        },
    );
    reg.fn2(
        "Physics.velocity",
        "Physics.velocity(v, body)",
        |v: FunctorLangVec3, body: FunctorLangBody| {
            let (x, y, z) = v.0;
            FunctorLangBody(body.0.with_velocity([x, y, z]))
        },
    );
    reg.fn2(
        "Physics.mass",
        "Physics.mass(n, body)",
        |n: f64, body: FunctorLangBody| {
            Ok(FunctorLangBody(
                body.0.with_mass(positive(n, "Physics.mass")? as f32),
            ))
        },
    );
    reg.fn2(
        "Physics.friction",
        "Physics.friction(n, body)",
        |n: f64, body: FunctorLangBody| {
            Ok(FunctorLangBody(
                body.0
                    .with_friction(non_negative(n, "Physics.friction")? as f32),
            ))
        },
    );
    reg.fn2(
        "Physics.restitution",
        "Physics.restitution(n, body)",
        |n: f64, body: FunctorLangBody| {
            Ok(FunctorLangBody(
                body.0
                    .with_restitution(non_negative(n, "Physics.restitution")? as f32),
            ))
        },
    );
    reg.fn1("Physics.sensor", "Physics.sensor(body)", |body: FunctorLangBody| {
        FunctorLangBody(body.0.as_sensor())
    });
    reg.fn2(
        "Physics.scene",
        "Physics.scene(Vec3.make(gx, gy, gz), [body, …])",
        |g: FunctorLangVec3, bodies: Vec<FunctorLangBody>| {
            let (gx, gy, gz) = g.0;
            FunctorLangPhysicsScene(physics::PhysicsScene::create(
                [gx, gy, gz],
                bodies.into_iter().map(|b| b.0).collect(),
            ))
        },
    );
    // Reads of the LIVE stepped world (the singleton, world 0). Functor Lang
    // runs in the same process as the world the shell steps, so these are
    // direct reads — no boundary, no copy (the dylib producers can't do
    // this; Functor Lang can). A tag that isn't in the world is a loud
    // spanned error — declare the body before reading. (An Option-shaped
    // variant return could come now that B5 match exists, but
    // loud-by-default is right for the common case.)
    reg.fn1(
        "Physics.position",
        "Physics.position(tag)",
        |tag: std::rc::Rc<str>| match live_transform(&tag) {
            Some((pos, _)) => Ok(Value::Record(Rc::new(vec![
                ("x".to_string(), Value::Number(pos[0] as f64)),
                ("y".to_string(), Value::Number(pos[1] as f64)),
                ("z".to_string(), Value::Number(pos[2] as f64)),
            ]))),
            None => Err(no_body(&tag)),
        },
    );
    // Scene LAST (subject-last), so it pipes: the way Functor Lang draws a physics body —
    // `Scene.cube() |> Scene.lit(…) |> Physics.transformed(crateTag)`
    // places the visual at the body's live pose (position + rotation).
    reg.fn2(
        "Physics.transformed",
        "Physics.transformed(tag, scene)",
        |tag: std::rc::Rc<str>, scene: FunctorLangScene| match live_transform(&tag) {
            Some((pos, rot)) => {
                // cgmath's Quaternion::new is scalar-FIRST (w, x, y, z).
                let rotation = cgmath::Quaternion::new(rot[3], rot[0], rot[1], rot[2]);
                let xform = Matrix4::from_translation(cgmath::vec3(pos[0], pos[1], pos[2]))
                    * Matrix4::from(rotation);
                Ok(FunctorLangScene(group(vec![scene.0], xform)))
            }
            None => Err(no_body(&tag)),
        },
    );
    // Command EFFECTS (docs/physics.md Phase 3): fire-and-forget, returned
    // beside the model like any effect —
    // `(model, Physics.applyImpulse(ballTag, Vec3.make(0.0, 5.0, 0.0)))`.
    // Performing one queues it on the singleton world; it applies at the
    // next stepped frame's first substep, AFTER reconcile — so a body
    // declared and commanded in the same frame works.
    fn physics_command(
        make: fn(String, [f32; 3]) -> physics::PhysicsCommand,
    ) -> impl Fn(std::rc::Rc<str>, FunctorLangVec3) -> FunctorLangEffect {
        move |tag, v| {
            let (x, y, z) = v.0;
            FunctorLangEffect(EffectTree::Physics(make(tag.to_string(), [x, y, z])))
        }
    }
    reg.fn2(
        "Physics.applyImpulse",
        "Physics.applyImpulse(tag, v)",
        physics_command(|tag, impulse| physics::PhysicsCommand::ApplyImpulse { tag, impulse }),
    );
    reg.fn2(
        "Physics.applyForce",
        "Physics.applyForce(tag, v)",
        physics_command(|tag, force| physics::PhysicsCommand::ApplyForce { tag, force }),
    );
    reg.fn2(
        "Physics.setVelocity",
        "Physics.setVelocity(tag, v)",
        physics_command(|tag, velocity| physics::PhysicsCommand::SetVelocity { tag, velocity }),
    );
    reg.fn2(
        "Physics.teleport",
        "Physics.teleport(tag, v)",
        physics_command(|tag, position| physics::PhysicsCommand::Teleport { tag, position }),
    );
    // Query EFFECT (docs/physics.md Phase 4): deferred until after the
    // frame's physics step, then the tagger receives the result record
    // `{hit, x, y, z, nx, ny, nz, distance, tag}` (hit: false with zeroed
    // fields for a miss) — fresh, same-frame.
    reg.fn4(
        "Physics.raycast",
        "Physics.raycast(origin, dir, maxDist, tagger)",
        |origin: FunctorLangVec3, dir: FunctorLangVec3, max_dist: f64, tagger: Tagger| {
            let (ox, oy, oz) = origin.0;
            let (dx, dy, dz) = dir.0;
            let dir = [dx, dy, dz];
            if dir == [0.0, 0.0, 0.0] {
                return Err("Physics.raycast: the direction must not be zero".to_string());
            }
            let max_dist = positive(max_dist, "Physics.raycast maxDist")? as f32;
            Ok(FunctorLangEffect(EffectTree::Raycast {
                origin: [ox, oy, oz],
                dir,
                max_dist,
                tagger: tagger.0,
            }))
        },
    );
    // Collision-event SUB (docs/physics.md Phase 5): what `subscriptions`
    // returns (alone or in Sub.batch). The tagger receives
    // {started, a, b, sensor} per contact begin/end, post-step (like query
    // answers).
    reg.fn1("Physics.events", "Physics.events(tagger)", |tagger: Tagger| {
        FunctorLangSub(SubTree::PhysicsEvents { tagger: tagger.0 })
    });
}

/// Typed asset locators (the typed-manifest front door): a branded value
/// naming an asset by path, constructed per KIND so a wrong-kind asset is a
/// teaching error at the consumer instead of a silent fallback at draw.
/// These are the PERMANENT dynamic constructors — the typed manifest
/// `functor import` grows into (Track B.2) will call the same ones; strings
/// live only at data boundaries.
fn register_assets(reg: &mut crate::host_registry::Registry) {
    // Dual-shape by hand (the ModelPath rule): a wrong TYPE gets the usage
    // line naming the accepted form, not a misleading "expected a string" —
    // and an empty path gets the same line.
    fn asset_ctor(
        path: &'static str,
        kind: AssetKind,
    ) -> impl Fn(Value) -> Result<FunctorLangAsset, String> {
        move |value| match value {
            Value::String(p) if !p.is_empty() => Ok(FunctorLangAsset {
                kind,
                path: p.to_string(),
            }),
            _ => Err(format!(
                "usage: {path}(\"{}\") — a non-empty {} path relative to the game dir",
                kind.example(),
                kind.noun(),
            )),
        }
    }
    reg.fn1(
        "Asset.model",
        "Asset.model(\"file.glb\") — a non-empty model path relative to the game dir",
        asset_ctor("Asset.model", AssetKind::Model),
    );
    reg.fn1(
        "Asset.texture",
        "Asset.texture(\"file.png\") — a non-empty texture path relative to the game dir",
        asset_ctor("Asset.texture", AssetKind::Texture),
    );
    reg.fn1(
        "Asset.sound",
        "Asset.sound(\"file.ogg\") — a non-empty sound path relative to the game dir",
        asset_ctor("Asset.sound", AssetKind::Sound),
    );
}

/// The Anim pose algebra — clip sampling, blending, the rest pose, additive
/// layers, masks, and per-joint rotation. Playheads/weights are explicit in
/// the values, so the pose stays a pure function of what the game derived.
fn register_anim(reg: &mut crate::host_registry::Registry) {
    // A clip sample as a value: the named glTF clip at a playhead in seconds
    // (looping by the clip's duration). The playhead is explicit — derive it
    // from `tts` / model state — so the pose is a pure function of the
    // frame's inputs and time-travel replays it exactly.
    const CLIP: &str = "Anim.clip(\"walk\", playheadSeconds) — a non-empty clip name \
(functor inspect lists a model's clips) and a playhead in seconds (loops)";
    reg.fn2("Anim.clip", CLIP, |name: String, playhead: f64| {
        if name.is_empty() {
            return Err(format!("usage: {CLIP}"));
        }
        Ok(FunctorLangAnim(AnimExpr::Clip {
            name,
            playhead: playhead as f32,
        }))
    });
    // A weighted mix of animations: [(anim, weight), …]. Weights are
    // normalized; a non-positive weight drops its entry (so driving a
    // weight to 0.0 in game code cleanly silences that clip). The entries
    // are TUPLES — a bespoke shape with per-entry teaching errors, so it
    // takes the raw `Value` and validates by hand (the extractors' RunError
    // spans are discarded: the registry reattaches the call span).
    const BLEND: &str = "Anim.blend([(anim, weight), …]) — a non-empty list of \
(Anim, weight) pairs, e.g. Anim.blend([(Anim.clip(\"walk\", tts), 0.7), \
(Anim.clip(\"run\", tts), 0.3)])";
    reg.fn1("Anim.blend", BLEND, |items: Value| {
        let Value::List(items) = items else {
            return Err(format!("usage: {BLEND}"));
        };
        if items.is_empty() {
            return Err(format!("usage: {BLEND}"));
        }
        let span = Span::new(0, 0);
        let mut blended = Vec::with_capacity(items.len());
        for item in items.iter() {
            let Value::Tuple(pair) = item else {
                return Err(format!(
                    "Anim.blend entries must be (anim, weight) tuples, got {}",
                    item.kind_name()
                ));
            };
            let [anim, weight] = pair.as_slice() else {
                return Err(format!(
                    "Anim.blend entries must be (anim, weight) pairs, got a \
{}-tuple",
                    pair.len()
                ));
            };
            let anim = anim_of(anim, "Anim.blend", span)
                .map_err(|e| e.message)?
                .clone();
            let weight = num(weight, span).map_err(|e| e.message)? as f32;
            blended.push((anim, weight));
        }
        Ok(FunctorLangAnim(AnimExpr::Blend(blended)))
    });
    // The bind (rest) pose — the base for purely programmatic posing
    // (Anim.rotate on a model with no authored clips, e.g. a hand).
    reg.fn0("Anim.rest", "Anim.rest()", || FunctorLangAnim(AnimExpr::Rest));
    // Additive layer (anim-last so the BASE pipes):
    // walk |> Anim.add(Anim.clip("headShake", tts), 1.0) layers the
    // shake's delta-from-bind on top of the walk.
    reg.fn3(
        "Anim.add",
        "Anim.add(layerAnim, weight, base) — layer the anim's delta-from-bind \
on top, scaled by weight; base last, so it pipes: base |> Anim.add(layerAnim, weight)",
        |layer: FunctorLangAnim, weight: f64, base: FunctorLangAnim| {
            FunctorLangAnim(AnimExpr::Add {
                base: Box::new(base.0),
                layer: Box::new(layer.0),
                weight: weight as f32,
            })
        },
    );
    // Restrict an anim's influence to the subtrees rooted at the named
    // joints (a name covers itself and every descendant).
    const MASK: &str = "Anim.mask([\"jointName\", …], anim) — a non-empty list of joint \
names; each covers its whole subtree (functor inspect lists a model's joints)";
    reg.fn2("Anim.mask", MASK, |joints: Vec<String>, anim: FunctorLangAnim| {
        if joints.is_empty() {
            return Err(format!("usage: {MASK}"));
        }
        if joints.iter().any(|j| j.is_empty()) {
            // An empty NAME is a string, so the legacy arm's kind-naming
            // error read "got a string" — kept byte-identical.
            return Err(
                "Anim.mask joint names must be non-empty strings, got a string".to_string(),
            );
        }
        Ok(FunctorLangAnim(AnimExpr::Mask {
            joints,
            expr: Box::new(anim.0),
        }))
    });
    // Post-multiply an additive local rotation onto one joint —
    // programmatic per-joint control (head aim, finger curl). Angles are
    // branded values (the Angle rule): XYZ Euler, local frame.
    const ROTATE: &str = "Anim.rotate(\"jointName\", xAngle, yAngle, zAngle, anim) — a \
non-empty joint name and three Angle values (Angle.degrees/radians), applied as an \
additive local XYZ rotation";
    reg.fn5(
        "Anim.rotate",
        ROTATE,
        |joint: String,
         x: FunctorLangAngle,
         y: FunctorLangAngle,
         z: FunctorLangAngle,
         anim: FunctorLangAnim| {
            if joint.is_empty() {
                return Err(format!("usage: {ROTATE}"));
            }
            let x: cgmath::Rad<f32> = x.0.into();
            let y: cgmath::Rad<f32> = y.0.into();
            let z: cgmath::Rad<f32> = z.0.into();
            Ok(FunctorLangAnim(AnimExpr::Rotate {
                joint,
                euler: [x.0, y.0, z.0],
                expr: Box::new(anim.0),
            }))
        },
    );
}

/// The Effect vocabulary — fire-and-forget commands returned beside the
/// model. Taggers are validated callable at construction (the [`Tagger`]
/// rule) so a typo fails at the call, not frames later when the result lands.
fn register_effects(reg: &mut crate::host_registry::Registry) {
    reg.fn0("Effect.none", "Effect.none()", || FunctorLangEffect(EffectTree::None));
    // The tagger is a Functor Lang function value, applied by the producer with the
    // performed result.
    reg.fn1("Effect.now", "Effect.now(tagger)", |tagger: Tagger| {
        FunctorLangEffect(EffectTree::Now { tagger: tagger.0 })
    });
    reg.fn1("Effect.random", "Effect.random(tagger)", |tagger: Tagger| {
        FunctorLangEffect(EffectTree::Random { tagger: tagger.0 })
    });
    reg.fn1(
        "Effect.batch",
        "Effect.batch([effect, …])",
        |fx: Vec<FunctorLangEffect>| {
            FunctorLangEffect(EffectTree::Batch(fx.into_iter().map(|f| f.0).collect()))
        },
    );
    reg.fn2(
        "Effect.send",
        "Effect.send(connId, text)",
        |conn: f64, text: String| {
            // A connection id is a non-negative whole number the host handed
            // the game; reject garbage rather than truncate it to some OTHER
            // live client.
            if conn < 0.0 || conn.fract() != 0.0 || conn > u64::MAX as f64 {
                return Err(format!(
                    "Effect.send: connId must be a non-negative whole number, got {conn}"
                ));
            }
            Ok(FunctorLangEffect(EffectTree::Send { conn, text }))
        },
    );
    // httpGet(url, tagger) / httpPost(url, body, tagger). The tagger is a
    // function of the Net.HttpResponse; performing the effect (drain) mints a
    // token, queues the request, and registers the tagger by token — see
    // EffectTree::Http.
    reg.fn2(
        "Effect.httpGet",
        "Effect.httpGet(url, tagger)",
        |url: String, tagger: Tagger| {
            FunctorLangEffect(EffectTree::Http {
                method: crate::net::HttpMethod::Get,
                url,
                body: String::new(),
                tagger: tagger.0,
            })
        },
    );
    reg.fn3(
        "Effect.httpPost",
        "Effect.httpPost(url, body, tagger)",
        |url: String, body: String, tagger: Tagger| {
            FunctorLangEffect(EffectTree::Http {
                method: crate::net::HttpMethod::Post,
                url,
                body,
                tagger: tagger.0,
            })
        },
    );
    // Fire-and-forget audio one-shots (the dual of `soundScape`). `play` is
    // non-spatial; `playAt` is positioned. Both queue an AudioCommand on
    // drain — see EffectTree::PlayAudio.
    reg.fn1("Effect.play", "Effect.play(sound)", |sound: SoundPath| {
        FunctorLangEffect(EffectTree::PlayAudio {
            sound: sound.0,
            position: None,
        })
    });
    reg.fn2(
        "Effect.playAt",
        "Effect.playAt(sound, v)",
        |sound: SoundPath, v: FunctorLangVec3| {
            let (x, y, z) = v.0;
            FunctorLangEffect(EffectTree::PlayAudio {
                sound: sound.0,
                position: Some([x, y, z]),
            })
        },
    );
    // Play once and deliver `msg` (a message VALUE, not a tagger) when the
    // sound finishes — see EffectTree::PlayAudioThen. Any value is a valid
    // message, so there is nothing to validate.
    reg.fn2(
        "Effect.playThen",
        "Effect.playThen(sound, msg)",
        |sound: SoundPath, msg: Value| {
            FunctorLangEffect(EffectTree::PlayAudioThen {
                sound: sound.0,
                message: msg,
            })
        },
    );
}

/// The Sub vocabulary — what the optional `subscriptions` hook returns.
fn register_subs(reg: &mut crate::host_registry::Registry) {
    reg.fn0("Sub.none", "Sub.none()", || FunctorLangSub(SubTree::None));
    // The msg is any Functor Lang value (typically an ADT variant), held by the
    // host and handed back verbatim when the timer fires.
    reg.fn2(
        "Sub.every",
        "Sub.every(duration, msg)",
        |duration: FunctorLangDuration, msg: Value| {
            FunctorLangSub(SubTree::Every {
                period_seconds: duration.0,
                msg,
            })
        },
    );
    reg.fn1("Sub.batch", "Sub.batch([sub, …])", |subs: Vec<FunctorLangSub>| {
        FunctorLangSub(SubTree::Batch(subs.into_iter().map(|s| s.0).collect()))
    });
    // Asset-loading progress SUB: the tagger receives
    // `{loaded, total, failed}` whenever the loading snapshot changes (see
    // functor_lang_producer's delivery). The loading-screen seam.
    reg.fn1("Sub.assets", "Sub.assets(tagger)", |tagger: Tagger| {
        FunctorLangSub(SubTree::Assets { tagger: tagger.0 })
    });
    // A persistent connection (client) / listener (server): the key is the
    // endpoint; the tagger receives `Net.NetEvent` values.
    fn conn(listen: bool) -> impl Fn(String, Tagger) -> FunctorLangSub {
        move |key, tagger| {
            FunctorLangSub(SubTree::Connect {
                key,
                listen,
                tagger: tagger.0,
            })
        }
    }
    reg.fn2("Sub.connect", "Sub.connect(endpoint, tagger)", conn(false));
    reg.fn2("Sub.listen", "Sub.listen(endpoint, tagger)", conn(true));
}

/// The Ui widgets (the optional `ui = (model) => …` hook): text lines,
/// stacks, panels, and the interactive widgets (docs/ui-interaction.md).
/// The shells lower the View through the shared egui overlay.
fn register_ui_widgets(reg: &mut crate::host_registry::Registry) {
    reg.fn1("Ui.text", "Ui.text(\"…\")", |text: String| {
        FunctorLangView(View::Text {
            text,
            color: [255, 255, 255],
            font: None,
        })
    });
    reg.fn2(
        "Ui.textColor",
        "Ui.textColor(color, \"…\")",
        |color: FunctorLangColor, text: String| {
            let (r, g, b) = color.0;
            FunctorLangView(View::Text {
                text,
                color: ui::rgb_u8(r, g, b),
                font: None,
            })
        },
    );
    reg.fn1("Ui.column", "Ui.column([view, …])", |views: Vec<FunctorLangView>| {
        FunctorLangView(View::Column(views.into_iter().map(|v| v.0).collect()))
    });
    reg.fn1("Ui.row", "Ui.row([view, …])", |views: Vec<FunctorLangView>| {
        FunctorLangView(View::Row(views.into_iter().map(|v| v.0).collect()))
    });
    // View LAST (subject-last), so it pipes:
    // `Ui.column([…]) |> Ui.panel(Ui.topLeft())`.
    reg.fn2(
        "Ui.panel",
        "Ui.panel(anchor, view)",
        |anchor: FunctorLangUiAnchor, view: FunctorLangView| {
            FunctorLangView(View::Panel {
                anchor: anchor.0,
                child: Box::new(view.0),
            })
        },
    );
    // The msg is any Functor Lang value (typically an ADT variant), registered in
    // the frame's handler table and delivered VERBATIM through `update` when
    // the shell reports a click on the stamped slot — the `Sub.every`
    // message shape.
    reg.fn2(
        "Ui.button",
        "Ui.button(\"label\", msg) — msg is delivered to update on click",
        |label: String, msg: Value| {
            let slot = push_ui_handler(UiHandler::Msg(msg));
            FunctorLangView(View::Button { slot, label })
        },
    );
    // A slider over min..=max showing the MODEL's value (a controlled
    // widget, docs/ui-interaction.md U4). The tagger is applied to the
    // dragged value and the message folds through `update` — the
    // `Effect.now` tagger shape.
    reg.fn4(
        "Ui.slider",
        "Ui.slider(min, max, value, tagger) — tagger: (newValue) => msg",
        |min: f64, max: f64, value: f64, tagger: Tagger| {
            if max <= min {
                return Err(format!(
                    "Ui.slider: max ({max}) must be greater than min ({min})"
                ));
            }
            let slot = push_ui_handler(UiHandler::Tagger(tagger.0));
            Ok(FunctorLangView(View::Slider {
                slot,
                min,
                max,
                value,
            }))
        },
    );
    // A single-line text input showing the MODEL's text (controlled,
    // docs/ui-interaction.md U4). The tagger is applied to the new text on
    // each edit — the `Effect.now` tagger shape.
    reg.fn2(
        "Ui.textInput",
        "Ui.textInput(value, tagger) — tagger: (newText) => msg",
        |value: String, tagger: Tagger| {
            let slot = push_ui_handler(UiHandler::Tagger(tagger.0));
            FunctorLangView(View::TextInput { slot, value })
        },
    );
}

/// The audio vocabulary — soundscape voices (the continuous, reconciled half
/// of audio; the one-shots live in `register_effects`). Voices are keyed for
/// cross-frame identity so the shell keeps a live voice playing.
fn register_audio(reg: &mut crate::host_registry::Registry) {
    // `ambient` is a non-spatial bed; `at` is positioned.
    reg.fn2(
        "AudioSource.ambient",
        "AudioSource.ambient(key, sound)",
        |key: String, sound: SoundPath| {
            FunctorLangAudioSource(crate::audio::AudioSource::ambient(key, sound.0))
        },
    );
    reg.fn3(
        "AudioSource.at",
        "AudioSource.at(key, sound, v)",
        |key: String, sound: SoundPath, v: FunctorLangVec3| {
            let (x, y, z) = v.0;
            FunctorLangAudioSource(crate::audio::AudioSource::at(key, sound.0, x, y, z))
        },
    );
    // Source LAST (subject-last) so it pipes:
    // `AudioSource.ambient(…) |> AudioSource.gain(0.35)`.
    reg.fn2(
        "AudioSource.gain",
        "AudioSource.gain(gain, source)",
        |gain: f64, source: FunctorLangAudioSource| {
            FunctorLangAudioSource(source.0.with_gain(gain as f32))
        },
    );
    reg.fn1(
        "AudioScene.create",
        "AudioScene.create([source, …])",
        |sources: Vec<FunctorLangAudioSource>| {
            FunctorLangAudioScene(crate::audio::AudioScene::new(
                sources.into_iter().map(|s| s.0).collect(),
            ))
        },
    );
    reg.fn0("AudioScene.empty", "AudioScene.empty()", || {
        FunctorLangAudioScene(crate::audio::AudioScene::default())
    });
}

/// Branded values convert through the registry with the same teaching errors
/// the legacy extractors give — written once, on each type. Handle-shaped
/// types without a teaching extractor (Scene, Light, Camera, Frame) get the
/// uniform "{path}: expected a X, got {kind}" typed error.
impl crate::host_registry::FromArg for FunctorLangColor {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        color_of(value, path, span).map(FunctorLangColor)
    }
}

/// A model-asset argument: a non-empty path string, or an `Asset.model`
/// locator (a wrong-kind asset gets `asset_path`'s teaching error) — the
/// typed-manifest front door at the registry seam.
struct ModelPath(String);

impl crate::host_registry::FromArg for ModelPath {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            Value::String(p) if !p.is_empty() => Ok(ModelPath(p.to_string())),
            v if asset_of(v).is_some() => {
                asset_path(v, AssetKind::Model, path, span).map(ModelPath)
            }
            _ => Err(RunError {
                message: "usage: Scene.model(\"file.glb\") — a non-empty glTF path \
relative to the game dir"
                    .to_string(),
                span,
            }),
        }
    }
}

/// A sound argument (the [`ModelPath`] dual-accept pattern, for audio —
/// #384): a non-empty path string, or an `Asset.sound` locator (a wrong-kind
/// asset gets `asset_path`'s teaching error). The legacy arms answered a
/// wrong-type sound with their own full `usage:` line; those flatten to this
/// shared teaching text (a pinned delta).
struct SoundPath(String);

impl crate::host_registry::FromArg for SoundPath {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            Value::String(p) if !p.is_empty() => Ok(SoundPath(p.to_string())),
            v if asset_of(v).is_some() => {
                asset_path(v, AssetKind::Sound, path, span).map(SoundPath)
            }
            _ => Err(RunError {
                message: format!(
                    "{path}: expected a sound — a non-empty path string or \
Asset.sound(\"file.ogg\")"
                ),
                span,
            }),
        }
    }
}

/// A tagger argument — a function of the performed result (a closure or an
/// ADT constructor), validated callable at construction so a typo fails at
/// the call, not frames later when the result lands.
struct Tagger(Value);

impl crate::host_registry::FromArg for Tagger {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            Value::Closure(_) | Value::Ctor { .. } => Ok(Tagger(value.clone())),
            other => Err(RunError {
                message: format!(
                    "{path}: the tagger must be a function of the result record, got {}",
                    other.kind_name()
                ),
                span,
            }),
        }
    }
}

/// Implements [`crate::host_registry::FromArg`] for a handle newtype by
/// downcasting, with the uniform typed error. `$name` carries its article
/// ("a Scene", "an Effect") so the message reads naturally.
macro_rules! handle_arg {
    ($($ty:ident => $name:literal),+ $(,)?) => {$(
        impl crate::host_registry::FromArg for $ty {
            fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
                if let Value::HostData(data) = value {
                    if let Some(inner) = data.as_any().downcast_ref::<$ty>() {
                        return Ok($ty(inner.0.clone()));
                    }
                }
                Err(RunError {
                    message: format!(
                        "{path}: expected {}, got {}",
                        $name,
                        value.kind_name()
                    ),
                    span,
                })
            }
        }
    )+};
}

handle_arg!(
    FunctorLangScene => "a Scene",
    FunctorLangLight => "a Light",
    FunctorLangCamera => "a Camera",
    FunctorLangFrame => "a Frame",
    FunctorLangShape => "a Shape",
    FunctorLangBody => "a Body",
    FunctorLangEffect => "an Effect",
    FunctorLangSub => "a Sub",
    FunctorLangView => "a View",
    FunctorLangAudioSource => "an AudioSource",
);

impl crate::host_registry::FromArg for FunctorLangAngle {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        angle_of(value, path, span).map(FunctorLangAngle)
    }
}

impl crate::host_registry::FromArg for FunctorLangVec3 {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        vec3_of(value, path, span).map(FunctorLangVec3)
    }
}

impl crate::host_registry::FromArg for FunctorLangTexture {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        texture_of(value, path, span).map(FunctorLangTexture)
    }
}

impl crate::host_registry::FromArg for FunctorLangAnim {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        anim_of(value, path, span).map(|a| FunctorLangAnim(a.clone()))
    }
}

impl crate::host_registry::FromArg for FunctorLangFog {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        fog_of(value, path, span).map(|f| FunctorLangFog(f.clone()))
    }
}

impl crate::host_registry::FromArg for FunctorLangRenderTarget {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        target_of(value, path, span).map(|t| FunctorLangRenderTarget(t.clone()))
    }
}

impl crate::host_registry::FromArg for FunctorLangSkybox {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        skybox_of(value, path, span).map(|s| FunctorLangSkybox(s.clone()))
    }
}

impl crate::host_registry::FromArg for FunctorLangDuration {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        duration_of(value, path, span).map(FunctorLangDuration)
    }
}

// `ui_anchor_of` names `Ui.panel` in its teaching errors itself (the only
// anchor-consuming external), so the path parameter is unused.
impl crate::host_registry::FromArg for FunctorLangUiAnchor {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        ui_anchor_of(value, span).map(FunctorLangUiAnchor)
    }
}

impl Host for FunctorHost {
    fn provides(&self, path: &str) -> bool {
        registry().provides(path)
    }

    fn call(&mut self, path: &str, args: Vec<Value>, span: Span) -> Result<Value, RunError> {
        match registry().call(path, &args, span) {
            Some(result) => result,
            // Defensive: unreachable in practice — eval checks `provides()`
            // (registry-only) and errors "unknown external `…`" before ever
            // calling the host. Kept with the legacy fallback's text so
            // nothing observable changes if a caller skips that check.
            None => Err(RunError {
                message: format!("internal: unregistered prelude path `{path}`"),
                span,
            }),
        }
    }
}

/// Protocol scalars must be finite f32s: NaN/inf (which Functor Lang numbers permit —
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
            .downcast_ref::<FunctorLangAngle>()
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

/// Extract an RGB color — color-taking functions accept ONLY Color values,
/// so bare numbers get a teaching error instead of a silent channel guess
/// (the [`angle_of`] rule, applied to color).
fn color_of(value: &Value, what: &str, span: Span) -> Result<(f32, f32, f32), RunError> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<FunctorLangColor>()
            .map(|c| c.0)
            .ok_or_else(|| RunError {
                message: format!("{what}: expected a Color, got {}", value.kind_name()),
                span,
            }),
        Value::Number(_) => Err(RunError {
            message: format!(
                "{what}: expected a Color, got a bare number — wrap the channels: \
Color.rgb(r, g, b)"
            ),
            span,
        }),
        other => Err(RunError {
            message: format!("{what}: expected a Color, got {}", other.kind_name()),
            span,
        }),
    }
}

/// Extract a 3-vector — position/direction parameters accept ONLY Vec3
/// values, so bare numbers get a teaching error instead of a silent axis
/// guess (the [`angle_of`] rule, applied to space).
fn vec3_of(value: &Value, what: &str, span: Span) -> Result<(f32, f32, f32), RunError> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<FunctorLangVec3>()
            .map(|v| v.0)
            .ok_or_else(|| RunError {
                message: format!("{what}: expected a Vec3, got {}", value.kind_name()),
                span,
            }),
        Value::Number(_) => Err(RunError {
            message: format!(
                "{what}: expected a Vec3, got a bare number — wrap the components: \
Vec3.make(x, y, z)"
            ),
            span,
        }),
        other => Err(RunError {
            message: format!("{what}: expected a Vec3, got {}", other.kind_name()),
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
            .downcast_ref::<FunctorLangDuration>()
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

fn sub_of(value: &Value) -> Option<&FunctorLangSub> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<FunctorLangSub>(),
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
        // Connections are reconciled + routed by the producer, not fired.
        SubTree::Connect { .. } => {}
        // Progress subs fire on snapshot changes, not the time grid — the
        // producer collects their taggers via `assets_taggers`.
        SubTree::Assets { .. } => {}
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

/// One declared connection from `subscriptions` — the producer reconciles
/// these keys (open/close) and routes inbound events to their taggers.
pub struct NetConnSub {
    pub key: String,
    pub listen: bool,
    pub tagger: Value,
}

/// The `Sub.connect`/`Sub.listen` declarations in a sub tree, in declaration
/// order. Same non-Sub error as `sub_messages_for_frame`.
pub fn net_conn_subs(subs: &Value) -> Result<Vec<NetConnSub>, String> {
    let Some(sub) = sub_of(subs) else {
        return Err(format!(
            "subscriptions must return a Sub (Sub.every / Sub.none / Sub.batch), got {}",
            subs.kind_name()
        ));
    };
    let mut out = Vec::new();
    collect_conn_subs(&sub.0, &mut out);
    Ok(out)
}

fn collect_conn_subs(sub: &SubTree, out: &mut Vec<NetConnSub>) {
    match sub {
        SubTree::Batch(items) => {
            for item in items.iter() {
                collect_conn_subs(item, out);
            }
        }
        SubTree::Connect {
            key,
            listen,
            tagger,
        } => out.push(NetConnSub {
            key: key.clone(),
            listen: *listen,
            tagger: tagger.clone(),
        }),
        SubTree::None
        | SubTree::Every { .. }
        | SubTree::PhysicsEvents { .. }
        | SubTree::Assets { .. } => {}
    }
}

/// Build the `Net.NetEvent` value the host hands a connection's tagger. The
/// canonical ctor names come from the built-in `Net` module (see
/// `functor_lang::project`); `text` is the message/error payload (unused for
/// Connected/Disconnected).
pub fn net_event_value(kind: NetEventKind, conn: u64, text: &str) -> EffectValue {
    let id = EffectValue::Number(conn as f64);
    match kind {
        NetEventKind::Connected => EffectValue::Variant("Net.Connected".into(), vec![id]),
        NetEventKind::Disconnected => EffectValue::Variant("Net.Disconnected".into(), vec![id]),
        NetEventKind::Message => EffectValue::Variant(
            "Net.Message".into(),
            vec![id, EffectValue::Text(text.to_string())],
        ),
        NetEventKind::Error => EffectValue::Variant(
            "Net.Error".into(),
            vec![id, EffectValue::Text(text.to_string())],
        ),
    }
}

/// Build the `Net.HttpResponse` value handed to an `Effect.httpGet`/`httpPost`
/// tagger: `Response(status, body)` for a completed request (any HTTP status),
/// `Failure(error)` for a transport error. Canonical ctor names from the
/// built-in `Net` module (see `functor_lang::project`).
pub fn http_response_value(result: &crate::net::HttpResult) -> Value {
    let variant = if result.is_ok() {
        EffectValue::Variant(
            "Net.Response".into(),
            vec![
                EffectValue::Number(result.status as f64),
                EffectValue::Text(result.body_text()),
            ],
        )
    } else {
        EffectValue::Variant("Net.Failure".into(), vec![EffectValue::Text(result.error_text())])
    };
    variant.to_functor_lang()
}

thread_local! {
    /// In-flight `Effect.httpGet`/`httpPost` taggers, keyed by request token —
    /// the Functor Lang analogue of the F# `net::registry`. Populated when the effect is
    /// performed (see `EffectTree::Http`); drained by the producer's HTTP
    /// arrival hook when the response lands. The map is self-draining: the
    /// shell's dispatch (`net_dispatch::perform_http`) returns EXACTLY ONE
    /// completion per request — a `Response` for any HTTP status or an `Error`
    /// for a transport failure — so every registered token is later taken. It
    /// is bounded by concurrently in-flight requests, and cleared on hot reload
    /// (an in-flight tagger closes over the OLD session and must not outlive
    /// it — the same rule the producer applies to deferred queries).
    static PENDING_HTTP: std::cell::RefCell<std::collections::HashMap<u64, Value>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Register an HTTP tagger for `token` (see [`PENDING_HTTP`]).
pub fn register_http_tagger(token: u64, tagger: Value) {
    PENDING_HTTP.with(|m| m.borrow_mut().insert(token, tagger));
}

/// Take the tagger registered for a completed request's `token`, if any (a hot
/// reload clears the map, so a late response can arrive orphaned — dropped).
pub fn take_http_tagger(token: u64) -> Option<Value> {
    PENDING_HTTP.with(|m| m.borrow_mut().remove(&token))
}

/// Drop all in-flight HTTP taggers (called on hot reload).
pub fn clear_http_taggers() {
    PENDING_HTTP.with(|m| m.borrow_mut().clear());
}

thread_local! {
    /// In-flight `Effect.playThen` completion MESSAGES, keyed by the one-shot's
    /// token — the audio analogue of [`PENDING_HTTP`]. Populated when the effect
    /// is performed (see `EffectTree::PlayAudioThen`); drained by the producer's
    /// audio-finished hook when the sound ends (`audio_push_finished`). Unlike
    /// the HTTP map this holds a plain message VALUE, delivered verbatim (F#'s
    /// `playThen` takes a message, not a tagger). Bounded by the sounds in
    /// flight, and cleared on hot reload (a stored message may close over the
    /// OLD session — the same rule the producer applies to HTTP taggers).
    static PENDING_AUDIO: std::cell::RefCell<std::collections::HashMap<u64, Value>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Cap on in-flight `playThen` completion messages. Unlike [`PENDING_HTTP`]
/// — whose shell dispatch returns EXACTLY ONE completion per token, so the map
/// self-drains — an audio finish is BEST-EFFORT: a sound that never starts (no
/// device, decode failure, a headless run that drops the command), or a backend
/// that does not report finishes (wasm today), leaves its message un-taken. This
/// bound keeps a game that fires `playThen` in a loop from growing the map
/// without limit; on overflow the OLDEST pending message (lowest token) is
/// evicted — its completion, if it ever arrives, is then dropped like a
/// hot-reloaded one.
const PENDING_AUDIO_CAP: usize = 4096;

/// Register a `playThen` completion message for `token` (see [`PENDING_AUDIO`]).
pub fn register_audio_completion(token: u64, message: Value) {
    PENDING_AUDIO.with(|m| {
        let mut map = m.borrow_mut();
        map.insert(token, message);
        // Best-effort finishes mean this map is not self-draining; evict the
        // oldest entry rather than grow unbounded (see PENDING_AUDIO_CAP).
        while map.len() > PENDING_AUDIO_CAP {
            if let Some(oldest) = map.keys().copied().min() {
                map.remove(&oldest);
            } else {
                break;
            }
        }
    });
}

/// Take the completion message for a finished one-shot's `token`, if any (a hot
/// reload clears the map, so a late finish can arrive orphaned — dropped).
pub fn take_audio_completion(token: u64) -> Option<Value> {
    PENDING_AUDIO.with(|m| m.borrow_mut().remove(&token))
}

/// Drop all in-flight `playThen` completion messages (called on hot reload).
pub fn clear_audio_completions() {
    PENDING_AUDIO.with(|m| m.borrow_mut().clear());
}

/// What an interactive UI widget delivers when the shell reports an
/// interaction on it (docs/ui-interaction.md): either a message VALUE handed
/// to `update` verbatim (a button — the `Sub.every` shape) or a TAGGER applied
/// to the event's payload first (a slider / text input — the `Effect.now`
/// shape).
pub enum UiHandler {
    Msg(Value),
    Tagger(Value),
}

thread_local! {
    /// The handler table for the `ui(model)` evaluation in progress: each
    /// interactive `Ui.*` constructor pushes its handler and stamps the node
    /// with the returned slot index (construction order). The producer drains
    /// it with [`take_ui_handlers`] right after the evaluation and keeps it
    /// beside the frame's cached `View`, so an event the shell reports later
    /// resolves against the exact table that produced the tree it saw. Unlike
    /// [`PENDING_HTTP`] this never spans frames — it is rebuilt every `ui`
    /// evaluation — so hot reload needs no clearing here (the producer drops
    /// its own kept copy instead).
    static UI_HANDLERS: std::cell::RefCell<Vec<UiHandler>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Register a widget's handler for the `ui(model)` evaluation in progress and
/// return its slot index (see [`UI_HANDLERS`]).
pub fn push_ui_handler(handler: UiHandler) -> u32 {
    UI_HANDLERS.with(|h| {
        let mut handlers = h.borrow_mut();
        handlers.push(handler);
        (handlers.len() - 1) as u32
    })
}

/// Drain the handlers the just-finished `ui(model)` evaluation registered.
/// Call it after EVERY evaluation — including a failed one, whose partial
/// table must not leak into the next frame's slots.
pub fn take_ui_handlers() -> Vec<UiHandler> {
    UI_HANDLERS.with(|h| std::mem::take(&mut *h.borrow_mut()))
}

/// Put a saved handler table back — the inspector-replay bracket: replaying a
/// journaled call (or draw) that evaluates `Ui.*` pushes handlers the NEXT
/// real `ui` pass would take as its own, shifting every slot. The replay
/// saves with [`take_ui_handlers`], runs, drops what it pushed, and restores.
pub fn restore_ui_handlers(handlers: Vec<UiHandler>) {
    UI_HANDLERS.with(|h| *h.borrow_mut() = handlers);
}

#[derive(Clone, Copy)]
pub enum NetEventKind {
    Connected,
    Message,
    Disconnected,
    Error,
}

fn collect_event_taggers(sub: &SubTree, taggers: &mut Vec<Value>) {
    match sub {
        SubTree::None
        | SubTree::Every { .. }
        | SubTree::Connect { .. }
        | SubTree::Assets { .. } => {}
        SubTree::Batch(items) => {
            for item in items.iter() {
                collect_event_taggers(item, taggers);
            }
        }
        SubTree::PhysicsEvents { tagger } => taggers.push(tagger.clone()),
    }
}

/// The `Sub.assets` taggers in a subscription tree, in declaration order —
/// the producer applies each to the progress record whenever the loading
/// snapshot changes and folds the messages through `update`. A non-Sub value
/// yields the same error `sub_messages_for_frame` reports.
pub fn assets_taggers(subs: &Value) -> Result<Vec<Value>, String> {
    let Some(sub) = sub_of(subs) else {
        return Err(format!(
            "subscriptions must return a Sub (Sub.every / Sub.none / Sub.batch), got {}",
            subs.kind_name()
        ));
    };
    let mut taggers = Vec::new();
    collect_assets_taggers(&sub.0, &mut taggers);
    Ok(taggers)
}

fn collect_assets_taggers(sub: &SubTree, taggers: &mut Vec<Value>) {
    match sub {
        SubTree::None
        | SubTree::Every { .. }
        | SubTree::PhysicsEvents { .. }
        | SubTree::Connect { .. } => {}
        SubTree::Batch(items) => {
            for item in items.iter() {
                collect_assets_taggers(item, taggers);
            }
        }
        SubTree::Assets { tagger } => taggers.push(tagger.clone()),
    }
}

/// The tagger-facing record for a `Sub.assets` progress snapshot:
/// `{loaded, total, failed: [{path, error}, …]}`. The settled gate is
/// `total > 0 && loaded + List.length(failed) == total` — failures never
/// join `loaded`, and frame one can deliver 0/0 (see examples/loading).
pub fn asset_progress_value(progress: &crate::asset::AssetProgress) -> Value {
    let failed: Vec<Value> = progress
        .failed
        .iter()
        .map(|(path, error)| {
            Value::Record(Rc::new(vec![
                ("path".to_string(), Value::String(Rc::from(path.as_str()))),
                ("error".to_string(), Value::String(Rc::from(error.as_str()))),
            ]))
        })
        .collect();
    Value::Record(Rc::new(vec![
        ("loaded".to_string(), Value::Number(progress.loaded as f64)),
        ("total".to_string(), Value::Number(progress.total as f64)),
        ("failed".to_string(), Value::List(Rc::new(failed))),
    ]))
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
    session: &functor_lang::Session,
    model: &mut Value,
    taggers: &[Value],
    events: &[physics::PhysicsEvent],
    runner: &mut dyn EffectRunner,
    log: &mut EffectLog,
    report: &mut dyn FnMut(String),
    suppress_outbound: bool,
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
                    report(format!("[functor-lang] Physics.events tagger error: {}", e.message));
                    continue;
                }
            };
            // Journal this collision-driven `update` for the paused inspector
            // (PR2); a no-op unless journaling is armed.
            let args = vec![model.clone(), msg];
            crate::functor_lang_producer::journal_push(
                "update",
                &args,
                crate::functor_lang_producer::Provenance::Collision,
            );
            match session.call("update", args, &mut FunctorHost) {
                Ok(returned) => {
                    let (next_model, more) = split_model_effect(returned);
                    *model = next_model;
                    if let Some(more) = more {
                        drain_mode(
                            session,
                            model,
                            vec![more],
                            runner,
                            log,
                            report,
                            None,
                            suppress_outbound,
                        );
                    }
                }
                Err(e) => report(format!("[functor-lang] update error: {}", e.message)),
            }
        }
    }
}

/// Physical dimensions (shape extents, radii, mass) must be strictly
/// positive: Rapier accepts a negative radius and silently builds a
/// degenerate collider that misbehaves far from the declaration — so reject
/// it loud at the boundary.
fn positive(n: f64, what: &str) -> Result<f64, String> {
    if n > 0.0 {
        Ok(n)
    } else {
        Err(format!("{what} must be positive, got {n}"))
    }
}

/// Friction/restitution are coefficients: zero is meaningful, negative is not.
fn non_negative(n: f64, what: &str) -> Result<f64, String> {
    if n >= 0.0 {
        Ok(n)
    } else {
        Err(format!("{what} must not be negative, got {n}"))
    }
}

fn effect_of(value: &Value) -> Option<&FunctorLangEffect> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<FunctorLangEffect>(),
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
/// only (host values are opaque and cannot hold Functor Lang values).
pub fn contains_effect(value: &Value) -> bool {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<FunctorLangEffect>().is_some(),
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
    session: &functor_lang::Session,
    model: &mut Value,
    first: EffectTree,
    runner: &mut dyn EffectRunner,
    log: &mut EffectLog,
    report: &mut dyn FnMut(String),
    suppress_outbound: bool,
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
        suppress_outbound,
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
        EffectTree::None
        | EffectTree::Physics(_)
        | EffectTree::Send { .. }
        // The request is fire-and-forget; its RESPONSE needs `update`, but
        // that arrives frames later through the producer's HTTP pump.
        | EffectTree::Http { .. }
        // Audio one-shots are fire-and-forget too. `playThen`'s completion
        // needs `update`, but arrives frames later via the audio-finished
        // hook — the same shape as Http.
        | EffectTree::PlayAudio { .. }
        | EffectTree::PlayAudioThen { .. } => false,
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
    session: &functor_lang::Session,
    model: &mut Value,
    deferred: Vec<EffectTree>,
    runner: &mut dyn EffectRunner,
    log: &mut EffectLog,
    report: &mut dyn FnMut(String),
    suppress_outbound: bool,
) {
    if deferred.is_empty() {
        return;
    }
    drain_mode(
        session,
        model,
        deferred,
        runner,
        log,
        report,
        None,
        suppress_outbound,
    );
}

fn drain_mode(
    session: &functor_lang::Session,
    model: &mut Value,
    queue: Vec<EffectTree>,
    runner: &mut dyn EffectRunner,
    log: &mut EffectLog,
    report: &mut dyn FnMut(String),
    // `Some` = pre-step drain: hold queries here instead of performing.
    // `None` = post-step drain: answer queries now.
    mut defer_queries: Option<&mut Vec<EffectTree>>,
    // When true (a dry-run forward-step, docs/time-travel.md T6b), the SIX
    // outbound arms (physics command, timeline control, net send, http, audio,
    // audioThen) still LOG and `continue` but skip their `push_*`/`queue_*`/
    // `register_*` side effect — the model still evolves (runner reads proceed),
    // but nothing escapes to the live world / global queues. Always `false` on
    // the live path.
    suppress_outbound: bool,
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
                "[functor-lang] effect drain hit the per-frame cap ({MAX_EFFECTS_PER_FRAME}); \
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
                // Outbound suppression protects the LIVE world (a dry run must
                // not kick real bodies) — but a dry-run world SCOPE routes
                // commands to its own throwaway world, which is exactly where a
                // replayed kick belongs (docs/time-travel.md T6b).
                let target = physics::active_world();
                if !suppress_outbound || target != physics::DEFAULT_WORLD {
                    physics::with_world(target, |w| w.queue_command(command));
                }
                log.push(EffectRecord {
                    kind,
                    value: EffectValue::Text(tag),
                });
                if log.len() > EFFECT_LOG_CAP {
                    log.remove(0);
                }
                continue;
            }
            EffectTree::Send { conn, text } => {
                // Fire-and-forget, like a physics command: queue a Send on the
                // shell's connection manager (drained via
                // net_drain_conn_commands). Logged for the effect record.
                if !suppress_outbound {
                    crate::net::push_conn_command(crate::net::ConnCommand::Send {
                        conn: conn as crate::net::ConnectionId,
                        payload: text.clone().into_bytes(),
                    });
                }
                log.push(EffectRecord {
                    kind: "net.send",
                    value: EffectValue::Text(text),
                });
                if log.len() > EFFECT_LOG_CAP {
                    log.remove(0);
                }
                continue;
            }
            EffectTree::Http {
                method,
                url,
                body,
                tagger,
            } => {
                // Fire-and-forget request (like Send): mint a token, register
                // the tagger by it, and queue the HttpRequest for the shell to
                // perform. The response lands frames later; the producer's HTTP
                // pump applies the tagger then. Logged for the effect record.
                if !suppress_outbound {
                    let token = crate::net::next_token();
                    register_http_tagger(token, tagger);
                    crate::net::push_command(crate::net::NetCommand::HttpRequest {
                        token,
                        method,
                        url: url.clone(),
                        headers: Vec::new(),
                        body: body.into_bytes(),
                    });
                }
                log.push(EffectRecord {
                    kind: "net.http",
                    value: EffectValue::Text(url),
                });
                if log.len() > EFFECT_LOG_CAP {
                    log.remove(0);
                }
                continue;
            }
            EffectTree::PlayAudio { sound, position } => {
                // Fire-and-forget one-shot: push an AudioCommand on the shell's
                // audio queue (drained via audio_drain_commands). No token —
                // nothing folds back through update.
                if !suppress_outbound {
                    crate::audio::push_command(crate::audio::AudioCommand::PlayOneShot {
                        token: None,
                        sound: sound.clone(),
                        gain: 1.0,
                        position,
                    });
                }
                log.push(EffectRecord {
                    kind: "audio.play",
                    value: EffectValue::Text(sound),
                });
                if log.len() > EFFECT_LOG_CAP {
                    log.remove(0);
                }
                continue;
            }
            EffectTree::PlayAudioThen { sound, message } => {
                // Fire-and-forget request (like Http): mint a token, register
                // the completion MESSAGE by it, and queue a tokened one-shot.
                // The finish lands frames later; the producer's audio-finished
                // hook delivers the message then. Logged for the effect record.
                if !suppress_outbound {
                    let token = crate::audio::next_token();
                    register_audio_completion(token, message);
                    crate::audio::push_command(crate::audio::AudioCommand::play_one_shot_token(
                        token,
                        sound.clone(),
                    ));
                }
                log.push(EffectRecord {
                    kind: "audio.playThen",
                    value: EffectValue::Text(sound),
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
        let functor_lang_value = value.to_functor_lang();
        log.push(EffectRecord { kind, value });
        if log.len() > EFFECT_LOG_CAP {
            log.remove(0);
        }
        let msg = match session.apply(
            tagger,
            vec![functor_lang_value],
            &format!("Effect.{kind} tagger"),
            &mut FunctorHost,
        ) {
            Ok(msg) => msg,
            Err(e) => {
                report(format!("[functor-lang] Effect.{kind} tagger error: {}", e.message));
                continue;
            }
        };
        // Journal this effect-result `update` for the paused inspector (PR2): a
        // raycast result is a physics QUERY, everything else an effect result.
        // A no-op unless journaling is armed (the live desktop frame only).
        let args = vec![model.clone(), msg];
        let provenance = if kind == "physics.raycast" {
            crate::functor_lang_producer::Provenance::PhysicsQuery
        } else {
            crate::functor_lang_producer::Provenance::EffectResult
        };
        crate::functor_lang_producer::journal_push("update", &args, provenance);
        match session.call("update", args, &mut FunctorHost) {
            Ok(returned) => {
                let (next_model, more) = split_model_effect(returned);
                *model = next_model;
                if let Some(more) = more {
                    queue.push(more);
                }
            }
            Err(e) => report(format!("[functor-lang] update error: {}", e.message)),
        }
    }
}

/// Extract an [`AnimExpr`] — animation-consuming functions accept ONLY the
/// branded value, so the predictable mistake (a bare clip-name string) gets a
/// teaching error pointing at `Anim.clip` (the [`angle_of`] rule).
fn anim_of<'a>(value: &'a Value, what: &str, span: Span) -> Result<&'a AnimExpr, RunError> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<FunctorLangAnim>()
            .map(|a| &a.0)
            .ok_or_else(|| RunError {
                message: format!("{what}: expected an Anim, got {}", value.kind_name()),
                span,
            }),
        Value::String(_) => Err(RunError {
            message: format!(
                "{what}: got a bare clip name — wrap it with a playhead: \
Anim.clip(\"walk\", tts)"
            ),
            span,
        }),
        other => Err(RunError {
            message: format!(
                "{what}: expected an Anim (Anim.clip / Anim.blend), got {}",
                other.kind_name()
            ),
            span,
        }),
    }
}

/// Extract the [`crate::audio::AudioScene`] from a Functor Lang value (a
/// `soundScape` return), for the shells' soundscape reconcile.
pub fn audio_scene_of(value: &Value) -> Option<&crate::audio::AudioScene> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<FunctorLangAudioScene>().map(|s| &s.0),
        _ => None,
    }
}

/// Extract a [`SkyboxDescription`] — `Frame.withSkybox` accepts ONLY the
/// branded value, so the predictable mistake (a bare path string) gets a
/// teaching error pointing at `Skybox.files` (the [`angle_of`] rule).
fn skybox_of<'a>(
    value: &'a Value,
    what: &str,
    span: Span,
) -> Result<&'a SkyboxDescription, RunError> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<FunctorLangSkybox>()
            .map(|s| &s.0)
            .ok_or_else(|| RunError {
                message: format!("{what}: expected a Skybox, got {}", value.kind_name()),
                span,
            }),
        Value::String(_) => Err(RunError {
            message: format!(
                "{what}: expected a Skybox, got a bare string — build one with \
Skybox.files(px, nx, py, ny, pz, nz)"
            ),
            span,
        }),
        other => Err(RunError {
            message: format!("{what}: expected a Skybox, got {}", other.kind_name()),
            span,
        }),
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
            .downcast_ref::<FunctorLangRenderTarget>()
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
            .downcast_ref::<FunctorLangUiAnchor>()
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

/// Extract a [`TextureDescription`] — texture materials accept the branded
/// `Texture.file` value or a texture Asset (`Asset.texture`), never a bare
/// path string: that predictable mistake gets a teaching error pointing at
/// the constructors (the [`angle_of`] rule, applied to assets).
fn texture_of(value: &Value, what: &str, span: Span) -> Result<TextureDescription, RunError> {
    match value {
        Value::HostData(data) => {
            if let Some(t) = data.as_any().downcast_ref::<FunctorLangTexture>() {
                return Ok(t.0.clone());
            }
            if asset_of(value).is_some() {
                // A texture asset carries a file path; a wrong-kind asset is
                // a teaching error from asset_path.
                return asset_path(value, AssetKind::Texture, what, span)
                    .map(TextureDescription::File);
            }
            Err(RunError {
                message: format!("{what}: expected a Texture, got {}", value.kind_name()),
                span,
            })
        }
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

/// Downcast to the branded Asset locator, if the value is one.
fn asset_of(value: &Value) -> Option<&FunctorLangAsset> {
    match value {
        Value::HostData(data) => data.as_any().downcast_ref::<FunctorLangAsset>(),
        _ => None,
    }
}

/// The path inside an Asset argument, enforcing the KIND: a sound asset
/// where a model asset belongs is a teaching error naming the right
/// constructor. Callers guard with [`asset_of`] first.
fn asset_path(value: &Value, kind: AssetKind, what: &str, span: Span) -> Result<String, RunError> {
    let asset = asset_of(value).expect("asset_path callers guard with asset_of");
    if asset.kind == kind {
        Ok(asset.path.clone())
    } else {
        Err(RunError {
            message: format!(
                "{what}: expected a {} asset, got a {} asset (\"{}\") — construct it with {}",
                kind.noun(),
                asset.kind.noun(),
                asset.path,
                kind.constructor(),
            ),
            span,
        })
    }
}

/// Extract a [`Fog`] — `Frame.withFog` accepts ONLY the branded value, so
/// the predictable mistake (a bare number where a Fog belongs) gets a
/// teaching error pointing at the constructors (the [`angle_of`] rule).
fn fog_of<'a>(value: &'a Value, what: &str, span: Span) -> Result<&'a Fog, RunError> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<FunctorLangFog>()
            .map(|f| &f.0)
            .ok_or_else(|| RunError {
                message: format!("{what}: expected a Fog, got {}", value.kind_name()),
                span,
            }),
        Value::Number(_) => Err(RunError {
            message: format!(
                "{what}: expected a Fog, got a bare number — build one with \
Fog.linear(near, far, color) or Fog.exp(density, color)"
            ),
            span,
        }),
        other => Err(RunError {
            message: format!("{what}: expected a Fog, got {}", other.kind_name()),
            span,
        }),
    }
}

/// Live pose of a body in the ACTIVE world (normally the singleton world the
/// shell steps — same process, same crate statics as this prelude; under a
/// dry-run forward-step scope, the throwaway projected world, so ghost draws
/// read the stepped poses — docs/time-travel.md T6b).
fn live_transform(tag: &str) -> Option<([f32; 3], [f32; 4])> {
    physics::with_world(physics::active_world(), |w| w.body_transform(tag)).flatten()
}

fn no_body(tag: &str) -> String {
    format!(
        "no body tagged \"{tag}\" in the physics world (bodies exist after the \
         frame's `physics` declaration has been reconciled and stepped)"
    )
}

/// A `Group` wrapper carrying `xform` — the transform representation the
/// prelude uses everywhere (see the module doc for why).
fn group(scenes: Vec<Scene3D>, xform: Matrix4<f32>) -> Scene3D {
    Scene3D {
        obj: SceneObject::Group(scenes),
        xform,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use functor_lang::Tracing;

    /// Downcast to the Scene inside a host value (test-side only since the
    /// registry migration — the prelude consumes Scenes via FromArg).
    fn scene_of(value: &Value) -> Option<&Scene3D> {
        match value {
            Value::HostData(data) => data.as_any().downcast_ref::<FunctorLangScene>().map(|s| &s.0),
            _ => None,
        }
    }

    /// Downcast to the AudioSource inside a host value (test-side only since
    /// the registry migration — the prelude consumes sources via FromArg).
    fn audio_source_of(value: &Value) -> Option<&crate::audio::AudioSource> {
        match value {
            Value::HostData(data) => data
                .as_any()
                .downcast_ref::<FunctorLangAudioSource>()
                .map(|s| &s.0),
            _ => None,
        }
    }

    /// Evaluate a Functor Lang `main` under the prelude and return its value.
    fn eval(src: &str) -> Value {
        let module = functor_lang::lower(functor_lang::parse(src).expect("parse")).expect("lower");
        let record = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("run failed: {}", f.error.message));
        match record.outcome {
            functor_lang::RunOutcome::Main(value) => value,
            _ => panic!("expected a main result"),
        }
    }

    fn frame_of(src: &str) -> Frame {
        let value = eval(src);
        frame_value(&value)
            .expect("main should return a Frame")
            .clone()
    }

    // Drift guard: a HARD BIJECTION between the `functor-prelude` `.funi`
    // signatures and the registered externals. Every `.funi` `val`/`let`
    // signature must name a real host external, AND every host external must
    // have exactly one signature — so the declared types and the Rust
    // implementations cannot silently diverge in either direction. We parse
    // the `.funi` sources to extract `Module.name` signatures and compare the
    // two sets.
    #[test]
    fn prelude_signatures_map_to_host_paths() {
        use std::collections::BTreeSet;
        let mut signatures: BTreeSet<String> = BTreeSet::new();
        for (module, src) in functor_prelude::modules() {
            let program = functor_lang::parse_interface(&src)
                .unwrap_or_else(|e| panic!("prelude module `{module}` must parse: {}", e.message));
            for item in &program.items {
                if let functor_lang::ast::Item::Sig(sig) = item {
                    let path = format!("{module}.{}", sig.name);
                    assert!(
                        signatures.insert(path.clone()),
                        "prelude signature `{path}` is authored more than once"
                    );
                }
            }
        }

        let paths: BTreeSet<String> = registry().paths().map(|p| p.to_string()).collect();

        let phantom: Vec<&String> = signatures.difference(&paths).collect();
        assert!(
            phantom.is_empty(),
            "prelude signatures with no registered host external \
(functor_lang_prelude.rs) — phantom signatures: {phantom:?}"
        );

        let missing: Vec<&String> = paths.difference(&signatures).collect();
        assert!(
            missing.is_empty(),
            "registered host externals with no `.funi` signature — an interface-only \
module is CLOSED, so games referencing these break at load: {missing:?}"
        );

        // Registered arities must match the `.funi` signatures' parameter
        // counts — the registry's usage text and the interface cannot teach
        // different shapes. (A `.funi` function type parses as the reserved
        // name `=>` with params + return as args.)
        let mut funi_arity: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (module, src) in functor_prelude::modules() {
            let program = functor_lang::parse_interface(&src).expect("parsed above");
            for item in &program.items {
                if let functor_lang::ast::Item::Sig(sig) = item {
                    let arity = if sig.ty.name == "=>" {
                        sig.ty.args.len() - 1
                    } else {
                        0
                    };
                    funi_arity.insert(format!("{module}.{}", sig.name), arity);
                }
            }
        }
        for (path, arity) in registry().arities() {
            assert_eq!(
                funi_arity.get(path),
                Some(&arity),
                "registered arity for `{path}` must match its .funi signature"
            );
        }
    }

    // The C1 verify criterion (docs/functor-lang.md): an .fun snippet emits exactly
    // the protocol data the shells consume — pinned as the serialized wire
    // form the protocol tests use.
    #[test]
    fn functor_lang_snippet_emits_protocol_frame() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(Camera.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())",
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
               Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
               Scene.cube() |> Scene.color(Color.rgb(1.0, 0.0, 0.0)) |> Scene.translate(Vec3.make(2.0, 0.0, 0.0)))",
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
               Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
               Scene.cube() |> Scene.rotateY(Angle.degrees(90.0)) |> Scene.translate(Vec3.make(3.0, 0.0, 0.0)))",
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

    #[test]
    fn scale_xyz_is_non_uniform() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(\n\
               Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
               Scene.cube() |> Scene.scaleXYZ(2.0, 3.0, 4.0))",
        );
        // The wrapper Group carries a non-uniform scale on its diagonal — the
        // three axes stretch independently (unlike Scene.scale's uniform k).
        let s = frame.scene.xform;
        assert_eq!((s.x.x, s.y.y, s.z.z), (2.0, 3.0, 4.0));
    }

    // The hello-cubes shape: a List.map-built group of colored cubes.
    #[test]
    fn mapped_group_builds_n_children() {
        let frame = frame_of(
            "let cubeAt = (i) => Scene.cube() |> Scene.color(Color.rgb(1.0, 0.5, 0.2)) |> Scene.translate(Vec3.make(i, 0.0, 0.0))\n\
             let main = () =>\n\
             Frame.create(\n\
               Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
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
               Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
               Scene.model(\"shark.glb\") |> Scene.scale(0.002) |> Scene.translate(Vec3.make(3.0, 1.0, 3.0)))",
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
    // spanned error, not a silently-empty scene. (Dual-accept — a path
    // string OR an Asset.model locator — so a wrong TYPE gets the usage
    // line naming the accepted forms, not a misleading "expected a string".)
    #[test]
    fn model_requires_a_nonempty_path_string() {
        assert_eq!(
            run_fail("let main = () => Scene.model(42.0)"),
            "usage: Scene.model(\"file.glb\") — a non-empty glTF path relative to \
the game dir"
        );
        assert_eq!(
            run_fail("let main = () => Scene.model(\"\")"),
            "usage: Scene.model(\"file.glb\") — a non-empty glTF path relative to \
the game dir"
        );
    }

    // --- typed assets (Track B.1) ---

    /// A run failure's message, for teaching-error assertions.
    fn fail_message(src: &str) -> String {
        let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
        functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail")
            .error
            .message
    }

    /// `Asset.model` flows into `Scene.model` exactly like the (deprecated)
    /// bare path string — byte-identical protocol frames.
    #[test]
    fn asset_model_matches_string_form() {
        let camera = "Camera.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0))";
        let by_string =
            frame_of(&format!("let main = () => Frame.create({camera}, Scene.model(\"shark.glb\"))"));
        let by_asset = frame_of(&format!(
            "let main = () => Frame.create({camera}, Scene.model(Asset.model(\"shark.glb\")))"
        ));
        assert_eq!(
            serde_json::to_string(&by_string).unwrap(),
            serde_json::to_string(&by_asset).unwrap()
        );
    }

    /// `Asset.texture` feeds the texture materials exactly like a
    /// `Texture.file` value (both lit and the normal-map slot).
    #[test]
    fn asset_texture_matches_texture_file_form() {
        let camera = "Camera.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0))";
        for (by_texture, by_asset) in [
            (
                "Scene.plane() |> Scene.litTexture(Texture.file(\"wood.png\"))",
                "Scene.plane() |> Scene.litTexture(Asset.texture(\"wood.png\"))",
            ),
            (
                "Scene.plane() |> Scene.litNormalMapped(Color.rgb(1.0, 1.0, 1.0), Texture.file(\"n.png\"))",
                "Scene.plane() |> Scene.litNormalMapped(Color.rgb(1.0, 1.0, 1.0), Asset.texture(\"n.png\"))",
            ),
        ] {
            let a = frame_of(&format!("let main = () => Frame.create({camera}, {by_texture})"));
            let b = frame_of(&format!("let main = () => Frame.create({camera}, {by_asset})"));
            assert_eq!(
                serde_json::to_string(&a).unwrap(),
                serde_json::to_string(&b).unwrap()
            );
        }
    }

    /// `Asset.sound` feeds the soundscape voices exactly like a bare path
    /// (the key stays a string — identity, not an asset).
    #[test]
    fn asset_sound_matches_string_form_in_soundscape() {
        let by_string = eval("let main = () => AudioSource.ambient(\"bed\", \"wind.ogg\")");
        let by_asset =
            eval("let main = () => AudioSource.ambient(\"bed\", Asset.sound(\"wind.ogg\"))");
        assert_eq!(
            audio_source_of(&by_string).unwrap(),
            audio_source_of(&by_asset).unwrap()
        );
        let by_string = eval(
            "let main = () => AudioSource.at(\"fire\", \"crackle.ogg\", Vec3.make(1.0, 0.0, 2.0))",
        );
        let by_asset = eval(
            "let main = () => AudioSource.at(\"fire\", Asset.sound(\"crackle.ogg\"), Vec3.make(1.0, 0.0, 2.0))",
        );
        assert_eq!(
            audio_source_of(&by_string).unwrap(),
            audio_source_of(&by_asset).unwrap()
        );
    }

    /// `Asset.sound` in the one-shot effects queues the same AudioCommands
    /// as the bare-path form.
    #[test]
    fn asset_sound_matches_string_form_in_one_shots() {
        let _guard = crate::audio::OUTBOUND_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = crate::audio::drain_commands(); // clear the shared queue
        let src = "\
            let init = 0.0\n\
            let shoot = Effect.play(Asset.sound(\"gunshot.wav\"))\n\
            let blast = Effect.playAt(Asset.sound(\"explosion.wav\"), Vec3.make(5.0, 0.5, -2.0))\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let mut model = session.global("init").unwrap();
        let mut log = EffectLog::new();
        for name in ["shoot", "blast"] {
            let effect = effect_of(&session.global(name).unwrap()).unwrap().0.clone();
            let _ = drain_effects(
                &session,
                &mut model,
                effect,
                &mut FakeEffects::new(0.0, vec![]),
                &mut log,
                &mut |m| panic!("unexpected report: {m}"),
                false,
            );
        }
        assert_eq!(
            crate::audio::drain_commands(),
            vec![
                crate::audio::AudioCommand::PlayOneShot {
                    token: None,
                    sound: "gunshot.wav".to_string(),
                    gain: 1.0,
                    position: None,
                },
                crate::audio::AudioCommand::PlayOneShot {
                    token: None,
                    sound: "explosion.wav".to_string(),
                    gain: 1.0,
                    position: Some([5.0, 0.5, -2.0]),
                },
            ]
        );
    }

    /// The Asset constructors teach their usage: a bare number or an empty
    /// path is a spanned error naming the kind's example file.
    #[test]
    fn asset_constructors_teach_usage() {
        for (src, want) in [
            (
                "let main = () => Asset.model(42.0)",
                "usage: Asset.model(\"file.glb\") — a non-empty model path relative to the game dir",
            ),
            (
                "let main = () => Asset.model(\"\")",
                "usage: Asset.model(\"file.glb\") — a non-empty model path relative to the game dir",
            ),
            (
                "let main = () => Asset.texture(42.0)",
                "usage: Asset.texture(\"file.png\") — a non-empty texture path relative to the game dir",
            ),
            (
                "let main = () => Asset.sound()",
                "usage: Asset.sound(\"file.ogg\") — a non-empty sound path relative to the game dir",
            ),
        ] {
            assert_eq!(fail_message(src), want, "for {src}");
        }
    }

    /// A wrong-kind asset at a consumer is a teaching error naming the value,
    /// its actual kind, and the constructor that fits — never a silent
    /// fallback at draw.
    #[test]
    fn wrong_kind_assets_are_teaching_errors() {
        for (src, want) in [
            (
                "let main = () => Scene.model(Asset.sound(\"boom.ogg\"))",
                "Scene.model: expected a model asset, got a sound asset (\"boom.ogg\") — \
construct it with Asset.model(…)",
            ),
            (
                "let main = () => Scene.plane() |> Scene.litTexture(Asset.model(\"shark.glb\"))",
                "Scene.litTexture: expected a texture asset, got a model asset (\"shark.glb\") — \
construct it with Asset.texture(…)",
            ),
            (
                "let main = () => Effect.play(Asset.texture(\"wood.png\"))",
                "Effect.play: expected a sound asset, got a texture asset (\"wood.png\") — \
construct it with Asset.sound(…)",
            ),
            (
                "let main = () => AudioSource.ambient(\"bed\", Asset.model(\"shark.glb\"))",
                "AudioSource.ambient: expected a sound asset, got a model asset (\"shark.glb\") — \
construct it with Asset.sound(…)",
            ),
        ] {
            assert_eq!(fail_message(src), want, "for {src}");
        }
    }

    /// An Asset is plain data (kind + path), so storing one in the model must
    /// not invalidate hot-reload time-travel history.
    #[test]
    fn assets_are_reload_safe_snapshots() {
        let value = eval("let main = () => Asset.model(\"shark.glb\")");
        match &value {
            Value::HostData(data) => assert!(data.is_reload_safe_snapshot()),
            other => panic!("expected a HostData asset, got {}", other.kind_name()),
        }
    }

    // The lit pipeline: materials, all three light kinds, shadow flag, and
    // firstPerson camera flow into a protocol Frame with lights.
    #[test]
    fn lit_frame_carries_lights_and_materials() {
        let frame = frame_of(
            "let main = () =>
             Frame.createLit(
               Camera.firstPerson(Vec3.make(0.0, 3.5, -8.0), Angle.radians(0.0), Angle.radians(-0.3), Angle.degrees(60.0)),
               Scene.group([
                 Scene.plane() |> Scene.scale(24.0) |> Scene.lit(Color.rgb(0.6, 0.6, 0.62)),
                 Scene.sphere() |> Scene.emissive(Color.rgb(1.0, 0.3, 0.25)),
               ]),
               [
                 Light.ambient(Color.rgb(0.1, 0.1, 0.13)),
                 Light.directional(Vec3.make(0.5, -1.0, 0.35), Color.rgb(1.0, 0.98, 0.95), 0.85) |> Light.castShadows,
                 Light.point(Vec3.make(1.0, 2.2, 0.0), Color.rgb(1.0, 0.3, 0.25), 1.4, 4.0),
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

    #[test]
    fn spot_light_and_normal_mapped_material() {
        let frame = frame_of(
            "let bumps = Texture.file(\"bumps-normal.png\")\n\
             let main = () =>\n\
             Frame.createLit(\n\
               Camera.firstPerson(Vec3.make(0.0, 3.0, -8.0), Angle.radians(0.0), Angle.radians(-0.25), Angle.degrees(60.0)),\n\
               Scene.cube() |> Scene.litNormalMapped(Color.rgb(0.9, 0.9, 0.92), bumps),\n\
               [\n\
                 Light.spot(Vec3.make(0.0, 7.0, 5.0), Vec3.make(0.0, -1.0, -0.5), Color.rgb(1.0, 1.0, 0.95), 5.0, 18.0, Angle.radians(0.5))\n\
                   |> Light.castShadows,\n\
               ])",
        );
        // The spot light is present, casts shadows, and carries its Angle-branded
        // cone angle (in radians) and range.
        assert_eq!(frame.lights.len(), 1);
        assert!(frame.lights[0].casts_shadows(), "spot casts shadows");
        match &frame.lights[0] {
            Light::Spot {
                cone_angle, range, ..
            } => {
                assert!((cone_angle - 0.5).abs() < 1e-5, "cone angle in radians");
                assert_eq!(*range, 18.0);
            }
            other => panic!("expected a Spot light, got {other:?}"),
        }
        // The material serializes as a Lit with a non-null normal map.
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(
            json.contains("normal_map") && json.contains("bumps-normal.png"),
            "expected a normal-mapped Lit material: {json}"
        );
    }

    // Host errors are spanned Functor Lang runtime errors, not panics.
    #[test]
    fn prelude_errors_are_spanned() {
        let module = functor_lang::lower(
            functor_lang::parse("let main = () => Scene.color(Color.rgb(1.0, \"x\", 0.0), Scene.cube())")
                .unwrap(),
        )
        .unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(failure.error.message, "expected a number, got a string");
    }

    // [units, tier 1] rotations/camera angles refuse bare numbers with a
    // teaching error — degree/radian confusion is unrepresentable.
    #[test]
    fn bare_numbers_are_not_angles() {
        let module =
            functor_lang::lower(functor_lang::parse("let main = () => Scene.cube() |> Scene.rotateY(1.57)").unwrap())
                .unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(
            failure.error.message,
            "Scene.rotateY: expected an Angle, got a bare number — say which unit: \
Angle.degrees(…) or Angle.radians(…)"
        );
    }

    // [strong-typing track] color parameters refuse bare numbers with a
    // teaching error — channel swaps and arg miscounts are unrepresentable
    // (the Angle rule, applied to color).
    #[test]
    fn bare_numbers_are_not_colors() {
        let fail = |src: &str| {
            let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
            functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
                .err()
                .expect("should fail")
                .error
                .message
        };
        // A bare number where the Color goes gets the teaching error…
        assert_eq!(
            fail("let main = () => Scene.cube() |> Scene.lit(0.5)"),
            "Scene.lit: expected a Color, got a bare number — wrap the channels: \
Color.rgb(r, g, b)"
        );
        assert_eq!(
            fail("let main = () => Light.ambient(0.1)"),
            "Light.ambient: expected a Color, got a bare number — wrap the channels: \
Color.rgb(r, g, b)"
        );
        // …and the pre-Color three-float spelling teaches the new shape.
        assert_eq!(
            fail("let main = () => Scene.cube() |> Scene.lit(0.5, 0.5, 0.5)"),
            "usage: Scene.lit(color, scene)"
        );
        // Color.rgb itself still validates its channels.
        assert_eq!(
            fail("let main = () => Color.rgb(1.0, \"x\", 0.0)"),
            "expected a number, got a string"
        );
    }

    // [strong-typing track] the physics tag brand has teeth at CHECK time:
    // physics.funi types every tag parameter as the abstract `Physics.tag`,
    // so a bare string is a build error. Runtime stays erased — a tag IS its
    // string — so /state, the journal, and event-tag equality are unchanged.
    #[test]
    fn bare_strings_are_not_physics_tags() {
        let dir =
            std::env::temp_dir().join(format!("functor-physics-tag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("game.fun"),
            "let bad = () => Physics.position(\"ball\")\n\
             let good = () => Physics.position(Physics.tag(\"ball\"))\n",
        )
        .unwrap();
        let project = functor_lang::project::load_with_prelude(
            &dir.join("game.fun"),
            &Default::default(),
            &functor_prelude::modules(),
        )
        .unwrap_or_else(|e| panic!("loads: {}", e.render()));
        let diags: Vec<String> = project.check().into_iter().map(|d| d.message).collect();
        assert_eq!(
            diags,
            vec![
                "argument 1 of `Physics.position`: expected Physics.tag, got string".to_string()
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // [strong-typing track] spatial parameters refuse bare numbers with a
    // teaching error — axis transposition and eye/target swaps are
    // unrepresentable (the Angle rule, applied to space).
    #[test]
    fn bare_numbers_are_not_vec3s() {
        let fail = |src: &str| {
            let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
            functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
                .err()
                .expect("should fail")
                .error
                .message
        };
        assert_eq!(
            fail("let main = () => Scene.cube() |> Scene.translate(1.0)"),
            "Scene.translate: expected a Vec3, got a bare number — wrap the components: \
Vec3.make(x, y, z)"
        );
        // The pre-Vec3 spelling teaches the new shape.
        assert_eq!(
            fail("let main = () => Scene.cube() |> Scene.translate(1.0, 2.0, 3.0)"),
            "usage: Scene.translate(v, scene)"
        );
        assert_eq!(
            fail(
                "let main = () => Camera.lookAt(Vec3.make(0.0, 2.0, -6.0), 0.0)"
            ),
            "Camera.lookAt: expected a Vec3, got a bare number — wrap the components: \
Vec3.make(x, y, z)"
        );
        // Vec3.make itself still validates its components.
        assert_eq!(
            fail("let main = () => Vec3.make(1.0, \"x\", 0.0)"),
            "expected a number, got a string"
        );
    }

    // A Vec3 stored in the model (a spawn point, a velocity) survives hot
    // reload, like Color.
    #[test]
    fn vec3s_are_reload_safe_snapshots() {
        let v = eval("let main = () => Vec3.make(0.1, 0.2, 0.3)");
        assert!(v.is_reload_safe_snapshot());
    }

    // A Color stored in the model is plain enough to survive hot reload —
    // colors were bare floats before the brand, and a palette field must not
    // invalidate the time-travel history (unlike taggers-carrying host
    // values, which stay conservatively unsafe).
    #[test]
    fn colors_are_reload_safe_snapshots() {
        let color = eval("let main = () => Color.rgb(0.1, 0.2, 0.3)");
        assert!(color.is_reload_safe_snapshot());
        let in_model = eval("let main = () => { tint: Color.rgb(0.1, 0.2, 0.3), hp: 3.0 }");
        assert!(in_model.is_reload_safe_snapshot());
        // The conservative default still holds for tagger-carrying values.
        let sub = eval("let main = () => Sub.every(Time.seconds(1.0), 3.0)");
        assert!(!sub.is_reload_safe_snapshot());
    }

    // Scene.animate pipes after Scene.model and lands the expression on the
    // Model node — the declarative "what pose" seam the renderer evaluates.
    #[test]
    fn scene_animate_sets_clip_on_model() {
        let value = eval(
            "let main = () => Scene.model(\"Xbot.glb\") |> Scene.animate(Anim.clip(\"walk\", 1.5))",
        );
        let scene = scene_of(&value).expect("a Scene");
        let SceneObject::Model(description) = &scene.obj else {
            panic!("expected a Model node, got {:?}", scene.obj);
        };
        match &description.animation {
            Some(AnimExpr::Clip { name, playhead }) => {
                assert_eq!(name, "walk");
                assert_eq!(*playhead, 1.5);
            }
            other => panic!("expected a Clip animation, got {other:?}"),
        }
    }

    // Applied over a group, Scene.animate reaches the Model nodes inside it
    // (geometry is untouched), and Anim.blend carries its (clip, weight) pairs.
    #[test]
    fn scene_animate_reaches_models_in_groups() {
        let value = eval(
            "let main = () =>\n\
             Scene.group([Scene.model(\"Xbot.glb\"), Scene.cube()])\n\
               |> Scene.animate(Anim.blend([\n\
                    (Anim.clip(\"idle\", 0.0), 0.25),\n\
                    (Anim.clip(\"run\", 2.0), 0.75),\n\
                  ]))",
        );
        let scene = scene_of(&value).expect("a Scene");
        let SceneObject::Group(children) = &scene.obj else {
            panic!("expected a Group, got {:?}", scene.obj);
        };
        let SceneObject::Model(description) = &children[0].obj else {
            panic!("expected a Model child, got {:?}", children[0].obj);
        };
        match &description.animation {
            Some(AnimExpr::Blend(entries)) => {
                assert_eq!(entries.len(), 2);
                assert!(
                    matches!(&entries[1], (AnimExpr::Clip { name, playhead }, weight)
                        if name == "run" && *playhead == 2.0 && *weight == 0.75),
                    "entries: {entries:?}"
                );
            }
            other => panic!("expected a Blend animation, got {other:?}"),
        }
        assert!(
            matches!(&children[1].obj, SceneObject::Geometry(Shape::Cube)),
            "the cube is untouched"
        );
        // The animated scene serializes through the protocol.
        let json = serde_json::to_string(&scene).expect("serialize");
        assert!(json.contains("\"Blend\""), "json: {json}");
        let back: Scene3D = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // The Angle rule for animations: a bare clip name teaches Anim.clip.
    #[test]
    fn bare_strings_are_not_anims() {
        let module = functor_lang::lower(
            functor_lang::parse(
                "let main = () => Scene.model(\"Xbot.glb\") |> Scene.animate(\"walk\")",
            )
            .unwrap(),
        )
        .unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(
            failure.error.message,
            "Scene.animate: got a bare clip name — wrap it with a playhead: \
Anim.clip(\"walk\", tts)"
        );
    }

    // Blend entries must be (anim, weight) pairs.
    #[test]
    fn anim_blend_requires_pairs() {
        let module = functor_lang::lower(
            functor_lang::parse("let main = () => Anim.blend([Anim.clip(\"walk\", 0.0)])").unwrap(),
        )
        .unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert!(
            failure.error.message.contains("(anim, weight)"),
            "message: {}",
            failure.error.message
        );
    }

    // The full pose algebra composes through pipes and lands on the Model
    // node as nested AnimExpr data: rest |> rotate, masked blends, additive
    // layers.
    #[test]
    fn anim_algebra_composes_and_serializes() {
        let value = eval(
            "let main = () =>\n\
             Scene.model(\"glove.glb\") |> Scene.animate(\n\
               Anim.rest()\n\
                 |> Anim.rotate(\"finger_index_0_r\", Angle.degrees(45.0), Angle.degrees(0.0), Angle.degrees(0.0))\n\
                 |> Anim.mask([\"wrist_r\"])\n\
                 |> Anim.add(Anim.clip(\"wave\", 1.0), 0.5))",
        );
        let scene = scene_of(&value).expect("a Scene");
        let SceneObject::Model(description) = &scene.obj else {
            panic!("expected a Model node, got {:?}", scene.obj);
        };
        let Some(AnimExpr::Add {
            base,
            layer,
            weight,
        }) = &description.animation
        else {
            panic!("expected an Add at the root, got {:?}", description.animation);
        };
        assert_eq!(*weight, 0.5);
        assert!(
            matches!(&**layer, AnimExpr::Clip { name, .. } if name == "wave"),
            "layer: {layer:?}"
        );
        let AnimExpr::Mask { joints, expr } = &**base else {
            panic!("expected a Mask base, got {base:?}");
        };
        assert_eq!(joints, &vec!["wrist_r".to_string()]);
        let AnimExpr::Rotate { joint, euler, expr } = &**expr else {
            panic!("expected a Rotate inside the mask, got {expr:?}");
        };
        assert_eq!(joint, "finger_index_0_r");
        assert!((euler[0] - 45.0_f32.to_radians()).abs() < 1e-6);
        assert!(matches!(&**expr, AnimExpr::Rest));
        // And the whole expression round-trips through the protocol.
        let json = serde_json::to_string(&scene).expect("serialize");
        let back: Scene3D = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // The Angle rule applies to Anim.rotate: bare numbers are refused.
    #[test]
    fn anim_rotate_refuses_bare_numbers() {
        let module = functor_lang::lower(
            functor_lang::parse(
                "let main = () => Anim.rest() |> Anim.rotate(\"head\", 0.5, 0.0, 0.0)",
            )
            .unwrap(),
        )
        .unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert!(
            failure.error.message.contains("Angle"),
            "message: {}",
            failure.error.message
        );
    }

    // [registry migration pins] the deliberate teaching-text deltas, made
    // loud: usage sigs are registry-derived and must START with the path, so
    // the legacy pipe-form spellings (anim |> Anim.mask(…)) are positional
    // now; taggers share one FromArg message ("result record"); wrong list
    // elements get the uniform typed error. Domain messages (zero-direction)
    // stay byte-identical.
    #[test]
    fn migrated_usage_and_tagger_texts_are_pinned() {
        assert_eq!(
            fail_message("let main = () => Anim.rest() |> Anim.mask([])"),
            "usage: Anim.mask([\"jointName\", …], anim) — a non-empty list of joint \
names; each covers its whole subtree (functor inspect lists a model's joints)"
        );
        assert_eq!(
            fail_message("let main = () => Physics.events(3.0)"),
            "Physics.events: the tagger must be a function of the result record, got a number"
        );
        assert_eq!(
            fail_message(
                "let main = () => Physics.raycast(Vec3.make(0.0, 0.0, 0.0), \
Vec3.make(0.0, 0.0, 0.0), 10.0, (h) => h)"
            ),
            "Physics.raycast: the direction must not be zero"
        );
        assert_eq!(
            fail_message("let main = () => Physics.scene(Vec3.make(0.0, 0.0, 0.0), [3.0])"),
            "Physics.scene: expected a Body, got a number"
        );
        // [registry delta] the sound-taking externals answered a wrong-type
        // (or empty) sound with their own full usage line ("usage:
        // Effect.play(sound)" etc.); all five now share the SoundPath
        // teaching text.
        assert_eq!(
            fail_message("let main = () => Effect.play(3.0)"),
            "Effect.play: expected a sound — a non-empty path string or \
Asset.sound(\"file.ogg\")"
        );
        assert_eq!(
            fail_message("let main = () => AudioSource.ambient(\"bed\", 3.0)"),
            "AudioSource.ambient: expected a sound — a non-empty path string or \
Asset.sound(\"file.ogg\")"
        );
    }

    // Degrees and radians agree where they should: 90° == τ/4 rad.
    #[test]
    fn degrees_and_radians_agree() {
        let deg = frame_of(
            "let main = () => Frame.create(Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), \
Scene.cube() |> Scene.rotateY(Angle.degrees(90.0)))",
        );
        let rad = frame_of(
            "let main = () => Frame.create(Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), \
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
            functor_lang::lower(functor_lang::parse("let main = () => Scene.frobnicate()").unwrap()).unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
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
        for path in registry().paths() {
            let result = host.call(path, vec![Value::Bool(true)], functor_lang::Span::new(0, 0));
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
    fn functor_lang_snippet_declares_a_render_target_frame() {
        let frame = frame_of(
            "let feed = RenderTarget.named(\"security\") |> RenderTarget.sized(256.0, 128.0)\n\
             let main = () =>\n\
             Frame.createLit(\n\
               Camera.lookAt(Vec3.make(0.0, 2.0, -8.0), Vec3.make(0.0, 1.0, 0.0)),\n\
               Scene.group([\n\
                 Scene.plane() |> Scene.lit(Color.rgb(0.6, 0.6, 0.6)),\n\
                 Scene.quad() |> Scene.screen(feed),\n\
               ]),\n\
               [Light.ambient(Color.rgb(0.1, 0.1, 0.1))])\n\
             |> Frame.withRenderTarget(feed, Frame.createLit(\n\
                  Camera.lookAt(Vec3.make(0.0, 4.0, -6.0), Vec3.make(0.0, 0.5, 0.0)),\n\
                  Scene.cube() |> Scene.lit(Color.rgb(0.8, 0.2, 0.2)),\n\
                  [Light.ambient(Color.rgb(0.2, 0.2, 0.2))]))",
        );
        assert_eq!(frame.render_targets.len(), 1);
        let pass = &frame.render_targets[0];
        assert_eq!(pass.target.id, "security");
        assert_eq!((pass.target.width, pass.target.height), (256, 128));
        assert_eq!(pass.frame.lights.len(), 1);
        assert!(pass.frame.render_targets.is_empty());
        // The reader's wire shape: the screen material samples the target by id.
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(
            json.contains(r#""RenderTarget":"security""#),
            "json: {json}"
        );
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
               Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
               Scene.quad() |> Scene.screen(RenderTarget.named(\"feed\")))",
        );
        let json = serde_json::to_string(&frame.scene).expect("serialize");
        assert!(
            json.contains(
                r#""Emissive":{"color":[1.0,1.0,1.0,1.0],"texture":{"RenderTarget":"feed"}}"#
            ),
            "json: {json}"
        );
    }

    // [units, tier 1 — the Angle rule applied to identity] both sites accept
    // ONLY the branded RenderTarget value; a bare string / wrong value is a
    // spanned usage error, so writer/reader id typos are unrepresentable.
    #[test]
    fn bare_strings_are_not_render_targets() {
        let fail = |src: &str| {
            let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
            functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
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
                "let main = () => Frame.create(Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), \
Scene.cube()) |> Frame.withRenderTarget(\"feed\", Frame.create(\
Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube()))"
            ),
            "Frame.withRenderTarget: expected a RenderTarget, got a bare string — declare \
it once with RenderTarget.named(\"…\") and pass that value at both sites"
        );
        assert_eq!(
            fail("let main = () => RenderTarget.named(\"x\") |> RenderTarget.sized(-1.0, 4.0)"),
            "RenderTarget.sized width must be positive, got -1"
        );
        // A wrong-TYPE argument gets the precise conversion error (the
        // registry rule — matching how number-taking arms always behaved);
        // arity mistakes and empty names still teach the full usage line.
        assert_eq!(
            fail("let main = () => RenderTarget.named(3.0)"),
            "expected a string, got a number"
        );
        assert_eq!(
            fail("let main = () => RenderTarget.named(\"a\", \"b\")"),
            "usage: RenderTarget.named(\"id\") — a non-empty name; 512x512 unless \
piped through RenderTarget.sized"
        );
        assert_eq!(
            fail("let main = () => RenderTarget.named(\"\")"),
            "usage: RenderTarget.named(\"id\") — a non-empty name; 512x512 unless \
piped through RenderTarget.sized"
        );
    }

    // The fog vocabulary: a branded Fog on the frame, round-tripping the
    // protocol wire shape.
    #[test]
    fn functor_lang_snippet_declares_fog() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(Camera.lookAt(Vec3.make(0.0, 2.0, -8.0), Vec3.make(0.0, 1.0, 0.0)), Scene.cube())\n\
             |> Frame.withFog(Fog.linear(4.0, 30.0, Color.rgb(0.5, 0.6, 0.7)))",
        );
        assert_eq!(frame.fog, Some(Fog::linear(4.0, 30.0, 0.5, 0.6, 0.7)));
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(json.contains(r#""Linear""#), "json: {json}");
        let back: Frame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // Frame.withClearColor sets the frame's explicit clear color, and it wins
    // over the fog color as the resolved background (fog blending unchanged).
    #[test]
    fn functor_lang_snippet_declares_clear_color() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(Camera.lookAt(Vec3.make(0.0, 2.0, -8.0), Vec3.make(0.0, 1.0, 0.0)), Scene.cube())\n\
             |> Frame.withClearColor(Color.rgb(0.2, 0.4, 0.6))",
        );
        assert_eq!(frame.clear_color, Some([0.2, 0.4, 0.6]));
        assert_eq!(frame.resolved_clear_color(), [0.2, 0.4, 0.6]);

        // Without the override, the resolved clear color is the engine default;
        // with fog it's the fog color; withClearColor overrides even that.
        let plain = frame_of(
            "let main = () => \
             Frame.create(Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())",
        );
        assert_eq!(plain.clear_color, None);
        assert_eq!(plain.resolved_clear_color(), [0.1, 0.2, 0.3]);

        let both = frame_of(
            "let main = () => \
             Frame.create(Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube()) \
             |> Frame.withFog(Fog.linear(4.0, 30.0, Color.rgb(0.5, 0.6, 0.7))) \
             |> Frame.withClearColor(Color.rgb(0.0, 0.0, 0.0))",
        );
        assert_eq!(both.resolved_clear_color(), [0.0, 0.0, 0.0]);
    }

    // [units, tier 1 — the Angle rule] Frame.withFog accepts ONLY the branded
    // value; a bare number gets the teaching error.
    #[test]
    fn bare_numbers_are_not_fog() {
        let fail = |src: &str| {
            let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
            functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
                .err()
                .expect("should fail")
                .error
                .message
        };
        assert_eq!(
            fail(
                "let main = () => Frame.create(Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), \
Scene.cube()) |> Frame.withFog(0.5)"
            ),
            "Frame.withFog: expected a Fog, got a bare number — build one with \
Fog.linear(near, far, color) or Fog.exp(density, color)"
        );
        // Degenerate parameters are teaching errors at construction, not
        // silent bad renders.
        assert_eq!(
            fail("let main = () => Fog.linear(10.0, 5.0, Color.rgb(0.5, 0.5, 0.5))"),
            "Fog.linear: far (5) must be greater than near (10)"
        );
        assert_eq!(
            fail("let main = () => Fog.exp(-1.0, Color.rgb(0.5, 0.5, 0.5))"),
            "Fog.exp density must be positive, got -1"
        );
    }

    // The skybox vocabulary: a branded Skybox on the frame, six faces in GL
    // upload order, round-tripping the protocol wire shape.
    #[test]
    fn functor_lang_snippet_declares_a_skybox() {
        let frame = frame_of(
            "let sky = Skybox.files(\"px.jpg\", \"nx.jpg\", \"py.jpg\", \"ny.jpg\", \
\"pz.jpg\", \"nz.jpg\")\n\
             let main = () =>\n\
             Frame.create(Camera.lookAt(Vec3.make(0.0, 2.0, -8.0), Vec3.make(0.0, 1.0, 0.0)), Scene.cube())\n\
             |> Frame.withSkybox(sky)",
        );
        let sky = frame.skybox.as_ref().expect("skybox set");
        assert_eq!(
            sky.faces(),
            ["px.jpg", "nx.jpg", "py.jpg", "ny.jpg", "pz.jpg", "nz.jpg"]
        );
        let json = serde_json::to_string(&frame).expect("serialize");
        let back: Frame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // [units, tier 1 — the Angle rule] Frame.withSkybox accepts ONLY the
    // branded value; a bare path string gets the teaching error, and empty
    // face paths are rejected at construction.
    #[test]
    fn bare_strings_are_not_skyboxes() {
        let fail = |src: &str| {
            let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
            functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
                .err()
                .expect("should fail")
                .error
                .message
        };
        assert_eq!(
            fail(
                "let main = () => Frame.create(Camera.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), \
Scene.cube()) |> Frame.withSkybox(\"sky.jpg\")"
            ),
            "Frame.withSkybox: expected a Skybox, got a bare string — build one with \
Skybox.files(px, nx, py, ny, pz, nz)"
        );
        assert_eq!(
            fail(
                "let main = () => Skybox.files(\"px.jpg\", \"\", \"py.jpg\", \"ny.jpg\", \
\"pz.jpg\", \"nz.jpg\")"
            ),
            "usage: Skybox.files(px, nx, py, ny, pz, nz) — six non-empty face \
paths (+X, -X, +Y, -Y, +Z, -Z)"
        );
    }

    // The physics vocabulary: a Functor Lang snippet declares a PhysicsScene the
    // shells can hand to `World::reconcile` — bodies, attributes, gravity.
    #[test]
    fn functor_lang_snippet_declares_a_physics_scene() {
        let value = eval(
            "let crate1 = Physics.dynamic(\"crate-1\", Physics.box(1.0, 1.0, 1.0))\n\
             |> Physics.at(Vec3.make(0.0, 5.0, 0.0))\n\
             |> Physics.velocity(Vec3.make(1.0, 0.0, 0.0))\n\
             |> Physics.mass(2.0)\n\
             |> Physics.restitution(0.5)\n\
             let main = () => Physics.scene(Vec3.make(0.0, -9.81, 0.0), [\n\
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
    // way the FunctorLangGame driver does, then read it back from Functor Lang — the in-process
    // live read that is the whole point of the Functor Lang surface.
    #[test]
    fn physics_reads_see_the_stepped_world() {
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        let declare = eval(
            "let main = () => Physics.scene(Vec3.make(0.0, -9.81, 0.0), [\n\
               Physics.dynamic(\"ball\", Physics.sphere(0.5)) |> Physics.at(Vec3.make(0.0, 5.0, 0.0))])",
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

    // The dry-run world scope (docs/time-travel.md T6b): under an
    // ActiveWorldScope, game-code physics readback answers against the SCOPED world
    // (the ghost's projected world), and a suppressed-outbound drain routes
    // commands to it — a replayed kick belongs to the throwaway world. Outside
    // the scope everything resolves to the default world, and a suppressed
    // drain with NO scope still queues nothing
    // (`suppress_outbound_logs_but_queues_nothing`).
    #[test]
    fn physics_reads_and_commands_follow_the_active_world_scope() {
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        let ball_at = |y: f32| {
            crate::physics::PhysicsScene::create(
                [0.0, 0.0, 0.0],
                vec![crate::physics::Body::dynamic(
                    "ball".to_string(),
                    crate::physics::Shape::Sphere { radius: 0.5 },
                )
                .at([0.0, y, 0.0])],
            )
        };
        crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| w.reconcile(&ball_at(5.0)));
        let scoped = crate::physics::create_world([0.0, 0.0, 0.0]);
        crate::physics::with_world(scoped, |w| w.reconcile(&ball_at(9.0)));

        let ball_y = || {
            let drawn = eval("let main = () => Scene.sphere() |> Physics.transformed(\"ball\")");
            scene_of(&drawn).expect("a Scene").xform.w.y
        };
        assert!((ball_y() - 5.0).abs() < 1e-6, "unscoped read = default world");
        {
            let _scope = crate::physics::ActiveWorldScope::enter(scoped);
            assert!((ball_y() - 9.0).abs() < 1e-6, "scoped read = scoped world");

            // A suppressed drain (the dry-run forward-step) under the scope
            // queues the command on the SCOPED world…
            let effect = eval("let main = () => Physics.applyImpulse(\"ball\", Vec3.make(2.0, 0.0, 0.0))");
            let Value::HostData(data) = &effect else {
                panic!("expected an Effect");
            };
            let tree = &data
                .as_any()
                .downcast_ref::<FunctorLangEffect>()
                .expect("Effect")
                .0;
            let module =
                functor_lang::lower(functor_lang::parse("let update = (m, msg) => m").unwrap())
                    .unwrap();
            let session = functor_lang::Session::load(&module, &mut FunctorHost)
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
                true, // suppress_outbound
            );
            assert!(deferred.is_empty(), "nothing should defer here");
        }
        crate::physics::with_world(scoped, |w| {
            w.step_frame(1.0 / 60.0);
            let v = w.body_velocity("ball").unwrap();
            assert!(v[0] > 0.0, "impulse should land on the scoped world: {v:?}");
        });
        // …and the DEFAULT world got nothing.
        crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| {
            w.step_frame(1.0 / 60.0);
            let v = w.body_velocity("ball").unwrap();
            assert_eq!(v[0], 0.0, "default world must be untouched: {v:?}");
        });
        crate::physics::remove_world(scoped);
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
    }

    // Degenerate physical dimensions are boundary errors — Rapier would
    // silently build a broken collider, and Functor Lang can't branch to notice.
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
                functor_lang::lower(functor_lang::parse(&format!("let main = () => {src}")).unwrap()).unwrap();
            let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
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
        let effect = eval("let main = () => Physics.applyImpulse(\"ball\", Vec3.make(2.0, 0.0, 0.0))");
        let Value::HostData(data) = &effect else {
            panic!("expected an Effect");
        };
        let tree = &data.as_any().downcast_ref::<FunctorLangEffect>().expect("Effect").0;

        // Drain it the way the producer does (no session/update involvement —
        // physics effects are tagger-less).
        let module = functor_lang::lower(functor_lang::parse("let update = (m, msg) => m").unwrap()).unwrap();
        let session = functor_lang::Session::load(&module, &mut FunctorHost)
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
            false,
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
            functor_lang::lower(functor_lang::parse("let main = () => Physics.position(\"ghost\")").unwrap())
                .unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
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
        let module = functor_lang::lower(functor_lang::parse("let main = Scene.cube").unwrap()).unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        assert_eq!(
            failure.error.message,
            "`main` must take no parameters to be runnable"
        );
    }

    // Functor Lang permits non-finite numbers (IEEE division); the protocol boundary
    // does not — they become spanned errors, not NaN matrices.
    #[test]
    fn non_finite_numbers_are_rejected_at_the_boundary() {
        let module = functor_lang::lower(
            functor_lang::parse("let main = () => Scene.translate(Vec3.make(1.0 / 0.0, 0.0, 0.0), Scene.cube())")
                .unwrap(),
        )
        .unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
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
        let module = functor_lang::lower(functor_lang::parse(src).expect("parse")).expect("lower");
        let session = match functor_lang::Session::load(&module, &mut FunctorHost) {
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
                false,
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
                   let main = () => Physics.raycast(Vec3.make(0.0, 5.0, 0.0), Vec3.make(0.0, -1.0, 0.0), 100.0, (hit) => hit)";
        let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
        let session = functor_lang::Session::load(&module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("load failed: {}", f.error.message));
        let record = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("run failed: {}", f.error.message));
        let effect = match record.outcome {
            functor_lang::RunOutcome::Main(value) => value,
            _ => panic!("expected main"),
        };
        let Value::HostData(data) = &effect else {
            panic!("expected an Effect")
        };
        let tree = data.as_any().downcast_ref::<FunctorLangEffect>().unwrap().0.clone();
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
        let deferred =
            drain_effects(&session, &mut model, tree, &mut runner, &mut log, &mut fail, false);
        assert_eq!(deferred.len(), 1);
        assert!(log.is_empty());
        assert!(matches!(model, Value::Number(_)), "model must be untouched");

        // Post-step drain: performed against the live world.
        perform_deferred_queries(
            &session,
            &mut model,
            deferred,
            &mut runner,
            &mut log,
            &mut fail,
            false,
        );
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
            false,
        );
        perform_deferred_queries(
            &session,
            &mut replay_model,
            deferred,
            &mut replay,
            &mut replay_log,
            &mut fail,
            false,
        );
        assert_eq!(replay_log, log, "replay must reproduce the log");
        assert_eq!(replay_model.to_string(), model.to_string());

        // Fake: canned hits, no world.
        let mut fake = FakeEffects::new(0.0, vec![]).with_ray_hits(vec![ray_result_value(None)]);
        let miss = fake.raycast([0.0; 3], [0.0, -1.0, 0.0], 10.0);
        let EffectValue::Record(f) = &miss else {
            panic!()
        };
        assert!(f
            .iter()
            .any(|(k, v)| k == "hit" && *v == EffectValue::Bool(false)));
    }

    /// The Phase 5 event path end to end: a `Physics.events` sub's tagger
    /// receives contact records, folding through `update` post-step.
    #[test]
    fn physics_events_flow_to_update() {
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        let src = "let subscriptions = (m) => Physics.events((e) => e)\n\
                   let update = (m, msg) =>\n\
                     { contacts: m.contacts + 1.0, a: msg.a, b: msg.b, began: msg.started }";
        let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
        let session = functor_lang::Session::load(&module, &mut FunctorHost)
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

        let mut model = Value::Record(Rc::new(vec![("contacts".to_string(), Value::Number(0.0))]));
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
            false,
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

    /// Structured effect values convert to the Functor Lang values taggers receive,
    /// and round-trip through serde (the future disk-replay seam).
    #[test]
    fn effect_values_convert_and_serialize() {
        let value = EffectValue::Record(vec![
            ("hit".to_string(), EffectValue::Bool(true)),
            ("distance".to_string(), EffectValue::Number(4.25)),
            ("tag".to_string(), EffectValue::Text("crate-1".to_string())),
        ]);
        let functor_lang = value.to_functor_lang();
        let Value::Record(fields) = &functor_lang else {
            panic!("expected a record, got {}", functor_lang.kind_name());
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
        let module = functor_lang::lower(functor_lang::parse("let main = () => Effect.now(3.0)").unwrap()).unwrap();
        let failure = functor_lang::run_with_host(&module, functor_lang::Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        // [registry delta] was "Effect.now(tagger): the tagger must be a
        // function of the result, got a number" — flattened to the shared
        // Tagger teaching text.
        assert_eq!(
            failure.error.message,
            "Effect.now: the tagger must be a function of the result record, got a number"
        );
        let module =
            functor_lang::lower(functor_lang::parse("let main = () => Effect.batch([1.0])").unwrap()).unwrap();
        let failure = functor_lang::run_with_host(&module, functor_lang::Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        // [registry delta] was "Effect.batch items must be Effects, got a
        // number" — the uniform typed list-element error.
        assert_eq!(
            failure.error.message,
            "Effect.batch: expected an Effect, got a number"
        );
    }

    /// The drain cap stops a self-sustaining effect chain instead of hanging
    /// the frame.
    #[test]
    fn effect_drain_cap_stops_runaway_chains() {
        let src = "type Msg = | Again(n: Float)\n\
             let again = (n) => Again(n)\n\
             let update = (m, msg) => (m + 1.0, Effect.random(Again))";
        let module = functor_lang::lower(functor_lang::parse(src).expect("parse")).expect("lower");
        let session = match functor_lang::Session::load(&module, &mut FunctorHost) {
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
            false,
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
            functor_lang::lower(functor_lang::parse("let noop = (m, msg) => m").expect("parse")).expect("lower");
        let session = match functor_lang::Session::load(&module, &mut FunctorHost) {
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
            false,
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
        let module = functor_lang::lower(
            functor_lang::parse("type Msg = | Pulse\nlet main = () => Sub.every(0.5, Pulse)").unwrap(),
        )
        .unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
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
        let module = functor_lang::lower(functor_lang::parse("let main = () => Sub.batch([1.0])").unwrap()).unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("should fail");
        // [registry delta] was "Sub.batch items must be Subs, got a number" —
        // the uniform typed list-element error.
        assert_eq!(
            failure.error.message,
            "Sub.batch: expected a Sub, got a number"
        );
    }

    fn run_fail(src: &str) -> String {
        let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
        functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
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
               Camera.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),\n\
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
               Camera.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),\n\
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
               Camera.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),\n\
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
            json.contains(
                r#""Emissive":{"color":[1.0,1.0,1.0,1.0],"texture":{"File":"grid.png"}}"#
            ),
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
               Ui.textColor(Color.rgb(1.0, 0.85, 0.4), \"eye  0.0 0.0 -5.0\"),\n\
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

    // Ui.center() pins a panel to the screen center — the menu anchor. It
    // lowers to a Panel with the Center anchor and round-trips over the wire.
    #[test]
    fn ui_center_anchors_a_panel_to_the_middle() {
        let value = eval(
            "let main = () =>\n\
             Ui.column([Ui.text(\"Play\"), Ui.text(\"Quit\")]) |> Ui.panel(Ui.center())",
        );
        let view = view_value(&value).expect("main should return a View");
        assert!(matches!(
            view,
            View::Panel {
                anchor: ui::Anchor::Center,
                ..
            }
        ));
        let json = serde_json::to_string(view).expect("serialize");
        assert!(json.contains(r#""anchor":"Center""#), "json: {json}");
        let back: View = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // Ui.button (docs/ui-interaction.md U3): each button registers its msg in
    // the frame's handler table and the node carries the slot, in
    // construction order — the serializable tree never holds the msg itself.
    #[test]
    fn ui_button_registers_its_msg_and_stamps_the_slot() {
        let _ = take_ui_handlers(); // isolate from other tests on this thread
        let value = eval(
            "type Msg = | Inc | Reset\n\
             let main = () =>\n\
             Ui.column([\n\
               Ui.button(\"+1\", Inc),\n\
               Ui.button(\"Reset\", Reset),\n\
             ])",
        );
        let view = view_value(&value).expect("main should return a View");
        assert_eq!(
            serde_json::to_string(view).unwrap(),
            r#"{"Column":[{"Button":{"slot":0,"label":"+1"}},{"Button":{"slot":1,"label":"Reset"}}]}"#
        );
        let handlers = take_ui_handlers();
        assert_eq!(handlers.len(), 2);
        // Construction order: slot 0 carries Inc, slot 1 carries Reset.
        match (&handlers[0], &handlers[1]) {
            (UiHandler::Msg(inc), UiHandler::Msg(reset)) => {
                assert_eq!(inc.to_string(), "Inc");
                assert_eq!(reset.to_string(), "Reset");
            }
            _ => panic!("both handlers should be verbatim msgs"),
        }
    }

    // Ui.slider (docs/ui-interaction.md U4): registers its tagger (the
    // Effect.now shape), carries min/max/value in the node, and rejects a
    // non-function tagger and an empty range with teaching errors.
    #[test]
    fn ui_slider_registers_its_tagger_and_validates() {
        let _ = take_ui_handlers();
        let value = eval(
            "type Msg = | SetSpeed(v: Float)\n\
             let main = () => Ui.row([\n\
               Ui.slider(0.0, 10.0, 2.5, SetSpeed),\n\
             ])",
        );
        let view = view_value(&value).expect("main should return a View");
        assert_eq!(
            serde_json::to_string(view).unwrap(),
            r#"{"Row":[{"Slider":{"slot":0,"min":0.0,"max":10.0,"value":2.5}}]}"#
        );
        let handlers = take_ui_handlers();
        assert_eq!(handlers.len(), 1);
        assert!(matches!(&handlers[0], UiHandler::Tagger(_)));

        // [registry delta] was "Ui.slider(min, max, value, tagger): the
        // tagger must be a function of the new value, got a number" —
        // flattened to the shared Tagger teaching text.
        assert_eq!(
            run_fail("let main = () => Ui.slider(0.0, 10.0, 2.5, 42.0)"),
            "Ui.slider: the tagger must be a function of the result record, got a number"
        );
        let _ = take_ui_handlers();

        assert_eq!(
            run_fail("let main = () => Ui.slider(5.0, 5.0, 5.0, (v) => v)"),
            "Ui.slider: max (5) must be greater than min (5)"
        );
        let _ = take_ui_handlers();
    }

    // Ui.textInput (docs/ui-interaction.md U4): the text sibling of the
    // slider — registers its tagger, carries the model's value, rejects a
    // non-function tagger.
    #[test]
    fn ui_text_input_registers_its_tagger_and_validates() {
        let _ = take_ui_handlers();
        let value = eval(
            "type Msg = | SetName(s: String)\n\
             let main = () => Ui.textInput(\"functor\", SetName)",
        );
        let view = view_value(&value).expect("main should return a View");
        assert_eq!(
            serde_json::to_string(view).unwrap(),
            r#"{"TextInput":{"slot":0,"value":"functor"}}"#
        );
        let handlers = take_ui_handlers();
        assert_eq!(handlers.len(), 1);
        assert!(matches!(&handlers[0], UiHandler::Tagger(_)));

        // [registry delta] was "Ui.textInput(value, tagger): the tagger must
        // be a function of the new text, got a number" — flattened to the
        // shared Tagger teaching text.
        assert_eq!(
            run_fail("let main = () => Ui.textInput(\"x\", 42.0)"),
            "Ui.textInput: the tagger must be a function of the result record, got a number"
        );
        let _ = take_ui_handlers();
    }

    // --- Networking (Sub.connect/listen, Effect.send, NetEvent) ---

    #[test]
    fn net_conn_subs_extracts_declarations() {
        let subs = eval(
            "type Msg = | Ws(ev: Net.NetEvent)\n\
             let main = () => Sub.batch([\n\
               Sub.connect(\"ws://a/echo\", Ws),\n\
               Sub.listen(\"127.0.0.1:9001\", Ws),\n\
             ])",
        );
        let conns = net_conn_subs(&subs).expect("a Sub tree");
        assert_eq!(conns.len(), 2);
        assert_eq!(conns[0].key, "ws://a/echo");
        assert!(!conns[0].listen);
        assert_eq!(conns[1].key, "127.0.0.1:9001");
        assert!(conns[1].listen);
    }

    #[test]
    fn effect_send_queues_a_conn_command() {
        crate::net::drain_conn_commands(); // clear
        let module =
            functor_lang::lower(functor_lang::parse("let main = () => Effect.send(7.0, \"hi\")").unwrap()).unwrap();
        let session = match functor_lang::Session::load(&module, &mut FunctorHost) {
            Ok(s) => s,
            Err(f) => panic!("load: {}", f.error.message),
        };
        let fx = session.global("main").unwrap();
        let fx = session.apply(fx, vec![], "main", &mut FunctorHost).unwrap();
        let (_, effect) =
            split_model_effect(Value::Tuple(std::rc::Rc::new(vec![Value::Number(0.0), fx])));
        let mut model = Value::Number(0.0);
        let mut log = EffectLog::new();
        drain_effects(
            &session,
            &mut model,
            effect.expect("a Send effect"),
            &mut FakeEffects::new(0.0, vec![]),
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            false,
        );
        let cmds = crate::net::drain_conn_commands();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            crate::net::ConnCommand::Send { conn: 7, payload } if payload == b"hi"
        ));
    }

    /// The NetEvent value the host hands a tagger matches the built-in `Net`
    /// module's canonical ctors — a game's `match ev with | Net.Message(id, t)`
    /// binds them.
    #[test]
    fn net_event_value_matches_the_net_module() {
        let ev = net_event_value(NetEventKind::Message, 3, "yo").to_functor_lang();
        match ev {
            Value::Variant { ctor, args } => {
                assert_eq!(ctor.as_ref(), "Net.Message");
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].to_string(), "3");
                assert_eq!(args[1].to_string(), "\"yo\"");
            }
            other => panic!("expected a variant, got {other}"),
        }
    }

    /// Effect.send rejects a garbage connection id rather than truncating it
    /// to some OTHER live client. [Codex M — net review]
    #[test]
    fn effect_send_rejects_bad_conn_ids() {
        for (id, src) in [
            ("-1.0", "let main = () => Effect.send(-1.0, \"x\")"),
            ("1.5", "let main = () => Effect.send(1.5, \"x\")"),
        ] {
            let msg = run_fail(src);
            assert!(
                msg.contains("connId must be a non-negative whole number"),
                "id {id}: {msg}"
            );
        }
    }

    #[test]
    fn http_get_rejects_a_non_function_tagger() {
        // [registry delta] was "…the tagger must be a function of the
        // Net.HttpResponse…" — flattened to the shared Tagger teaching text.
        assert_eq!(
            run_fail("let main = () => Effect.httpGet(\"http://x\", 3.0)"),
            "Effect.httpGet: the tagger must be a function of the result record, got a number"
        );
    }

    /// The `http_response_value` builder maps a transport error to `Net.Failure`
    /// and any completed request (incl. a 404) to `Net.Response`.
    #[test]
    fn http_response_value_maps_ok_and_error() {
        let ok = http_response_value(&crate::net::HttpResult {
            token: 1,
            status: 404,
            body: b"nope".to_vec(),
            error: None,
        });
        assert_eq!(ok.to_string(), "Net.Response(404, \"nope\")");
        let failed = http_response_value(&crate::net::HttpResult {
            token: 1,
            status: 0,
            body: vec![],
            error: Some("dns".to_string()),
        });
        assert_eq!(failed.to_string(), "Net.Failure(\"dns\")");
    }

    /// Headless HTTP round trip (roadmap E2, the netdemo primitive), with no
    /// network — the interpreter analogue of the F# `net_http.rs` test.
    /// Firing `Effect.httpGet` queues a `NetCommand::HttpRequest` and registers
    /// the tagger by token; when the response lands (frames later), routing it
    /// through the tagger → `update` moves the model to `Done(status, body)`.
    #[test]
    fn http_request_response_round_trip() {
        let _ = crate::net::drain_commands(); // clear the shared queue
        clear_http_taggers();
        let src = "\
            type Phase = | Loading | Done(status: Float, body: String) | Failed(text: String)\n\
            type Model = { phase: Phase }\n\
            type Msg = | Got(resp: Net.HttpResponse)\n\
            let init = { phase: Loading }\n\
            let fetch = Effect.httpGet(\"http://127.0.0.1:9000/hello\", Got)\n\
            let update = (m: Model, msg: Msg) =>\n\
              match msg with\n\
              | Got(resp) =>\n\
                (match resp with\n\
                 | Net.Response(status, body) => { m with phase: Done(status, body) }\n\
                 | Net.Failure(err) => { m with phase: Failed(err) })\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));

        // Perform the fetch effect: queues the request + registers the tagger.
        let fetch = effect_of(&session.global("fetch").unwrap()).unwrap().0.clone();
        let mut model = session.global("init").unwrap();
        let mut log = EffectLog::new();
        let _ = drain_effects(
            &session,
            &mut model,
            fetch,
            &mut FakeEffects::new(0.0, vec![]),
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            false,
        );
        // One HttpRequest command was queued, carrying the request's token.
        let token = match crate::net::drain_commands().as_slice() {
            [crate::net::NetCommand::HttpRequest { token, method, url, .. }] => {
                assert_eq!(*method, crate::net::HttpMethod::Get);
                assert_eq!(url, "http://127.0.0.1:9000/hello");
                *token
            }
            other => panic!("expected one HttpRequest, got: {other:?}"),
        };
        assert_eq!(log.last().map(|r| r.kind), Some("net.http"));

        // The response lands: route it through the registered tagger → update.
        let tagger = take_http_tagger(token).expect("a tagger for the token");
        let resp = http_response_value(&crate::net::HttpResult {
            token,
            status: 200,
            body: b"hello!".to_vec(),
            error: None,
        });
        let msg = session.apply(tagger, vec![resp], "http", &mut FunctorHost).unwrap();
        let (model, _) =
            split_model_effect(session.call("update", vec![model, msg], &mut FunctorHost).unwrap());
        assert_eq!(model.to_string(), "{ phase: Done(200, \"hello!\") }");

        // The token is consumed (a duplicate/late response finds no tagger).
        assert!(take_http_tagger(token).is_none());
    }

    /// `suppress_outbound` (docs/time-travel.md T6b, the dry-run forward-step):
    /// draining an Http and an Audio effect with suppression on pushes NOTHING
    /// to the global queues (no request, no audio command, no registered
    /// tagger) but STILL appends the structured effect record to the log — the
    /// model evolves but nothing escapes to the live shell.
    #[test]
    fn suppress_outbound_logs_but_queues_nothing() {
        let _guard = crate::audio::OUTBOUND_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = crate::net::drain_commands(); // clear the shared queues
        let _ = crate::audio::drain_commands();
        clear_http_taggers();
        let src = "\
            let init = 0.0\n\
            let fetch = Effect.httpGet(\"http://127.0.0.1:9000/hello\", (r) => r)\n\
            let shoot = Effect.play(\"gunshot.wav\")\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let mut model = session.global("init").unwrap();
        let mut log = EffectLog::new();

        let fetch = effect_of(&session.global("fetch").unwrap()).unwrap().0.clone();
        let shoot = effect_of(&session.global("shoot").unwrap()).unwrap().0.clone();
        for effect in [fetch, shoot] {
            let _ = drain_effects(
                &session,
                &mut model,
                effect,
                &mut FakeEffects::new(0.0, vec![]),
                &mut log,
                &mut |m| panic!("unexpected report: {m}"),
                true, // suppress_outbound
            );
        }

        // Logged both — the model-facing record is unaffected by suppression.
        assert_eq!(
            log.iter().map(|r| r.kind).collect::<Vec<_>>(),
            vec!["net.http", "audio.play"]
        );
        // But nothing escaped: no net/http command, no audio command, no tagger.
        assert!(crate::net::drain_commands().is_empty(), "no outbound net command");
        assert!(crate::audio::drain_commands().is_empty(), "no outbound audio command");
    }

    // --- audio (roadmap E2): one-shots + soundScape ---

    /// `Effect.play(sound)` queues a non-spatial `PlayOneShot` AudioCommand
    /// (fire-and-forget, no token), and `Effect.playAt` sets `position`.
    #[test]
    fn play_and_play_at_queue_one_shot_commands() {
        let _guard = crate::audio::OUTBOUND_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = crate::audio::drain_commands(); // clear the shared queue
        let src = "\
            let init = 0.0\n\
            let shoot = Effect.play(\"gunshot.wav\")\n\
            let blast = Effect.playAt(\"explosion.wav\", Vec3.make(5.0, 0.5, -2.0))\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let mut model = session.global("init").unwrap();
        let mut log = EffectLog::new();

        let shoot = effect_of(&session.global("shoot").unwrap()).unwrap().0.clone();
        let _ = drain_effects(
            &session,
            &mut model,
            shoot,
            &mut FakeEffects::new(0.0, vec![]),
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            false,
        );
        assert_eq!(log.last().map(|r| r.kind), Some("audio.play"));

        let blast = effect_of(&session.global("blast").unwrap()).unwrap().0.clone();
        let _ = drain_effects(
            &session,
            &mut model,
            blast,
            &mut FakeEffects::new(0.0, vec![]),
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            false,
        );

        assert_eq!(
            crate::audio::drain_commands(),
            vec![
                crate::audio::AudioCommand::PlayOneShot {
                    token: None,
                    sound: "gunshot.wav".to_string(),
                    gain: 1.0,
                    position: None,
                },
                crate::audio::AudioCommand::PlayOneShot {
                    token: None,
                    sound: "explosion.wav".to_string(),
                    gain: 1.0,
                    position: Some([5.0, 0.5, -2.0]),
                },
            ]
        );
    }

    /// `Effect.playThen(sound, msg)` mints a token, queues a tokened one-shot,
    /// and registers the completion MESSAGE by that token; when the sound
    /// finishes, taking the message and folding it through `update` moves the
    /// model — the message is delivered verbatim (no tagger to apply).
    #[test]
    fn play_then_registers_message_and_completes_through_update() {
        let _guard = crate::audio::OUTBOUND_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = crate::audio::drain_commands(); // clear the shared queue
        clear_audio_completions();
        let src = "\
            type Model = | Playing | Finished\n\
            let init = Playing\n\
            let ping = Effect.playThen(\"chime.wav\", Finished)\n\
            let update = (m: Model, msg: Model) => msg\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let mut model = session.global("init").unwrap();
        let mut log = EffectLog::new();

        let ping = effect_of(&session.global("ping").unwrap()).unwrap().0.clone();
        let _ = drain_effects(
            &session,
            &mut model,
            ping,
            &mut FakeEffects::new(0.0, vec![]),
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            false,
        );
        assert_eq!(log.last().map(|r| r.kind), Some("audio.playThen"));

        // One tokened one-shot was queued.
        let token = match crate::audio::drain_commands().as_slice() {
            [crate::audio::AudioCommand::PlayOneShot { token: Some(t), sound, .. }] => {
                assert_eq!(sound, "chime.wav");
                *t
            }
            other => panic!("expected one tokened PlayOneShot, got: {other:?}"),
        };

        // The sound finishes: take the registered message and fold it through
        // `update` (delivered verbatim — no tagger).
        let message = take_audio_completion(token).expect("a message for the token");
        let (model, _) = split_model_effect(
            session
                .call("update", vec![model, message], &mut FunctorHost)
                .unwrap(),
        );
        assert_eq!(model.to_string(), "Finished");

        // The token is consumed (a duplicate/late finish finds no message).
        assert!(take_audio_completion(token).is_none());
    }

    /// `PENDING_AUDIO` is bounded: because audio finishes are best-effort (a
    /// sound may never start or report), the completion map is NOT self-draining
    /// like `PENDING_HTTP`. Registering past the cap evicts the oldest (lowest
    /// token), so a game that fires `playThen` in a loop can't grow it without
    /// limit.
    #[test]
    fn pending_audio_completions_are_bounded() {
        clear_audio_completions();
        // Fill past the cap; oldest tokens should be evicted.
        for token in 1..=(PENDING_AUDIO_CAP as u64 + 10) {
            register_audio_completion(token, Value::Number(token as f64));
        }
        // The 10 oldest are gone; the map holds exactly the cap.
        assert!(take_audio_completion(1).is_none());
        assert!(take_audio_completion(10).is_none());
        // A recent token survives.
        assert!(take_audio_completion(PENDING_AUDIO_CAP as u64 + 10).is_some());
        clear_audio_completions();
    }

    /// A `soundScape` returning a two-voice `AudioScene` (an ambient bed with a
    /// piped gain + a positioned emitter) serializes to JSON carrying both keys
    /// and the gain — exactly what the shell reconciles.
    #[test]
    fn sound_scape_serializes_to_reconcilable_json() {
        let src = "\
            let init = 0.0\n\
            let soundScape = (m) =>\n\
              AudioScene.create([\n\
                AudioSource.ambient(\"wind\", \"wind-loop.wav\") |> AudioSource.gain(0.35),\n\
                AudioSource.at(\"fountain\", \"water.wav\", Vec3.make(5.0, 0.5, 0.0))\n\
              ])\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let model = session.global("init").unwrap();

        let value = session
            .call("soundScape", vec![model], &mut FunctorHost)
            .unwrap();
        let scene = audio_scene_of(&value).expect("soundScape must return an AudioScene");
        let json = crate::audio::scene_to_json(scene);

        // Round-trips through the wire form the shell deserializes.
        let back: crate::audio::AudioScene = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sources.len(), 2);
        let wind = &back.sources[0];
        assert_eq!(wind.key, "wind");
        assert_eq!(wind.sound, "wind-loop.wav");
        assert!((wind.gain - 0.35).abs() < 1e-6);
        assert_eq!(wind.position, None);
        let fountain = &back.sources[1];
        assert_eq!(fountain.key, "fountain");
        assert_eq!(fountain.position, Some([5.0, 0.5, 0.0]));
    }

    /// Headless server-lifecycle test for the `mpserver` port (roadmap E2),
    /// with no socket. Loads the SHIPPED `game.fun` so this tracks the example,
    /// and exercises the whole server spine — `toMsg` decoding, the
    /// join/move/left `update` logic, `wrapAxis` integration, the `Text.*` wire
    /// encoding, and the broadcast-the-whole-world-to-every-client `Effect.send`
    /// (the naive server's defining behavior), plus a malformed packet ignored
    /// and a disconnect dropping a player.
    #[test]
    fn mpserver_broadcasts_the_world_to_every_client() {
        // load_single_source injects the built-in Net module, like the runner.
        let src = include_str!("../../../examples/mpserver/game.fun");
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load mpserver: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));

        // toMsg(event) folded through update; update returns a bare Model here.
        fn call(session: &functor_lang::Session, name: &str, args: Vec<Value>) -> Value {
            let f = session.global(name).unwrap_or_else(|| panic!("no `{name}`"));
            session
                .apply(f, args, name, &mut FunctorHost)
                .unwrap_or_else(|e| panic!("{name}: {}", e.message))
        }
        fn feed(session: &functor_lang::Session, model: Value, ev: Value) -> Value {
            let msg = call(session, "toMsg", vec![ev]);
            split_model_effect(call(session, "update", vec![model, msg])).0
        }
        // One tick's broadcast, drained into (conn, payload) Send pairs.
        fn broadcast(session: &functor_lang::Session, model: Value) -> (Value, Vec<(u64, Vec<u8>)>) {
            crate::net::drain_conn_commands(); // clear
            let (mut m, fx) = split_model_effect(call(
                session,
                "tick",
                vec![model, Value::Number(0.5), Value::Number(0.5)],
            ));
            let mut log = EffectLog::new();
            if let Some(tree) = fx {
                let _ = drain_effects(
                    session,
                    &mut m,
                    tree,
                    &mut FakeEffects::new(0.0, vec![]),
                    &mut log,
                    &mut |r| panic!("unexpected report: {r}"),
                    false,
                );
            }
            let sends = crate::net::drain_conn_commands()
                .into_iter()
                .filter_map(|c| match c {
                    crate::net::ConnCommand::Send { conn, payload } => Some((conn, payload)),
                    _ => None,
                })
                .collect();
            (m, sends)
        }
        let event =
            |kind, id: u64, text: &str| net_event_value(kind, id, text).to_functor_lang();

        // The listener is declared on the arena address.
        let subs = call(&session, "subscriptions", vec![session.global("init").unwrap()]);
        let conns = net_conn_subs(&subs).expect("a Sub tree");
        assert!(conns.len() == 1 && conns[0].listen && conns[0].key == "127.0.0.1:9001");

        // Two clients join (pid 0 on cid 1, pid 1 on cid 2), each sends a
        // velocity, and a single-token packet from cid 1 is IGNORED (its
        // velocity is not reset — a 2-token "vx vz" is the only valid form).
        let mut model = session.global("init").unwrap();
        model = feed(&session, model, event(NetEventKind::Connected, 1, ""));
        model = feed(&session, model, event(NetEventKind::Connected, 2, ""));
        model = feed(&session, model, event(NetEventKind::Message, 1, "1 0")); // pid 0: +x
        model = feed(&session, model, event(NetEventKind::Message, 2, "0 1")); // pid 1: +z
        model = feed(&session, model, event(NetEventKind::Message, 1, "junk")); // ignored

        // Tick (dt = 0.5). pid 0: x = -2 + 1·2·0.5 = -1.0, z = -1.8 -> "0,-100,-180".
        // pid 1: x = -2.0, z = 0 + 1·2·0.5 = 1.0 -> "1,-200,100". pid 0 still
        // moving proves the "junk" packet did NOT reset its velocity.
        let (model, sends) = broadcast(&session, model);
        // The WHOLE world goes to EVERY client — two identical full snapshots.
        let snapshot = b"1,-200,100|0,-100,-180".to_vec();
        assert_eq!(sends.len(), 2, "one Send per client, got: {sends:?}");
        let mut recipients: Vec<u64> = sends.iter().map(|(c, _)| *c).collect();
        recipients.sort();
        assert_eq!(recipients, vec![1, 2], "broadcast reaches both clients");
        assert!(
            sends.iter().all(|(_, p)| *p == snapshot),
            "every client receives the full world, got: {sends:?}"
        );

        // Client 1 disconnects -> Left drops pid 0. The next tick broadcasts
        // only pid 1, and only to the client that is still connected (cid 2).
        let model = feed(&session, model, event(NetEventKind::Disconnected, 1, ""));
        let (_, sends) = broadcast(&session, model);
        assert_eq!(sends.len(), 1, "only the remaining client is served: {sends:?}");
        assert_eq!(sends[0].0, 2);
        assert!(
            sends[0].1.starts_with(b"1,"),
            "snapshot no longer contains pid 0, got: {:?}",
            String::from_utf8_lossy(&sends[0].1)
        );
    }

    /// Headless client-lifecycle test for the `mpclient` port (roadmap E2),
    /// with no socket. Loads the SHIPPED `game.fun` and drives connect (auto-move
    /// sent), a snapshot decoded into the world, WASD `input` producing sends,
    /// and a disconnect. The snapshot fed in is the exact wire string the
    /// `mpserver` port broadcasts, so this doubles as a wire round-trip check
    /// between the two ports.
    #[test]
    fn mpclient_decodes_snapshots_and_sends_input() {
        let src = include_str!("../../../examples/mpclient/game.fun");
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load mpclient: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        fn call(session: &functor_lang::Session, name: &str, args: Vec<Value>) -> Value {
            let f = session.global(name).unwrap_or_else(|| panic!("no `{name}`"));
            session
                .apply(f, args, name, &mut FunctorHost)
                .unwrap_or_else(|e| panic!("{name}: {}", e.message))
        }
        fn field<'a>(v: &'a Value, name: &str) -> &'a Value {
            match v {
                Value::Record(fields) => fields
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| v)
                    .unwrap_or_else(|| panic!("no field `{name}`")),
                other => panic!("not a record: {other}"),
            }
        }
        fn num(v: &Value) -> f64 {
            match v {
                Value::Number(n) => *n,
                other => panic!("not a number: {other}"),
            }
        }
        // Drain whatever an entry point emitted into (conn, payload) Send pairs.
        fn sends_of(session: &functor_lang::Session, returned: Value) -> Vec<(u64, Vec<u8>)> {
            crate::net::drain_conn_commands(); // clear
            let (mut m, fx) = split_model_effect(returned);
            if let Some(tree) = fx {
                let mut log = EffectLog::new();
                let _ = drain_effects(
                    session,
                    &mut m,
                    tree,
                    &mut FakeEffects::new(0.0, vec![]),
                    &mut log,
                    &mut |r| panic!("unexpected report: {r}"),
                    false,
                );
            }
            crate::net::drain_conn_commands()
                .into_iter()
                .filter_map(|c| match c {
                    crate::net::ConnCommand::Send { conn, payload } => Some((conn, payload)),
                    _ => None,
                })
                .collect()
        }
        let event = |kind, id: u64, text: &str| net_event_value(kind, id, text).to_functor_lang();
        // Keys are the built-in `Key` module's variants (`Key.W`), as the
        // producers deliver them.
        let key = |k: &str| Value::Variant {
            ctor: std::rc::Rc::from(format!("Key.{k}").as_str()),
            args: std::rc::Rc::new(Vec::new()),
        };

        // subscriptions declares an OUTBOUND connection (not a listener).
        let subs = call(&session, "subscriptions", vec![session.global("init").unwrap()]);
        let conns = net_conn_subs(&subs).expect("a Sub tree");
        assert!(!conns[0].listen && conns[0].key == "ws://127.0.0.1:9001/play");

        // The socket opens (id 5): store it and auto-move +x (Effect.send "1 0").
        let connected = call(&session, "toMsg", vec![event(NetEventKind::Connected, 5, "")]);
        let joined = call(&session, "update", vec![session.global("init").unwrap(), connected]);
        let (model, _) = split_model_effect(joined.clone());
        assert_eq!(field(&model, "conn").to_string(), "Online(5)");
        assert_eq!(field(&model, "status").to_string(), "\"connected\"");
        assert_eq!(sends_of(&session, joined), vec![(5, b"1 0".to_vec())]);

        // WASD `input` (the trickiest hook: nested match, mixed bare/tuple arms)
        // sends the mapped velocity on keydown, a stop on keyup, and NOTHING for
        // a non-WASD key or before the socket opens.
        assert_eq!(
            sends_of(&session, call(&session, "input", vec![model.clone(), key("W"), Value::Bool(true)])),
            vec![(5, b"0 1".to_vec())]
        );
        assert_eq!(
            sends_of(&session, call(&session, "input", vec![model.clone(), key("A"), Value::Bool(true)])),
            vec![(5, b"-1 0".to_vec())]
        );
        assert!(
            sends_of(&session, call(&session, "input", vec![model.clone(), key("X"), Value::Bool(true)])).is_empty(),
            "a non-WASD key sends nothing"
        );
        assert_eq!(
            sends_of(&session, call(&session, "input", vec![model.clone(), key("W"), Value::Bool(false)])),
            vec![(5, b"0 0".to_vec())],
            "key release sends a stop"
        );
        assert!(
            sends_of(&session, call(&session, "input", vec![session.global("init").unwrap(), key("W"), Value::Bool(true)])).is_empty(),
            "input before connect sends nothing"
        );

        // A server snapshot (the exact wire string mpserver broadcasts)
        // decodes into the world, binding each pid to its own coordinates.
        let msg = call(&session, "toMsg", vec![event(NetEventKind::Message, 5, "1,-200,100|0,-100,-180")]);
        let (model, _) = split_model_effect(call(&session, "update", vec![model, msg]));
        assert_eq!(field(&model, "status").to_string(), "\"in-world\"");
        let world = match field(&model, "world") {
            Value::List(items) => items.clone(),
            other => panic!("world is not a list: {other}"),
        };
        assert_eq!(world.len(), 2, "two players decoded, got: {}", field(&model, "world"));
        let at = |pid: f64| -> (f64, f64) {
            let p = world
                .iter()
                .find(|p| num(field(p, "pid")) == pid)
                .unwrap_or_else(|| panic!("no player pid {pid}"));
            (num(field(p, "x")), num(field(p, "z")))
        };
        // *100 fixed-point undone, per-pid (a swapped decoder would fail here).
        assert_eq!(at(0.0), (-1.0, -1.8));
        assert_eq!(at(1.0), (-2.0, 1.0));

        // Disconnect drops the connection and clears the id.
        let dropped = call(&session, "toMsg", vec![event(NetEventKind::Disconnected, 5, "")]);
        let (model, _) = split_model_effect(call(&session, "update", vec![model, dropped]));
        assert_eq!(field(&model, "conn").to_string(), "Offline");
        assert_eq!(field(&model, "status").to_string(), "\"disconnected\"");
    }

    // Ui teaching errors: non-View children and unbranded anchors fail loud.
    #[test]
    fn ui_teaches_its_usage() {
        // [registry delta] was "Ui.column items must be Views, got a number"
        // — the uniform typed list-element error.
        assert_eq!(
            run_fail("let main = () => Ui.column([Ui.text(\"a\"), 3.0])"),
            "Ui.column: expected a View, got a number"
        );
        assert_eq!(
            run_fail("let main = () => Ui.text(\"a\") |> Ui.panel(\"topLeft\")"),
            "Ui.panel: expected an Anchor, got a bare string — pin a corner \
with Ui.topLeft()"
        );
        assert_eq!(
            run_fail("let main = () => Ui.textColor(Color.rgb(1.0, \"x\", 0.0), \"a\")"),
            "expected a number, got a string"
        );
        // The pre-Color four-float spelling teaches the new shape.
        assert_eq!(
            run_fail("let main = () => Ui.textColor(1.0, 0.85, 0.4, \"a\")"),
            "usage: Ui.textColor(color, \"…\")"
        );
    }

    /// `Debug.log` added to `tick` via HOT-RELOAD fires on the next frame AND
    /// still routes through the installed sink — the sink lives on the process
    /// (installed once at host startup, not on any `Session`), so it survives
    /// the reload's `Session` rebuild. This regression-guards the failure mode
    /// where a live-added `Debug.log` would fall back to a raw stdout
    /// `println!` and corrupt the ndjson stream / live telemetry panel.
    ///
    /// The test provides a CAPTURING sink (standing in for the real
    /// region-aware one `install_debug_log_sink` wires up) and reads back the
    /// emitted lines.
    #[test]
    fn debug_log_added_by_hot_reload_still_routes_through_the_installed_sink() {
        use std::sync::{Arc, Mutex};

        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_buf = Arc::clone(&captured);
        // The sink is process-global; installed once, BEFORE the reload.
        functor_lang::set_trace_sink(Box::new(move |m| sink_buf.lock().unwrap().push(m)));

        fn load(src: &str) -> functor_lang::Session {
            let module = functor_lang::lower(functor_lang::parse(src).expect("parse")).expect("lower");
            functor_lang::Session::load(&module, &mut FunctorHost)
                .unwrap_or_else(|f| panic!("load failed: {}", f.error.message))
        }
        fn tick(session: &functor_lang::Session, m: f64) -> Value {
            session
                .call(
                    "tick",
                    vec![Value::Number(m), Value::Number(0.016), Value::Number(1.0)],
                    &mut FunctorHost,
                )
                .expect("tick")
        }

        // v1: tick has NO Debug.log. A frame emits nothing.
        let v1 = load(
            "let init = 0.0\n\
             let tick = (m, dt, tts) => m + 1.0\n\
             let draw = (m, tts) => m",
        );
        assert_eq!(tick(&v1, 3.0).to_string(), "4");
        assert!(
            captured.lock().unwrap().is_empty(),
            "no trace before a Debug.log is added"
        );

        // Hot-reload: a NEW Session built from edited source that ADDS a
        // Debug.log in tick (the producer's reload path rebuilds the Session).
        let v2 = load(
            "let init = 0.0\n\
             let tick = (m, dt, tts) => Debug.log(\"tick m\", m + 1.0)\n\
             let draw = (m, tts) => m",
        );
        // The trace fires on the next frame — model unaffected (returns m + 1).
        assert_eq!(tick(&v2, 4.0).to_string(), "5");
        assert_eq!(
            *captured.lock().unwrap(),
            vec!["tick m: 5".to_string()],
            "the added Debug.log routed through the sink that survived the reload"
        );

        // The reverse: reload back to a Debug.log-free tick → it stops cleanly.
        let v3 = load(
            "let init = 0.0\n\
             let tick = (m, dt, tts) => m + 1.0\n\
             let draw = (m, tts) => m",
        );
        let _ = tick(&v3, 9.0);
        assert_eq!(
            captured.lock().unwrap().len(),
            1,
            "removing the Debug.log stops emission (still just the one line)"
        );
    }
}
