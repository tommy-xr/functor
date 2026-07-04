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
//! Scene.group([scene, …])                                   -> Scene
//! Scene.color(scene, r, g, b)                               -> Scene
//! Scene.translate(scene, x, y, z)                           -> Scene
//! Scene.rotateX/rotateY/rotateZ(scene, angle)               -> Scene
//! Angle.degrees(n) / Angle.radians(n)                       -> Angle
//!   (rotations and camera angles take Angle VALUES, never bare numbers —
//!    degree/radian confusion is unrepresentable)
//! Scene.scale(scene, k)                                     -> Scene
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

use crate::math::Angle;
use crate::physics;
use crate::render_target::RenderTargetDescriptor;
use crate::scene3d::{MaterialDescription, TextureDescription};
use crate::{Camera, Frame, Light, Scene3D, SceneObject};

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
}

pub struct MleEffect(pub EffectTree);

/// Performs effects. `Real` asks the world; `Fake` gives fixed values
/// (tests); `Replay` feeds back a recorded [`EffectLog`] — same program,
/// three worlds, one contract (docs/mle.md B6).
pub trait EffectRunner {
    fn now(&mut self) -> f64;
    fn random(&mut self) -> f64;
}

/// The structured effect log keeps this many most-recent records — enforced
/// INSIDE the drain, so the bound holds even mid-frame.
pub const EFFECT_LOG_CAP: usize = 256;

/// One performed effect: what kind, and what value the tagger received —
/// the structured effect log (LLM-readable, and the input to replay).
#[derive(Clone, Debug, PartialEq)]
pub struct EffectRecord {
    pub kind: &'static str,
    pub value: f64,
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
}

impl FakeEffects {
    pub fn new(now: f64, randoms: Vec<f64>) -> FakeEffects {
        FakeEffects {
            now,
            randoms,
            next: 0,
        }
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
    fn take(&mut self, kind: &'static str) -> f64 {
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
        record.value
    }
}

impl EffectRunner for ReplayEffects {
    fn now(&mut self) -> f64 {
        self.take("now")
    }
    fn random(&mut self) -> f64 {
        self.take("random")
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
    "Scene.group",
    "Scene.color",
    "Scene.translate",
    "Scene.rotateX",
    "Scene.rotateY",
    "Scene.rotateZ",
    "Scene.scale",
    "Scene.lit",
    "Scene.emissive",
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
    "RenderTarget.named",
    "RenderTarget.sized",
    "Scene.screen",
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
pub fn drain_effects(
    session: &mle::Session,
    model: &mut Value,
    first: EffectTree,
    runner: &mut dyn EffectRunner,
    log: &mut EffectLog,
    report: &mut dyn FnMut(String),
) {
    const MAX_EFFECTS_PER_FRAME: usize = 1000;
    let mut queue: Vec<EffectTree> = vec![first];
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
                // (there is no result value to feed a tagger), continue. Not
                // routed through the runner — commands are per-frame *inputs*
                // recorded by the physics Timeline, not environment reads that
                // replay must fake.
                let kind = match &command {
                    physics::PhysicsCommand::ApplyImpulse { .. } => "physics.applyImpulse",
                    physics::PhysicsCommand::ApplyForce { .. } => "physics.applyForce",
                    physics::PhysicsCommand::SetVelocity { .. } => "physics.setVelocity",
                    physics::PhysicsCommand::Teleport { .. } => "physics.teleport",
                };
                physics::with_world(physics::DEFAULT_WORLD, |w| w.queue_command(command));
                log.push(EffectRecord { kind, value: 0.0 });
                if log.len() > EFFECT_LOG_CAP {
                    log.remove(0);
                }
                continue;
            }
            EffectTree::Now { tagger } => ("now", runner.now(), tagger),
            EffectTree::Random { tagger } => ("random", runner.random(), tagger),
        };
        log.push(EffectRecord { kind, value });
        if log.len() > EFFECT_LOG_CAP {
            log.remove(0);
        }
        let msg = match session.apply(
            tagger,
            vec![Value::Number(value)],
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
        drain_effects(
            &session,
            &mut model,
            tree.clone(),
            &mut runner,
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
        );
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].kind, "physics.applyImpulse");

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
            drain_effects(
                &session,
                &mut model,
                fx.expect("an effect"),
                runner,
                &mut log,
                &mut |msg| panic!("unexpected report: {msg}"),
            );
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
                    value: 0.25
                },
                EffectRecord {
                    kind: "now",
                    value: 99.5
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
        assert!((0.0..1.0).contains(&real_log[0].value));
        assert_eq!(real_log[1].kind, "now");
        assert!(real_model.starts_with("{ rolls: 0."));
    }

    /// A diverged replay fails loud with the position, not silently wrong.
    #[test]
    #[should_panic(expected = "replay diverged at effect 0")]
    fn replay_divergence_fails_loud() {
        ReplayEffects::new(vec![EffectRecord {
            kind: "now",
            value: 1.0,
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
        drain_effects(
            &session,
            &mut model,
            EffectTree::Random {
                tagger: session.global("again").expect("tagger fn"),
            },
            &mut FakeEffects::new(0.0, vec![0.5]),
            &mut log,
            &mut |msg| reports.push(msg),
        );
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
        drain_effects(
            &session,
            &mut model,
            EffectTree::Batch(vec![EffectTree::None; 1500]),
            &mut FakeEffects::new(0.0, vec![0.5]),
            &mut log,
            &mut |msg| reports.push(msg),
        );
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
}
