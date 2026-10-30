//! The Functor prelude for Functor Lang — Track C1 of `docs/functor-lang.md`.
//!
//! A [`functor_lang::Host`] giving Functor Lang programs the engine vocabulary: scene
//! constructors, transforms, a camera, and frame assembly, all producing the
//! exact protocol types this crate already speaks ([`Scene3D`], [`Camera`],
//! [`Frame`] — see [`crate::protocol`]). Most engine values cross into Functor
//! Lang as opaque [`functor_lang::Value::HostData`]; `Sprite.t` is the deliberate
//! exception, represented by a private tree of ordinary Functor values so it
//! stays inspectable and reload-safe. Functor Lang code hands back a `Frame`,
//! which the shells extract with [`frame_value`].
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
//! Camera3D.lookAt(Vec3.make(ex, ey, ez), Vec3.make(tx, ty, tz))                     -> Camera3D
//!   (up is +Y; vertical fov pinned at 45°, near/far at protocol defaults)
//! Camera3D.toWorldRay(mouse, camera)                         -> Option<{ origin, direction }>
//!   (top-left logical mouse coordinates through the authored perspective;
//!    both fields are Vec3 values and direction is normalized)
//! Anim.lookAt(joint, target, maxDeflection, weight, anim)  -> Anim
//!   (post-pass aim of local +Z at a model-space Vec3 target)
//! Anim.reach(root, middle, end, target, weight, anim)      -> Anim
//!   (post-pass two-bone reach preserving the evaluated bend side)
//! Frame.create(camera, scene)                               -> Frame
//! Camera2D.create(width, height)                             -> Camera2D
//! Camera2D.at(x, y, camera) / Camera2D.zoom(k, camera)       -> Camera2D
//! Camera2D.toWorld(mouse, camera)                            -> Option<Input.point2>
//!   (center-origin, Y-up world units, aspect-fitted without stretching)
//! Sprite.blank() / rectangle(color, w, h) / square(color, n) -> Sprite
//! Sprite.image(w, h, texture) / imageRegion(w, h, region, texture) -> Sprite
//! Sprite.region(x, y, width, height)                         -> Sprite.region
//! Sprite.group([sprite, …])                                  -> Sprite
//! Sprite.move(x, y, sprite) / moveX / moveY                  -> Sprite
//! Sprite.rotate(angle, sprite) / scale / scaleXY             -> Sprite
//! Sprite.fade(alpha, sprite) / tint(color, sprite)           -> Sprite
//! Sprite.nearest(sprite) / Sprite.linear(sprite)              -> Sprite
//!   (the abstract Sprite.t is a private PLAIN-DATA picture tree; later
//!    group items paint above earlier ones; regions are top-left source
//!    pixels; subtree alpha/tint/sampling compose)
//! Frame.create2D(camera2d, sprite)                           -> Frame
//! Frame.with2D(camera2d, sprite, frame)                      -> Frame
//!   (2D-only frame, or a sprite layer above an existing frame)
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
//! Frame.withPassthrough(frame)                              -> Frame
//!   (the background is the real world: no skybox; a headset compositing
//!    its camera feed clears transparent, elsewhere a stand-in room)
//! Frame.withStandInRoom(room, frame)                        -> Frame
//!   (scene geometry for that stand-in room, drawn only where it is; under
//!    `Xr.scene(true)` it is also the emulated environment depth)
//! Frame.withSurfaceGrid(cell, color, frame)                 -> Frame
//!   (a world-aligned grid shaded onto the room's surfaces, read per pixel
//!    from the environment depth; nothing without `Xr.scene(true)`)
//! Frame.withFusedRoom(cell, color, frame)                   -> Frame
//!   (the room meshed from that depth over time, drawn with the same grid)
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
//! Physics.rotateX/rotateY/rotateZ(angle, body)              -> Body
//! Physics.mass/friction/restitution(n, body)                -> Body
//! Physics.linearDamping/angularDamping(n, body)             -> Body
//! Physics.sensor(body)                                      -> Body
//! Physics.upright(body)                                     -> Body
//! Physics.continuous(body)                                  -> Body
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
use functor_lang::reads::ReadClass;
use functor_lang::value::HostData;
use functor_lang::{Host, HostApply, HostReturn, RunError, Span, Value};
use std::rc::Rc;

use crate::anim::AnimExpr;
use crate::fog::Fog;
use crate::math::Angle;
use crate::physics;
use crate::render_target::RenderTargetDescriptor;
use crate::scene3d::{
    InstanceData, InstanceList, MaterialDescription, ModelDescription, ModelHandle, SpriteSampling,
    TextureDescription,
};
use crate::skybox::SkyboxDescription;
use crate::terrain::TerrainDescription;
use crate::timetravel::{args_digest, Entry};
use crate::ui::{self, View};
use crate::webview::HtmlNode;
use crate::{Camera, Camera2D, Frame, Light, Scene3D, SceneObject, Shape, SpriteLayer};

pub mod bytes;
pub mod cells;
pub mod lightmap;
pub mod mesh;
pub mod model;
pub mod path;
pub mod solid;
mod sprite;
mod texture;

/// A [`Scene3D`] as an opaque Functor Lang value.
pub struct FunctorLangScene(pub Scene3D);

/// One `Instance.t` — a copy's placement channels for `Scene.instanced`
/// (position, rotation, per-axis scale, tint), built by the `Instance.*`
/// combinators. Plain data underneath, so it is reload-safe.
#[derive(Clone)]
pub struct FunctorLangInstance(pub InstanceData);

/// A [`TerrainDescription`] as an opaque, immutable Functor Lang value.
pub struct FunctorLangTerrain(pub TerrainDescription);

/// A [`Camera`] as an opaque Functor Lang value.
pub struct FunctorLangCamera(pub Camera);

/// A sampled mouse position plus the logical surface extent sharing its
/// top-left-origin coordinate space. Camera2D and Camera deliberately share
/// this decoder so their screen-to-world APIs accept the exact same input.
struct FunctorLangMouse {
    x: f32,
    y: f32,
    surface_width: f32,
    surface_height: f32,
}

fn finite_record_number(
    fields: &[(String, Value)],
    field: &str,
    record_name: &str,
    path: &str,
    span: Span,
) -> Result<f32, RunError> {
    match fields.iter().find(|(name, _)| name == field) {
        Some((_, Value::Number(n))) if (*n as f32).is_finite() => Ok(*n as f32),
        Some((_, Value::Number(n))) => Err(RunError {
            message: format!("{path}: {record_name} `{field}` must be finite, got {n}"),
            span,
        }),
        Some((_, other)) => Err(RunError {
            message: format!(
                "{path}: {record_name} `{field}` must be a number, got {}",
                other.kind_name()
            ),
            span,
        }),
        None => Err(RunError {
            message: format!("{path}: expected a {record_name} record, missing `{field}`"),
            span,
        }),
    }
}

impl crate::host_registry::FromArg for FunctorLangMouse {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let Value::Record(fields) = value else {
            return Err(RunError {
                message: format!(
                    "{path}: expected an Input.mouse record, got {}",
                    value.kind_name()
                ),
                span,
            });
        };
        Ok(FunctorLangMouse {
            x: finite_record_number(fields, "x", "mouse", path, span)?,
            y: finite_record_number(fields, "y", "mouse", path, span)?,
            surface_width: finite_record_number(fields, "surfaceWidth", "mouse", path, span)?,
            surface_height: finite_record_number(fields, "surfaceHeight", "mouse", path, span)?,
        })
    }
}

/// A `Camera3D.mappedPose` record (`position`, `forward`, `up`, each an
/// `Input.point3`), as `Scene.atPose` takes it.
struct FunctorLangMappedPose(crate::MappedTrackingPose);

impl crate::host_registry::FromArg for FunctorLangMappedPose {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let fail = |message: String| RunError { message, span };
        let Value::Record(fields) = value else {
            return Err(fail(format!(
                "{path}: expected a Camera3D.mappedPose record, got {}",
                value.kind_name()
            )));
        };
        let point = |name: &str| -> Result<[f32; 3], RunError> {
            match fields.iter().find(|(field, _)| field == name) {
                Some((_, Value::Record(p))) => {
                    let what = format!("mappedPose.{name}");
                    Ok([
                        finite_record_number(p, "x", &what, path, span)?,
                        finite_record_number(p, "y", &what, path, span)?,
                        finite_record_number(p, "z", &what, path, span)?,
                    ])
                }
                Some((_, other)) => Err(fail(format!(
                    "{path}: mappedPose `{name}` must be an Input.point3 record, got {}",
                    other.kind_name()
                ))),
                None => Err(fail(format!(
                    "{path}: expected a Camera3D.mappedPose record, missing `{name}`"
                ))),
            }
        };
        Ok(FunctorLangMappedPose(crate::MappedTrackingPose {
            position: point("position")?,
            forward: point("forward")?,
            up: point("up")?,
        }))
    }
}

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
    /// Send a plain-data VALUE on an open connection
    /// (`Effect.sendMsg(id, msg)`). The payload is converted to its
    /// [`EffectValue`] at construction (a closure inside is a teaching error
    /// at the call site) and framed as [`TYPED_MSG_PREFIX`] + JSON on drain;
    /// the receiving end decodes it back and delivers `Net.Data(id, value)`
    /// through the connection's tagger. Tagger-less, like `Send`.
    SendMsg {
        conn: f64,
        payload: EffectValue,
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
    /// Persist a plain-data value in a local save slot (`Persistence.save(slot,
    /// value)`). The payload is converted to its [`EffectValue`] at
    /// construction (a closure inside is a teaching error at the call site,
    /// exactly like [`Self::SendMsg`]) and written by the runner —
    /// `Real` to the platform store ([`crate::storage`]), `Fake`/`Replay`
    /// to memory. Tagger-less: a save reports nothing back.
    Save {
        slot: String,
        payload: EffectValue,
    },
    /// Read a local save slot back (`Persistence.load(slot, tagger)`). A same-frame
    /// runner read like [`Self::Now`]: the tagger receives
    /// `Option.Some(value)` or `Option.None` (absent OR unreadable/corrupt —
    /// a broken save must not kill a running game), and the result lands in
    /// the frame log, so a replay feeds back exactly what was loaded.
    Load {
        slot: String,
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
    /// Warm the asset cache ahead of `draw` referencing the asset
    /// (`Effect.preload`) — the imperative prefetch; declarative loading
    /// (draw references it → it loads) stays the default. Fire-and-forget:
    /// queues a [`crate::asset::preload::PreloadCommand`] the shell drains
    /// into a cache load and then POLLS to settlement (the
    /// `resolve_while_pending` liveness rule — an unpolled preload would
    /// strand in `Sub.assets`' total).
    PreloadAsset {
        kind: crate::asset::preload::PreloadKind,
        locator: String,
    },
    /// A preload whose settlement is reported back frames later
    /// (`Effect.preloadThen`) — the [`Self::PlayAudioThen`] shape: mint a
    /// token, register `message` by it, queue a tokened command. When the
    /// shell reports the load SETTLED — loaded OR failed (`Sub.assets`'
    /// `failed` list says which) — the producer delivers `message` VERBATIM
    /// through `update`.
    PreloadAssetThen {
        kind: crate::asset::preload::PreloadKind,
        locator: String,
        message: Value,
    },
}

pub struct FunctorLangEffect(pub EffectTree);

/// Performs effects. `Real` asks the world; `Fake` gives fixed values
/// (tests); `Replay` answers from the recorded frame log — same program,
/// three worlds, one contract (docs/functor-lang.md B6).
pub trait EffectRunner {
    fn now(&mut self) -> f64;
    fn random(&mut self) -> f64;
    /// The raycast result record (`{hit, x, y, z, nx, ny, nz, distance,
    /// tag}` — `hit: false` with zeroed fields for a miss). `Real` and
    /// `Replay` ask the active physics world (an ask is recomputed, never
    /// recorded); `Fake` returns canned records — physics queries are
    /// testable without a world at all.
    fn raycast(&mut self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> EffectValue;
    /// Persist a save slot (`Persistence.save`). `Real` writes the platform store;
    /// `Fake` records the write in memory (so a test can assert what a game
    /// saved without touching a disk); `Replay` keeps it in its own overlay —
    /// a replay must not rewrite the world it is replaying. An error is
    /// returned for the drain to report; the game keeps running.
    fn save(&mut self, slot: &str, payload: &EffectValue) -> Result<(), String>;
    /// Read a save slot (`Persistence.load`) as `Option.Some(value)` /
    /// `Option.None` — see [`save_slot_value`]. `Real` asks the platform
    /// store; `Fake` answers from what it recorded (empty = `None`); `Replay`
    /// replays the recorded answer.
    fn load(&mut self, slot: &str) -> EffectValue;
    /// An outbound intent was issued (logged as [`Entry::Issued`]). `Replay`
    /// checks it against the log; the others ignore it.
    fn issued(&mut self, _kind: &'static str, _args: u64) {}
    /// A replayed frame starts: its recorded log, or `None` past the log.
    fn begin_frame(&mut self, _recorded: Option<&[Entry]>) {}
}

/// The tagger-facing `Option` for a loaded save slot. The ctors are the
/// stdlib `Option`'s canonical names, so a game matches the ordinary
/// `Option.Some(v)` / `Option.None` it already knows.
pub fn save_slot_value(loaded: Option<EffectValue>) -> EffectValue {
    match loaded {
        Some(value) => EffectValue::Variant("Option.Some".to_string(), vec![value]),
        None => EffectValue::Variant("Option.None".to_string(), Vec::new()),
    }
}

/// Read a slot through the platform store and decode it — the `Real`/dry-run
/// load. Absent OR undecodable both answer `None` (loudly logged): a save
/// written by an older version of the game must degrade to "no save", never
/// to a crash.
fn read_saved_slot(slot: &str) -> EffectValue {
    let text = match crate::storage::read_slot(slot) {
        Ok(Some(text)) => text,
        Ok(None) => return save_slot_value(None),
        Err(error) => {
            log::warn!("[functor-lang] Persistence.load(\"{slot}\"): {error}");
            return save_slot_value(None);
        }
    };
    match serde_json::from_str::<EffectValue>(&text) {
        // Convertibility is part of "readable": a slot file is EXTERNAL data
        // (hand-edited, foreign, version-skewed), and a value that decodes but
        // cannot become a Functor value would otherwise report mid-drain and
        // silently never call the tagger — the game would wait forever for a
        // restore message. Answer None instead.
        Ok(value) if value.to_functor_lang().is_ok() => save_slot_value(Some(value)),
        Ok(_) => {
            log::warn!(
                "[functor-lang] Persistence.load(\"{slot}\"): the saved data is not \
representable — answering Option.None"
            );
            save_slot_value(None)
        }
        Err(error) => {
            log::warn!(
                "[functor-lang] Persistence.load(\"{slot}\"): the saved data could not be \
read ({error}) — answering Option.None"
            );
            save_slot_value(None)
        }
    }
}

/// The tagger-facing record for a raycast result. Always the full shape —
/// `hit: Bool` discriminates — so field access typechecks on both arms.
pub fn ray_result_value(hit: Option<physics::RayHit>) -> EffectValue {
    let (h, p, n, d, tag, triangle) = match hit {
        Some(hit) => (true, hit.position, hit.normal, hit.distance, hit.tag, hit.triangle),
        None => (false, [0.0; 3], [0.0; 3], 0.0, String::new(), None),
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
        (
            "triangle".to_string(),
            EffectValue::Number(triangle.map_or(-1.0, f64::from)),
        ),
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

/// A serializable key for an [`EffectValue::Map`]. It mirrors Functor Lang's
/// deliberately bounded scalar key domain.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EffectMapKey {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl EffectMapKey {
    fn to_functor_lang(&self) -> Result<functor_lang::value::MapKey, String> {
        match self {
            EffectMapKey::Bool(value) => Ok(functor_lang::value::MapKey::Bool(*value)),
            // JSON refuses NaN/infinity, but fake/replay values are also
            // publicly constructible in memory. Validate that external seam
            // instead of letting MapKey::compare panic or admitting infinity.
            EffectMapKey::Number(value) if value.is_finite() => Ok(
                functor_lang::value::MapKey::Number(if *value == 0.0 { 0.0 } else { *value }),
            ),
            EffectMapKey::Number(value) => Err(format!(
                "effect map keys must be finite; got {value} (NaN/Infinity cannot be map keys)"
            )),
            EffectMapKey::Text(value) => Ok(functor_lang::value::MapKey::String(Rc::from(
                value.as_str(),
            ))),
        }
    }
}

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
    /// Canonically key-sorted immutable map data.
    Map(Vec<(EffectMapKey, EffectValue)>),
    /// Structural tuple (at least two elements), mirroring [`Value::Tuple`] —
    /// so an `Effect.sendMsg` payload carrying a tuple field roundtrips as a
    /// tuple, not a list.
    Tuple(Vec<EffectValue>),
    /// Field order is construction order (deterministic, like Functor Lang records).
    Record(Vec<(String, EffectValue)>),
    /// A variant value: a (canonical) constructor name and its positional
    /// args — how the host hands a game a prelude-declared ADT like
    /// `Net.Connected(id)` (see the built-in `Net` module).
    Variant(String, Vec<EffectValue>),
}

impl EffectValue {
    /// The Functor Lang value handed to the effect's tagger. External
    /// fake/replay map keys are validated here; malformed structured data is
    /// an error rather than a host panic.
    pub fn to_functor_lang(&self) -> Result<Value, String> {
        Ok(match self {
            EffectValue::Number(n) => Value::Number(*n),
            EffectValue::Bool(b) => Value::Bool(*b),
            EffectValue::Text(s) => Value::String(Rc::from(s.as_str())),
            EffectValue::List(items) => Value::List(Rc::new(
                items
                    .iter()
                    .map(EffectValue::to_functor_lang)
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            EffectValue::Map(entries) => {
                let sorted = entries
                    .iter()
                    .map(|(key, value)| Ok((key.to_functor_lang()?, value.to_functor_lang()?)))
                    .collect::<Result<Vec<_>, String>>()?;
                // Replay/network data is an external seam, so restore the
                // same canonical order and last-write-wins rule as
                // Map.fromList instead of trusting serialized entry order.
                let (canonical, _) = functor_lang::value::canonicalize_map_entries(sorted);
                Value::Map(functor_lang::pmap::PMap::from_sorted(canonical).0)
            }
            EffectValue::Tuple(items) => Value::Tuple(Rc::new(
                items
                    .iter()
                    .map(EffectValue::to_functor_lang)
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            EffectValue::Record(fields) => Value::Record(Rc::new(
                fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), v.to_functor_lang()?)))
                    .collect::<Result<Vec<_>, String>>()?,
            )),
            EffectValue::Variant(ctor, args) => Value::Variant {
                ctor: Rc::from(ctor.as_str()),
                args: Rc::new(
                    args.iter()
                        .map(EffectValue::to_functor_lang)
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            },
        })
    }
}

/// Depth bound for [`value_to_json`], measured in **emitted JSON
/// containers**, not Functor values — a tuple or variant emits two nested
/// containers (the sigil object plus its array), so charging per value would
/// overshoot the real JSON depth 2×. Value nesting is user-controlled (see
/// the `Value` `Display` doc — a fold wrapping `[acc]` per step builds depth
/// past any recursion limit), so unbounded depth must be refused somewhere —
/// and for JSON output the bound is a property of the FORMAT, not this
/// walker: `serde_json::Value` recurses in its own `Serialize` and `Drop`
/// impls, and serde_json's default *parse* recursion limit on the consuming
/// side is 128 containers, so deeper output would overflow producing or be
/// rejected consuming. 120 leaves headroom for the wrapping `RuntimeState`
/// object and the truncation marker itself. Past the bound a node becomes
/// `{"$truncated": "max depth"}` — visible, not silently dropped; the
/// `model` Display text on the same response still shows the full value.
const VALUE_JSON_MAX_DEPTH: usize = 120;

/// A total, LOSSY JSON view of a [`Value`], for observation surfaces (the
/// debug server's `GET /state` `model`, protocol v4). Unlike
/// [`effect_value_from_value`] it never fails: plain data maps structurally
/// (numbers, strings, bools; lists as arrays; maps as `$map` entry arrays;
/// records as objects — field order is the serde_json map's, not construction
/// order), and everything
/// else becomes a sigil-keyed object no source-authored record field can
/// collide with (`$` is not a Functor Lang identifier character; a record key
/// arriving off the network via a typed message CAN carry `$`, so treat
/// sigils as a strong convention, not a proof):
///
/// - maps: `{"$map": [[key, value], ...]}` (canonical key order)
/// - tuples: `{"$tuple": [...]}` (so they stay distinct from lists)
/// - variants: `{"$ctor": "Some", "args": [...]}`
/// - callables: `{"$fn": "<fn(dt)>"}` — the `Display` form
/// - host values: `{"$host": "SceneNode"}` — the `type_name`
/// - non-finite numbers: `{"$number": "NaN" | "Infinity" | "-Infinity"}`
///   (JSON numbers can't carry them; the JS spellings, not Rust's `inf`)
///
/// A one-way observation format: there is deliberately no parser back.
pub fn value_to_json(value: &Value) -> serde_json::Value {
    value_to_json_at(value, 0)
}

/// `depth` is the JSON-container depth this node is emitted at: lists and
/// records place children one container deeper, tuples and variants two (the
/// sigil object wrapping the args array).
fn value_to_json_at(value: &Value, depth: usize) -> serde_json::Value {
    use serde_json::{json, Value as Json};
    if depth > VALUE_JSON_MAX_DEPTH {
        return json!({ "$truncated": "max depth" });
    }
    let items_json = |items: &[Value], charge: usize| -> Vec<Json> {
        items
            .iter()
            .map(|v| value_to_json_at(v, depth + charge))
            .collect()
    };
    match value {
        Value::Number(n) if n.is_finite() => json!(n),
        Value::Number(n) if n.is_nan() => json!({ "$number": "NaN" }),
        Value::Number(n) => {
            json!({ "$number": if *n > 0.0 { "Infinity" } else { "-Infinity" } })
        }
        Value::String(s) => json!(s.as_ref()),
        Value::Bool(b) => json!(b),
        Value::List(items) => Json::Array(items_json(items, 1)),
        Value::Map(entries) => json!({
            "$map": entries
                .iter()
                .map(|(key, value)| {
                    Json::Array(vec![
                        value_to_json_at(&key.to_value(), depth + 3),
                        value_to_json_at(value, depth + 3),
                    ])
                })
                .collect::<Vec<_>>()
        }),
        Value::Tuple(items) => json!({ "$tuple": items_json(items, 2) }),
        Value::Record(fields) => Json::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json_at(v, depth + 1)))
                .collect(),
        ),
        Value::Variant { ctor, args } => {
            json!({ "$ctor": ctor.as_ref(), "args": items_json(args, 2) })
        }
        Value::Ctor { .. }
        | Value::Closure(_)
        | Value::Partial(_)
        | Value::Builtin(_)
        | Value::HostFn(_) => json!({ "$fn": value.to_string() }),
        Value::HostData(host) => json!({ "$host": host.type_name() }),
    }
}

/// The inverse of [`EffectValue::to_functor_lang`]: the plain-data subset of
/// [`Value`], for payloads the GAME hands the host (`Effect.sendMsg`). A
/// non-data value — a closure, an unapplied constructor, a host handle — is a
/// teaching error naming what was found: only what can cross the wire (and
/// land in the effect log) is accepted.
pub fn effect_value_from_value(value: &Value) -> Result<EffectValue, String> {
    match value {
        // JSON cannot carry NaN/Infinity — serde_json would write `null`,
        // which the PEER then fails to decode (an asymmetric, silent loss).
        // Refuse at the construction site instead, like a closure.
        Value::Number(n) if !n.is_finite() => Err(format!(
            "not plain data: a non-finite number ({n}) — the wire cannot carry NaN/Infinity"
        )),
        Value::Number(n) => Ok(EffectValue::Number(*n)),
        Value::Bool(b) => Ok(EffectValue::Bool(*b)),
        Value::String(s) => Ok(EffectValue::Text(s.to_string())),
        Value::List(items) => Ok(EffectValue::List(
            items.iter().map(effect_value_from_value).collect::<Result<_, _>>()?,
        )),
        Value::Map(entries) => Ok(EffectValue::Map(
            entries
                .iter()
                .map(|(key, value)| {
                    let key = match key {
                        functor_lang::value::MapKey::Bool(value) => EffectMapKey::Bool(*value),
                        // No finiteness check needed here (unlike a non-finite
                        // VALUE above): the interpreter already refuses a
                        // NaN/Infinity map key at insertion, so one cannot
                        // reach this walker.
                        functor_lang::value::MapKey::Number(value) => EffectMapKey::Number(*value),
                        functor_lang::value::MapKey::String(value) => {
                            EffectMapKey::Text(value.to_string())
                        }
                    };
                    Ok((key, effect_value_from_value(value)?))
                })
                .collect::<Result<_, String>>()?,
        )),
        Value::Tuple(items) => Ok(EffectValue::Tuple(
            items.iter().map(effect_value_from_value).collect::<Result<_, _>>()?,
        )),
        Value::Record(fields) => Ok(EffectValue::Record(
            fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), effect_value_from_value(v)?)))
                .collect::<Result<_, String>>()?,
        )),
        Value::Variant { ctor, args } => Ok(EffectValue::Variant(
            ctor.to_string(),
            args.iter().map(effect_value_from_value).collect::<Result<_, _>>()?,
        )),
        other => Err(format!(
            "not plain data: {} — a message must be numbers, strings, bools, \
lists, maps, tuples, records, and variants of those (no functions, no host values)",
            value_kind(other)
        )),
    }
}

/// A short human name for a [`Value`]'s kind, for teaching errors.
fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Bool(_) => "a bool",
        Value::List(_) => "a list",
        Value::Map(_) => "a map",
        Value::Tuple(_) => "a tuple",
        Value::Record(_) => "a record",
        Value::Variant { .. } => "a variant",
        Value::Ctor { .. } => "an unapplied constructor",
        Value::Closure(_) | Value::Partial(_) | Value::Builtin(_) | Value::HostFn(_) => {
            "a function"
        }
        Value::HostData(_) => "an opaque host value",
    }
}

/// The wire framing for `Effect.sendMsg` payloads: a control-character prefix
/// (never produced by ordinary game text) followed by the payload's
/// [`EffectValue`] as canonical JSON. `Net.Message` text without the prefix
/// passes through untouched, so typed and plain-text traffic share a
/// connection.
pub const TYPED_MSG_PREFIX: &str = "\u{1}fun:";

/// Frame a payload for the wire (see [`TYPED_MSG_PREFIX`]).
pub fn encode_typed_msg(payload: &EffectValue) -> String {
    let json = serde_json::to_string(payload)
        .expect("EffectValue is a closed plain-data enum; serialization cannot fail");
    format!("{TYPED_MSG_PREFIX}{json}")
}

/// Decode a wire text IF it is a typed-message frame: `None` for plain text,
/// `Some(Err)` for a frame that does not parse (version skew, corruption).
pub fn decode_typed_msg(text: &str) -> Option<Result<EffectValue, String>> {
    let json = text.strip_prefix(TYPED_MSG_PREFIX)?;
    Some(serde_json::from_str(json).map_err(|e| format!("malformed typed message: {e}")))
}

impl From<f64> for EffectValue {
    fn from(n: f64) -> EffectValue {
        EffectValue::Number(n)
    }
}

/// The real world: system clock, xorshift PRNG seeded from it (no new
/// dependency; game-quality randomness, not crypto).
pub struct RealEffects {
    rng_state: u64,
}

/// Epoch seconds from the platform clock. `std::time::SystemTime` is
/// unimplemented on `wasm32-unknown-unknown` (it panics), so the browser
/// build asks `Date.now()` instead. `pub(crate)`: the embedded producer
/// times reloads with it (its only portable ms clock).
pub(crate) fn epoch_seconds() -> f64 {
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

/// Milliseconds on the shared portable clock (see [`epoch_seconds`]). The
/// embedded producer times reloads/rewinds/seeks with it — one portable clock
/// instead of a platform-specific `Date::now()`.
pub(crate) fn now_ms() -> f64 {
    epoch_seconds() * 1000.0
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
    fn save(&mut self, slot: &str, payload: &EffectValue) -> Result<(), String> {
        let text = serde_json::to_string(payload)
            .expect("EffectValue is a closed plain-data enum; serialization cannot fail");
        crate::storage::write_slot(slot, &text)
    }
    fn load(&mut self, slot: &str) -> EffectValue {
        read_saved_slot(slot)
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
    /// The in-memory save store, newest last: every `Persistence.save` this runner
    /// performed, appended after anything [`FakeEffects::with_saves`] seeded
    /// (seeding reads as "the game had already saved this"). One vector, so
    /// it is both the store and the journal of what a game wrote — a test
    /// asserts persistence with no disk and no browser. `Persistence.load` answers
    /// with the LAST value written to that slot (`Option.None` when the slot
    /// was never written), which is exactly the real store's rule.
    pub saves: Vec<(String, EffectValue)>,
}

impl FakeEffects {
    pub fn new(now: f64, randoms: Vec<f64>) -> FakeEffects {
        FakeEffects {
            now,
            randoms,
            next: 0,
            ray_hits: Vec::new(),
            next_ray: 0,
            saves: Vec::new(),
        }
    }

    /// Pre-seed the in-memory save store — a game booting against an
    /// EXISTING save, with no disk involved.
    pub fn with_saves(mut self, saves: Vec<(String, EffectValue)>) -> FakeEffects {
        self.saves = saves;
        self
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
    fn save(&mut self, slot: &str, payload: &EffectValue) -> Result<(), String> {
        self.saves.push((slot.to_string(), payload.clone()));
        Ok(())
    }
    fn load(&mut self, slot: &str) -> EffectValue {
        save_slot_value(
            self.saves
                .iter()
                .rev()
                .find(|(key, _)| key == slot)
                .map(|(_, value)| value.clone()),
        )
    }
}

/// The fork's runner (branch replay, ghost, counterfactual; design-systems
/// §2.4): each `now` / `random` / `Persistence.load` hands back the recorded
/// answer of the frame being replayed, and each outbound intent is checked
/// against the recorded one — positionally, by kind and args digest. A
/// mismatch is a divergence. Once answers stop lining up (a missing, extra or
/// different call), and on frames with no log (a projection past the recorded
/// window, a scripted ghost), answers are deterministic stand-ins — `now` and
/// `random` 0, `load` the replay's own saves or `Option.None` — never the live
/// world. Raycasts are asks, recomputed against the ACTIVE world (the fork's
/// throwaway world). Saves go to the replay's own overlay, never the player's
/// store.
pub struct ReplayEffects {
    /// The current frame's unconsumed recorded ops; `None` = unrecorded.
    ops: Option<std::collections::VecDeque<Entry>>,
    /// The stand-in answers and the save overlay.
    stand_in: FakeEffects,
    /// Frames begun so far.
    frames: usize,
    divergence: Option<String>,
    /// The frame (0-based) where answers stopped lining up with the log.
    lost_at: Option<usize>,
}

impl ReplayEffects {
    /// No recorded frame until [`EffectRunner::begin_frame`] gives one.
    #[allow(clippy::new_without_default)]
    pub fn new() -> ReplayEffects {
        ReplayEffects {
            ops: None,
            stand_in: FakeEffects::new(0.0, vec![0.0]),
            frames: 0,
            divergence: None,
            lost_at: None,
        }
    }

    /// Replay one recorded frame (the fork feeds frames itself).
    #[cfg(test)]
    pub fn from_log(recorded: &[Entry]) -> ReplayEffects {
        let mut replay = ReplayEffects::new();
        replay.begin_frame(Some(recorded));
        replay
    }

    /// The first divergence, after checking the current frame was used up.
    pub fn finish(&mut self) -> Option<&str> {
        self.end_frame();
        self.divergence.as_deref()
    }

    /// The frame where answers stopped lining up, if they did.
    pub fn lost_at(&self) -> Option<usize> {
        self.lost_at
    }

    fn diverge(&mut self, message: String) {
        self.divergence.get_or_insert(message);
    }

    /// Diverge and stop consuming the log: its answers no longer line up.
    fn lose(&mut self, message: String) {
        self.diverge(message);
        self.lost_at.get_or_insert(self.frames.saturating_sub(1));
        self.ops = None;
    }

    fn end_frame(&mut self) {
        let left = self.ops.as_ref().and_then(|ops| {
            ops.front()
                .and_then(Entry::op)
                .map(|(op, kind, _)| (ops.len(), op, kind))
        });
        if let Some((n, op, kind)) = left {
            self.lose(format!(
                "replay diverged at {op:?}: the program never asked for the recorded {kind} \
({n} recorded op(s) unused)"
            ));
        }
    }

    /// Consume the next recorded op if the frame is recorded: its answer
    /// value, or `None` for a stand-in (unrecorded, diverged, or an intent).
    fn take(&mut self, kind: &'static str, args: u64) -> Option<EffectValue> {
        let ops = self.ops.as_mut()?;
        let Some(entry) = ops.pop_front() else {
            self.lose(format!(
                "replay diverged: the program asked for {kind} past its frame's recorded ops"
            ));
            return None;
        };
        let (op, recorded, recorded_args) = entry.op().expect("only ops are queued");
        if recorded != kind {
            self.lose(format!(
                "replay diverged at {op:?}: recorded {recorded}, program asked for {kind}"
            ));
            return None;
        }
        let message = format!("replay diverged at {op:?}: {kind} with different arguments");
        match entry {
            Entry::Answer { value, .. } if recorded_args == args => Some(value),
            Entry::Answer { .. } => {
                self.lose(message);
                None
            }
            // An intent carries no answer, so later answers still line up.
            _ => {
                if recorded_args != args {
                    self.diverge(message);
                }
                None
            }
        }
    }

    fn take_number(&mut self, kind: &'static str) -> Option<f64> {
        match self.take(kind, args_digest(&())) {
            Some(EffectValue::Number(n)) => Some(n),
            _ => None,
        }
    }
}

impl EffectRunner for ReplayEffects {
    fn now(&mut self) -> f64 {
        self.take_number("now")
            .unwrap_or_else(|| self.stand_in.now())
    }
    fn random(&mut self) -> f64 {
        self.take_number("random")
            .unwrap_or_else(|| self.stand_in.random())
    }
    fn raycast(&mut self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> EffectValue {
        active_world_raycast(origin, dir, max_dist)
    }
    fn save(&mut self, slot: &str, payload: &EffectValue) -> Result<(), String> {
        self.stand_in.save(slot, payload)
    }
    fn load(&mut self, slot: &str) -> EffectValue {
        self.take("storage.load", args_digest(&slot))
            .unwrap_or_else(|| self.stand_in.load(slot))
    }
    fn issued(&mut self, kind: &'static str, args: u64) {
        self.take(kind, args);
    }
    fn begin_frame(&mut self, recorded: Option<&[Entry]>) {
        self.end_frame();
        self.frames += 1;
        if self.lost_at.is_none() {
            self.ops = recorded.map(|entries| {
                entries
                    .iter()
                    .filter(|entry| entry.op().is_some())
                    .cloned()
                    .collect()
            });
        }
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
    // A locator, a render-target id, or shared pixels: plain data.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
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
/// door). Since the flag day (B.6) the asset consumers accept ONLY these
/// values and check the KIND, so a wrong-kind asset is a teaching error at
/// the call instead of a silent fallback at draw.
#[derive(Clone)]
struct FunctorLangAsset {
    kind: AssetKind,
    path: String,
    /// `Asset.whilePending` placeholder locators, tried in order while
    /// `path` is still loading (empty for a plain locator). Same kind as
    /// `path` by construction.
    while_pending: Vec<String>,
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

/// An [`HtmlNode`] as an opaque Functor Lang value — what the optional
/// `webview(model)` entry point returns (`Html.div` / `Html.text` / …).
/// The shells render it as HTML/CSS: blitz-composited natively, a real DOM
/// overlay on wasm.
pub struct FunctorLangHtmlNode(pub HtmlNode);

impl HostData for FunctorLangHtmlNode {
    fn type_name(&self) -> &'static str {
        "Html"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// One attribute of an `Html.*` element, before it is folded into the node.
/// Interactive attributes registered their handler at construction time (the
/// `Ui.button` shape) and carry only the slot.
#[derive(Clone)]
pub enum HtmlAttr {
    /// A plain `name="value"` pair (`Attr.class` / `Attr.style` / `Attr.attr`).
    Pair(String, String),
    /// `Attr.onClick(msg)` — msg registered in the frame's handler table.
    Click(u32),
    /// `Attr.onInput(tagger)` — tagger registered in the frame's handler table.
    Input(u32),
}

/// An [`HtmlAttr`] as an opaque Functor Lang value — `Attr.class(…)` / `Attr.onClick(msg)`.
pub struct FunctorLangHtmlAttr(pub HtmlAttr);

impl HostData for FunctorLangHtmlAttr {
    fn type_name(&self) -> &'static str {
        "Attr"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// One or more formatted CSS declarations (`width: 300px`) as an opaque
/// Functor Lang value — `Style.widthPx(300.0)`. `Attr.styles` folds a list of
/// these into ONE `style="…"` attribute (the Angle rule applied to inline
/// styles); values are formatted at construction, so a Style is plain text
/// by the time it reaches the fold.
pub struct FunctorLangStyle(pub String);

impl HostData for FunctorLangStyle {
    fn type_name(&self) -> &'static str {
        "Style"
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

/// A `Physics.memo` result: typed `body` in Functor Lang, but only
/// `Physics.scene` accepts it.
pub struct FunctorLangBodyGroup(pub physics::BodyGroup);

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
    // Collision geometry, no handles or live state (`physics::Shape`).
    fn is_reload_safe_snapshot(&self) -> bool {
        true
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

impl HostData for FunctorLangBodyGroup {
    fn type_name(&self) -> &'static str {
        "Physics.memo group"
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

impl HostData for FunctorLangInstance {
    fn type_name(&self) -> &'static str {
        "Instance"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // Plain channel data — a model holding Instance values stays hot-reload
    // and time-travel safe.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

impl HostData for FunctorLangTerrain {
    fn type_name(&self) -> &'static str {
        "Terrain"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

impl HostData for FunctorLangCamera {
    fn type_name(&self) -> &'static str {
        "Camera3D"
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

/// Extract the [`HtmlNode`] from a Functor Lang value (an `Html.*` result), for the
/// shells' webview overlay — the `webview` hook's [`view_value`].
pub fn html_node_value(value: &Value) -> Option<&HtmlNode> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<FunctorLangHtmlNode>()
            .map(|n| &n.0),
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
        register_vec3(&mut reg);
        register_ui_anchors(&mut reg);
        register_terrain(&mut reg);
        solid::register(&mut reg);
        bytes::register(&mut reg);
        path::register(&mut reg);
        mesh::register(&mut reg);
        cells::register(&mut reg);
        model::register(&mut reg);
        register_scene(&mut reg);
        register_instance(&mut reg);
        sprite::register(&mut reg);
        register_camera(&mut reg);
        register_light(&mut reg);
        register_frame(&mut reg);
        register_render_resources(&mut reg);
        texture::register(&mut reg);
        lightmap::register(&mut reg);
        register_physics(&mut reg);
        register_assets(&mut reg);
        register_anim(&mut reg);
        register_effects(&mut reg);
        register_persistence(&mut reg);
        register_subs(&mut reg);
        register_ui_widgets(&mut reg);
        register_html(&mut reg);
        register_style(&mut reg);
        register_audio(&mut reg);
        settings::register(&mut reg);
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
    // Angle arithmetic — what `unit deg (+)` / `(-)` / `(*)` in `angle.funi`
    // resolve to, so `90deg + 45deg` is exactly `Angle.add(90deg, 45deg)`.
    reg.fn2(
        "Angle.add",
        "Angle.add(a, b)",
        |a: FunctorLangAngle, b: FunctorLangAngle| {
            FunctorLangAngle(Angle::from_radians(a.0.radians() + b.0.radians()))
        },
    );
    reg.fn2(
        "Angle.sub",
        "Angle.sub(a, b)",
        |a: FunctorLangAngle, b: FunctorLangAngle| {
            FunctorLangAngle(Angle::from_radians(a.0.radians() - b.0.radians()))
        },
    );
    reg.fn2(
        "Angle.scale",
        "Angle.scale(angle, factor)",
        |a: FunctorLangAngle, factor: f64| {
            FunctorLangAngle(Angle::from_radians(a.0.radians() * factor as f32))
        },
    );
    // Angle comparison — what `unit deg (==)` / `(<)` resolve to. An angle
    // is an opaque host value with no structure, so this is the ONLY way one
    // can be compared; underneath it is float equality/ordering on radians
    // (documented as such in `angle.funi`).
    reg.fn2(
        "Angle.equals",
        "Angle.equals(a, b)",
        |a: FunctorLangAngle, b: FunctorLangAngle| Value::Bool(a.0.radians() == b.0.radians()),
    );
    reg.fn2(
        "Angle.less",
        "Angle.less(a, b)",
        |a: FunctorLangAngle, b: FunctorLangAngle| Value::Bool(a.0.radians() < b.0.radians()),
    );
    reg.fn1("Time.seconds", "Time.seconds(n)", FunctorLangDuration);
    // Duration comparison — `unit s (==)` / `(<)` in `time.funi`. Durations
    // are stored in seconds, so every suffix compares against every other.
    reg.fn2(
        "Time.equals",
        "Time.equals(a, b)",
        |a: FunctorLangDuration, b: FunctorLangDuration| Value::Bool(a.0 == b.0),
    );
    reg.fn2(
        "Time.less",
        "Time.less(a, b)",
        |a: FunctorLangDuration, b: FunctorLangDuration| Value::Bool(a.0 < b.0),
    );
    // Duration arithmetic — `unit s (+)` / `(-)` / `(*)` in `time.funi`.
    // Durations are stored in seconds, so every suffix shares these.
    reg.fn2(
        "Time.add",
        "Time.add(a, b)",
        |a: FunctorLangDuration, b: FunctorLangDuration| FunctorLangDuration(a.0 + b.0),
    );
    reg.fn2(
        "Time.sub",
        "Time.sub(a, b)",
        |a: FunctorLangDuration, b: FunctorLangDuration| FunctorLangDuration(a.0 - b.0),
    );
    reg.fn2(
        "Time.scale",
        "Time.scale(duration, factor)",
        |a: FunctorLangDuration, factor: f64| FunctorLangDuration(a.0 * factor),
    );
    reg.fn1("Time.millis", "Time.millis(n)", |n: f64| {
        FunctorLangDuration(n / 1000.0)
    });
    reg.fn1("Time.micros", "Time.micros(n)", |n: f64| {
        FunctorLangDuration(n / 1_000_000.0)
    });
    reg.fn1("Time.minutes", "Time.minutes(n)", |n: f64| {
        FunctorLangDuration(n * 60.0)
    });
    reg.fn1("Time.hours", "Time.hours(n)", |n: f64| {
        FunctorLangDuration(n * 3600.0)
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
    FunctorLangTerrain,
    FunctorLangScene,
    FunctorLangInstance,
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
    FunctorLangHtmlNode,
    FunctorLangHtmlAttr,
    FunctorLangStyle,
    FunctorLangAudioSource,
    FunctorLangAudioScene,
);

/// `Vec3` component accessors and arithmetic (vec3.funi).
///
/// The brand stays opaque — these let a game keep vectors as `Vec3` all the
/// way through instead of unpacking to a `{x, y, z}` record, doing the math
/// itself, and rebuilding with `Vec3.make` at every prelude boundary. (Every
/// game-jam entry that needed 3D math wrote that module by hand; four wrote
/// it in parallel and one shipped a whole `v3.fun`.)
///
/// **Argument order is thread-last**, like the rest of the prelude
/// (`Scene.translate(v, scene)`): the subject is LAST, so `Vec3.sub(b, a)`
/// computes `a - b` and `v |> Vec3.sub(origin)` reads "v minus origin".
///
/// Every intermediate is computed in `f64` and the result is finite-checked
/// before it is re-branded ([`vec3_finite`]). `Vec3.make` can only ever build
/// a finite vector (its components go through the finite-checked `f64`
/// extractor), and the arithmetic below preserves that invariant — so a
/// `Vec3` reaching the renderer is ALWAYS componentwise finite, and a NaN can
/// never reach a transform (where it would blank the scene and serialize as
/// `null` in the `Frame` wire form).
fn register_vec3(reg: &mut crate::host_registry::Registry) {
    reg.fn1("Vec3.x", "Vec3.x(v)", |v: FunctorLangVec3| Value::Number(v.0 .0 as f64));
    reg.fn1("Vec3.y", "Vec3.y(v)", |v: FunctorLangVec3| Value::Number(v.0 .1 as f64));
    reg.fn1("Vec3.z", "Vec3.z(v)", |v: FunctorLangVec3| Value::Number(v.0 .2 as f64));

    reg.fn2(
        "Vec3.add",
        "Vec3.add(b, a) — a + b; a |> Vec3.add(b)",
        |b: FunctorLangVec3, a: FunctorLangVec3| {
            let (a, b) = (widen(a), widen(b));
            vec3_finite("Vec3.add", (a.0 + b.0, a.1 + b.1, a.2 + b.2))
        },
    );
    reg.fn2(
        "Vec3.sub",
        "Vec3.sub(b, a) — a - b; v |> Vec3.sub(origin)",
        |b: FunctorLangVec3, a: FunctorLangVec3| {
            let (a, b) = (widen(a), widen(b));
            vec3_finite("Vec3.sub", (a.0 - b.0, a.1 - b.1, a.2 - b.2))
        },
    );
    reg.fn2(
        "Vec3.scale",
        "Vec3.scale(k, v) — v |> Vec3.scale(2.0)",
        |k: f64, v: FunctorLangVec3| {
            let v = widen(v);
            vec3_finite("Vec3.scale", (v.0 * k, v.1 * k, v.2 * k))
        },
    );
    reg.fn2(
        "Vec3.dot",
        "Vec3.dot(b, a) — a · b",
        |b: FunctorLangVec3, a: FunctorLangVec3| Value::Number(vec3_dot(widen(a), widen(b))),
    );
    reg.fn2(
        "Vec3.cross",
        "Vec3.cross(b, a) — a × b; a |> Vec3.cross(b)",
        |b: FunctorLangVec3, a: FunctorLangVec3| {
            let ((ax, ay, az), (bx, by, bz)) = (widen(a), widen(b));
            vec3_finite(
                "Vec3.cross",
                (ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx),
            )
        },
    );
    reg.fn1("Vec3.length", "Vec3.length(v)", |v: FunctorLangVec3| {
        Value::Number(vec3_length(widen(v)))
    });
    reg.fn1("Vec3.normalize", "Vec3.normalize(v)", |v: FunctorLangVec3| {
        let v = widen(v);
        let len = vec3_length(v);
        // `len` is finite for every representable input (see `vec3_length`),
        // so the only non-dividable case is a true zero.
        let unit = if len > 0.0 {
            (v.0 / len, v.1 / len, v.2 / len)
        } else {
            (0.0, 0.0, 0.0)
        };
        vec3_finite("Vec3.normalize", unit)
    });
    reg.fn2(
        "Vec3.distance",
        "Vec3.distance(b, a) — |a - b|",
        |b: FunctorLangVec3, a: FunctorLangVec3| {
            let (a, b) = (widen(a), widen(b));
            Value::Number(vec3_length((a.0 - b.0, a.1 - b.1, a.2 - b.2)))
        },
    );
    reg.fn3(
        "Vec3.lerp",
        "Vec3.lerp(target, t, from) — from |> Vec3.lerp(target, 0.5)",
        |target: FunctorLangVec3, t: f64, from: FunctorLangVec3| {
            let (f, g) = (widen(from), widen(target));
            // Endpoint-exact: `t == 0` yields `from` and `t == 1` yields
            // `target` bit-for-bit, which `from + (target - from) * t` does
            // not guarantee once `target - from` overflows.
            let mix = |f: f64, g: f64| {
                if t == 0.0 {
                    f
                } else if t == 1.0 {
                    g
                } else {
                    f * (1.0 - t) + g * t
                }
            };
            vec3_finite("Vec3.lerp", (mix(f.0, g.0), mix(f.1, g.1), mix(f.2, g.2)))
        },
    );
}

/// The brand's `f32` storage widened to `f64` for arithmetic. Widening is
/// exact, so no precision is invented; it only removes the overflow cliff
/// (squaring an `f32` component overflows well below the `f32` range, which
/// would make `Vec3.length` infinite for a length that is representable).
fn widen(v: FunctorLangVec3) -> (f64, f64, f64) {
    (v.0 .0 as f64, v.0 .1 as f64, v.0 .2 as f64)
}

fn vec3_dot(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    a.0 * b.0 + a.1 * b.1 + a.2 * b.2
}

/// Always finite for a componentwise-finite `f32` input: the largest possible
/// sum of squares is ~3·(3.4e38)² ≈ 3.5e77, comfortably inside `f64`.
fn vec3_length(v: (f64, f64, f64)) -> f64 {
    vec3_dot(v, v).sqrt()
}

/// Narrow an `f64` result back into the brand, refusing a component that is
/// not finite in `f32`.
///
/// This is the invariant that keeps NaN out of the renderer. Overflowing to
/// infinity is a real possibility from perfectly legal inputs — a divergent
/// integrator (`pos |> Vec3.add(vel |> Vec3.scale(dt))` with a too-stiff
/// spring) doubles every frame and crosses the `f32` range in seconds — and
/// an infinity that then reaches `Vec3.normalize` or a transform would
/// silently blank the scene. Failing loudly at the operation that overflowed
/// names the culprit instead.
fn vec3_finite(path: &str, v: (f64, f64, f64)) -> Result<FunctorLangVec3, String> {
    let (x, y, z) = (v.0 as f32, v.1 as f32, v.2 as f32);
    if x.is_finite() && y.is_finite() && z.is_finite() {
        Ok(FunctorLangVec3((x, y, z)))
    } else {
        Err(format!(
            "{path}: the result is not a finite vector ({x}, {y}, {z}) — a Vec3 component \
must stay within the 32-bit float range"
        ))
    }
}

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

/// Immutable terrain descriptions are shared by rendering and heightfield
/// physics. Dimensions and elevation stay attached to the heightmap locator,
/// so the two consumers cannot silently drift.
fn register_terrain(reg: &mut crate::host_registry::Registry) {
    const HEIGHTMAP: &str = "Terrain.heightmap(asset, width, depth, minHeight, maxHeight) — \
a texture Asset, positive dimensions, and maxHeight greater than minHeight";
    reg.fn5(
        "Terrain.heightmap",
        HEIGHTMAP,
        |asset: TextureAssetPath, width: f64, depth: f64, min_height: f64, max_height: f64| {
            let (width, depth, min_height, max_height) = (
                width as f32,
                depth as f32,
                min_height as f32,
                max_height as f32,
            );
            if !width.is_finite()
                || !depth.is_finite()
                || !min_height.is_finite()
                || !max_height.is_finite()
                || width <= 0.0
                || depth <= 0.0
                || max_height <= min_height
            {
                return Err(format!("usage: {HEIGHTMAP}"));
            }
            Ok(FunctorLangTerrain(TerrainDescription::heightmap(
                asset.path,
                asset.while_pending,
                width,
                depth,
                min_height,
                max_height,
            )))
        },
    );
    reg.fn2(
        "Terrain.maxPixelError",
        "Terrain.maxPixelError(pixels, terrain) — a positive finite pixel error",
        |pixels: f64, terrain: FunctorLangTerrain| {
            let pixels = pixels as f32;
            if !pixels.is_finite() || pixels <= 0.0 {
                return Err(
                    "Terrain.maxPixelError pixels must be a positive finite number".to_string(),
                );
            }
            Ok(FunctorLangTerrain(terrain.0.with_max_pixel_error(pixels)))
        },
    );
    reg.fn2(
        "Terrain.color",
        "Terrain.color(color, terrain)",
        |color: FunctorLangColor, terrain: FunctorLangTerrain| {
            let (r, g, b) = color.0;
            FunctorLangTerrain(terrain.0.with_color([r, g, b]))
        },
    );
    const TEXTURED: &str = "Terrain.textured(low, high, rock, snow, tileSize, terrain) — \
four texture Assets and a positive finite tile size in world units";
    reg.fn6(
        "Terrain.textured",
        TEXTURED,
        |low: TextureAssetPath,
         high: TextureAssetPath,
         rock: TextureAssetPath,
         snow: TextureAssetPath,
         tile_size: f64,
         terrain: FunctorLangTerrain| {
            let tile_size = tile_size as f32;
            if !tile_size.is_finite() || tile_size <= 0.0 {
                return Err(format!("usage: {TEXTURED}"));
            }
            let map = |asset: TextureAssetPath| crate::terrain::TerrainDetailTexture {
                locator: asset.path,
                while_pending: asset.while_pending,
            };
            Ok(FunctorLangTerrain(terrain.0.with_textures(
                crate::terrain::TerrainTextures {
                    low: map(low),
                    high: map(high),
                    rock: map(rock),
                    snow: map(snow),
                    tile_size,
                },
            )))
        },
    );
    reg.fn6(
        "Terrain.layered",
        "Terrain.layered(low, high, rock, snow, snowHeight, terrain)",
        |low: FunctorLangColor,
         high: FunctorLangColor,
         rock: FunctorLangColor,
         snow: FunctorLangColor,
         snow_height: f64,
         terrain: FunctorLangTerrain| {
            let snow_height = snow_height as f32;
            if !snow_height.is_finite() {
                return Err("Terrain.layered snowHeight must be finite".to_string());
            }
            let color = |FunctorLangColor((r, g, b))| [r, g, b];
            Ok(FunctorLangTerrain(terrain.0.with_layers(
                crate::terrain::TerrainLayers {
                    low: color(low),
                    high: color(high),
                    rock: color(rock),
                    snow: color(snow),
                    snow_height,
                },
            )))
        },
    );
    reg.fn5(
        "Terrain.grass",
        "Terrain.grass(spacing, distance, bladeHeight, color, terrain)",
        |spacing: f64,
         distance: f64,
         blade_height: f64,
         color: FunctorLangColor,
         terrain: FunctorLangTerrain| {
            let (spacing, distance, blade_height) = (
                spacing as f32,
                distance as f32,
                blade_height as f32,
            );
            if !spacing.is_finite()
                || !distance.is_finite()
                || !blade_height.is_finite()
                || spacing <= 0.0
                || distance <= 0.0
                || blade_height <= 0.0
            {
                return Err(
                    "Terrain.grass spacing, distance, and bladeHeight must be positive finite numbers"
                        .to_string(),
                );
            }
            let (r, g, b) = color.0;
            Ok(FunctorLangTerrain(
                terrain.0.with_grass(crate::terrain::TerrainGrass {
                    spacing,
                    distance,
                    blade_height,
                    color: [r, g, b],
                }),
            ))
        },
    );
}

/// The Scene vocabulary — geometry constructors, assets, composition,
/// materials, and transforms. Materials/transforms are scene-LAST
/// (subject-last), so they pipe: `Scene.cube() |> Scene.lit(color)`.
/// `Instance.t` builders (instance.funi): the per-copy placement channels
/// for `Scene.instanced`. Channels compose within themselves — rotations
/// multiply (the outer pipe applies last), scales and tints multiply
/// componentwise — and the copy's transform always applies
/// translate * rotate * scale. Subject-last, like every prelude builder.
fn register_instance(reg: &mut crate::host_registry::Registry) {
    use cgmath::{Quaternion, Rad, Rotation3};

    reg.fn1("Instance.at", "Instance.at(position)", |v: FunctorLangVec3| {
        let (x, y, z) = v.0;
        FunctorLangInstance(InstanceData::at([x, y, z]))
    });
    reg.fn2(
        "Instance.scale",
        "Instance.scale(factor, instance)",
        |factor: f64, instance: FunctorLangInstance| {
            FunctorLangInstance(instance.0.scaled(factor as f32))
        },
    );
    reg.fn4(
        "Instance.scaleXYZ",
        "Instance.scaleXYZ(x, y, z, instance)",
        |x: f64, y: f64, z: f64, instance: FunctorLangInstance| {
            FunctorLangInstance(instance.0.scaled_xyz(x as f32, y as f32, z as f32))
        },
    );
    reg.fn2(
        "Instance.rotateX",
        "Instance.rotateX(angle, instance)",
        |angle: FunctorLangAngle, instance: FunctorLangInstance| {
            let angle: Rad<f32> = angle.0.into();
            FunctorLangInstance(instance.0.rotated(Quaternion::from_angle_x(angle)))
        },
    );
    reg.fn2(
        "Instance.rotateY",
        "Instance.rotateY(angle, instance)",
        |angle: FunctorLangAngle, instance: FunctorLangInstance| {
            let angle: Rad<f32> = angle.0.into();
            FunctorLangInstance(instance.0.rotated(Quaternion::from_angle_y(angle)))
        },
    );
    reg.fn2(
        "Instance.rotateZ",
        "Instance.rotateZ(angle, instance)",
        |angle: FunctorLangAngle, instance: FunctorLangInstance| {
            let angle: Rad<f32> = angle.0.into();
            FunctorLangInstance(instance.0.rotated(Quaternion::from_angle_z(angle)))
        },
    );
    reg.fn5(
        "Instance.trs",
        "Instance.trs(position, rotationY, scaleX, scaleY, scaleZ)",
        |v: FunctorLangVec3, angle: FunctorLangAngle, sx: f64, sy: f64, sz: f64| {
            let (x, y, z) = v.0;
            let angle: Rad<f32> = angle.0.into();
            FunctorLangInstance(
                InstanceData::at([x, y, z])
                    .scaled_xyz(sx as f32, sy as f32, sz as f32)
                    .rotated(Quaternion::from_angle_y(angle)),
            )
        },
    );
    reg.fn2(
        "Instance.tint",
        "Instance.tint(color, instance)",
        |color: FunctorLangColor, instance: FunctorLangInstance| {
            let (r, g, b) = color.0;
            FunctorLangInstance(instance.0.tinted([r, g, b]))
        },
    );
}

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
    reg.fn0("Scene.billboard", "Scene.billboard()", || {
        FunctorLangScene(Scene3D::billboard())
    });
    reg.fn0("Scene.plane", "Scene.plane()", || FunctorLangScene(Scene3D::plane()));
    // A glTF model by file path (relative to the game dir), the Functor Lang
    // face of F#'s `Model.file |> Graphics.Scene3D.model`. Loading is the
    // shells' asset pipeline; a missing file logs an error and renders as
    // the empty fallback.
    const MODEL: &str = "Scene.model(asset) — a model Asset value: the generated \
manifest's Assets.<name>, or Asset.model(\"file.glb\")";
    reg.fn1("Scene.model", MODEL, |path: ModelPath| {
        FunctorLangScene(Scene3D::model(ModelDescription {
            handle: ModelHandle::File(path.path),
            overrides: Vec::new(),
            animation: None,
            while_pending: path.while_pending,
        }))
    });
    reg.fn1(
        "Scene.terrain",
        "Scene.terrain(terrain)",
        |terrain: FunctorLangTerrain| FunctorLangScene(Scene3D::terrain(terrain.0)),
    );
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
    // Translucency (scene-last, so it pipes). At alpha 1 this is the IDENTITY —
    // it returns the scene untouched rather than wrapping it — so a game that
    // never asks for translucency (or asks for none this frame) produces the
    // exact same scene value, and the renderer's transparent pass never arms.
    // Out of range is an error, matching `Sprite.fade`: silently clamping a
    // computed fade hides the bug that produced 1.4.
    reg.fn2(
        "Scene.opacity",
        "Scene.opacity(alpha, scene)",
        |alpha: f64, scene: FunctorLangScene| {
            if !(0.0..=1.0).contains(&alpha) {
                return Err(format!(
                    "Scene.opacity alpha must be between 0 and 1, got {alpha}"
                ));
            }
            // Narrow FIRST, then test for the identity: an alpha of
            // 0.999_999_999 narrows to exactly 1.0 at 32-bit precision, and
            // wrapping it would build an `Opacity` node whose alpha is 1 —
            // contradicting the variant's invariant and paying for a
            // transparent pass that changes no pixel.
            let alpha = alpha as f32;
            if alpha == 1.0 {
                return Ok(scene);
            }
            Ok(FunctorLangScene(Scene3D {
                obj: SceneObject::Opacity(alpha, vec![scene.0]),
                xform: Matrix4::from_scale(1.0),
            }))
        },
    );
    // Stamp a template subtree once per instance — semantically the group of
    // transformed, tinted copies; hardware-instanced when the renderer
    // recognizes the template. `Scene.opacity` INSIDE the template is
    // rejected: translucency defers whole subtrees by reference, which a
    // per-copy expansion cannot flow through — wrapping the instanced node
    // expresses the same thing and is supported.
    reg.fn2(
        "Scene.instanced",
        "Scene.instanced(instances, scene)",
        |instances: InstanceListArg, scene: FunctorLangScene| {
            if scene.0.has_opacity() {
                return Err(
                    "Scene.instanced: the template contains Scene.opacity — wrap the whole \
instanced node instead: scene |> Scene.instanced(instances) |> Scene.opacity(alpha)"
                        .to_string(),
                );
            }
            Ok(FunctorLangScene(Scene3D::instanced(scene.0, instances.0)))
        },
    );
    // A memoized pure builder (see `scene_memo`).
    reg.fn3(
        "Scene.memo",
        "Scene.memo(key, deps, builder) — e.g. Scene.memo(\"trees\", model.trees, drawTrees)",
        scene_memo,
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
    // Full orientation from a mapped pose: local +Z onto forward, +Y onto up
    // (the `Anim.lookAt` / `Scene.quad` front), as one rigid transform.
    reg.fn2(
        "Scene.atPose",
        "Scene.atPose(pose, scene) — a Camera3D.mappedPose record (position, forward, up); \
up must not be parallel to forward",
        |pose: FunctorLangMappedPose, scene: FunctorLangScene| {
            let xform = pose.0.to_matrix().ok_or_else(|| {
                "Scene.atPose: forward and up must be non-zero and not parallel".to_string()
            })?;
            Ok::<_, String>(transformed(scene, xform))
        },
    );
    // Structural equality, as an explicit call rather than `==` — `Scene.t` is
    // host-opaque, and the O(scene-size) walk should be visible at the call
    // site. It exists for inline `expect` tests over `draw` output.
    reg.fn2(
        "Scene.equals",
        "Scene.equals(a, b)",
        |a: FunctorLangScene, b: FunctorLangScene| Value::Bool(a.0 == b.0),
    );
}

fn register_camera(reg: &mut crate::host_registry::Registry) {
    // Eye + target (Y-up, right-handed), a fixed 45° fov.
    reg.fn2(
        "Camera3D.lookAt",
        "Camera3D.lookAt(eye, target) — Vec3 values (Vec3.make)",
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
        "Camera3D.firstPerson",
        "Camera3D.firstPerson(eye, yaw, pitch, fov) — a Vec3 eye and Angle values \
(Angle.degrees/Angle.radians)",
        |eye: FunctorLangVec3, yaw: FunctorLangAngle, pitch: FunctorLangAngle, fov: FunctorLangAngle| {
            let (ex, ey, ez) = eye.0;
            FunctorLangCamera(Camera::first_person([ex, ey, ez], yaw.0, pitch.0, fov.0))
        },
    );
    reg.fn2(
        "Camera3D.mapTrackedPose",
        "Camera3D.mapTrackedPose(camera, pose)",
        |camera: FunctorLangCamera, pose: Value| {
            let tracked = crate::tracking_pose_from_value(&pose)?;
            let mapped = camera
                .0
                .map_tracking_pose(crate::TrackingPose::IDENTITY, tracked)
                .ok_or_else(|| {
                    "Camera3D.mapTrackedPose: invalid pose or degenerate authored camera".to_string()
                })?;
            let point = |xyz: [f32; 3]| {
                Value::Record(Rc::new(vec![
                    ("x".to_string(), Value::Number(xyz[0] as f64)),
                    ("y".to_string(), Value::Number(xyz[1] as f64)),
                    ("z".to_string(), Value::Number(xyz[2] as f64)),
                ]))
            };
            Ok(Value::Record(Rc::new(vec![
                ("position".to_string(), point(mapped.position)),
                ("forward".to_string(), point(mapped.forward)),
                ("up".to_string(), point(mapped.up)),
            ])))
        },
    );
    // A forward-based camera that keeps roll: the mappedPose's forward is the
    // gaze, its up the screen's up (authored +Z forward / +Y up basis).
    reg.fn2(
        "Camera3D.fromPose",
        "Camera3D.fromPose(pose, fov) — a Camera3D.mappedPose record (position, forward, up) \
and an Angle fov; up must not be parallel to forward",
        |pose: Value, fov: FunctorLangAngle| {
            let pose = crate::mapped_pose_from_value(&pose)
                .map_err(|e| format!("Camera3D.fromPose: {e}"))?;
            Camera::from_pose(pose.position, pose.forward, pose.up, fov.0)
                .map(FunctorLangCamera)
                .ok_or_else(|| {
                    "Camera3D.fromPose: forward and up must be non-zero and not parallel"
                        .to_string()
                })
        },
    );
    reg.fn2(
        "Camera3D.toWorldRay",
        "Camera3D.toWorldRay(mouse, camera)",
        |mouse: FunctorLangMouse, camera: FunctorLangCamera| {
            crate::input::option_value(
                camera
                    .0
                    .to_world_ray(
                        mouse.x,
                        mouse.y,
                        mouse.surface_width,
                        mouse.surface_height,
                    )
                    .map(|ray| {
                        let vec3 = |[x, y, z]: [f32; 3]| {
                            Value::HostData(Rc::new(FunctorLangVec3((x, y, z))))
                        };
                        Value::Record(Rc::new(vec![
                            ("origin".to_string(), vec3(ray.origin)),
                            ("direction".to_string(), vec3(ray.direction)),
                        ]))
                    }),
            )
        },
    );
    reg.fn3(
        "Camera3D.clip",
        "Camera3D.clip(near, far, camera) — finite distances with 0 < near < far",
        |near: f64, far: f64, mut camera: FunctorLangCamera| {
            let (near, far) = (near as f32, far as f32);
            if !near.is_finite() || !far.is_finite() || near <= 0.0 || far <= near {
                return Err(
                    "usage: Camera3D.clip(near, far, camera) — finite distances with 0 < near < far"
                        .to_string(),
                );
            }
            camera.0.near = near;
            camera.0.far = far;
            Ok(camera)
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
    reg.fn1(
        "Light.baked",
        "Light.baked(light)",
        |light: FunctorLangLight| FunctorLangLight(light.0.baked()),
    );
}

/// A grid for `Frame.withSurfaceGrid`/`withFusedRoom`; `cell` must be positive.
fn surface_grid(
    call: &str,
    cell: f64,
    color: FunctorLangColor,
) -> Result<crate::surface_grid::SurfaceGrid, String> {
    if !(cell as f32 > 0.0) {
        return Err(format!("{call}: the cell size must be positive, got {cell}"));
    }
    let (r, g, b) = color.0;
    Ok(crate::surface_grid::SurfaceGrid {
        cell: cell as f32,
        color: [r, g, b],
    })
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
                ui_targets: vec![],
                fog: None,
                skybox: None,
                clear_color: None,
                sprite_layers: vec![],
                pure_2d: false,
                passthrough: false,
                stand_in_room: None,
                surface_grid: None,
                fused_room: None,
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
    reg.fn3(
        "Frame.withUiTarget",
        "Frame.withUiTarget(target, view, frame) — view is a Ui.* tree \
painted into the target each frame, before frame's main pass. Display-only: \
interactive widgets render but their handlers are inert",
        |target: FunctorLangRenderTarget, view: FunctorLangView, frame: FunctorLangFrame| {
            FunctorLangFrame(Frame::with_ui_target(frame.0, target.0, view.0))
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
    reg.fn1(
        "Frame.withPassthrough",
        "Frame.withPassthrough(frame)",
        |frame: FunctorLangFrame| FunctorLangFrame(Frame::with_passthrough(frame.0)),
    );
    reg.fn2(
        "Frame.withStandInRoom",
        "Frame.withStandInRoom(room, frame) — room is a Scene drawn with a \
passthrough frame wherever no camera feed is composited",
        |room: FunctorLangScene, frame: FunctorLangFrame| {
            FunctorLangFrame(Frame::with_stand_in_room(frame.0, room.0))
        },
    );
    reg.fn3(
        "Frame.withSurfaceGrid",
        "Frame.withSurfaceGrid(cell, color, frame) — cell is the grid spacing in \
world units (metres in passthrough), > 0",
        |cell: f64, color: FunctorLangColor, frame: FunctorLangFrame| {
            let grid = surface_grid("Frame.withSurfaceGrid", cell, color)?;
            Ok(FunctorLangFrame(Frame::with_surface_grid(frame.0, grid)))
        },
    );
    reg.fn3(
        "Frame.withFusedRoom",
        "Frame.withFusedRoom(cell, color, frame) — cell is the grid spacing in \
world units (metres in passthrough), > 0",
        |cell: f64, color: FunctorLangColor, frame: FunctorLangFrame| {
            let grid = surface_grid("Frame.withFusedRoom", cell, color)?;
            Ok(FunctorLangFrame(Frame::with_fused_room(frame.0, grid)))
        },
    );
    // See `Scene.equals` — the same explicit structural walk, one level up.
    reg.fn2(
        "Frame.equals",
        "Frame.equals(a, b)",
        |a: FunctorLangFrame, b: FunctorLangFrame| Value::Bool(a.0 == b.0),
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
///
/// Body rotations use the protocol's existing quaternion field. The prelude
/// composes and canonicalizes them here so the shells only see ordinary body
/// declarations and need no rotation-specific path.
fn rotate_physics_body(
    path: &str,
    axis: [f32; 3],
    angle: FunctorLangAngle,
    body: FunctorLangBody,
) -> Result<FunctorLangBody, String> {
    if matches!(&body.0.shape, physics::Shape::Heightfield { .. }) {
        return Err(format!(
            "{path}: heightfield bodies cannot be rotated — terrain rendering is \
translation-only; use Physics.at with an unrotated Scene.terrain"
        ));
    }
    fused_room_stays_put(path, &body)?;

    let angle: cgmath::Rad<f32> = angle.0.into();
    // One canonical turn keeps equivalent angles (90° and 450°) on the same
    // side of the quaternion double cover before multiplication.
    let radians =
        (angle.0 + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    let (sin, cos) = (radians * 0.5).sin_cos();
    let axis_rotation = [axis[0] * sin, axis[1] * sin, axis[2] * sin, cos];
    // Match Scene's subject-last transform semantics: the outer pipe modifier
    // applies last in world space, so compose q_axis * q_current.
    let rotation = canonical_quaternion(quaternion_product(axis_rotation, body.0.rotation));
    Ok(FunctorLangBody(body.0.facing(rotation)))
}

/// Hamilton product for `[x, y, z, w]` quaternions.
fn quaternion_product(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn canonical_quaternion(mut q: [f32; 4]) -> [f32; 4] {
    let length_squared = q.iter().map(|component| component * component).sum::<f32>();
    if !length_squared.is_finite() || length_squared <= f32::EPSILON {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let inverse_length = length_squared.sqrt().recip();
    for component in &mut q {
        *component *= inverse_length;
    }

    // A quaternion and its negation encode the same rotation. Pick one stable
    // hemisphere (then a lexicographic tie-breaker at exactly 180°) so hot
    // reload and debug-state comparisons do not churn between q and -q.
    let negate = q[3] < 0.0
        || (q[3] == 0.0
            && (q[0] < 0.0
                || (q[0] == 0.0 && (q[1] < 0.0 || (q[1] == 0.0 && q[2] < 0.0)))));
    if negate {
        for component in &mut q {
            *component = -*component;
        }
    }
    for component in &mut q {
        if *component == 0.0 {
            *component = 0.0;
        }
    }
    q
}

/// The fused room is where the room is: it cannot be moved or rotated.
fn fused_room_stays_put(path: &str, body: &FunctorLangBody) -> Result<(), String> {
    match &body.0.shape {
        physics::Shape::FusedRoom { .. } => Err(format!(
            "{path}: the fused room is already in world space — declare \
Physics.fusedRoom(tag) without moving or rotating it"
        )),
        _ => Ok(()),
    }
}

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
    // `Model.skinned`'s byte layout. Each call copies and validates the data,
    // so games build it once (a top-level `let`) and re-declare the value.
    reg.fn1(
        "Physics.trimesh",
        "Physics.trimesh({ positions, triangles })",
        |data: Value| {
            let field = |name: &str| -> Result<bytes::ByteView, String> {
                let Value::Record(fields) = &data else {
                    return Err(format!(
                        "Physics.trimesh: expected a {{ positions, triangles }} record, got {}",
                        data.kind_name()
                    ));
                };
                match fields.iter().find(|(field, _)| field == name) {
                    Some((_, Value::HostData(d))) => d
                        .as_any()
                        .downcast_ref::<FunctorLangBytes>()
                        .map(|b| b.0.clone()),
                    _ => None,
                }
                .ok_or_else(|| format!("Physics.trimesh: `{name}` must be Bytes"))
            };
            // Little-endian triples: f32 x, y, z per vertex; u32 indices.
            let triples = |name: &str| {
                field(name).and_then(|raw| {
                    bytes::entries::<[u8; 4], 3>(raw.as_slice(), 4, |b| b.try_into().unwrap())
                        .map_err(|e| format!("Physics.trimesh: {name}: {e}"))
                })
            };
            let positions = triples("positions")?
                .into_iter()
                .map(|v| v.map(f32::from_le_bytes))
                .collect();
            let triangles = triples("triangles")?
                .into_iter()
                .map(|t| t.map(u32::from_le_bytes))
                .collect();
            physics::TriMeshData::new(positions, triangles)
                .map(|mesh| {
                    FunctorLangShape(physics::Shape::TriMesh {
                        mesh: std::sync::Arc::new(mesh),
                    })
                })
                .map_err(|e| format!("Physics.trimesh: {e}"))
        },
    );
    reg.fn2(
        "Physics.heightfield",
        "Physics.heightfield(tag, terrain)",
        |tag: std::rc::Rc<str>, terrain: FunctorLangTerrain| {
            // This constructor remains plain immutable protocol data. The
            // physics adapter requests and hydrates the source whenever it
            // consumes a scene, including bodies hoisted into eager globals.
            FunctorLangBody(physics::Body::fixed(
                tag.to_string(),
                physics::Shape::Heightfield {
                    geometry: terrain.0.geometry(),
                    data: None,
                },
            ))
        },
    );
    reg.fn1(
        "Physics.fusedRoom",
        "Physics.fusedRoom(tag)",
        |tag: std::rc::Rc<str>| {
            // Empty here; the physics adapter resolves the shell's latest
            // fused room whenever it consumes a scene.
            FunctorLangBody(physics::Body::fixed(
                tag.to_string(),
                physics::Shape::FusedRoom {
                    room: Default::default(),
                },
            ))
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
        |tag: std::rc::Rc<str>, shape: FunctorLangShape| {
            if matches!(shape.0, physics::Shape::TriMesh { .. }) {
                return Err("Physics.dynamic: a trimesh is a surface with no volume or mass, \
so it can't be dynamic — use Physics.fixed or Physics.kinematic for it, and a box, sphere or \
capsule for the moving body"
                    .to_string());
            }
            Ok(body_ctor(physics::Body::dynamic)(tag, shape))
        },
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
    reg.fn3(
        "Physics.character",
        "Physics.character(tag, capsule, { stepHeight, maxSlope, snap, offset })",
        |tag: std::rc::Rc<str>, shape: FunctorLangShape, settings: Value| {
            const WHAT: &str = "Physics.character";
            if !matches!(shape.0, physics::Shape::Capsule { .. }) {
                return Err(format!("{WHAT}: the shape must be a Physics.capsule"));
            }
            let Value::Record(fields) = &settings else {
                return Err(format!(
                    "{WHAT}: expected a {{ stepHeight, maxSlope, snap, offset }} record, got {}",
                    settings.kind_name()
                ));
            };
            let get = |name: &str| fields.iter().find(|(f, _)| f == name).map(|(_, v)| v);
            let length = |name: &str| match get(name) {
                Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => Ok(*n as f32),
                _ => Err(format!("{WHAT}: `{name}` must be a non-negative number of metres")),
            };
            let max_slope = get("maxSlope")
                .ok_or_else(|| format!("{WHAT}: missing `maxSlope`"))
                .and_then(|v| {
                    angle_of(v, "Physics.character maxSlope", Span::new(0, 0)).map_err(|e| e.message)
                })?
                .radians();
            if !(max_slope > 0.0 && max_slope < std::f32::consts::FRAC_PI_2) {
                return Err(format!("{WHAT}: `maxSlope` must be between 0deg and 90deg"));
            }
            // Rapier's controller needs a gap to stay numerically stable.
            let offset = length("offset")?;
            if offset == 0.0 {
                return Err(format!("{WHAT}: `offset` must be positive"));
            }
            let settings = physics::CharacterSettings {
                step_height: length("stepHeight")?,
                max_slope,
                snap: length("snap")?,
                offset,
            };
            Ok(FunctorLangBody(physics::Body::character(tag.to_string(), shape.0, settings)))
        },
    );
    // Body LAST (subject-last), so they pipe:
    // `Physics.dynamic(crateTag, Physics.box(1.0, 1.0, 1.0)) |> Physics.at(Vec3.make(0.0, 5.0, 0.0))`.
    reg.fn2(
        "Physics.at",
        "Physics.at(v, body)",
        |v: FunctorLangVec3, body: FunctorLangBody| {
            fused_room_stays_put("Physics.at", &body)?;
            let (x, y, z) = v.0;
            Ok(FunctorLangBody(body.0.at([x, y, z])))
        },
    );
    reg.fn2(
        "Physics.rotateX",
        "Physics.rotateX(angle, body)",
        |angle: FunctorLangAngle, body: FunctorLangBody| {
            rotate_physics_body("Physics.rotateX", [1.0, 0.0, 0.0], angle, body)
        },
    );
    reg.fn2(
        "Physics.rotateY",
        "Physics.rotateY(angle, body)",
        |angle: FunctorLangAngle, body: FunctorLangBody| {
            rotate_physics_body("Physics.rotateY", [0.0, 1.0, 0.0], angle, body)
        },
    );
    reg.fn2(
        "Physics.rotateZ",
        "Physics.rotateZ(angle, body)",
        |angle: FunctorLangAngle, body: FunctorLangBody| {
            rotate_physics_body("Physics.rotateZ", [0.0, 0.0, 1.0], angle, body)
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
    reg.fn2(
        "Physics.linearDamping",
        "Physics.linearDamping(n, body)",
        |n: f64, body: FunctorLangBody| {
            Ok(FunctorLangBody(body.0.with_linear_damping(
                non_negative(n, "Physics.linearDamping")? as f32,
            )))
        },
    );
    reg.fn2(
        "Physics.angularDamping",
        "Physics.angularDamping(n, body)",
        |n: f64, body: FunctorLangBody| {
            Ok(FunctorLangBody(body.0.with_angular_damping(
                non_negative(n, "Physics.angularDamping")? as f32,
            )))
        },
    );
    reg.fn1("Physics.sensor", "Physics.sensor(body)", |body: FunctorLangBody| {
        FunctorLangBody(body.0.as_sensor())
    });
    reg.fn1(
        "Physics.upright",
        "Physics.upright(body)",
        |body: FunctorLangBody| FunctorLangBody(body.0.as_upright()),
    );
    reg.fn1(
        "Physics.continuous",
        "Physics.continuous(body)",
        |body: FunctorLangBody| FunctorLangBody(body.0.as_continuous()),
    );
    reg.fn2(
        "Physics.scene",
        "Physics.scene(Vec3.make(gx, gy, gz), [body, …])",
        |g: FunctorLangVec3, entries: Vec<PhysicsEntry>| {
            let (gx, gy, gz) = g.0;
            let mut scene = physics::PhysicsScene::create([gx, gy, gz], Vec::new());
            for entry in entries {
                match entry {
                    PhysicsEntry::Body(body) => scene.bodies.push(body),
                    PhysicsEntry::Group(group) => scene.groups.push(group),
                }
            }
            check_tag_owners(&scene.bodies, &scene.groups)?;
            Ok(FunctorLangPhysicsScene(scene))
        },
    );
    // A memoized group of fixed/kinematic bodies (see `physics_memo`).
    reg.fn3(
        "Physics.memo",
        "Physics.memo(key, deps, builder) — e.g. Physics.memo(\"statics\", model.rev, staticBodies)",
        physics_memo,
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
    // The read counterpart of the `Physics.setVelocity` command. Named
    // `linearVelocity` because `Physics.velocity` is already the body-builder
    // attribute (an *initial* velocity), and a read and a builder must not
    // share a name.
    reg.fn1(
        "Physics.linearVelocity",
        "Physics.linearVelocity(tag)",
        |tag: std::rc::Rc<str>| match live_velocity(&tag) {
            Some(v) => Ok(Value::Record(Rc::new(vec![
                ("x".to_string(), Value::Number(v[0] as f64)),
                ("y".to_string(), Value::Number(v[1] as f64)),
                ("z".to_string(), Value::Number(v[2] as f64)),
            ]))),
            None => Err(no_body(&tag)),
        },
    );
    // A world read like `Physics.position`; `false` before a character's
    // first move (and while priming).
    reg.fn1("Physics.grounded", "Physics.grounded(tag)", |tag: std::rc::Rc<str>| {
        note_read(ReadClass::Physics);
        physics::with_world(physics::active_world(), |w| w.grounded(&tag))
            .flatten()
            .or_else(|| PRIMING.with(|p| p.get()).then_some(false))
            .map(Value::Bool)
            .ok_or_else(|| format!("no Physics.character tagged \"{tag}\" in the physics world"))
    });
    // SYNCHRONOUS world queries — the character-controller surface. Unlike
    // `Physics.raycast` (an Effect deferred past the step, so its answer
    // reaches `tick` a frame late) these answer in-place, in the same class as
    // `Physics.position`: a direct read of the ACTIVE world as of the last
    // step, NOT an environment read. That is why they are not routed through
    // the `EffectRunner` and not logged — the physics world is deterministic
    // state reconstructed from the Timeline's recorded declarations and
    // commands, so a replayed frame reads back exactly what it read live.
    //
    // In `tick` these see last step's world; in `draw`, this frame's. That one
    // step of read latency is inherent to read → decide → step and is what
    // every fixed-step controller works against.
    reg.fn3(
        "Physics.cast",
        "Physics.cast(origin, dir, maxDist)",
        |origin: FunctorLangVec3, dir: FunctorLangVec3, max_dist: f64| {
            sync_query(None, origin, dir, max_dist, None, "Physics.cast")
        },
    );
    // Excludes the named body, so a character can probe out of its own
    // collider — a downward ray from inside a capsule would otherwise hit that
    // capsule at distance 0 and report standing on itself.
    reg.fn4(
        "Physics.castExcluding",
        "Physics.castExcluding(tag, origin, dir, maxDist)",
        |tag: std::rc::Rc<str>, origin: FunctorLangVec3, dir: FunctorLangVec3, max_dist: f64| {
            sync_query(None, origin, dir, max_dist, Some(&tag), "Physics.castExcluding")
        },
    );
    // The swept counterparts of `Physics.cast`, same record and read rules.
    fn swept(shape: &FunctorLangShape, what: &str) -> Result<(), String> {
        match shape.0 {
            physics::Shape::Cuboid { .. }
            | physics::Shape::Sphere { .. }
            | physics::Shape::Capsule { .. } => Ok(()),
            _ => Err(format!("{what}: sweep a box, sphere or capsule")),
        }
    }
    reg.fn4(
        "Physics.shapeCast",
        "Physics.shapeCast(shape, origin, dir, maxDist)",
        |shape: FunctorLangShape, origin: FunctorLangVec3, dir: FunctorLangVec3, max_dist: f64| {
            swept(&shape, "Physics.shapeCast")?;
            sync_query(Some(&shape.0), origin, dir, max_dist, None, "Physics.shapeCast")
        },
    );
    reg.fn5(
        "Physics.shapeCastExcluding",
        "Physics.shapeCastExcluding(tag, shape, origin, dir, maxDist)",
        |tag: std::rc::Rc<str>,
         shape: FunctorLangShape,
         origin: FunctorLangVec3,
         dir: FunctorLangVec3,
         max_dist: f64| {
            let what = "Physics.shapeCastExcluding";
            swept(&shape, what)?;
            sync_query(Some(&shape.0), origin, dir, max_dist, Some(&tag), what)
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
    reg.fn2(
        "Physics.move",
        "Physics.move(tag, v)",
        physics_command(|tag, translation| physics::PhysicsCommand::Move { tag, translation }),
    );
    // Per-axis velocity writes. `setVelocity` replaces all three components,
    // which forces a character controller to author a vertical velocity every
    // frame even though the solver is the one that owns it while grounded —
    // and the only values available to the game are a stale read or a guess.
    // These two split the Y-up engine's natural partition: the horizontal
    // plane the game steers, and the vertical axis a jump owns for one frame.
    reg.fn3(
        "Physics.setVelocityXZ",
        "Physics.setVelocityXZ(tag, x, z)",
        |tag: std::rc::Rc<str>, x: f64, z: f64| {
            FunctorLangEffect(EffectTree::Physics(physics::PhysicsCommand::SetVelocityXZ {
                tag: tag.to_string(),
                x: x as f32,
                z: z as f32,
            }))
        },
    );
    reg.fn2(
        "Physics.setVelocityY",
        "Physics.setVelocityY(tag, v)",
        |tag: std::rc::Rc<str>, v: f64| {
            FunctorLangEffect(EffectTree::Physics(physics::PhysicsCommand::SetVelocityY {
                tag: tag.to_string(),
                y: v as f32,
            }))
        },
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
                while_pending: Vec::new(),
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
    // Placeholder-LAST subject threading: `asset |> Asset.whilePending(ph)`.
    // The placeholder renders while the asset streams in; it is just another
    // asset, so it chains (a low-poly proxy can carry its own placeholder).
    // Sounds are rejected: they decode at play time, outside the asset
    // cache, so there is no pending state to render.
    reg.fn2(
        "Asset.whilePending",
        "Asset.whilePending(placeholder, asset) — same-kind model or texture Asset values \
(pipes: asset |> Asset.whilePending(placeholder))",
        |placeholder: AssetArg, asset: AssetArg| -> Result<FunctorLangAsset, String> {
            let (placeholder, asset) = (placeholder.0, asset.0);
            if asset.kind == AssetKind::Sound || placeholder.kind == AssetKind::Sound {
                return Err(
                    "Asset.whilePending: sounds have no pending state to render (they \
decode at play time, outside the asset cache) — whilePending applies to model and \
texture assets"
                        .to_string(),
                );
            }
            if placeholder.kind != asset.kind {
                return Err(format!(
                    "Asset.whilePending: the placeholder must match the asset's kind — \
got a {} placeholder for a {} asset; construct it with {}",
                    placeholder.kind.noun(),
                    asset.kind.noun(),
                    asset.kind.constructor(),
                ));
            }
            // The OUTER placeholder is tried first, then its own chain, then
            // any chain the asset already carried.
            let mut while_pending =
                Vec::with_capacity(1 + placeholder.while_pending.len() + asset.while_pending.len());
            while_pending.push(placeholder.path);
            while_pending.extend(placeholder.while_pending);
            while_pending.extend(asset.while_pending);
            Ok(FunctorLangAsset {
                kind: asset.kind,
                path: asset.path,
                while_pending,
            })
        },
    );
}

/// A branded Asset argument at the registry seam (`Asset.whilePending`) —
/// teaches the Angle rule for assets: bare strings must go through the
/// `Asset.*` constructors first.
struct AssetArg(FunctorLangAsset);

impl crate::host_registry::FromArg for AssetArg {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        if let Some(asset) = asset_of(value) {
            return Ok(AssetArg(asset.clone()));
        }
        let message = match value {
            Value::String(s) => format!(
                "{path}: expected an Asset value, got a bare string (\"{s}\") — construct \
one with Asset.model/texture(…) first"
            ),
            other => format!(
                "{path}: expected an Asset value (Asset.model/texture(…)), got {}",
                other.kind_name()
            ),
        };
        Err(RunError { message, span })
    }
}

/// The Anim pose algebra — clip sampling, blending, the rest pose, additive
/// layers, masks, per-joint rotation, model-space look-at, and two-bone reach.
/// Playheads, targets, and weights are explicit in the values, so the pose
/// stays a pure function of what the game derived.
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
    // A pure post-pass over the expression beneath it: aim the joint's local
    // +Z axis at a model-space target after clips/blends have settled. The
    // explicit limit keeps arbitrary targets from producing unnatural full
    // turns; the weight makes the correction blendable.
    const LOOK_AT: &str = "Anim.lookAt(\"jointName\", target, maxDeflection, weight, anim) — \
a non-empty joint name, model-space Vec3 target, Angle limit from 0 to 180 degrees, and weight";
    reg.fn5(
        "Anim.lookAt",
        LOOK_AT,
        |joint: String,
         target: FunctorLangVec3,
         max_deflection: FunctorLangAngle,
         weight: f64,
         anim: FunctorLangAnim| {
            let max_deflection: cgmath::Rad<f32> = max_deflection.0.into();
            if joint.is_empty()
                || !(0.0..=std::f32::consts::PI).contains(&max_deflection.0)
            {
                return Err(format!("usage: {LOOK_AT}"));
            }
            Ok(FunctorLangAnim(AnimExpr::LookAt {
                joint,
                target: [target.0 .0, target.0 .1, target.0 .2],
                max_angle: max_deflection.0,
                weight: weight as f32,
                expr: Box::new(anim.0),
            }))
        },
    );
    // A pure two-bone IK post-pass: solve a direct root -> middle -> end
    // chain toward a model-space target after the expression below has
    // settled. The evaluated middle position supplies the bend side.
    const REACH: &str = "Anim.reach(\"rootJoint\", \"middleJoint\", \"endJoint\", target, \
weight, anim) — three distinct non-empty joint names forming a direct two-bone chain \
with uniform root scale, a model-space Vec3 target, and weight";
    reg.fn6(
        "Anim.reach",
        REACH,
        |root: String,
         middle: String,
         end: String,
         target: FunctorLangVec3,
         weight: f64,
         anim: FunctorLangAnim| {
            if root.is_empty()
                || middle.is_empty()
                || end.is_empty()
                || root == middle
                || root == end
                || middle == end
            {
                return Err(format!("usage: {REACH}"));
            }
            Ok(FunctorLangAnim(AnimExpr::Reach {
                root,
                middle,
                end,
                target: [target.0 .0, target.0 .1, target.0 .2],
                weight: weight as f32,
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
    // sendMsg(connId, msg): the typed sibling of `send` — msg is any
    // plain-data value (usually a shared-module ADT variant); the other end
    // receives it as `Net.Data(id, value)`. Converted to an EffectValue HERE
    // so a non-data payload errors at the construction site, not mid-drain.
    reg.fn2(
        "Effect.sendMsg",
        "Effect.sendMsg(connId, msg)",
        |conn: f64, msg: Value| {
            if conn < 0.0 || conn.fract() != 0.0 || conn > u64::MAX as f64 {
                return Err(format!(
                    "Effect.sendMsg: connId must be a non-negative whole number, got {conn}"
                ));
            }
            let payload = effect_value_from_value(&msg)
                .map_err(|e| format!("Effect.sendMsg: {e}"))?;
            Ok(FunctorLangEffect(EffectTree::SendMsg { conn, payload }))
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
    // Warm the asset cache ahead of `draw` referencing the asset — the
    // imperative prefetch (start streaming the boss when the player picks up
    // the key). Declarative loading stays the default; a preload just makes
    // the later reference a cache hit. Asset VALUES only (a new post-B.3
    // surface grows no new string coercion).
    reg.fn1(
        "Effect.preload",
        "Effect.preload(asset) — a model or texture Asset value to start loading now",
        |asset: PreloadableAsset| {
            FunctorLangEffect(EffectTree::PreloadAsset {
                kind: asset.kind,
                locator: asset.locator,
            })
        },
    );
    // Preload and deliver `msg` (a message VALUE, like playThen) through
    // `update` when the load SETTLES — loaded or failed (Sub.assets' failed
    // list says which).
    reg.fn2(
        "Effect.preloadThen",
        "Effect.preloadThen(asset, msg) — a model or texture Asset value, and the \
message delivered when its load settles",
        |asset: PreloadableAsset, msg: Value| {
            FunctorLangEffect(EffectTree::PreloadAssetThen {
                kind: asset.kind,
                locator: asset.locator,
                message: msg,
            })
        },
    );
}

/// The Persistence vocabulary — durable LOCAL state through the same broker
/// (and the same plain-data codec as `Effect.sendMsg`) as every other effect:
/// a game's "write my progress" and "read it back at boot". The slot key and
/// the payload are validated HERE, at the construction site, so a bad key or
/// a closure in the model errors at the call instead of mid-drain.
fn register_persistence(reg: &mut crate::host_registry::Registry) {
    reg.fn2(
        "Persistence.save",
        "Persistence.save(slot, value)",
        |slot: String, value: Value| {
            crate::storage::validate_slot(&slot).map_err(|e| format!("Persistence.save: {e}"))?;
            let payload =
                effect_value_from_value(&value).map_err(|e| format!("Persistence.save: {e}"))?;
            Ok(FunctorLangEffect(EffectTree::Save { slot, payload }))
        },
    );
    reg.fn2(
        "Persistence.load",
        "Persistence.load(slot, tagger)",
        |slot: String, tagger: Tagger| {
            crate::storage::validate_slot(&slot).map_err(|e| format!("Persistence.load: {e}"))?;
            Ok(FunctorLangEffect(EffectTree::Load {
                slot,
                tagger: tagger.0,
            }))
        },
    );
}

/// An `Effect.preload` argument: a MODEL or TEXTURE Asset value. Sounds
/// teach (they decode at play time, outside the asset cache — there is
/// nothing to warm), and bare strings teach toward the constructors — a new
/// post-flag-day surface accepts no string coercion.
struct PreloadableAsset {
    kind: crate::asset::preload::PreloadKind,
    locator: String,
}

impl crate::host_registry::FromArg for PreloadableAsset {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        if let Some(asset) = asset_of(value) {
            let kind = match asset.kind {
                AssetKind::Model => crate::asset::preload::PreloadKind::Model,
                AssetKind::Texture => crate::asset::preload::PreloadKind::Texture,
                AssetKind::Sound => {
                    return Err(RunError {
                        message: format!(
                            "{path}: sounds have nothing to preload (they decode at play \
time, outside the asset cache) — preload applies to model and texture assets"
                        ),
                        span,
                    })
                }
            };
            return Ok(PreloadableAsset {
                kind,
                locator: asset.path.clone(),
            });
        }
        let message = match value {
            Value::String(s) => format!(
                "{path}: expected an Asset value, got a bare string (\"{s}\") — construct \
one with Asset.model/texture(…) first"
            ),
            other => format!(
                "{path}: expected an Asset value (Asset.model/texture(…)), got {}",
                other.kind_name()
            ),
        };
        Err(RunError { message, span })
    }
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

/// A simple identifier-ish name: letters, then letters/digits/dashes — the
/// shape of every real tag and attribute name. Anything else could break out
/// of the serialized HTML (names are emitted unescaped).
fn valid_html_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Tags that execute or reframe content — never legitimate game UI. The wasm
/// overlay is a REAL DOM, so `Html.element("script", …)` would otherwise run
/// code in the host page (game logic has no DOM access by design).
fn forbidden_tag(tag: &str) -> bool {
    matches!(
        tag,
        "script" | "iframe" | "object" | "embed" | "link" | "base" | "meta" | "frame" | "frameset"
    )
}

/// Attribute names that execute code (`on*`), navigate (`href`, `action`,
/// `formaction`), embed documents (`srcdoc`), or spoof the runtime's handler
/// slots (`data-fn-*` — those come only from `Attr.onClick`/`Attr.onInput`).
fn forbidden_attr(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("on")
        || lower.starts_with("data-fn-")
        || matches!(lower.as_str(), "href" | "srcdoc" | "action" | "formaction")
}

/// Fold typed `Html.*` builder arguments — attrs and child nodes — into an
/// [`HtmlNode::Element`]. Type errors were already handled by the registry's
/// argument conversion; this owns only the fold.
fn build_html_element(
    tag: &str,
    attrs: Vec<FunctorLangHtmlAttr>,
    children: Vec<FunctorLangHtmlNode>,
) -> FunctorLangHtmlNode {
    let mut pairs = Vec::new();
    let mut click_slot = None;
    let mut input_slot = None;
    for FunctorLangHtmlAttr(attr) in attrs {
        match attr {
            HtmlAttr::Pair(name, value) => pairs.push((name, value)),
            HtmlAttr::Click(slot) => click_slot = Some(slot),
            HtmlAttr::Input(slot) => input_slot = Some(slot),
        }
    }
    FunctorLangHtmlNode(HtmlNode::Element {
        tag: tag.to_string(),
        attrs: pairs,
        click_slot,
        input_slot,
        children: children.into_iter().map(|n| n.0).collect(),
    })
}

/// The webview vocabulary (the optional `webview = (model) => …` hook): an
/// Elm-style HTML/CSS tree — `Html.div([Attr.class("hud")], […])`.
/// Interactive attrs register their handler like Ui.button/textInput
/// (docs/ui-interaction.md); the shells render the tree via blitz (native)
/// or a DOM overlay (wasm).
fn register_html(reg: &mut crate::host_registry::Registry) {
    reg.fn1("Html.text", "Html.text(\"…\")", |text: String| {
        FunctorLangHtmlNode(HtmlNode::Text(text))
    });
    // A stylesheet: the CSS string is emitted VERBATIM inside <style> (CSS
    // needs `>` selectors etc.; `Html.text` is escaped).
    reg.fn1("Html.style", "Html.style(\"css…\")", |css: String| {
        FunctorLangHtmlNode(HtmlNode::Element {
            tag: "style".to_string(),
            attrs: vec![],
            click_slot: None,
            input_slot: None,
            children: vec![HtmlNode::Text(css)],
        })
    });
    // The generic element. Names are emitted unescaped and the wasm overlay
    // is a REAL DOM — validate here so a script-capable tag is a
    // construction-time teaching error, not an injection.
    reg.fn3(
        "Html.element",
        "Html.element(\"tag\", [attr, …], [node, …])",
        |tag: String,
         attrs: Vec<FunctorLangHtmlAttr>,
         children: Vec<FunctorLangHtmlNode>| {
            if !valid_html_name(&tag) || forbidden_tag(&tag) {
                return Err(format!(
                    "Html.element: `{tag}` is not an allowed tag name \
(letters/digits/dashes; script-capable tags are refused)"
                ));
            }
            Ok(build_html_element(&tag, attrs, children))
        },
    );
    for tag in ["div", "span", "button", "h1", "h2", "p"] {
        reg.fn2(
            format!("Html.{tag}").leak() as &'static str,
            format!("Html.{tag}([attr, …], [node, …])").leak() as &'static str,
            move |attrs: Vec<FunctorLangHtmlAttr>, children: Vec<FunctorLangHtmlNode>| {
                build_html_element(tag, attrs, children)
            },
        );
    }
    // A void element: attrs only. Pair Attr.value (controlled) with
    // Attr.onInput.
    reg.fn1(
        "Html.input",
        "Html.input([attr, …])",
        |attrs: Vec<FunctorLangHtmlAttr>| build_html_element("input", attrs, vec![]),
    );
    for name in ["class", "style", "id", "value", "placeholder"] {
        reg.fn1(
            format!("Attr.{name}").leak() as &'static str,
            format!("Attr.{name}(\"…\")").leak() as &'static str,
            move |value: String| FunctorLangHtmlAttr(HtmlAttr::Pair(name.to_string(), value)),
        );
    }
    // The `Html.element` rule for attribute names: reject executable (`on*`),
    // navigating (`href`/`action`), and handler-slot-spoofing (`data-fn-*`)
    // names.
    reg.fn2(
        "Attr.attr",
        "Attr.attr(\"name\", \"value\")",
        |name: String, value: String| {
            if !valid_html_name(&name) || forbidden_attr(&name) {
                return Err(format!(
                    "Attr.attr: `{name}` is not an allowed attribute name \
(letters/digits/dashes; on*/href/data-fn-* are refused)"
                ));
            }
            Ok(FunctorLangHtmlAttr(HtmlAttr::Pair(name, value)))
        },
    );
    // The msg is any Functor Lang value (typically an ADT variant), registered in
    // the frame's handler table and delivered VERBATIM through `update` when
    // the shell reports a click — the Ui.button shape.
    reg.fn1(
        "Attr.onClick",
        "Attr.onClick(msg) — msg is delivered to update on click",
        |msg: Value| {
            let slot = push_ui_handler(UiHandler::Msg(msg));
            FunctorLangHtmlAttr(HtmlAttr::Click(slot))
        },
    );
    // The tagger is applied to the new text on each edit — the Ui.textInput
    // shape.
    reg.fn1(
        "Attr.onInput",
        "Attr.onInput(tagger) — tagger: (newText) => msg",
        |tagger: Tagger| {
            let slot = push_ui_handler(UiHandler::Tagger(tagger.0));
            FunctorLangHtmlAttr(HtmlAttr::Input(slot))
        },
    );
    // Typed inline styles: the list folds into ONE `style="k: v; k: v"` pair
    // at construction, so the shells need no new lowering — it is exactly an
    // `Attr.style` by the time they see it.
    reg.fn1(
        "Attr.styles",
        "Attr.styles([Style.widthPx(300.0), …])",
        |styles: Vec<FunctorLangStyle>| {
            let css = styles
                .into_iter()
                .map(|FunctorLangStyle(decl)| decl)
                .collect::<Vec<_>>()
                .join("; ");
            FunctorLangHtmlAttr(HtmlAttr::Pair("style".to_string(), css))
        },
    );
}

/// `Color.t` -> the CSS `rgb(r, g, b)` form, quantized through [`ui::rgb_u8`]
/// so inline styles and `Ui.textColor` agree on the same 0..1 -> 0..255
/// rounding.
fn css_color(color: FunctorLangColor) -> String {
    let (r, g, b) = color.0;
    let [r, g, b] = ui::rgb_u8(r, g, b);
    format!("rgb({r}, {g}, {b})")
}

/// The `Style` vocabulary — typed INLINE styles for `Attr.styles` (the
/// elm-css idea, scoped to the declaration half of CSS). Each constructor
/// formats its value at construction (`Style.widthPx(300.0)` IS
/// `width: 300px`); colors are `Color.t` (the one color type across 3D and
/// UI). Stylesheet CSS — selectors, `:hover`, keyframes — deliberately stays
/// a string in `Html.style`.
fn register_style(reg: &mut crate::host_registry::Registry) {
    /// A fixed declaration (`Style.bold()` -> `font-weight: bold`).
    fn fixed(reg: &mut crate::host_registry::Registry, path: &'static str, css: &'static str) {
        reg.fn0(path, format!("{path}()").leak() as &'static str, move || {
            FunctorLangStyle(css.to_string())
        });
    }
    /// A `px`-valued declaration (`Style.widthPx(300.0)` -> `width: 300px`).
    fn px(reg: &mut crate::host_registry::Registry, path: &'static str, prop: &'static str) {
        reg.fn1(path, format!("{path}(n)").leak() as &'static str, move |n: f64| {
            FunctorLangStyle(format!("{prop}: {n}px"))
        });
    }
    /// A `%`-valued declaration (`Style.widthPct(50.0)` -> `width: 50%`).
    fn pct(reg: &mut crate::host_registry::Registry, path: &'static str, prop: &'static str) {
        reg.fn1(path, format!("{path}(n)").leak() as &'static str, move |n: f64| {
            FunctorLangStyle(format!("{prop}: {n}%"))
        });
    }
    /// A color-valued declaration (`Style.color(Color.rgb(…))`).
    fn colored(reg: &mut crate::host_registry::Registry, path: &'static str, prop: &'static str) {
        reg.fn1(
            path,
            format!("{path}(color)").leak() as &'static str,
            move |color: FunctorLangColor| FunctorLangStyle(format!("{prop}: {}", css_color(color))),
        );
    }

    // Layout (flexbox).
    fixed(reg, "Style.flexRow", "display: flex; flex-direction: row");
    fixed(reg, "Style.flexColumn", "display: flex; flex-direction: column");
    px(reg, "Style.gapPx", "gap");
    fixed(reg, "Style.justifyStart", "justify-content: flex-start");
    fixed(reg, "Style.justifyCenter", "justify-content: center");
    fixed(reg, "Style.justifyEnd", "justify-content: flex-end");
    fixed(reg, "Style.justifyBetween", "justify-content: space-between");
    fixed(reg, "Style.alignStart", "align-items: flex-start");
    fixed(reg, "Style.alignCenter", "align-items: center");
    fixed(reg, "Style.alignEnd", "align-items: flex-end");
    // Sizing.
    px(reg, "Style.widthPx", "width");
    pct(reg, "Style.widthPct", "width");
    px(reg, "Style.heightPx", "height");
    pct(reg, "Style.heightPct", "height");
    // Spacing (all sides).
    px(reg, "Style.paddingPx", "padding");
    px(reg, "Style.marginPx", "margin");
    // Color — Color.t, the one color type across 3D and UI.
    colored(reg, "Style.color", "color");
    colored(reg, "Style.background", "background");
    // Text.
    px(reg, "Style.fontSizePx", "font-size");
    fixed(reg, "Style.bold", "font-weight: bold");
    fixed(reg, "Style.textCenter", "text-align: center");
    // Border.
    reg.fn2(
        "Style.borderPx",
        "Style.borderPx(n, color)",
        |n: f64, color: FunctorLangColor| {
            FunctorLangStyle(format!("border: {n}px solid {}", css_color(color)))
        },
    );
    px(reg, "Style.roundedPx", "border-radius");
    // Misc.
    reg.fn1("Style.opacity", "Style.opacity(n) — 0..1", |n: f64| {
        if !(0.0..=1.0).contains(&n) {
            return Err(format!("Style.opacity: expected 0..1, got {n}"));
        }
        Ok(FunctorLangStyle(format!("opacity: {n}")))
    });
    // The escape hatch for the long tail of CSS. The `Attr.attr` rule for the
    // property NAME (letters/digits/dashes), so a hostile name can't smuggle
    // extra declarations or break out of the attribute; the value is escaped
    // by the serializer like any other attribute text.
    reg.fn2(
        "Style.raw",
        "Style.raw(\"property\", \"value\")",
        |name: String, value: String| {
            if !valid_html_name(&name) {
                return Err(format!(
                    "Style.raw: `{name}` is not an allowed property name (letters/digits/dashes)"
                ));
            }
            Ok(FunctorLangStyle(format!("{name}: {value}")))
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

/// A model-asset argument: an `Asset.model` locator (a wrong-kind asset gets
/// `asset_path`'s teaching error) — the typed-manifest front door at the
/// registry seam. Since the flag day (B.6) bare path strings teach instead
/// of coercing. Carries the asset's `Asset.whilePending` chain.
struct ModelPath {
    path: String,
    while_pending: Vec<String>,
}

/// A texture-asset locator for data-oriented consumers that need the original
/// image samples, rather than a material [`TextureDescription`]. Terrain must
/// retain the locator and placeholder chain so render and physics hydrate the
/// same source.
struct TextureAssetPath {
    path: String,
    while_pending: Vec<String>,
}

impl crate::host_registry::FromArg for TextureAssetPath {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            v if asset_of(v).is_some() => {
                let locator = asset_path(v, AssetKind::Texture, path, span)?;
                let chain = asset_of(v)
                    .map(|asset| asset.while_pending.clone())
                    .unwrap_or_default();
                Ok(Self {
                    path: locator,
                    while_pending: chain,
                })
            }
            Value::String(p) => Err(RunError {
                message: format!(
                    "{path}: bare asset paths are no longer accepted — reference the \
generated manifest (run `functor import`, then Assets.<name>), or construct \
Asset.texture({}) at the data boundary",
                    if p.is_empty() {
                        "\"file.png\"".to_string()
                    } else {
                        format!("\"{p}\"")
                    }
                ),
                span,
            }),
            other => Err(RunError {
                message: format!(
                    "{path}: expected a texture Asset value (the generated manifest's \
Assets.<name>, or Asset.texture(…)), got {}",
                    other.kind_name()
                ),
                span,
            }),
        }
    }
}

impl crate::host_registry::FromArg for ModelPath {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            v if asset_of(v).is_some() => {
                // asset_path enforces the KIND (teaching error); the chain
                // rides along.
                let locator = asset_path(v, AssetKind::Model, path, span)?;
                let chain = asset_of(v).map(|a| a.while_pending.clone()).unwrap_or_default();
                Ok(ModelPath {
                    path: locator,
                    while_pending: chain,
                })
            }
            // The flag day (B.6): the pre-manifest bare-string coercion is
            // retired — strings live only at data boundaries, inside the
            // Asset constructors.
            Value::String(p) => Err(RunError {
                message: format!(
                    "{path}: bare asset paths are no longer accepted — reference the \
generated manifest (run `functor import`, then Assets.<name>), or construct \
Asset.model({}) at the data boundary",
                    if p.is_empty() {
                        "\"file.glb\"".to_string()
                    } else {
                        format!("\"{p}\"")
                    }
                ),
                span,
            }),
            other => Err(RunError {
                message: format!(
                    "{path}: expected a model Asset value (the generated manifest's \
Assets.<name>, or Asset.model(…)), got {}",
                    other.kind_name()
                ),
                span,
            }),
        }
    }
}

/// A sound argument (the [`ModelPath`] dual-accept pattern, for audio —
/// A sound-asset argument: an `Asset.sound` locator (a wrong-kind asset gets
/// `asset_path`'s teaching error). Since the flag day (B.6) bare path
/// strings teach the migration instead of coercing.
struct SoundPath(String);

impl crate::host_registry::FromArg for SoundPath {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            v if asset_of(v).is_some() => {
                asset_path(v, AssetKind::Sound, path, span).map(SoundPath)
            }
            // The flag day (B.6): bare sound paths retired, like model paths.
            Value::String(p) => Err(RunError {
                message: format!(
                    "{path}: bare asset paths are no longer accepted — reference the \
generated manifest (run `functor import`, then Assets.<name>), or construct \
Asset.sound({}) at the data boundary",
                    if p.is_empty() {
                        "\"file.ogg\"".to_string()
                    } else {
                        format!("\"{p}\"")
                    }
                ),
                span,
            }),
            other => Err(RunError {
                message: format!(
                    "{path}: expected a sound Asset value (the generated manifest's \
Assets.<name>, or Asset.sound(…)), got {}",
                    other.kind_name()
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

use bytes::FunctorLangBytes;
use cells::FunctorLangCells;
use mesh::FunctorLangMesh;
use model::FunctorLangModel;
use solid::{FunctorLangFont, FunctorLangSketch, FunctorLangSolid};

handle_arg!(
    FunctorLangSolid => "a Solid",
    FunctorLangSketch => "a Sketch",
    FunctorLangFont => "a Typeface",
    FunctorLangBytes => "Bytes",
    FunctorLangMesh => "a Mesh",
    FunctorLangCells => "Cells",
    FunctorLangModel => "a Model",
    FunctorLangTerrain => "a Terrain",
    FunctorLangScene => "a Scene",
    FunctorLangInstance => "an Instance",
    FunctorLangLight => "a Light",
    FunctorLangCamera => "a Camera3D",
    FunctorLangFrame => "a Frame",
    FunctorLangShape => "a Shape",
    FunctorLangEffect => "an Effect",
    FunctorLangSub => "a Sub",
    FunctorLangView => "a View",
    FunctorLangHtmlNode => "an Html node",
    FunctorLangHtmlAttr => "an Attr",
    FunctorLangStyle => "a Style",
    FunctorLangAudioSource => "an AudioSource",
);

/// The body in `value`; a `Physics.memo` group gets a teaching error, since
/// only `Physics.scene` takes one.
fn body_ref<'a>(value: &'a Value, path: &str) -> Result<&'a physics::Body, String> {
    if let Value::HostData(data) = value {
        if let Some(body) = data.as_any().downcast_ref::<FunctorLangBody>() {
            return Ok(&body.0);
        }
        if let Some(group) = data.as_any().downcast_ref::<FunctorLangBodyGroup>() {
            return Err(format!(
                "{path}: Physics.memo(\"{}\") is a group, not one body — only \
Physics.scene takes it; apply {path} to the bodies inside its builder",
                group.0.key
            ));
        }
    }
    Err(format!(
        "{path}: expected a Body, got {}",
        value.kind_name()
    ))
}

impl crate::host_registry::FromArg for FunctorLangBody {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        body_ref(value, path)
            .map(|body| FunctorLangBody(body.clone()))
            .map_err(|message| RunError { message, span })
    }
}

/// One element of `Physics.scene`'s list: a body or a `Physics.memo` group.
enum PhysicsEntry {
    Body(physics::Body),
    Group(physics::BodyGroup),
}

impl crate::host_registry::FromArg for PhysicsEntry {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        if let Value::HostData(data) = value {
            if let Some(group) = data.as_any().downcast_ref::<FunctorLangBodyGroup>() {
                return Ok(PhysicsEntry::Group(group.0.clone()));
            }
        }
        FunctorLangBody::from_arg(value, path, span).map(|body| PhysicsEntry::Body(body.0))
    }
}

thread_local! {
    /// The groups `Physics.scene` last found disjoint. A group among them
    /// is only checked again against groups new since then.
    static CHECKED_GROUPS: std::cell::RefCell<Vec<std::sync::Arc<[physics::Body]>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// `Physics.scene`'s tag ownership rule: a tag belongs to one flat body or
/// one group. O(flat × groups) per frame, plus O(new group's bodies × groups)
/// when a group is new or rebuilt.
fn check_tag_owners(bodies: &[physics::Body], groups: &[physics::BodyGroup]) -> Result<(), String> {
    for body in bodies {
        if let Some(group) = groups.iter().find(|g| g.get(&body.tag).is_some()) {
            return Err(format!(
                "Physics.scene: tag \"{}\" is declared both flat and by Physics.memo(\"{}\") — \
a tag belongs to one place; drop the flat body or take it out of the memo",
                body.tag, group.key
            ));
        }
    }
    for (i, group) in groups.iter().enumerate() {
        if groups[..i].iter().any(|g| g.key == group.key) {
            return Err(format!(
                "Physics.scene: Physics.memo(\"{}\") appears twice — give each site its own key",
                group.key
            ));
        }
    }
    CHECKED_GROUPS.with(|checked| {
        let mut checked = checked.borrow_mut();
        for (i, group) in groups.iter().enumerate() {
            // Two groups checked together before are still disjoint.
            if checked
                .iter()
                .any(|c| std::sync::Arc::ptr_eq(c, &group.bodies))
            {
                continue;
            }
            for body in group.bodies.iter() {
                let other = groups
                    .iter()
                    .enumerate()
                    .find(|(j, g)| *j != i && g.get(&body.tag).is_some());
                if let Some((j, other)) = other {
                    let (first, second) = if j < i {
                        (other, group)
                    } else {
                        (group, other)
                    };
                    return Err(format!(
                        "Physics.scene: tag \"{}\" is declared by both Physics.memo(\"{}\") \
and Physics.memo(\"{}\") — a tag belongs to one group",
                        body.tag, first.key, second.key
                    ));
                }
            }
        }
        *checked = groups.iter().map(|g| g.bodies.clone()).collect();
        Ok(())
    })
}

// After `handle_arg!`, which it uses.
mod settings;
pub use settings::{config_of, externals_of, settings_of};

/// The host functions that read project files (`config` may not call them:
/// the CLI reads it first, to learn which paths are external).
pub const FILE_READERS: &[&str] = &["File.bytes", "File.exists", "Typeface.file"];

/// `Scene.instanced`'s instance list, converted once per Functor Lang list:
/// the same list `Rc` (e.g. one kept in the model) yields the same
/// [`InstanceList`] `Arc`, which is what lets the renderer skip the re-upload.
struct InstanceListArg(InstanceList);

/// Memo entries kept at most; the least recently used is evicted beyond it.
/// Dead lists are pruned on every miss, so live entries are what the game
/// holds — the cap only bounds lists kept alive elsewhere (e.g. in
/// time-travel history) and keeps an eviction scan cheap.
const INSTANCE_LIST_MEMO_CAP: usize = 256;

struct InstanceListMemoEntry {
    // Weak, so the memo never keeps a list alive — yet it keeps the `Rc`'s
    // allocation (and so its address) from being reused while the entry
    // lives, which is what makes the pointer key sound. Values are
    // immutable, and `Rc::get_mut`/`make_mut` refuse while a weak exists.
    list: std::rc::Weak<Vec<Value>>,
    converted: InstanceList,
    last_used: u64,
}

#[derive(Default)]
struct InstanceListMemo {
    entries: std::collections::HashMap<usize, InstanceListMemoEntry>,
    clock: u64,
}

thread_local! {
    /// Pure memoization (same list → same content), so it cannot affect
    /// determinism, replay, or hot reload — only skip repeated conversions.
    static INSTANCE_LISTS: std::cell::RefCell<InstanceListMemo> =
        std::cell::RefCell::new(InstanceListMemo::default());
}

impl crate::host_registry::FromArg for InstanceListArg {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let Value::List(items) = value else {
            // Not a list: the generic conversion owns the error text.
            return Vec::<FunctorLangInstance>::from_arg(value, path, span)
                .map(|instances| InstanceListArg(instances_into_list(instances)));
        };
        let key = Rc::as_ptr(items) as usize;
        let hit = INSTANCE_LISTS.with(|memo| {
            let mut memo = memo.borrow_mut();
            memo.clock += 1;
            let clock = memo.clock;
            memo.entries.get_mut(&key).map(|entry| {
                entry.last_used = clock;
                entry.converted.clone()
            })
        });
        if let Some(converted) = hit {
            return Ok(InstanceListArg(converted));
        }
        let converted =
            instances_into_list(Vec::<FunctorLangInstance>::from_arg(value, path, span)?);
        INSTANCE_LISTS.with(|memo| {
            let mut memo = memo.borrow_mut();
            // Dead lists can never be looked up again — drop them first, then
            // the least recently used if still full.
            memo.entries.retain(|_, entry| entry.list.strong_count() > 0);
            if memo.entries.len() >= INSTANCE_LIST_MEMO_CAP {
                if let Some(oldest) = memo
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(key, _)| *key)
                {
                    memo.entries.remove(&oldest);
                }
            }
            let last_used = memo.clock;
            memo.entries.insert(
                key,
                InstanceListMemoEntry {
                    list: Rc::downgrade(items),
                    converted: converted.clone(),
                    last_used,
                },
            );
        });
        Ok(InstanceListArg(converted))
    }
}

fn instances_into_list(instances: Vec<FunctorLangInstance>) -> InstanceList {
    instances
        .into_iter()
        .map(|instance| instance.0)
        .collect::<Vec<_>>()
        .into()
}

/// `Scene.memo` sites unused for more than this many frames are evicted
/// (see [`sweep_scene_memos`]).
const SCENE_MEMO_IDLE_FRAMES: u64 = 120;

/// Live memo sites kept at most — the bound for hosts that never sweep
/// (tests, tooling); the least recently used is evicted beyond it.
const SCENE_MEMO_CAP: usize = 1024;

/// One `Scene.memo` site's last build.
struct SceneMemo {
    deps: Value,
    /// The builder's lambda body, held so its address cannot be reused. A hot
    /// reload lowers fresh bodies, so every site rebuilds once after it — which
    /// is what makes an edit to a builder (or anything it calls) take effect.
    code: Rc<functor_lang::ir::Expr>,
    scene: Value,
    last_used: u64,
}

#[derive(Default)]
struct SceneMemos {
    sites: std::collections::HashMap<Rc<str>, SceneMemo>,
    frame: u64,
}

type MemoTable = std::thread::LocalKey<std::cell::RefCell<SceneMemos>>;

thread_local! {
    /// Derived shell state, like the instance-list memo: it never reaches the
    /// model, and a hit returns what the pure builder would have rebuilt, so
    /// it cannot affect determinism, replay, or time travel.
    static SCENE_MEMOS: std::cell::RefCell<SceneMemos> =
        std::cell::RefCell::new(SceneMemos::default());
    /// `Physics.memo`'s sites: the same rules, its own keys.
    static PHYSICS_MEMOS: std::cell::RefCell<SceneMemos> =
        std::cell::RefCell::new(SceneMemos::default());
}

/// `Scene.memo(key, deps, builder)`: the scene `builder(deps)` built the last
/// time this `key` saw structurally equal `deps` and the same builder code —
/// the SAME value, so the renderer's identity caches skip the upload too.
/// Otherwise the interpreter runs the builder and the result is cached.
fn scene_memo(key: Rc<str>, deps: Value, builder: Value) -> Result<HostReturn, String> {
    memo_site(
        &SCENE_MEMOS,
        "Scene.memo",
        "scene",
        key,
        deps,
        builder,
        |scene| {
            let Some(built) = (match &scene {
                Value::HostData(data) => data.as_any().downcast_ref::<FunctorLangScene>(),
                _ => None,
            }) else {
                return Err(format!(
                    "Scene.memo: the builder must return a Scene, got {}",
                    value_kind(&scene)
                ));
            };
            // Cache it shared, so wrapping it each frame copies a pointer.
            Ok(Value::HostData(Rc::new(FunctorLangScene(shared_scene(&built.0)))))
        },
    )
}

/// `Physics.memo(key, deps, builder)`: `Scene.memo` for physics. The bodies
/// become one shared [`physics::BodyGroup`], so a hit hands reconcile the
/// same `Arc` and it skips the group.
fn physics_memo(key: Rc<str>, deps: Value, builder: Value) -> Result<HostReturn, String> {
    let group_key = key.to_string();
    let finish = move |built| body_group(group_key, built);
    memo_site(
        &PHYSICS_MEMOS,
        "Physics.memo",
        "bodies",
        key,
        deps,
        builder,
        finish,
    )
}

/// A `Physics.memo` builder's bodies as a group: fixed/kinematic, ready
/// shapes, unique tags, sorted by tag.
fn body_group(key: String, built: Value) -> Result<Value, String> {
    let Value::List(items) = &built else {
        return Err(format!(
            "Physics.memo: the builder must return a list of bodies, got {}",
            value_kind(&built)
        ));
    };
    let mut bodies = Vec::with_capacity(items.len());
    for item in items.iter() {
        if let Value::HostData(data) = item {
            if data.as_any().is::<FunctorLangBodyGroup>() {
                return Err(
                    "Physics.memo: a memo can't contain another memo — list its bodies directly"
                        .to_string(),
                );
            }
        }
        let body = body_ref(item, "Physics.memo")?.clone();
        if body.kind == physics::BodyKind::Dynamic {
            return Err(format!(
                "Physics.memo: \"{}\" is dynamic — a memo holds fixed and kinematic \
bodies only; declare dynamic bodies flat in Physics.scene",
                body.tag
            ));
        }
        if matches!(body.shape, physics::Shape::Heightfield { .. }) {
            return Err(format!(
                "Physics.memo: \"{}\" is a heightfield — declare it flat in Physics.scene",
                body.tag
            ));
        }
        if matches!(body.shape, physics::Shape::FusedRoom { .. }) {
            return Err(format!(
                "Physics.memo: \"{}\" is a fused room — declare it flat in Physics.scene",
                body.tag
            ));
        }
        bodies.push(body);
    }
    bodies.sort_by(|a, b| a.tag.cmp(&b.tag));
    if let Some(pair) = bodies.windows(2).find(|pair| pair[0].tag == pair[1].tag) {
        return Err(format!(
            "Physics.memo(\"{key}\"): tag \"{}\" is declared twice",
            pair[0].tag
        ));
    }
    let group = physics::BodyGroup {
        key,
        bodies: bodies.into(),
    };
    Ok(Value::HostData(Rc::new(FunctorLangBodyGroup(group))))
}

/// The memo mechanism `Scene.memo` and `Physics.memo` share: a hit returns the
/// cached value; a miss applies `builder(deps)`, checks it with `finish`, and
/// caches it. `noun` names what the builder returns, for errors.
fn memo_site(
    table: &'static MemoTable,
    via: &'static str,
    noun: &'static str,
    key: Rc<str>,
    deps: Value,
    builder: Value,
    finish: impl FnOnce(Value) -> Result<Value, String> + 'static,
) -> Result<HostReturn, String> {
    // A lambda (or a def), so its body identifies the code: a hot reload lowers
    // new bodies. Builtins and partials of them hold no such identity.
    let Some((lambda, applied)) = builder_lambda(&builder) else {
        return Err(format!(
            "{via}: the builder must be a lambda or def `(deps) => {noun}`, got {}",
            value_kind(&builder)
        ));
    };
    if lambda.params.len() != applied + 1 {
        return Err(format!(
            "{via}: the builder takes the deps as its one argument — \
`(deps) => {noun}` — got a function of {} argument(s)",
            lambda.params.len() - applied
        ));
    }
    let code = lambda.body.clone();
    let hit = table.with(|memos| {
        let mut memos = memos.borrow_mut();
        let frame = memos.frame;
        let site = memos.sites.get_mut(&key)?;
        (Rc::ptr_eq(&site.code, &code) && same_plain_data(&site.deps, &deps)).then(|| {
            site.last_used = frame;
            site.scene.clone()
        })
    });
    if let Some(scene) = hit {
        // The skipped builder's reads go unrecorded, so a top-level value
        // built through a hit can't vouch for its inputs.
        note_unrecorded_inputs();
        return Ok(HostReturn::Value(scene));
    }
    // Only a miss walks the deps: a hit compared equal to deps that passed.
    if let Err(found) = plain_data(&deps) {
        return Err(format!(
            "{via}: deps must be plain data — numbers, strings, bools, and lists, \
maps, tuples, records, or variants of those — so they can be compared; found {found}. \
Pass the plain values the builder reads (e.g. `model.seed` or `model.trees`), not \
functions or engine values"
        ));
    }
    let args = vec![deps.clone()];
    let reads_before = read_count(ReadClass::Physics);
    Ok(HostReturn::Apply(Box::new(HostApply {
        callee: builder,
        args,
        via,
        then: Box::new(move |built| {
            if read_count(ReadClass::Physics) != reads_before {
                return Err(format!(
                    "{via}: the builder read the live physics world \
(Physics.position / linearVelocity / cast / transformed), which deps cannot capture, so \
the cached {noun} would freeze — make that read outside the memo and pass the values the \
builder needs in deps"
                ));
            }
            let scene = finish(built)?;
            table.with(|memos| {
                let mut memos = memos.borrow_mut();
                if memos.sites.len() >= SCENE_MEMO_CAP && !memos.sites.contains_key(&key) {
                    if let Some(oldest) = memos
                        .sites
                        .iter()
                        .min_by_key(|(_, site)| site.last_used)
                        .map(|(key, _)| key.clone())
                    {
                        memos.sites.remove(&oldest);
                    }
                }
                let last_used = memos.frame;
                memos.sites.insert(
                    key,
                    SceneMemo {
                        deps,
                        code,
                        scene: scene.clone(),
                        last_used,
                    },
                );
            });
            Ok(scene)
        }),
    })))
}

/// `scene` with its object behind one `SceneObject::Shared`.
fn shared_scene(scene: &Scene3D) -> Scene3D {
    let obj = match &scene.obj {
        shared @ SceneObject::Shared(_) => shared.clone(),
        obj => SceneObject::Shared(std::sync::Arc::new(obj.clone())),
    };
    Scene3D {
        obj,
        xform: scene.xform,
    }
}

/// The lambda a builder runs, and how many of its arguments a partial
/// application already supplied.
fn builder_lambda(builder: &Value) -> Option<(&functor_lang::value::Closure, usize)> {
    match builder {
        Value::Closure(closure) => Some((closure, 0)),
        Value::Partial(partial) => match &partial.callee {
            Value::Closure(closure) => Some((closure, partial.applied.len())),
            _ => None,
        },
        _ => None,
    }
}

/// `Ok` for plain data; otherwise what was found (`a function`, `a Vec3`).
/// Iterative, and each shared container is walked once: model data can be a
/// DAG whose tree expansion is exponential, or nested deeper than the stack.
fn plain_data(value: &Value) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    let mut work = vec![value];
    while let Some(value) = work.pop() {
        match value {
            Value::Number(_) | Value::String(_) | Value::Bool(_) => {}
            Value::List(items) | Value::Tuple(items) => {
                if seen.insert(Rc::as_ptr(items) as *const ()) {
                    work.extend(items.iter());
                }
            }
            Value::Map(entries) => {
                entries.for_each_unseen_leaf(&mut seen, |leaf| {
                    work.extend(leaf.iter().map(|(_, v)| v))
                });
            }
            Value::Record(fields) => {
                if seen.insert(Rc::as_ptr(fields) as *const ()) {
                    work.extend(fields.iter().map(|(_, v)| v));
                }
            }
            Value::Variant { args, .. } => {
                if seen.insert(Rc::as_ptr(args) as *const ()) {
                    work.extend(args.iter());
                }
            }
            Value::HostData(data) => {
                return Err(format!("a {} (an engine value)", data.type_name()))
            }
            other => return Err(value_kind(other).to_string()),
        }
    }
    Ok(())
}

/// Structural equality over plain data. A shared container (a model field that
/// did not change is the same list) is equal without a walk, and each pair of
/// containers is compared once, so it is linear in distinct nodes like
/// [`plain_data`]. Numbers compare by bits, so NaN deps still hit. Anything not
/// plain data is unequal.
fn same_plain_data(a: &Value, b: &Value) -> bool {
    let mut seen = std::collections::HashSet::new();
    // Map node pairs already compared (see `zip_unshared`).
    let mut map_nodes = std::collections::HashSet::new();
    // Whether a pair of containers still needs comparing.
    let mut fresh = |x: *const (), y: *const ()| x != y && seen.insert((x, y));
    let mut work = vec![(a, b)];
    while let Some(pair) = work.pop() {
        match pair {
            (Value::Number(x), Value::Number(y)) if x.to_bits() == y.to_bits() => {}
            (Value::String(x), Value::String(y)) if x == y => {}
            (Value::Bool(x), Value::Bool(y)) if x == y => {}
            (Value::List(x), Value::List(y)) | (Value::Tuple(x), Value::Tuple(y)) => {
                if fresh(Rc::as_ptr(x) as *const (), Rc::as_ptr(y) as *const ()) {
                    if x.len() != y.len() {
                        return false;
                    }
                    work.extend(x.iter().zip(y.iter()));
                }
            }
            // Node by node: subtrees the two versions share are skipped, so
            // deps one key apart compare in O(depth).
            (Value::Map(x), Value::Map(y)) => {
                let same_key = |a: &functor_lang::value::MapKey, b: &_| a == b;
                if !x.zip_unshared(y, &mut map_nodes, same_key, |va, vb| work.push((va, vb))) {
                    return false;
                }
            }
            (Value::Record(x), Value::Record(y)) => {
                if fresh(Rc::as_ptr(x) as *const (), Rc::as_ptr(y) as *const ()) {
                    if x.len() != y.len() {
                        return false;
                    }
                    for ((na, va), (nb, vb)) in x.iter().zip(y.iter()) {
                        if na != nb {
                            return false;
                        }
                        work.push((va, vb));
                    }
                }
            }
            (Value::Variant { ctor: ca, args: aa }, Value::Variant { ctor: cb, args: ab }) => {
                if ca != cb {
                    return false;
                }
                if fresh(Rc::as_ptr(aa) as *const (), Rc::as_ptr(ab) as *const ()) {
                    if aa.len() != ab.len() {
                        return false;
                    }
                    work.extend(aa.iter().zip(ab.iter()));
                }
            }
            _ => return false,
        }
    }
    true
}

/// Advance the `Scene.memo` and `Physics.memo` frame clocks and evict sites
/// unused for more than [`SCENE_MEMO_IDLE_FRAMES`] frames. Producers call it
/// once per `draw`.
pub fn sweep_scene_memos() {
    for table in [&SCENE_MEMOS, &PHYSICS_MEMOS] {
        table.with(|memos| {
            let mut memos = memos.borrow_mut();
            memos.frame += 1;
            let frame = memos.frame;
            memos
                .sites
                .retain(|_, site| frame - site.last_used <= SCENE_MEMO_IDLE_FRAMES);
        });
    }
}

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

thread_local! {
    /// What the evaluating top-level def has read so far, between
    /// [`FunctorHost`]'s `begin_inputs` and `end_inputs`; `None` outside.
    static DEF_INPUTS: std::cell::RefCell<Option<DefInputs>> =
        const { std::cell::RefCell::new(None) };
}

/// One def's reads: project files (`File.bytes`, `File.exists`,
/// `Typeface.file`), and the [`live_reads`] count it started at — any other
/// class's read makes the value non-reusable.
struct DefInputs {
    files: Vec<functor_lang::reuse::HostInput>,
    live_reads: u64,
    /// It read something no input describes.
    unrecorded: bool,
}

thread_local! {
    /// Reads so far, indexed by [`ReadClass`]: reload reuse and the memos
    /// compare counts around an evaluation to see what it read.
    static READS: [std::cell::Cell<u64>; ReadClass::ALL.len()] =
        const { [const { std::cell::Cell::new(0) }; ReadClass::ALL.len()] };
}

/// Note a read of ambient host state at call time. Every such read in the
/// prelude goes through here, and its built-in declares the class in its
/// `.funi` (`impure[physics]` above `let position`) — debug builds assert it
/// while a built-in's host closure runs.
fn note_read(class: ReadClass) {
    #[cfg(debug_assertions)]
    assert_declared(class);
    READS.with(|reads| {
        let count = &reads[class as usize];
        count.set(count.get() + 1);
    });
}

fn read_count(class: ReadClass) -> u64 {
    READS.with(|reads| reads[class as usize].get())
}

/// Reads of every class but `file` (whose reads are recorded as inputs).
fn live_reads() -> u64 {
    ReadClass::ALL
        .into_iter()
        .filter(|&class| class != ReadClass::File)
        .map(read_count)
        .sum()
}

/// Panics unless the running built-in's `.funi` signature declares `class`.
#[cfg(debug_assertions)]
fn assert_declared(class: ReadClass) {
    let Some(path) = crate::host_registry::running() else {
        return;
    };
    let declared = declared_reads()
        .get(path)
        .is_some_and(|reads| reads.contains(&class));
    assert!(
        declared,
        "`{path}` reads {class} state, but its .funi signature doesn't declare it — \
add `impure[{class}]` above it"
    );
}

/// Each prelude signature's `impure[…]` classes, by path (`File.bytes`).
#[cfg(any(debug_assertions, test))]
fn declared_reads() -> &'static std::collections::HashMap<String, Vec<ReadClass>> {
    static DECLARED: std::sync::OnceLock<std::collections::HashMap<String, Vec<ReadClass>>> =
        std::sync::OnceLock::new();
    DECLARED.get_or_init(|| {
        let mut declared = std::collections::HashMap::new();
        for (module, src) in functor_prelude::modules() {
            let program = functor_lang::parse_interface(&src).expect("the prelude parses");
            for item in program.items {
                if let functor_lang::ast::Item::Sig(sig) = item {
                    if !sig.reads.is_empty() {
                        declared.insert(format!("{module}.{}", sig.name), sig.reads);
                    }
                }
            }
        }
        declared
    })
}

/// Mark the evaluating def, if any, as non-reusable.
fn note_unrecorded_inputs() {
    DEF_INPUTS.with(|inputs| {
        if let Some(inputs) = inputs.borrow_mut().as_mut() {
            inputs.unrecorded = true;
        }
    });
}

/// A project-file read: noted, and recorded as an input of the evaluating
/// def, if one is.
fn note_file_read(input: impl FnOnce() -> (String, u64)) {
    note_read(ReadClass::File);
    DEF_INPUTS.with(|inputs| {
        if let Some(inputs) = inputs.borrow_mut().as_mut() {
            let (key, identity) = input();
            inputs
                .files
                .push(functor_lang::reuse::HostInput { key, identity });
        }
    });
}

/// Reload reuse (`functor_lang::reuse`): a top-level value is a pure function
/// of its arguments here except for its noted reads ([`note_read`]): project
/// files, recorded with their contents' hash, and any other class (the live
/// physics world, UI handler slots), which no input captures. Everything else
/// the prelude reads at call time is the call's arguments; input, time and
/// other live state reach a game through its hooks or `Effect`s, never
/// through a top-level value. A NEW host function that reads ambient state at
/// call time must call `note_read` with its class and declare that class in
/// its `.funi` (`impure[physics]`), or a reload will keep a value computed
/// from its stale answer.
impl Host for FunctorHost {
    fn provides(&self, path: &str) -> bool {
        registry().provides(path)
    }

    fn begin_inputs(&mut self) {
        let live_reads = live_reads();
        DEF_INPUTS.with(|inputs| {
            *inputs.borrow_mut() = Some(DefInputs {
                files: Vec::new(),
                live_reads,
                unrecorded: false,
            })
        });
    }

    fn end_inputs(&mut self) -> Option<Vec<functor_lang::reuse::HostInput>> {
        let inputs = DEF_INPUTS.with(|inputs| inputs.borrow_mut().take())?;
        let live = live_reads() != inputs.live_reads;
        (!live && !inputs.unrecorded).then_some(inputs.files)
    }

    fn inputs_current(&mut self, inputs: &[functor_lang::reuse::HostInput]) -> bool {
        inputs
            .iter()
            .all(|input| bytes::file_input_current(input) == Some(true))
    }

    fn call(&mut self, path: &str, args: Vec<Value>, span: Span) -> Result<HostReturn, RunError> {
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
`90deg` / `1.5rad`, or Angle.degrees(…) / Angle.radians(…)"
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
`0.5s` / `500ms`, or Time.seconds(…) / Time.millis(…)"
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
///
/// An inbound message carrying the [`TYPED_MSG_PREFIX`] frame (an
/// `Effect.sendMsg` from the other end) decodes into `Net.Data(id, value)`
/// instead of `Net.Message`; a frame that does not parse (version skew,
/// corruption) becomes `Net.Error` rather than leaking the raw frame as text.
/// This is the single decode site — the native and wasm paths all
/// deliver through it.
pub fn net_event_value(kind: NetEventKind, conn: u64, text: &str) -> EffectValue {
    let id = EffectValue::Number(conn as f64);
    match kind {
        NetEventKind::Connected => EffectValue::Variant("Net.Connected".into(), vec![id]),
        NetEventKind::Disconnected => EffectValue::Variant("Net.Disconnected".into(), vec![id]),
        NetEventKind::Message => match decode_typed_msg(text) {
            Some(Ok(payload)) => EffectValue::Variant("Net.Data".into(), vec![id, payload]),
            Some(Err(error)) => {
                EffectValue::Variant("Net.Error".into(), vec![id, EffectValue::Text(error)])
            }
            None => EffectValue::Variant(
                "Net.Message".into(),
                vec![id, EffectValue::Text(text.to_string())],
            ),
        },
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
    variant
        .to_functor_lang()
        .expect("host-built HTTP responses have no map keys")
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

thread_local! {
    /// In-flight `Effect.preloadThen` completion MESSAGES, keyed by the
    /// preload's token — the asset analogue of [`PENDING_AUDIO`], with the
    /// same rules: a plain message VALUE delivered verbatim; bounded (a
    /// shell that drops the command leaves its message un-taken); cleared on
    /// hot reload (a stored message may close over the OLD session).
    static PENDING_PRELOAD: std::cell::RefCell<std::collections::HashMap<u64, Value>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// An opaque copy of one pending `preloadThen` completion message for a
/// whole-shell time-travel snapshot. The message stays private so shells can
/// retain and restore it without depending on Functor Lang's runtime values.
#[derive(Clone)]
pub struct PreloadCompletionSnapshot {
    token: u64,
    message: Value,
}

/// Register a `preloadThen` completion message for `token` (see
/// [`PENDING_PRELOAD`]; the cap and eviction mirror
/// [`register_audio_completion`]).
pub fn register_preload_completion(token: u64, message: Value) {
    PENDING_PRELOAD.with(|m| {
        let mut map = m.borrow_mut();
        map.insert(token, message);
        while map.len() > PENDING_AUDIO_CAP {
            if let Some(oldest) = map.keys().copied().min() {
                map.remove(&oldest);
            } else {
                break;
            }
        }
    });
}

/// Take the completion message for a settled preload's `token`, if any.
pub fn take_preload_completion(token: u64) -> Option<Value> {
    PENDING_PRELOAD.with(|m| m.borrow_mut().remove(&token))
}

/// Clone the still-pending messages for `tokens` into a whole-shell snapshot.
/// Missing tokens are hot-reload orphans and are deliberately omitted.
pub fn snapshot_preload_completions(tokens: &[u64]) -> Vec<PreloadCompletionSnapshot> {
    PENDING_PRELOAD.with(|m| {
        let map = m.borrow();
        tokens
            .iter()
            .filter_map(|token| {
                map.get(token).cloned().map(|message| PreloadCompletionSnapshot {
                    token: *token,
                    message,
                })
            })
            .collect()
    })
}

/// Restore completion messages captured by [`snapshot_preload_completions`].
/// The caller must reject snapshots from a previous program revision: a
/// message may close over the session that created it.
pub fn restore_preload_completions(snapshots: Vec<PreloadCompletionSnapshot>) {
    PENDING_PRELOAD.with(|m| {
        let mut map = m.borrow_mut();
        for snapshot in snapshots {
            map.insert(snapshot.token, snapshot.message);
        }
    });
}

/// Drop all in-flight `preloadThen` completion messages (hot reload).
pub fn clear_preload_completions() {
    PENDING_PRELOAD.with(|m| m.borrow_mut().clear());
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
    // The slot depends on the widgets built before it this evaluation.
    note_read(ReadClass::Ui);
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

/// Drop whatever handlers the just-finished `draw` evaluation registered
/// (interactive widgets in `Frame.withUiTarget` trees — display-only for
/// now), so they neither accumulate across frames nor pollute the
/// `ui`/`webview` tables the producer adopts next. Returns whether anything
/// was dropped, so the caller can report [`DRAW_EVAL_HANDLERS_WARNING`] once.
pub fn drop_draw_eval_ui_handlers() -> bool {
    !take_ui_handlers().is_empty()
}

/// The one-time warning both producers report when
/// [`drop_draw_eval_ui_handlers`] drops something.
pub const DRAW_EVAL_HANDLERS_WARNING: &str =
    "[functor-lang] interactive Ui widgets in a draw-eval tree (e.g. \
Frame.withUiTarget) are display-only for now; their handlers are ignored";

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
    update_name: &'static str,
    model: &mut Value,
    taggers: &[Value],
    events: &[physics::PhysicsEvent],
    runner: &mut dyn EffectRunner,
    log: &mut Vec<Entry>,
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
                update_name,
                &args,
                crate::functor_lang_producer::Provenance::Collision,
            );
            match session.call(update_name, args, &mut FunctorHost) {
                Ok(returned) => {
                    let (next_model, more) = split_model_effect(returned);
                    *model = next_model;
                    if let Some(more) = more {
                        drain_mode(
                            session,
                            update_name,
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
                Err(e) => {
                    report(format!("[functor-lang] {update_name} error: {}", e.message))
                }
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

/// Friction/restitution/damping are coefficients: zero is meaningful, negative
/// is not (a negative damping would inject energy every step).
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
        Value::Map(entries) => entries.iter().any(|(_, value)| contains_effect(value)),
        Value::Record(fields) => fields.iter().any(|(_, v)| contains_effect(v)),
        Value::Variant { args, .. } => args.iter().any(contains_effect),
        _ => false,
    }
}

/// Perform `first` and drain to a fixed point: each performed effect's
/// tagger result is folded through `update`, whose return may carry MORE
/// effects — capped so a self-sustaining chain cannot hang the frame (the
/// F# executor's `maxEffectsPerFrame` discipline). Every answer and outbound
/// intent is appended to `log` (the frame's Δ; replay's input).
/// Errors report through `report` (deduped by the producer) and drop that
/// effect — one bad tagger must not stall the rest. `update_name` is the
/// ROLE's resolved update binding (`EntryNames::update` — `serverUpdate`
/// under a prefix), so a same-file role's messages land in its own hook.
///
/// Physics QUERIES are not performed here: they come back in the returned
/// list for the driver to hold until after the frame's physics step, then
/// hand to [`perform_deferred_queries`] — so their taggers answer against
/// the fresh world ("commands apply at the step; queries answer after it").
#[must_use = "deferred physics queries must be performed after the step"]
pub fn drain_effects(
    session: &functor_lang::Session,
    update_name: &'static str,
    model: &mut Value,
    first: EffectTree,
    runner: &mut dyn EffectRunner,
    log: &mut Vec<Entry>,
    report: &mut dyn FnMut(String),
    suppress_outbound: bool,
) -> Vec<EffectTree> {
    let mut deferred = Vec::new();
    drain_mode(
        session,
        update_name,
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
        // sendMsg is Send-shaped: fire-and-forget; the DELIVERY on the other
        // end folds through that game's `update`, not this one's.
        | EffectTree::SendMsg { .. }
        // The request is fire-and-forget; its RESPONSE needs `update`, but
        // that arrives frames later through the producer's HTTP pump.
        | EffectTree::Http { .. }
        // Audio one-shots are fire-and-forget too. `playThen`'s completion
        // needs `update`, but arrives frames later via the audio-finished
        // hook — the same shape as Http. Preloads likewise: `preloadThen`'s
        // settlement arrives via the shell's preload driver.
        | EffectTree::PlayAudio { .. }
        | EffectTree::PlayAudioThen { .. }
        | EffectTree::PreloadAsset { .. }
        | EffectTree::PreloadAssetThen { .. }
        // A save reports nothing back; only `Persistence.load` has a tagger.
        | EffectTree::Save { .. } => false,
        EffectTree::Now { .. }
        | EffectTree::Random { .. }
        | EffectTree::Raycast { .. }
        | EffectTree::Load { .. } => true,
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
    update_name: &'static str,
    model: &mut Value,
    deferred: Vec<EffectTree>,
    runner: &mut dyn EffectRunner,
    log: &mut Vec<Entry>,
    report: &mut dyn FnMut(String),
    suppress_outbound: bool,
) {
    if deferred.is_empty() {
        return;
    }
    drain_mode(
        session,
        update_name,
        model,
        deferred,
        runner,
        log,
        report,
        None,
        suppress_outbound,
    );
}

/// Log an outbound intent as an `Issued` op; a replay runner validates it.
fn issue(runner: &mut dyn EffectRunner, log: &mut Vec<Entry>, kind: &'static str, args: u64) {
    runner.issued(kind, args);
    log.push(Entry::issued(kind, args));
}

fn drain_mode(
    session: &functor_lang::Session,
    update_name: &'static str,
    model: &mut Value,
    queue: Vec<EffectTree>,
    runner: &mut dyn EffectRunner,
    log: &mut Vec<Entry>,
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
        // `kind` is the structured log's wire label; `api` is what a game
        // author actually typed, so a message about a failed tagger names a
        // function that exists (`Persistence.load`, not `Effect.storage.load`).
        // `args` is the op's digest; `None` for an ask (a raycast), which is
        // recomputed on replay, never logged.
        let (kind, api, value, tagger, args) = match tree {
            EffectTree::None => continue,
            EffectTree::Batch(items) => {
                // Preserve declaration order against the LIFO queue.
                for item in items.into_iter().rev() {
                    queue.push(item);
                }
                continue;
            }
            EffectTree::Physics(command) => {
                // Fire-and-forget: queue on the singleton world, continue. Not
                // a frame-log op — commands are per-frame *inputs* recorded by
                // the physics Timeline, not environment reads replay must fake.
                // Outbound suppression protects the LIVE world (a dry run must
                // not kick real bodies) — but a dry-run world SCOPE routes
                // commands to its own throwaway world, which is exactly where a
                // replayed kick belongs (docs/time-travel.md T6b).
                let target = physics::active_world();
                if !suppress_outbound || target != physics::DEFAULT_WORLD {
                    physics::with_world(target, |w| w.queue_command(command));
                }
                continue;
            }
            EffectTree::Send { conn, text } => {
                // Fire-and-forget, like a physics command: queue a Send on the
                // shell's connection manager (drained via
                // net_drain_conn_commands). Logged as an `Issued` op.
                if !suppress_outbound {
                    crate::net::push_conn_command(crate::net::ConnCommand::Send {
                        conn: conn as crate::net::ConnectionId,
                        payload: text.clone().into_bytes(),
                    });
                }
                issue(runner, log, "net.send", args_digest(&(conn, &text)));
                continue;
            }
            EffectTree::SendMsg { conn, payload } => {
                // Typed sibling of Send: same fire-and-forget queueing, with
                // the payload framed as TYPED_MSG_PREFIX + JSON. The log
                // record keeps the STRUCTURED payload, so typed traffic is
                // observable/replayable as data, not as an opaque string.
                if !suppress_outbound {
                    crate::net::push_conn_command(crate::net::ConnCommand::Send {
                        conn: conn as crate::net::ConnectionId,
                        payload: encode_typed_msg(&payload).into_bytes(),
                    });
                }
                issue(runner, log, "net.sendMsg", args_digest(&(conn, &payload)));
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
                // pump applies the tagger then. Logged as an `Issued` op.
                issue(
                    runner,
                    log,
                    "net.http",
                    args_digest(&(&method, &url, &body)),
                );
                if !suppress_outbound {
                    let token = crate::net::next_token();
                    register_http_tagger(token, tagger);
                    crate::net::push_command(crate::net::NetCommand::HttpRequest {
                        token,
                        method,
                        url,
                        headers: Vec::new(),
                        body: body.into_bytes(),
                    });
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
                issue(runner, log, "audio.play", args_digest(&(&sound, position)));
                continue;
            }
            EffectTree::PlayAudioThen { sound, message } => {
                // Fire-and-forget request (like Http): mint a token, register
                // the completion MESSAGE by it, and queue a tokened one-shot.
                // The finish lands frames later; the producer's audio-finished
                // hook delivers the message then. Logged as an `Issued` op.
                if !suppress_outbound {
                    let token = crate::audio::next_token();
                    register_audio_completion(token, message);
                    crate::audio::push_command(crate::audio::AudioCommand::play_one_shot_token(
                        token,
                        sound.clone(),
                    ));
                }
                issue(runner, log, "audio.playThen", args_digest(&sound));
                continue;
            }
            EffectTree::PreloadAsset { kind, locator } => {
                // Fire-and-forget prefetch: queue a PreloadCommand for the
                // shell's driver (drained via preload_drain_commands). No
                // token — nothing folds back through update.
                issue(runner, log, "asset.preload", args_digest(&(kind, &locator)));
                if !suppress_outbound {
                    crate::asset::preload::push_command(crate::asset::preload::PreloadCommand {
                        kind,
                        locator: locator.clone(),
                        token: None,
                    });
                }
                continue;
            }
            EffectTree::PreloadAssetThen {
                kind,
                locator,
                message,
            } => {
                // The playThen shape: mint a token, register the completion
                // MESSAGE by it, queue a tokened command. Settlement lands
                // frames later; the producer's preload-settled hook delivers
                // the message then.
                issue(
                    runner,
                    log,
                    "asset.preloadThen",
                    args_digest(&(kind, &locator)),
                );
                if !suppress_outbound {
                    let token = crate::asset::preload::next_token();
                    register_preload_completion(token, message);
                    crate::asset::preload::push_command(crate::asset::preload::PreloadCommand {
                        kind,
                        locator: locator.clone(),
                        token: Some(token),
                    });
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
                    "Physics.raycast",
                    runner.raycast(origin, dir, max_dist),
                    tagger,
                    None,
                ),
            },
            EffectTree::Save { slot, payload } => {
                // Tagger-less: perform the write and log an `Issued` op. A
                // failed write is reported and dropped — a full disk must not
                // kill the game.
                //
                // Unlike the other outbound arms, a save is NOT gated on
                // `suppress_outbound`: the RUNNER is the world seam here (a
                // replay writes its own overlay, so a later load in the same
                // replay still sees it).
                if let Err(error) = runner.save(&slot, &payload) {
                    report(format!(
                        "[functor-lang] Persistence.save(\"{slot}\") failed: {error}"
                    ));
                }
                issue(runner, log, "storage.save", args_digest(&(&slot, &payload)));
                continue;
            }
            EffectTree::Load { slot, tagger } => (
                "storage.load",
                "Persistence.load",
                runner.load(&slot),
                tagger,
                Some(args_digest(&slot)),
            ),
            EffectTree::Now { tagger } => (
                "now",
                "Effect.now",
                runner.now().into(),
                tagger,
                Some(args_digest(&())),
            ),
            EffectTree::Random { tagger } => (
                "random",
                "Effect.random",
                runner.random().into(),
                tagger,
                Some(args_digest(&())),
            ),
        };
        let value: EffectValue = value;
        let converted = value.to_functor_lang();
        // Logged even if unusable: the call happened, so a replay consumes it.
        if let Some(args) = args {
            log.push(Entry::answer(kind, args, value));
        }
        let functor_lang_value = match converted {
            Ok(value) => value,
            Err(error) => {
                report(format!(
                    "[functor-lang] {api} produced invalid structured data: {error}"
                ));
                continue;
            }
        };
        let msg = match session.apply(
            tagger,
            vec![functor_lang_value],
            &format!("{api} tagger"),
            &mut FunctorHost,
        ) {
            Ok(msg) => msg,
            Err(e) => {
                report(format!("[functor-lang] {api} tagger error: {}", e.message));
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
        crate::functor_lang_producer::journal_push(update_name, &args, provenance);
        match session.call(update_name, args, &mut FunctorHost) {
            Ok(returned) => {
                let (next_model, more) = split_model_effect(returned);
                *model = next_model;
                if let Some(more) = more {
                    queue.push(more);
                }
            }
            Err(e) => report(format!("[functor-lang] {update_name} error: {}", e.message)),
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
            if let Some(asset) = asset_of(value) {
                // A texture asset carries a file path (and possibly an
                // `Asset.whilePending` chain); a wrong-kind asset is a
                // teaching error from asset_path.
                let file = asset_path(value, AssetKind::Texture, what, span)?;
                return Ok(if asset.while_pending.is_empty() {
                    TextureDescription::File(file)
                } else {
                    TextureDescription::FileWhilePending {
                        file,
                        while_pending: asset.while_pending.clone(),
                    }
                });
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

thread_local! {
    /// Set only while the producer evaluates the `physics` hook to PRIME the
    /// world (docs/physics.md, "Cold start"). See [`PrimingScope`].
    static PRIMING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// RAII scope marking the COLD prime evaluation of the `physics` hook — the
/// one call with no stepped world behind it, because nothing has ever been
/// declared. Inside it a body read answers the IDENTITY pose (the origin, zero
/// velocity) instead of raising, so a hook that derives one body's declaration
/// from another's pose can bootstrap. Every later evaluation (frame 1 onward)
/// reads the real world, where a genuinely unknown tag raises exactly as
/// before — including a tag misspelled here, one frame later.
pub struct PrimingScope;

impl PrimingScope {
    pub fn enter() -> PrimingScope {
        // Set/clear rather than save/restore: priming is a single, top-level
        // producer call that never nests (the hook cannot prime the world).
        debug_assert!(
            !PRIMING.with(|p| p.get()),
            "PrimingScope does not nest — it clears the flag on drop"
        );
        PRIMING.with(|p| p.set(true));
        PrimingScope
    }
}

impl Drop for PrimingScope {
    fn drop(&mut self) {
        PRIMING.with(|p| p.set(false));
    }
}

/// Live pose of a body in the ACTIVE world (normally the singleton world the
/// shell steps — same process, same crate statics as this prelude; under a
/// dry-run forward-step scope, the throwaway projected world, so ghost draws
/// read the stepped poses — docs/time-travel.md T6b).
fn live_transform(tag: &str) -> Option<([f32; 3], [f32; 4])> {
    note_read(ReadClass::Physics);
    physics::with_world(physics::active_world(), |w| w.body_transform(tag))
        .flatten()
        .or_else(|| {
            PRIMING
                .with(|p| p.get())
                .then(|| ([0.0; 3], [0.0, 0.0, 0.0, 1.0]))
        })
}

/// Live linear velocity of a body in the ACTIVE world — the read counterpart
/// of `Physics.setVelocity`, on the same world-scope rules as
/// [`live_transform`].
fn live_velocity(tag: &str) -> Option<[f32; 3]> {
    note_read(ReadClass::Physics);
    physics::with_world(physics::active_world(), |w| w.body_velocity(tag))
        .flatten()
        .or_else(|| PRIMING.with(|p| p.get()).then_some([0.0; 3]))
}

/// A synchronous ray (`shape` `None`) or shape cast against the ACTIVE world,
/// shared by `Physics.cast`, `castExcluding` and `shapeCast`. Returns the same
/// record shape the deferred `Physics.raycast` effect hands its tagger
/// (`ray_result_value`), so the paths can never drift; a miss is `hit: false`
/// with zeroed fields rather than an error, because "nothing there" is an
/// ordinary answer a controller branches on.
fn sync_query(
    shape: Option<&physics::Shape>,
    origin: FunctorLangVec3,
    dir: FunctorLangVec3,
    max_dist: f64,
    exclude: Option<&str>,
    what: &str,
) -> Result<Value, String> {
    let (ox, oy, oz) = origin.0;
    let (dx, dy, dz) = dir.0;
    // Validate with the SAME predicate the query itself applies (`World::
    // raycast_excluding` normalizes and bails when the length is not finite
    // and positive). Checking the components against exact zero instead would
    // let three cases through to be silently answered as a MISS: non-finite
    // components, finite-but-tiny vectors whose f32 norm underflows to zero,
    // and huge ones whose squares overflow to infinity. For a grounding probe
    // a silent miss is the worst outcome — `onGround` is false forever and the
    // character simply never jumps — so these are loud errors instead.
    let d = [dx as f32, dy as f32, dz as f32];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if !(len.is_finite() && len > 0.0) {
        return Err(format!(
            "{what}: the direction must be finite and non-zero, got ({dx}, {dy}, {dz})"
        ));
    }
    // Formatted inline rather than via `positive(.., &format!(..))`: this is
    // the per-frame controller path, and that helper's label allocates a
    // String on every successful cast.
    if !(max_dist > 0.0) {
        return Err(format!("{what} maxDist must be positive, got {max_dist}"));
    }
    let max_dist = max_dist as f32;
    note_read(ReadClass::Physics);
    let (origin, dir) = ([ox, oy, oz], [dx, dy, dz]);
    let hit = physics::with_world(physics::active_world(), |w| match shape {
        Some(shape) => w.shape_cast(shape, origin, dir, max_dist, exclude),
        None => w.raycast_excluding(origin, dir, max_dist, exclude),
    })
    .flatten();
    ray_result_value(hit).to_functor_lang()
}

fn no_body(tag: &str) -> String {
    format!(
        "no body tagged \"{tag}\" in the physics world (bodies exist once the \
         `physics` hook has declared them — the world is declared from `init` \
         before the first frame, so this is a tag your hook never declares)"
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

    // Drift guard: the branded-value teaching errors name unit SUFFIXES
    // (`90deg`, `0.5s`), but the suffixes themselves are declared in the
    // `.funi` prelude, which this crate cannot read at runtime. Pin the two
    // together so renaming or dropping a `unit` fails here instead of leaving
    // a message that teaches a suffix nobody can write.
    #[test]
    fn prelude_units_match_the_teaching_errors() {
        use std::collections::BTreeSet;
        let mut suffixes: BTreeSet<String> = BTreeSet::new();
        for (module, src) in functor_prelude::modules() {
            let program = functor_lang::parse_interface(&src)
                .unwrap_or_else(|e| panic!("prelude module `{module}` must parse: {}", e.message));
            for item in &program.items {
                if let functor_lang::ast::Item::Unit(decl) = item {
                    assert!(
                        suffixes.insert(decl.suffix.clone()),
                        "unit `{}` is declared more than once in the prelude",
                        decl.suffix
                    );
                }
            }
        }
        let expected: BTreeSet<String> = ["deg", "rad", "s", "ms", "us", "min", "hr"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(
            suffixes, expected,
            "the prelude's unit suffixes changed — update the Angle/Duration teaching errors \
in this file (they quote `90deg` / `1.5rad` / `0.5s` / `500ms`) and this list together"
        );
    }

    /// Evaluate a `main` under the prelude WITH the engine's `.funi`
    /// interfaces linked, so unit suffixes and their operators resolve —
    /// `eval` above lowers one bare source, where no `unit` is declared.
    fn eval_with_prelude(src: &str) -> Result<Value, String> {
        eval_with_prelude_impl(src, true)
    }

    /// [`eval_with_prelude`] with the CHECKER skipped — the gradual seam
    /// (`functor-lang run`, a value arriving through an `unknown`), where the
    /// interpreter's own refusals are the only ones there are.
    fn eval_with_prelude_unchecked(src: &str) -> Result<Value, String> {
        eval_with_prelude_impl(src, false)
    }

    fn eval_with_prelude_impl(src: &str, typecheck: bool) -> Result<Value, String> {
        let project = functor_lang::project::load_sources_with_bundled_modules(
            vec![(std::path::PathBuf::from("game.fun"), src.to_string())],
            &functor_prelude::bundled_modules(),
        )
        .map_err(|e| e.message)?;
        if typecheck {
            if let Some(first) = functor_lang::check(&project.module).first() {
                return Err(first.message.clone());
            }
        }
        let record = functor_lang::run_with_host(&project.module, Tracing::Off, &mut FunctorHost)
            .map_err(|f| f.error.message)?;
        match record.outcome {
            functor_lang::RunOutcome::Main(value) => Ok(value),
            _ => Err("expected a main result".to_string()),
        }
    }

    fn radians(value: &Value) -> f32 {
        angle_of(value, "an Angle", Span::new(0, 0))
            .expect("an Angle")
            .radians()
    }

    fn seconds(value: &Value) -> f64 {
        duration_of(value, "a Duration", Span::new(0, 0)).expect("a Duration")
    }

    /// The headline: branded arithmetic, resolved through the `unit deg (+)`
    /// declaration in `angle.funi`, is exactly the handwritten call.
    #[test]
    fn angles_add_and_subtract_through_their_declared_operators() {
        let sum = eval_with_prelude("let main = () => 90deg + 45deg\n").expect("runs");
        assert!((radians(&sum) - radians(&
            eval_with_prelude("let main = () => Angle.add(Angle.degrees(90.0), Angle.degrees(45.0))\n")
                .expect("runs"))).abs() < 1e-6);
        let difference = eval_with_prelude("let main = () => 90deg - 1.5rad\n").expect("runs");
        assert!(
            (radians(&difference) - (std::f32::consts::FRAC_PI_2 - 1.5)).abs() < 1e-6,
            "mixed suffixes share one brand, so they subtract"
        );
    }

    /// Every suffix of one brand shares the operator: seconds and
    /// milliseconds are both `Time.t`, so they add and subtract directly.
    #[test]
    fn durations_of_different_suffixes_share_one_operator() {
        let value = eval_with_prelude("let main = () => 1.5s - 200ms\n").expect("runs");
        assert!((seconds(&value) - 1.3).abs() < 1e-9, "{}", seconds(&value));
        let value = eval_with_prelude("let main = () => 2min + 30s\n").expect("runs");
        assert_eq!(seconds(&value), 150.0);
    }

    /// Scaling commutes, so the brand may sit on either side of `*`.
    #[test]
    fn scaling_works_from_either_side() {
        for src in [
            "let main = () => 45deg * 2.0\n",
            "let main = () => 2.0 * 45deg\n",
        ] {
            let value = eval_with_prelude(src).expect("runs");
            assert!((radians(&value) - std::f32::consts::FRAC_PI_2).abs() < 1e-6, "{src}");
        }
        let value = eval_with_prelude("let main = () => 0.5s * 3.0\n").expect("runs");
        assert_eq!(seconds(&value), 1.5);
    }

    /// A branded value still refuses arithmetic nobody declared — with a
    /// message that names what the brand DOES have.
    #[test]
    fn an_undeclared_operator_on_a_brand_teaches_what_exists() {
        let error = eval_with_prelude("let main = () => 90deg / 2.0\n")
            .err()
            .expect("`/` is not declared on angles");
        assert!(error.contains("`Angle.t` declares"), "{error}");
        assert!(error.contains("but not `/`"), "{error}");
    }

    /// Top-level constants evaluate eagerly, before anything else runs, so
    /// branded arithmetic has to work there too — the shape a game's tuning
    /// constants and `init` actually take. [xreview: Critical]
    #[test]
    fn a_top_level_constant_may_use_branded_arithmetic() {
        let value = eval_with_prelude(
            "let quarter: Angle.t = 90deg + 45deg\n\
             let main = () => quarter\n",
        )
        .expect("runs");
        assert!((radians(&value) - std::f32::consts::FRAC_PI_2 * 1.5).abs() < 1e-6);
    }

    /// Branded comparison end to end: `90deg == 90deg` is TRUE where it used
    /// to be a runtime "host values cannot be compared" error, and the four
    /// derived orderings come from the one declared `<`.
    #[test]
    fn angles_and_durations_compare_and_order() {
        for (src, want) in [
            ("let main = () => 90deg == 90deg\n", true),
            ("let main = () => 90deg == 45deg\n", false),
            ("let main = () => 90deg != 45deg\n", true),
            ("let main = () => 45deg < 90deg\n", true),
            ("let main = () => 90deg > 45deg\n", true),
            ("let main = () => 45deg <= 45deg\n", true),
            ("let main = () => 45deg >= 90deg\n", false),
            // One brand, every suffix: seconds against milliseconds.
            ("let main = () => 1s == 1000ms\n", true),
            ("let main = () => 1.5s < 2000ms\n", true),
            ("let main = () => 2min > 90s\n", true),
            // Arithmetic feeds comparison — both resolve on the same brand.
            ("let main = () => (90deg + 45deg) > 90deg\n", true),
        ] {
            assert_eq!(
                eval_with_prelude(src).expect(src).to_string(),
                want.to_string(),
                "{src}"
            );
        }
    }

    /// The other half of the same story: an engine value that declares NO
    /// `==` still refuses comparison at run time — the gradual-seam backstop
    /// behind the check-time error.
    #[test]
    fn an_engine_value_still_refuses_equality_at_run_time() {
        let error = eval_with_prelude_unchecked("let main = () => Scene.cube() == Scene.cube()\n")
            .err()
            .expect("a scene cannot be compared");
        assert!(error.contains("host values cannot be compared"), "{error}");
    }

    /// A `Physics.tag` is a brand over a STRING, so it compares structurally
    /// with no declaration at all — what `examples/physics` reads collision
    /// events with.
    #[test]
    fn a_physics_tag_compares_as_its_string() {
        assert_eq!(
            eval_with_prelude("let main = () => Physics.tag(\"ball\") == Physics.tag(\"ball\")\n")
                .expect("runs")
                .to_string(),
            "true"
        );
        assert_eq!(
            eval_with_prelude("let main = () => Physics.tag(\"ball\") == Physics.tag(\"wall\")\n")
                .expect("runs")
                .to_string(),
            "false"
        );
    }

    /// The branded result flows on into the APIs that take it — the whole
    /// point of keeping arithmetic inside the brand.
    #[test]
    fn a_branded_sum_flows_into_a_branded_parameter() {
        eval_with_prelude("let main = () => Scene.cube() |> Scene.rotateY(90deg + 45deg)\n")
            .expect("a summed angle is still an Angle");
    }

    /// Drift guard: every namespace the prelude SHIPS must be reserved by the
    /// language, or a game's `persistence.fun` — or an inline `module
    /// Persistence` — silently steals it. `PROTECTED_NAMESPACES` is a static
    /// list, so without this a new `.funi` lands unprotected by omission
    /// (which is exactly what `Persistence` did, and `Terrain` before it).
    #[test]
    fn every_prelude_module_is_a_protected_namespace() {
        let unprotected: Vec<String> = functor_prelude::modules()
            .into_iter()
            .map(|(name, _)| name)
            .filter(|name| !functor_lang::project::is_protected_namespace(name))
            .collect();
        assert_eq!(
            unprotected,
            Vec::<String>::new(),
            "prelude modules missing from PROTECTED_NAMESPACES (functor-lang/src/project.rs)"
        );
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

    /// Drift guard: each `.funi` `impure[…]` header equals the classes its
    /// built-in notes when called. Every header needs a sample call here, so
    /// a declared class nothing notes fails; a noted class nothing declares
    /// fails `note_read`'s debug assertion.
    #[test]
    fn impure_headers_match_the_reads_each_builtin_notes() {
        use std::collections::BTreeSet;
        let at = "Vec3.make(0.0, 0.0, 0.0), Vec3.make(0.0, -1.0, 0.0), 1.0";
        let samples = [
            ("File.bytes", "File.bytes(Path.of(\"missing.bin\"))".to_string()),
            ("File.exists", "File.exists(Path.of(\"missing.bin\"))".to_string()),
            (
                "Typeface.file",
                "Typeface.file(\"missing.ttf\")".to_string(),
            ),
            (
                "Physics.position",
                "Physics.position(\"nobody\")".to_string(),
            ),
            (
                "Physics.linearVelocity",
                "Physics.linearVelocity(\"nobody\")".to_string(),
            ),
            (
                "Physics.grounded",
                "Physics.grounded(\"nobody\")".to_string(),
            ),
            ("Physics.cast", format!("Physics.cast({at})")),
            (
                "Physics.castExcluding",
                format!("Physics.castExcluding(\"nobody\", {at})"),
            ),
            (
                "Physics.shapeCast",
                format!("Physics.shapeCast(Physics.sphere(0.5), {at})"),
            ),
            (
                "Physics.shapeCastExcluding",
                format!("Physics.shapeCastExcluding(\"nobody\", Physics.sphere(0.5), {at})"),
            ),
            (
                "Physics.transformed",
                "Physics.transformed(\"nobody\", Scene.cube())".to_string(),
            ),
            ("Ui.button", "Ui.button(\"go\", 1.0)".to_string()),
            (
                "Ui.slider",
                "Ui.slider(0.0, 1.0, 0.5, (v) => v)".to_string(),
            ),
            ("Ui.textInput", "Ui.textInput(\"t\", (s) => s)".to_string()),
            ("Attr.onClick", "Attr.onClick(1.0)".to_string()),
            ("Attr.onInput", "Attr.onInput((s) => s)".to_string()),
        ];
        let declared = declared_reads();
        assert_eq!(
            samples
                .iter()
                .map(|(path, _)| *path)
                .collect::<BTreeSet<_>>(),
            declared.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            "every `impure[…]` built-in needs exactly one sample call here"
        );
        for (path, call) in samples {
            let src = format!("let main = () => {call}");
            let module =
                functor_lang::lower(functor_lang::parse(&src).expect("parse")).expect("lower");
            let before = ReadClass::ALL.map(|class| (class, read_count(class)));
            // The call may fail (no project files, no world); the read is noted first.
            let _ = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost);
            let noted: BTreeSet<ReadClass> = before
                .into_iter()
                .filter(|&(class, count)| read_count(class) != count)
                .map(|(class, _)| class)
                .collect();
            let expected: BTreeSet<ReadClass> = declared[path].iter().copied().collect();
            assert_eq!(
                noted, expected,
                "`{path}`: noted reads vs its `impure[…]` header"
            );
        }
        take_ui_handlers();
    }

    /// A built-in that notes a read its `.funi` doesn't declare fails loudly.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(
        expected = "`Physics.peek` reads physics state, but its .funi signature \
doesn't declare it — add `impure[physics]` above it"
    )]
    fn an_undeclared_read_fails_the_debug_assertion() {
        let mut reg = crate::host_registry::Registry::default();
        reg.fn0("Physics.peek", "Physics.peek()", || {
            note_read(ReadClass::Physics);
            Value::Bool(true)
        });
        let _ = reg.call("Physics.peek", &[], Span::new(0, 0));
    }

    // The C1 verify criterion (docs/functor-lang.md): an .fun snippet emits exactly
    // the protocol data the shells consume — pinned as the serialized wire
    // form the protocol tests use.
    #[test]
    fn functor_lang_snippet_emits_protocol_frame() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())",
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

    #[test]
    fn tracked_pose_maps_through_the_authored_camera_rig() {
        let value = eval(
            "let main = () =>\n\
             Camera3D.mapTrackedPose(\n\
               Camera3D.lookAt(Vec3.make(4.0, 2.0, -3.0), Vec3.make(4.0, 2.0, -2.0)),\n\
               { position: { x: 0.0, y: 0.0, z: 0.0 }, \
                 orientation: { x: 0.0, y: 0.0, z: 0.0, w: 1.0 } })",
        );
        assert_eq!(
            value.to_string(),
            "{ position: { x: 4, y: 2, z: -3 }, \
forward: { x: 0, y: 0, z: 1 }, up: { x: 0, y: 1, z: 0 } }"
        );
    }

    fn camera_of(src: &str) -> Camera {
        let Value::HostData(data) = eval(src) else {
            panic!("expected Camera host data");
        };
        data.as_any()
            .downcast_ref::<FunctorLangCamera>()
            .expect("Camera")
            .0
            .clone()
    }

    #[test]
    fn camera_from_pose_keeps_fov_and_clip() {
        let camera = camera_of(
            "let main = () =>\n\
             Camera3D.fromPose(\n\
               { position: { x: 1.0, y: 2.0, z: 3.0 }, forward: { x: 0.0, y: 0.0, z: 2.0 }, \
                 up: { x: 1.0, y: 0.0, z: 0.0 } },\n\
               Angle.degrees(6.0))\n\
             |> Camera3D.clip(1.0, 700.0)",
        );
        assert_eq!(camera.eye, [1.0, 2.0, 3.0]);
        assert_eq!(camera.target, [1.0, 2.0, 4.0]);
        assert_eq!(camera.up, [1.0, 0.0, 0.0]);
        assert!((camera.fov_radians - 6.0_f32.to_radians()).abs() < 1e-6);
        assert_eq!((camera.near, camera.far), (1.0, 700.0));
        assert_eq!(
            fail_message(
                "let main = () => Camera3D.fromPose(\n\
                   { position: { x: 0.0, y: 0.0, z: 0.0 }, forward: { x: 0.0, y: 1.0, z: 0.0 }, \
                     up: { x: 0.0, y: 2.0, z: 0.0 } }, Angle.degrees(60.0))"
            ),
            "Camera3D.fromPose: forward and up must be non-zero and not parallel"
        );
        assert_eq!(
            fail_message(
                "let main = () => Camera3D.fromPose(\n\
                   { position: { x: 0.0, y: 0.0, z: 0.0 }, forward: { x: 0.0, y: 1.0, z: 0.0 } }, \
                   Angle.degrees(60.0))"
            ),
            "Camera3D.fromPose: Camera3D.mappedPose is missing `up`"
        );
    }

    // The pipeline a held scope uses: a tracked pose through the authored
    // camera, then fromPose. Identity reproduces the authored firstPerson
    // camera; a controller rolled +90° about rig +Z (its top turned to the
    // player's left) rolls the camera's up to the world left of a +Z gaze.
    #[test]
    fn camera_from_mapped_tracked_pose_follows_the_controller() {
        let pose = |q: &str| {
            camera_of(&format!(
                "let rig = Camera3D.firstPerson(Vec3.make(0.0, 1.5, 0.0), Angle.radians(0.0), \
                   Angle.radians(0.0), Angle.degrees(60.0))\n\
                 let main = () => Camera3D.fromPose(Camera3D.mapTrackedPose(rig, \
                   {{ position: {{ x: 0.0, y: 0.0, z: 0.0 }}, orientation: {q} }}), \
                   Angle.degrees(60.0))"
            ))
        };
        let level = pose("{ x: 0.0, y: 0.0, z: 0.0, w: 1.0 }");
        let fp = Camera::first_person(
            [0.0, 1.5, 0.0],
            crate::math::Angle::from_degrees(0.0),
            crate::math::Angle::from_degrees(0.0),
            crate::math::Angle::from_degrees(60.0),
        );
        let (a, b) = (level.view_matrix(), fp.view_matrix());
        let (a, b): (&[f32; 16], &[f32; 16]) = (a.as_ref(), b.as_ref());
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5), "{a:?} != {b:?}");

        let half = std::f64::consts::FRAC_1_SQRT_2;
        let rolled = pose(&format!("{{ x: 0.0, y: 0.0, z: {half}, w: {half} }}"));
        assert!(rolled.up[0] > 0.999, "rolled up {:?}", rolled.up);
        assert!((rolled.target[2] - rolled.eye[2] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn camera_world_ray_is_normalized_and_uses_vec3_fields() {
        let value = eval(
            "let main = () =>\n\
             Camera3D.toWorldRay(\n\
               { x: 400.0, y: 300.0, surfaceWidth: 800.0, surfaceHeight: 600.0 },\n\
               Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)))",
        );
        let Value::Variant { ctor, args } = value else {
            panic!("world ray should return an Option");
        };
        assert_eq!(ctor.as_ref(), "Option.Some");
        let [Value::Record(fields)] = args.as_slice() else {
            panic!("Some should carry the ray record");
        };
        let field = |name: &str| {
            &fields
                .iter()
                .find(|(field, _)| field == name)
                .unwrap_or_else(|| panic!("ray should contain `{name}`"))
                .1
        };
        let span = Span::new(0, 0);
        assert_eq!(
            vec3_of(field("origin"), "ray origin", span).unwrap(),
            (0.0, 0.0, -5.0)
        );
        assert_eq!(
            vec3_of(field("direction"), "ray direction", span).unwrap(),
            (0.0, 0.0, 1.0)
        );
    }

    #[test]
    fn camera_world_ray_returns_none_for_an_invalid_surface() {
        let value = eval(
            "let main = () => Camera3D.toWorldRay(\n\
               { x: 0.0, y: 0.0, surfaceWidth: 0.0, surfaceHeight: 600.0 },\n\
               Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)))",
        );
        assert!(matches!(
            value,
            Value::Variant { ref ctor, ref args }
                if ctor.as_ref() == "Option.None" && args.is_empty()
        ));
    }

    #[test]
    fn sprite_values_are_plain_inspectable_data() {
        let sprite = eval(
            "let main = () =>\n\
             Sprite.square(Color.rgb(1.0, 0.5, 0.25), 2.0)\n\
               |> Sprite.move(3.0, 4.0)",
        );
        assert!(
            !matches!(sprite, Value::HostData(_)),
            "Sprite.t must stay plain Functor Lang data"
        );
        assert!(sprite.is_reload_safe_snapshot());
        assert_eq!(
            sprite.to_string(),
            "Sprite.Move(3, 4, Sprite.Rectangle(2, 2, 1, 0.5, 0.25))"
        );
        assert!(matches!(
            eval(
                "let a = () => Sprite.square(Color.rgb(1.0, 0.5, 0.25), 2.0)\n\
                 let main = () => a() == a()"
            ),
            Value::Bool(true)
        ));
    }

    #[test]
    fn sprite_regions_and_sampling_are_plain_inspectable_data() {
        let sprite = eval(
            "let main = () =>\n\
             Sprite.imageRegion(\n\
               2.0, 3.0,\n\
               Sprite.region(96.0, 0.0, 96.0, 96.0),\n\
               Asset.texture(\"hero-atlas.png\"))\n\
             |> Sprite.nearest()",
        );
        assert!(
            !matches!(sprite, Value::HostData(_)),
            "atlas sprites must stay plain Functor Lang data"
        );
        assert!(sprite.is_reload_safe_snapshot());
        assert_eq!(
            sprite.to_string(),
            "Sprite.Nearest(Sprite.ImageRegion(2, 3, 96, 0, 96, 96, \
\"hero-atlas.png\", []))"
        );
    }

    #[test]
    fn create2d_lowers_sprite_data_into_a_serializable_layer() {
        let frame = frame_of(
            "let main = () =>\n\
             Sprite.image(2.0, 3.0, Asset.texture(\"hero.png\"))\n\
               |> Sprite.tint(Color.rgb(0.5, 1.0, 0.25))\n\
               |> Sprite.fade(0.75)\n\
               |> Frame.create2D(\n\
                    Camera2D.create(16.0, 9.0)\n\
                      |> Camera2D.at(2.0, -1.0)\n\
                      |> Camera2D.zoom(1.5))",
        );
        assert!(matches!(
            &frame.scene.obj,
            SceneObject::Group(items) if items.is_empty()
        ));
        assert_eq!(frame.sprite_layers.len(), 1);
        assert_eq!(
            frame.sprite_layers[0].camera,
            Camera2D::new(16.0, 9.0)
                .with_center(2.0, -1.0)
                .with_zoom(1.5)
        );
        let json = serde_json::to_string(&frame).expect("sprite frame serializes");
        assert!(json.contains(r#""sprite_layers""#), "json: {json}");
        assert!(json.contains(r#""FileClamped""#), "json: {json}");
        assert!(json.contains(r#""hero.png""#), "json: {json}");
        assert!(json.contains(r#""sampling":"Linear""#), "json: {json}");
        let back: Frame = serde_json::from_str(&json).expect("sprite frame deserializes");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn camera2d_to_world_maps_input_mouse_and_rejects_bars() {
        let center = eval(
            "let main = () =>\n\
             Camera2D.toWorld(\n\
               { x: 500.0, y: 500.0, surfaceWidth: 1000.0, surfaceHeight: 1000.0,\n\
                 buttons: { left: false, right: false, middle: false } },\n\
               Camera2D.create(16.0, 9.0) |> Camera2D.at(4.0, -3.0) |> Camera2D.zoom(2.0))",
        );
        assert!(matches!(
            center,
            Value::Variant { ref ctor, ref args }
                if ctor.as_ref() == "Option.Some"
                    && matches!(
                        args.first(),
                        Some(Value::Record(fields))
                            if matches!(
                                fields.as_slice(),
                                [(x_name, Value::Number(x)), (y_name, Value::Number(y))]
                                    if x_name == "x" && y_name == "y" && *x == 4.0 && *y == -3.0
                            )
                    )
        ));

        let bar = eval(
            "let main = () =>\n\
             Camera2D.toWorld(\n\
               { x: 500.0, y: 100.0, surfaceWidth: 1000.0, surfaceHeight: 1000.0,\n\
                 buttons: { left: false, right: false, middle: false } },\n\
               Camera2D.create(16.0, 9.0))",
        );
        assert!(matches!(
            bar,
            Value::Variant { ref ctor, ref args }
                if ctor.as_ref() == "Option.None" && args.is_empty()
        ));
    }

    #[test]
    fn create2d_lowers_atlas_pixels_and_sampling_into_the_material() {
        let frame = frame_of(
            "let main = () =>\n\
             Sprite.imageRegion(\n\
               1.6, 1.6,\n\
               Sprite.region(192.0, 0.0, 96.0, 96.0),\n\
               Asset.texture(\"hero-atlas.png\"))\n\
             |> Sprite.nearest()\n\
             |> Frame.create2D(Camera2D.create(24.0, 13.5))",
        );
        let json = serde_json::to_string(&frame).expect("atlas frame serializes");
        assert!(json.contains(r#""SpriteTexture""#), "json: {json}");
        assert!(
            json.contains(r#""source_pixels":[192.0,0.0,96.0,96.0]"#),
            "json: {json}"
        );
        assert!(json.contains(r#""sampling":"Nearest""#), "json: {json}");
        assert!(json.contains(r#""hero-atlas.png""#), "json: {json}");
        let back: Frame = serde_json::from_str(&json).expect("atlas frame deserializes");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn filled_shapes_are_plain_inspectable_data() {
        // Same discipline as text: the node keeps the AUTHOR'S parameters, and
        // triangulation happens later at lowering — so a picture containing a
        // circle still compares, serializes, and time-travels as plain data
        // instead of carrying 32 expanded vertices around in the model.
        let circle = eval("let main = () => Sprite.circle(Color.rgb(1.0, 0.5, 0.25), 3.0)");
        assert!(!matches!(circle, Value::HostData(_)));
        assert!(circle.is_reload_safe_snapshot());
        assert_eq!(circle.to_string(), "Sprite.Circle(3, 1, 0.5, 0.25)");

        let line = eval(
            "let main = () =>\n\
             Sprite.line(Color.rgb(1.0, 1.0, 1.0), 0.25, { x: 0.0, y: 0.0 }, { x: 4.0, y: 3.0 })",
        );
        assert!(line.is_reload_safe_snapshot());
        assert_eq!(line.to_string(), "Sprite.Line(0.25, 0, 0, 4, 3, 1, 1, 1)");

        let polygon = eval(
            "let main = () =>\n\
             Sprite.polygon(Color.rgb(0.0, 1.0, 0.0), [\n\
               { x: 0.0, y: 0.0 }, { x: 2.0, y: 0.0 }, { x: 1.0, y: 1.5 }])",
        );
        assert!(polygon.is_reload_safe_snapshot());
        assert_eq!(
            polygon.to_string(),
            "Sprite.Polygon([{ x: 0, y: 0 }, { x: 2, y: 0 }, { x: 1, y: 1.5 }], 0, 1, 0)"
        );
        // Structural equality reaches the points.
        assert!(matches!(
            eval(
                "let tri = (h) =>\n\
                 Sprite.polygon(Color.rgb(1.0, 1.0, 1.0), [\n\
                   { x: 0.0, y: 0.0 }, { x: 2.0, y: 0.0 }, { x: 1.0, y: h }])\n\
                 let main = () => tri(1.5) == tri(1.5) && not (tri(1.5) == tri(2.0))"
            ),
            Value::Bool(true)
        ));
    }

    #[test]
    fn a_concave_outline_is_rejected_rather_than_filled_wrongly() {
        // The fill is a triangle fan, so a concave outline would paint outside
        // the shape. That must be an error at the construction site, not a
        // silently wrong picture.
        let arrow = "Sprite.polygon(Color.rgb(1.0, 1.0, 1.0), [\
                     { x: 0.0, y: 0.0 }, { x: 4.0, y: 0.0 }, \
                     { x: 2.0, y: 1.0 }, { x: 4.0, y: 3.0 }, { x: 0.0, y: 3.0 }])";
        let message = run_fail(&format!("let main = () => {arrow}"));
        assert!(
            message.contains("must form a CONVEX outline"),
            "message: {message}"
        );

        // A STAR is the subtle case: it turns the same way at every vertex, so a
        // sign-consistency check alone accepts it, and the fan then fills
        // overlapping triangles instead of the star. A pentagram winds twice.
        let pentagram = "Sprite.polygon(Color.rgb(1.0, 1.0, 1.0), [\
                         { x: 1.0, y: 0.0 }, { x: -0.809, y: 0.588 }, \
                         { x: 0.309, y: -0.951 }, { x: 0.309, y: 0.951 }, \
                         { x: -0.809, y: -0.588 }])";
        let message = run_fail(&format!("let main = () => {pentagram}"));
        assert!(
            message.contains("wind around 2.0 times"),
            "message: {message}"
        );

        // ...and the legitimate cases still work, in EITHER winding.
        for winding in [
            "[{ x: 0.0, y: 0.0 }, { x: 2.0, y: 0.0 }, { x: 2.0, y: 2.0 }, { x: 0.0, y: 2.0 }]",
            "[{ x: 0.0, y: 2.0 }, { x: 2.0, y: 2.0 }, { x: 2.0, y: 0.0 }, { x: 0.0, y: 0.0 }]",
        ] {
            let value = eval(&format!(
                "let main = () => Sprite.polygon(Color.rgb(1.0, 1.0, 1.0), {winding})"
            ));
            assert!(matches!(value, Value::Variant { .. }), "winding: {winding}");
        }

        // A 180-degree REVERSAL is the subtle one that a turning test alone misses:
        // it contributes exactly pi, so a self-intersecting outline containing one
        // can still total a single revolution and pass. This outline has
        // same-signed turns and totals -2pi, yet edge (0,1)->(2,1) crosses the
        // closing edge (3,2)->(0,0).
        let spiked = "Sprite.polygon(Color.rgb(1.0, 1.0, 1.0), [\
                      { x: 0.0, y: 0.0 }, { x: 0.0, y: 1.0 }, { x: 2.0, y: 1.0 }, \
                      { x: 1.0, y: 1.0 }, { x: 3.0, y: 2.0 }])";
        let message = run_fail(&format!("let main = () => {spiked}"));
        assert!(message.contains("doubles back on itself"), "message: {message}");

        // The 32-gon a circle lowers to must still pass, or circles break.
        let ring = eval(
            "let main = () =>\n\
             Sprite.polygon(Color.rgb(1.0, 1.0, 1.0),\n\
               List.range(32.0) |> List.map((i) =>\n\
                 let a = i / 32.0 * 2.0 * Math.pi in\n\
                 { x: Math.cos(a), y: Math.sin(a) }))",
        );
        assert!(matches!(ring, Value::Variant { .. }));

        // A collinear vertex is convex, so it is allowed.
        let straightened = eval(
            "let main = () =>\n\
             Sprite.polygon(Color.rgb(1.0, 1.0, 1.0), [\n\
               { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 }, { x: 2.0, y: 0.0 }, { x: 1.0, y: 2.0 }])",
        );
        assert!(matches!(straightened, Value::Variant { .. }));
    }

    #[test]
    fn shape_domains_fail_loudly() {
        for (src, expected) in [
            (
                "Sprite.circle(Color.rgb(1.0, 1.0, 1.0), 0.0)",
                "Sprite.circle radius must be a positive number",
            ),
            (
                "Sprite.line(Color.rgb(1.0, 1.0, 1.0), -1.0, { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 })",
                "Sprite.line thickness must be a positive number",
            ),
            (
                "Sprite.polygon(Color.rgb(1.0, 1.0, 1.0), [{ x: 0.0, y: 0.0 }, { x: 1.0, y: 1.0 }])",
                "needs at least 3 points",
            ),
            // All on one line: no area to fill.
            (
                "Sprite.polygon(Color.rgb(1.0, 1.0, 1.0), [\
                 { x: 0.0, y: 0.0 }, { x: 1.0, y: 1.0 }, { x: 2.0, y: 2.0 }])",
                "must enclose an area",
            ),
            // A point record is required — a tuple or bare number is not one.
            (
                "Sprite.polygon(Color.rgb(1.0, 1.0, 1.0), [1.0, 2.0, 3.0])",
                "expected a point record",
            ),
            // A tiny-but-positive f64 narrows to exactly 0.0 as f32, which would
            // render invisibly instead of erroring (1e-60, spelled without
            // scientific notation since the language has no such literal).
            (
                "Sprite.circle(Color.rgb(1.0, 1.0, 1.0), 1.0 / (100000000000000000000.0 \
                 * 100000000000000000000.0 * 100000000000000000000.0))",
                "Sprite.circle radius must be a positive number",
            ),
            (
                "Sprite.line(Color.rgb(1.0, 1.0, 1.0), 1.0 / (100000000000000000000.0 \
                 * 100000000000000000000.0 * 100000000000000000000.0), \
                 { x: 0.0, y: 0.0 }, { x: 1.0, y: 0.0 })",
                "Sprite.line thickness must be a positive number",
            ),
            // Each endpoint is representable as f32 (2e38 < f32::MAX) but their
            // SPAN (4e38) is not, so the length must be rejected rather than
            // becoming a silently infinite transform. `hypot` already covers the
            // merely-large cases that squaring would have overflowed; this is the
            // residue it cannot save.
            (
                "Sprite.line(Color.rgb(1.0, 1.0, 1.0), 1.0, \
                 { x: 0.0 - 2.0 * 10000000000000000000.0 * 10000000000000000000.0, y: 0.0 }, \
                 { x: 2.0 * 10000000000000000000.0 * 10000000000000000000.0, y: 0.0 })",
                "too far apart to place",
            ),
            (
                "Sprite.line(Color.rgb(1.0, 1.0, 1.0), 1.0, { x: 0.0 }, { x: 1.0, y: 0.0 })",
                "missing `y`",
            ),
        ] {
            let message = run_fail(&format!("let main = () => {src}"));
            assert!(
                message.contains(expected),
                "`{src}` should contain `{expected}`, got `{message}`"
            );
        }
    }

    #[test]
    fn create2d_lowers_shapes_into_serializable_geometry() {
        let frame = frame_of(
            "let main = () =>\n\
             Sprite.group([\n\
               Sprite.circle(Color.rgb(1.0, 0.0, 0.0), 2.0),\n\
               Sprite.polygon(Color.rgb(0.0, 1.0, 0.0), [\n\
                 { x: 0.0, y: 0.0 }, { x: 2.0, y: 0.0 }, { x: 1.0, y: 1.5 }]),\n\
             ])\n\
             |> Frame.create2D(Camera2D.create(16.0, 9.0))",
        );
        let json = serde_json::to_string(&frame).expect("shape frame serializes");
        assert_eq!(json.matches(r#""ConvexPolygon""#).count(), 2, "json: {json}");
        // The author's triangle survives verbatim — points are NOT re-centered.
        assert!(
            json.contains(r#""points":[[0.0,0.0],[2.0,0.0],[1.0,1.5]]"#),
            "json: {json}"
        );
        // The circle is the unit ring (radius comes from the transform), so every
        // circle in a frame shares one cached mesh.
        assert!(json.contains(r#"[1.0,0.0]"#), "unit ring: {json}");
        let back: Frame = serde_json::from_str(&json).expect("shape frame deserializes");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn a_zero_length_line_draws_nothing() {
        let frame = frame_of(
            "let main = () =>\n\
             Sprite.line(Color.rgb(1.0, 1.0, 1.0), 0.5, { x: 1.0, y: 1.0 }, { x: 1.0, y: 1.0 })\n\
             |> Frame.create2D(Camera2D.create(16.0, 9.0))",
        );
        let json = serde_json::to_string(&frame).expect("line frame serializes");
        assert!(!json.contains("Emissive"), "no draw call: {json}");
    }

    #[test]
    fn a_line_is_thick_across_its_own_axis_at_any_angle() {
        // The artifact this avoids: building the quad then rotating the assembled
        // GROUP makes thickness vary with angle. Thickness is applied in the
        // segment's own frame, so a 45-degree line is as thick as a flat one.
        let width_of = |to: &str| {
            let frame = frame_of(&format!(
                "let main = () =>\n\
                 Sprite.line(Color.rgb(1.0, 1.0, 1.0), 0.5, {{ x: 0.0, y: 0.0 }}, {to})\n\
                 |> Frame.create2D(Camera2D.create(16.0, 9.0))"
            ));
            // The leaf transform's basis: column 0 is the length axis, column 1
            // the thickness axis. Its magnitude IS the rendered thickness.
            fn thickness(scene: &Scene3D) -> Option<f32> {
                if let SceneObject::Material(_, _) = &scene.obj {
                    return None;
                }
                let m = scene.xform;
                let candidate = (m.y.x * m.y.x + m.y.y * m.y.y).sqrt();
                match &scene.obj {
                    SceneObject::Group(items) => items
                        .iter()
                        .find_map(thickness)
                        .or(if candidate > 0.0 { Some(candidate) } else { None }),
                    _ => Some(candidate),
                }
            }
            thickness(&frame.sprite_layers[0].scene).expect("a line lowers to a transform")
        };
        let flat = width_of("{ x: 4.0, y: 0.0 }");
        let diagonal = width_of("{ x: 3.0, y: 3.0 }");
        let steep = width_of("{ x: 1.0, y: 6.0 }");
        for (label, value) in [("flat", flat), ("diagonal", diagonal), ("steep", steep)] {
            assert!(
                (value - 0.5).abs() < 1e-5,
                "{label} line thickness should be 0.5, got {value}"
            );
        }
    }

    #[test]
    fn sprite_text_keeps_its_string_as_plain_inspectable_data() {
        // The whole point of expanding glyphs at LOWERING rather than in the
        // value: a picture containing text is still ordinary comparable,
        // serializable, time-travellable data carrying the string itself.
        let sprite = eval(
            "let main = () =>\n\
             Sprite.text(Color.rgb(1.0, 0.5, 0.25), 2.0, \"HI\")\n\
               |> Sprite.move(3.0, 4.0)",
        );
        assert!(
            !matches!(sprite, Value::HostData(_)),
            "a text sprite must stay plain Functor Lang data"
        );
        assert!(sprite.is_reload_safe_snapshot());
        assert_eq!(
            sprite.to_string(),
            "Sprite.Move(3, 4, Sprite.Text(2, 1, 0.5, 0.25, \"HI\"))"
        );
        assert!(matches!(
            eval(
                "let a = () => Sprite.text(Color.rgb(1.0, 1.0, 1.0), 2.0, \"HI\")\n\
                 let main = () => a() == a()"
            ),
            Value::Bool(true)
        ));
        // Different strings are different pictures — equality reaches the text.
        assert!(matches!(
            eval(
                "let main = () =>\n\
                 Sprite.text(Color.rgb(1.0, 1.0, 1.0), 2.0, \"HI\")\n\
                   == Sprite.text(Color.rgb(1.0, 1.0, 1.0), 2.0, \"HO\")"
            ),
            Value::Bool(false)
        ));
    }

    #[test]
    fn sprite_measure_reports_monospace_metrics_in_world_units() {
        let width = |src: &str| match eval(src) {
            Value::Number(n) => n,
            other => panic!("expected a number, got {other}"),
        };
        // The built-in font is monospace: width is exactly size * characters.
        assert_eq!(width("let main = () => Sprite.measure(2.0, \"SCORE\").width"), 10.0);
        assert_eq!(width("let main = () => Sprite.measure(0.5, \"SCORE\").width"), 2.5);
        assert_eq!(width("let main = () => Sprite.measure(2.0, \"\").width"), 0.0);
        // Height is the LINE height for every string, including the empty one,
        // so stacking lines by `measure(...).height` cannot overlap them — the
        // layout bug that is invisible in code review.
        assert_eq!(width("let main = () => Sprite.measure(2.0, \"SCORE\").height"), 2.0);
        assert_eq!(width("let main = () => Sprite.measure(2.0, \"\").height"), 2.0);
    }

    #[test]
    fn sprite_measure_counts_every_character_that_text_advances_past() {
        // measure() and text() must agree exactly, including on characters that
        // draw nothing: blanks occupy their cell so unsupported text reads as
        // gaps instead of sliding the rest of the line left.
        let width = |src: &str| match eval(src) {
            Value::Number(n) => n,
            other => panic!("expected a number, got {other}"),
        };
        assert_eq!(width("let main = () => Sprite.measure(1.0, \"A B\").width"), 3.0);

        // ...and only the visible glyphs emit a quad.
        assert_eq!(glyph_quads("A B"), 2, "the space draws nothing");
        assert_eq!(glyph_quads("SCORE"), 5);
        assert_eq!(glyph_quads(""), 0);
        assert_eq!(glyph_quads("café"), 3, "the non-ASCII scalar is blank");
    }

    /// How many atlas-textured quads `text` lowers to — one per visible glyph.
    fn glyph_quads(text: &str) -> usize {
        let frame = frame_of(&format!(
            "let main = () =>\n\
             Sprite.text(Color.rgb(1.0, 1.0, 1.0), 1.0, \"{text}\")\n\
             |> Frame.create2D(Camera2D.create(16.0, 9.0))"
        ));
        serde_json::to_string(&frame)
            .expect("text frame serializes")
            .matches(r#""Builtin""#)
            .count()
    }

    #[test]
    fn sprite_text_breaks_lines_on_a_newline_and_measure_agrees() {
        let metric = |src: &str| match eval(src) {
            Value::Number(n) => n,
            other => panic!("expected a number, got {other}"),
        };
        // Width is the WIDEST line; height is one `size` per line. Both stay
        // exact, so a block still stacks against its own measured height.
        assert_eq!(
            metric("let main = () => Sprite.measure(2.0, \"HI\\nSCORE\").width"),
            10.0
        );
        assert_eq!(
            metric("let main = () => Sprite.measure(2.0, \"HI\\nSCORE\").height"),
            4.0
        );
        // A trailing newline is a real, empty final line.
        assert_eq!(metric("let main = () => Sprite.measure(2.0, \"HI\\n\").height"), 4.0);
        assert_eq!(metric("let main = () => Sprite.measure(2.0, \"HI\\n\").width"), 4.0);
        // An empty line in the middle keeps its row.
        assert_eq!(
            metric("let main = () => Sprite.measure(1.0, \"HI\\n\\nHO\").height"),
            3.0
        );

        // The newline itself never draws, and never consumes a cell of width.
        assert_eq!(glyph_quads("HI\\nHO"), 4);
        assert_eq!(glyph_quads("HI\\n"), 2);

        // The two lines land one full `size` apart, centered on the block: with
        // two lines of size 2 the row centers are +1 and -1.
        let frame = frame_of(
            "let main = () =>\n\
             Sprite.text(Color.rgb(1.0, 1.0, 1.0), 2.0, \"I\\nI\")\n\
             |> Frame.create2D(Camera2D.create(16.0, 9.0))",
        );
        let ys = glyph_translations_y(&frame.sprite_layers[0].scene);
        assert_eq!(ys, vec![1.0, -1.0], "top line first, one size apart");
    }

    /// The accumulated Y translation of every glyph quad in a lowered text run,
    /// in lowering order. Transforms are `Group` wrappers carrying an `xform`,
    /// so summing `xform.w.y` down to each material leaf gives the glyph's
    /// world Y.
    fn glyph_translations_y(scene: &Scene3D) -> Vec<f32> {
        let y = scene.xform.w.y;
        match &scene.obj {
            SceneObject::Group(items) | SceneObject::Material(_, items) => items
                .iter()
                .flat_map(glyph_translations_y)
                .map(|inner| inner + y)
                .collect(),
            _ => vec![y],
        }
    }

    #[test]
    fn create2d_lowers_text_into_one_atlas_quad_per_visible_glyph() {
        let frame = frame_of(
            "let main = () =>\n\
             Sprite.text(Color.rgb(1.0, 1.0, 1.0), 2.0, \"HI\")\n\
             |> Sprite.nearest()\n\
             |> Frame.create2D(Camera2D.create(16.0, 9.0))",
        );
        let json = serde_json::to_string(&frame).expect("text frame serializes");
        // The atlas is named symbolically, never as a locator a game could
        // collide with, and carries no file path at all.
        assert!(json.contains(r#""Builtin":"FontAtlas""#), "json: {json}");
        assert!(!json.contains(r#""FileClamped""#), "json: {json}");
        assert_eq!(json.matches(r#""Builtin""#).count(), 2, "json: {json}");
        // 'H' is U+0048, cell 40 => column 8, row 2 of the 16-wide atlas.
        assert!(
            json.contains(r#""source_pixels":[64.0,16.0,8.0,8.0]"#),
            "json: {json}"
        );
        // Text honors the ambient sampling context like any other sprite image.
        assert!(json.contains(r#""sampling":"Nearest""#), "json: {json}");
        let back: Frame = serde_json::from_str(&json).expect("text frame deserializes");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn inner_sprite_sampling_overrides_an_outer_group_choice() {
        let frame = frame_of(
            "let main = () =>\n\
             Sprite.group([\n\
               Sprite.image(1.0, 1.0, Asset.texture(\"nearest.png\")),\n\
               Sprite.image(1.0, 1.0, Asset.texture(\"linear.png\"))\n\
                 |> Sprite.linear(),\n\
             ])\n\
             |> Sprite.nearest()\n\
             |> Frame.create2D(Camera2D.create(16.0, 9.0))",
        );
        let json = serde_json::to_string(&frame).expect("sampling frame serializes");
        assert_eq!(json.matches(r#""sampling":"Nearest""#).count(), 1);
        assert_eq!(json.matches(r#""sampling":"Linear""#).count(), 1);
    }

    #[test]
    fn sprite_and_camera_domains_fail_loudly() {
        for (src, expected) in [
            (
                "Sprite.rectangle(Color.rgb(1.0, 1.0, 1.0), 0.0, 2.0)",
                "Sprite.rectangle width and height must be positive",
            ),
            (
                "Sprite.text(Color.rgb(1.0, 1.0, 1.0), 0.0, \"HI\")",
                "Sprite.text size must be a positive number",
            ),
            // NaN never reaches the size check — the registry's numeric
            // conversion rejects non-finite arguments first.
            (
                "Sprite.text(Color.rgb(1.0, 1.0, 1.0), 0.0 / 0.0, \"HI\")",
                "expected a finite number, got NaN",
            ),
            // A finite f64 that OVERFLOWS f32 (1e40) is already refused by the
            // registry's numeric conversion, which requires f32-finiteness.
            (
                "Sprite.text(Color.rgb(1.0, 1.0, 1.0), \
                 100000000000000000000.0 * 100000000000000000000.0, \"HI\")",
                "expected a finite number",
            ),
            // ...and one that UNDERFLOWS f32 to zero (1e-60), which would
            // otherwise let measure report a positive box while text drew
            // degenerate quads.
            (
                "Sprite.text(Color.rgb(1.0, 1.0, 1.0), 1.0 / (100000000000000000000.0 \
                 * 100000000000000000000.0 * 100000000000000000000.0), \"HI\")",
                "Sprite.text size must be a positive number",
            ),
            (
                "Sprite.measure(-1.0, \"HI\")",
                "Sprite.measure size must be a positive number",
            ),
            (
                "Sprite.measure(1.0 / (100000000000000000000.0 \
                 * 100000000000000000000.0 * 100000000000000000000.0), \"HI\")",
                "Sprite.measure size must be a positive number",
            ),
            (
                "Sprite.image(-1.0, 2.0, Asset.texture(\"hero.png\"))",
                "Sprite.image width and height must be positive",
            ),
            (
                "Sprite.imageRegion(-1.0, 2.0, Sprite.region(0.0, 0.0, 16.0, 16.0), Asset.texture(\"hero.png\"))",
                "Sprite.imageRegion width and height must be positive",
            ),
            (
                "Sprite.region(-1.0, 0.0, 16.0, 16.0)",
                "Sprite.region x and y must be non-negative",
            ),
            (
                "Sprite.region(0.0, 0.0, 0.0, 16.0)",
                "Sprite.region width and height must be positive",
            ),
            (
                "Sprite.region(0.5, 0.0, 16.0, 16.0)",
                "Sprite.region coordinates and size must be whole source pixels",
            ),
            (
                "Sprite.square(Color.rgb(1.0, 1.0, 1.0), 2.0) |> Sprite.fade(1.5)",
                "Sprite.fade alpha must be between 0 and 1",
            ),
            (
                "Scene.cube() |> Scene.opacity(1.5)",
                "Scene.opacity alpha must be between 0 and 1",
            ),
            (
                "Scene.cube() |> Scene.opacity(-0.1)",
                "Scene.opacity alpha must be between 0 and 1",
            ),
            (
                "Camera2D.create(16.0, 0.0)",
                "Camera2D.create width and height must be positive",
            ),
            (
                "Camera2D.create(16.0, 9.0) |> Camera2D.zoom(0.0)",
                "Camera2D.zoom scale must be positive",
            ),
            (
                "Camera2D.create(1.0 / 0.0, 9.0)",
                "expected a finite number",
            ),
            (
                "Camera2D.create(16.0, 9.0) |> Camera2D.at(0.0 / 0.0, 0.0)",
                "expected a finite number",
            ),
            (
                "Camera2D.create(16.0, 9.0) |> Camera2D.zoom(100000000000000000000.0 * 100000000000000000000.0)",
                "expected a finite number",
            ),
        ] {
            let actual = run_fail(&format!("let main = () => {src}"));
            assert!(
                actual.contains(expected),
                "`{src}` should contain `{expected}`, got `{actual}`"
            );
        }
    }

    // Milestone-0 quirk 1: the renderer drops a Material node's own xform, so
    // a transform applied over Scene.color must survive via a Group wrapper.
    #[test]
    fn transform_over_material_is_not_dropped() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(\n\
               Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
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
               Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
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
               Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
               Scene.cube() |> Scene.scaleXYZ(2.0, 3.0, 4.0))",
        );
        // The wrapper Group carries a non-uniform scale on its diagonal — the
        // three axes stretch independently (unlike Scene.scale's uniform k).
        let s = frame.scene.xform;
        assert_eq!((s.x.x, s.y.y, s.z.z), (2.0, 3.0, 4.0));
    }

    /// `main`'s scene as one world matrix: the product of every wrapper's
    /// xform down a chain of single-child groups to the leaf.
    fn pose_chain_world(src: &str) -> Matrix4<f32> {
        let value = eval(src);
        let mut node = scene_of(&value).expect("main should return a Scene");
        let mut world = node.xform;
        while let SceneObject::Group(children) = &node.obj {
            assert_eq!(children.len(), 1, "expected a single-child chain");
            node = &children[0];
            world = world * node.xform;
        }
        world
    }

    fn pose_src(position: [f32; 3], forward: [f32; 3], up: [f32; 3]) -> String {
        let p = |v: [f32; 3]| format!("{{ x: {:?}, y: {:?}, z: {:?} }}", v[0], v[1], v[2]);
        format!(
            "{{ position: {}, forward: {}, up: {} }}",
            p(position),
            p(forward),
            p(up)
        )
    }

    fn pose_world(pose: &str, inner: &str) -> Matrix4<f32> {
        pose_chain_world(&format!(
            "let main = () => Scene.cube(){inner} |> Scene.atPose({pose})"
        ))
    }

    fn pose_apply(m: Matrix4<f32>, p: [f32; 3]) -> [f32; 3] {
        let v = m * cgmath::vec4(p[0], p[1], p[2], 1.0);
        [v.x, v.y, v.z]
    }

    fn pose_assert_near(a: &[f32], b: &[f32]) {
        assert!(
            a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5),
            "{a:?} != {b:?}"
        );
    }

    fn pose_assert_matrix(a: Matrix4<f32>, b: Matrix4<f32>) {
        let (a, b): (&[f32; 16], &[f32; 16]) = (a.as_ref(), b.as_ref());
        pose_assert_near(a, b);
    }

    // Local +Z goes to `forward` and +Y to `up`: yaw, pitch, and roll poses
    // each reproduce the matching Euler rotate, so a mirrored basis (which
    // flips local +X) cannot pass the roll case.
    #[test]
    fn at_pose_orients_local_z_forward_and_y_up() {
        let origin = [0.0; 3];
        let identity = pose_world(&pose_src(origin, [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]), "");
        pose_assert_matrix(identity, Matrix4::from_scale(1.0));

        let (s, c) = 30f32.to_radians().sin_cos();
        let rotated = |op: &str| {
            pose_chain_world(&format!(
                "let main = () => Scene.cube() |> Scene.{op}(Angle.degrees(30.0))"
            ))
        };
        // Yaw: forward swings from +Z toward +X.
        pose_assert_matrix(
            pose_world(&pose_src(origin, [s, 0.0, c], [0.0, 1.0, 0.0]), ""),
            rotated("rotateY"),
        );
        // Pitch: Scene.rotateX(+30°) tips forward DOWN (+Z toward -Y).
        pose_assert_matrix(
            pose_world(&pose_src(origin, [0.0, -s, c], [0.0, c, s]), ""),
            rotated("rotateX"),
        );
        // Roll: Scene.rotateZ(+30°) turns +Y toward -X.
        let roll = pose_world(&pose_src(origin, [0.0, 0.0, 1.0], [-s, c, 0.0]), "");
        pose_assert_matrix(roll, rotated("rotateZ"));
        pose_assert_near(&pose_apply(roll, [1.0, 0.0, 0.0]), &[c, s, 0.0]);
    }

    // The pose wraps the subtree: inner transforms happen in the pose's local
    // frame, outer ones after it in world space.
    #[test]
    fn at_pose_composes_with_inner_and_outer_transforms() {
        // Yawed 90° (forward +X) at (5, 1, 0).
        let p = pose_src([5.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let inner = pose_world(&p, " |> Scene.translate(Vec3.make(0.0, 0.0, 2.0))");
        pose_assert_near(&pose_apply(inner, [0.0; 3]), &[7.0, 1.0, 0.0]);
        // An inner rotateY(90°) sends local +Z to +X; the pose then to -Z.
        let spun = pose_world(&p, " |> Scene.rotateY(Angle.degrees(90.0))");
        pose_assert_near(&pose_apply(spun, [0.0, 0.0, 1.0]), &[5.0, 1.0, -1.0]);
        let outer = pose_chain_world(&format!(
            "let main = () => Scene.cube() |> Scene.atPose({p}) \
             |> Scene.translate(Vec3.make(0.0, 0.0, 2.0))"
        ));
        pose_assert_near(&pose_apply(outer, [0.0, 0.0, 1.0]), &[6.0, 1.0, 2.0]);
    }

    // `forward` need not be unit length and `up` need not be perpendicular:
    // forward is kept, up re-orthogonalized against it.
    #[test]
    fn at_pose_normalizes_its_basis() {
        pose_assert_matrix(
            pose_world(
                &pose_src([1.0, 2.0, 3.0], [0.0, 0.0, 5.0], [0.0, 2.0, 0.7]),
                "",
            ),
            Matrix4::from_translation(cgmath::vec3(1.0, 2.0, 3.0)),
        );
    }

    // The held-object pipeline against OpenXR ground truth: a controller
    // rolled +90° about rig +Z turns its top to the player's left. Under a
    // rig looking down world +Z that is world +X, and the held object's
    // local +X (the hand's left) points world DOWN. A mirrored basis puts it
    // up.
    #[test]
    fn at_pose_follows_a_rolled_tracked_controller() {
        let world = |q: &str| {
            pose_chain_world(&format!(
                "let rig = Camera3D.firstPerson(Vec3.make(0.0, 1.5, 0.0), Angle.radians(0.0), \
                   Angle.radians(0.0), Angle.degrees(60.0))\n\
                 let main = () => Scene.cube() |> Scene.atPose(Camera3D.mapTrackedPose(rig, \
                   {{ position: {{ x: 0.0, y: 0.0, z: -0.5 }}, orientation: {q} }}))"
            ))
        };
        let level = world("{ x: 0.0, y: 0.0, z: 0.0, w: 1.0 }");
        pose_assert_near(&pose_apply(level, [0.0; 3]), &[0.0, 1.5, 0.5]);
        pose_assert_near(&pose_apply(level, [0.0, 0.0, 1.0]), &[0.0, 1.5, 1.5]);
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let rolled = world(&format!("{{ x: 0.0, y: 0.0, z: {half}, w: {half} }}"));
        pose_assert_near(&pose_apply(rolled, [0.0, 0.0, 1.0]), &[0.0, 1.5, 1.5]);
        pose_assert_near(&pose_apply(rolled, [0.0, 1.0, 0.0]), &[1.0, 1.5, 0.5]);
        pose_assert_near(&pose_apply(rolled, [1.0, 0.0, 0.0]), &[0.0, 0.5, 0.5]);
    }

    #[test]
    fn at_pose_rejects_degenerate_poses() {
        let fail = |p: &str| {
            fail_message(&format!(
                "let main = () => Scene.cube() |> Scene.atPose({p})"
            ))
        };
        let degenerate = "Scene.atPose: forward and up must be non-zero and not parallel";
        for (forward, up) in [
            ([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]),
            ([0.0, 0.0, 1.0], [0.0, 0.0, -3.0]),
            ([0.0, 0.0, 1.0], [0.0, 1e-4, 1.0]),
            ([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, 1.0], [0.0, 0.0, 0.0]),
        ] {
            assert_eq!(
                fail(&pose_src([0.0; 3], forward, up)),
                degenerate,
                "{forward:?} {up:?}"
            );
        }
        // Nearly, but not, parallel still defines a roll.
        pose_world(&pose_src([0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.01, 1.0]), "");
        assert!(fail(
            "{ position: { x: 0.0, y: 0.0, z: 0.0 }, forward: { x: 0.0, y: 0.0, z: 1.0 } }"
        )
        .contains("missing `up`"));
        assert!(fail(
            // 1e39 overflows f32.
            "{ position: { x: 1000000000000000000000000000000000000000.0, y: 0.0, z: 0.0 }, forward: { x: 0.0, y: 0.0, z: 1.0 }, \
               up: { x: 0.0, y: 1.0, z: 0.0 } }"
        )
        .contains("must be finite"));
    }

    // A posed node is an ordinary transform group, so structural equality and
    // the scene's serialized form see the pose.
    #[test]
    fn at_pose_is_plain_scene_data() {
        let value = eval(&format!(
            "let a = Scene.cube() |> Scene.atPose({})\n\
             let b = Scene.cube() |> Scene.atPose({})\n\
             let main = () => [Scene.equals(a, a), Scene.equals(a, b), \
               Scene.equals(Scene.cube() |> Scene.atPose({}), Scene.group([Scene.cube()]))]",
            pose_src([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            pose_src([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0]),
            pose_src([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ));
        let Value::List(flags) = value else {
            panic!("expected a list")
        };
        let flags: Vec<_> = flags
            .iter()
            .map(|v| matches!(v, Value::Bool(true)))
            .collect();
        assert_eq!(flags, [true, false, true]);

        let posed = eval(&format!(
            "let main = () => Scene.cube() |> Scene.atPose({})",
            pose_src([1.0, 2.0, 3.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
        ));
        let scene = scene_of(&posed).unwrap();
        let json = serde_json::to_string(scene).unwrap();
        let back: Scene3D = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, scene);
    }

    // The hello-cubes shape: a List.map-built group of colored cubes.
    #[test]
    fn mapped_group_builds_n_children() {
        let frame = frame_of(
            "let cubeAt = (i) => Scene.cube() |> Scene.color(Color.rgb(1.0, 0.5, 0.2)) |> Scene.translate(Vec3.make(i, 0.0, 0.0))\n\
             let main = () =>\n\
             Frame.create(\n\
               Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
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
               Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
               Scene.model(Asset.model(\"shark.glb\")) |> Scene.scale(0.002) |> Scene.translate(Vec3.make(3.0, 1.0, 3.0)))",
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
        let ModelHandle::File(path) = &model.handle else {
            panic!("expected a file model");
        };
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
        // The flag day (B.6): a bare number teaches the Asset shape, and a
        // bare STRING — the retired pre-manifest coercion — teaches the
        // migration.
        assert_eq!(
            run_fail("let main = () => Scene.model(42.0)"),
            "Scene.model: expected a model Asset value (the generated manifest's \
Assets.<name>, or Asset.model(…)), got a number"
        );
        assert_eq!(
            run_fail("let main = () => Scene.model(\"shark.glb\")"),
            "Scene.model: bare asset paths are no longer accepted — reference the \
generated manifest (run `functor import`, then Assets.<name>), or construct \
Asset.model(\"shark.glb\") at the data boundary"
        );
        // An empty string can't be echoed into the advice (Asset.model("")
        // is itself rejected) — a fixed valid example stands in.
        assert!(run_fail("let main = () => Scene.model(\"\")")
            .contains("Asset.model(\"file.glb\")"));
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


    /// `Asset.texture` feeds the texture materials exactly like a
    /// `Texture.file` value (both lit and the normal-map slot).
    #[test]
    fn asset_texture_matches_texture_file_form() {
        let camera = "Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0))";
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


    /// `Asset.sound` in the one-shot effects queues the expected
    /// AudioCommands (paths unwrapped from the branded values).
    #[test]
    fn asset_sound_one_shots_queue_audio_commands() {
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
        let mut log: Vec<Entry> = Vec::new();
        for name in ["shoot", "blast"] {
            let effect = effect_of(&session.global(name).unwrap()).unwrap().0.clone();
            let _ = drain_effects(
                &session,
                "update",
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
    /// not invalidate hot-reload time-travel history — with or without a
    /// whilePending chain.
    #[test]
    fn assets_are_reload_safe_snapshots() {
        for src in [
            "let main = () => Asset.model(\"shark.glb\")",
            "let main = () => Asset.model(\"shark.glb\") |> Asset.whilePending(Asset.model(\"box.glb\"))",
        ] {
            let value = eval(src);
            match &value {
                Value::HostData(data) => assert!(data.is_reload_safe_snapshot()),
                other => panic!("expected a HostData asset, got {}", other.kind_name()),
            }
        }
    }

    // --- Asset.whilePending (Track B.4) ---

    /// The chain reaches the protocol: `Scene.model` carries `while_pending`,
    /// and nested placeholders flatten OUTER-first (the last-applied
    /// placeholder is tried first, then its own chain, then the asset's).
    #[test]
    fn while_pending_chains_flatten_into_the_model_description() {
        let camera = "Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0))";
        let frame = frame_of(&format!(
            "let proxy = Asset.model(\"low.glb\") |> Asset.whilePending(Asset.model(\"cube.glb\"))\n\
             let boss = Asset.model(\"boss.glb\") |> Asset.whilePending(proxy)\n\
             let main = () => Frame.create({camera}, Scene.model(boss))"
        ));
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(
            json.contains("\"while_pending\":[\"low.glb\",\"cube.glb\"]"),
            "json: {json}"
        );
        assert!(json.contains("boss.glb"), "json: {json}");
        // And it round-trips through the protocol.
        let back: Frame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    /// A chainless model keeps the exact pre-B.4 wire shape (no
    /// `while_pending` key at all) — replays and cross-version scenes agree.
    #[test]
    fn chainless_models_keep_the_v1_wire_shape() {
        let camera = "Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0))";
        let frame = frame_of(&format!(
            "let main = () => Frame.create({camera}, Scene.model(Asset.model(\"shark.glb\")))"
        ));
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(!json.contains("while_pending"), "json: {json}");
    }

    /// Texture chains produce the `FileWhilePending` description; chainless
    /// texture assets keep the plain `File` shape.
    #[test]
    fn while_pending_textures_flow_into_materials() {
        let camera = "Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0))";
        let frame = frame_of(&format!(
            "let wood = Asset.texture(\"wood.png\") |> Asset.whilePending(Asset.texture(\"grey.png\"))\n\
             let main = () => Frame.create({camera}, Scene.plane() |> Scene.litTexture(wood))"
        ));
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(
            json.contains("FileWhilePending") && json.contains("grey.png"),
            "json: {json}"
        );
        let frame = frame_of(&format!(
            "let main = () => Frame.create({camera}, \
Scene.plane() |> Scene.litTexture(Asset.texture(\"wood.png\")))"
        ));
        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(!json.contains("FileWhilePending"), "json: {json}");
    }

    /// whilePending teaches: sounds have no pending state, kinds must match,
    /// and bare strings must construct an Asset first.
    #[test]
    fn while_pending_teaching_errors() {
        for (src, want) in [
            (
                "let main = () => Asset.sound(\"a.ogg\") |> Asset.whilePending(Asset.sound(\"b.ogg\"))",
                "Asset.whilePending: sounds have no pending state to render (they decode at \
play time, outside the asset cache) — whilePending applies to model and texture assets",
            ),
            (
                "let main = () => Asset.model(\"a.glb\") |> Asset.whilePending(Asset.texture(\"b.png\"))",
                "Asset.whilePending: the placeholder must match the asset's kind — got a \
texture placeholder for a model asset; construct it with Asset.model(…)",
            ),
            (
                "let main = () => Asset.model(\"a.glb\") |> Asset.whilePending(\"b.glb\")",
                "Asset.whilePending: expected an Asset value, got a bare string (\"b.glb\") \
— construct one with Asset.model/texture(…) first",
            ),
        ] {
            assert_eq!(fail_message(src), want, "for {src}");
        }
    }

    // The lit pipeline: materials, all three light kinds, shadow flag, and
    // firstPerson camera flow into a protocol Frame with lights.
    #[test]
    fn lit_frame_carries_lights_and_materials() {
        let frame = frame_of(
            "let main = () =>
             Frame.createLit(
               Camera3D.firstPerson(Vec3.make(0.0, 3.5, -8.0), Angle.radians(0.0), Angle.radians(-0.3), Angle.degrees(60.0)),
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
    fn camera_clip_supports_large_worlds_and_validates_depth_precision() {
        let value = eval(
            "let main = () =>\n\
               Camera3D.lookAt(Vec3.make(0.0, 100.0, -500.0), Vec3.make(0.0, 0.0, 0.0))\n\
               |> Camera3D.clip(0.5, 6000.0)",
        );
        let Value::HostData(data) = value else {
            panic!("expected Camera host data");
        };
        let camera = data
            .as_any()
            .downcast_ref::<FunctorLangCamera>()
            .expect("Camera");
        assert_eq!((camera.0.near, camera.0.far), (0.5, 6000.0));
        assert_eq!(
            fail_message(
                "let main = () => Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)) |> Camera3D.clip(1.0, 0.5)"
            ),
            "usage: Camera3D.clip(near, far, camera) — finite distances with 0 < near < far"
        );
        assert_eq!(
            fail_message(
                "let main = () => Camera3D.lookAt(Vec3.make(0.0, 1.0, -3.0), Vec3.make(0.0, 0.0, 0.0)) |> Camera3D.clip(1.0, 1.00000001)"
            ),
            "usage: Camera3D.clip(near, far, camera) — finite distances with 0 < near < far"
        );
    }

    #[test]
    fn spot_light_and_normal_mapped_material() {
        let frame = frame_of(
            "let bumps = Texture.file(\"bumps-normal.png\")\n\
             let main = () =>\n\
             Frame.createLit(\n\
               Camera3D.firstPerson(Vec3.make(0.0, 3.0, -8.0), Angle.radians(0.0), Angle.radians(-0.25), Angle.degrees(60.0)),\n\
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
`90deg` / `1.5rad`, or Angle.degrees(…) / Angle.radians(…)"
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
        let project = functor_lang::project::load_with_bundled_modules(
            &dir.join("game.fun"),
            &Default::default(),
            &functor_prelude::bundled_modules(),
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
                "let main = () => Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), 0.0)"
            ),
            "Camera3D.lookAt: expected a Vec3, got a bare number — wrap the components: \
Vec3.make(x, y, z)"
        );
        // Vec3.make itself still validates its components.
        assert_eq!(
            fail("let main = () => Vec3.make(1.0, \"x\", 0.0)"),
            "expected a number, got a string"
        );
    }

    // --- Vec3 accessors and arithmetic ---

    /// Evaluate a Functor Lang expression to a number.
    fn eval_number(expr: &str) -> f64 {
        match eval(&format!("let main = () => {expr}")) {
            Value::Number(n) => n,
            other => panic!("expected a number from `{expr}`, got a {}", value_kind(&other)),
        }
    }

    /// Evaluate a Functor Lang expression to a `Vec3`, read back through the
    /// public accessors (so the test exercises the same path a game does).
    fn eval_vec3(expr: &str) -> (f64, f64, f64) {
        (
            eval_number(&format!("({expr}) |> Vec3.x()")),
            eval_number(&format!("({expr}) |> Vec3.y()")),
            eval_number(&format!("({expr}) |> Vec3.z()")),
        )
    }

    /// f32 arithmetic widened to f64 is not exactly the f64 answer, and the
    /// x86/ARM rounding of a `sqrt`-derived value can differ in the last
    /// bits, so vector comparisons are always toleranced.
    fn assert_close(actual: (f64, f64, f64), expected: (f64, f64, f64)) {
        let ok = (actual.0 - expected.0).abs() < 1e-6
            && (actual.1 - expected.1).abs() < 1e-6
            && (actual.2 - expected.2).abs() < 1e-6;
        assert!(ok, "expected {expected:?}, got {actual:?}");
    }

    /// The components a game stores go in and come back out unchanged.
    #[test]
    fn vec3_accessors_read_components() {
        assert_close(
            eval_vec3("Vec3.make(1.5, -2.25, 3.0)"),
            (1.5, -2.25, 3.0),
        );
    }

    /// Every arithmetic op, in direct-call form.
    #[test]
    fn vec3_arithmetic() {
        let a = "Vec3.make(1.0, 2.0, 3.0)";
        let b = "Vec3.make(10.0, 20.0, 30.0)";

        // add is commutative; both spellings agree.
        assert_close(eval_vec3(&format!("Vec3.add({b}, {a})")), (11.0, 22.0, 33.0));
        assert_close(eval_vec3(&format!("Vec3.add({a}, {b})")), (11.0, 22.0, 33.0));

        // sub(b, a) == a - b — the subject is the LAST argument.
        assert_close(eval_vec3(&format!("Vec3.sub({b}, {a})")), (-9.0, -18.0, -27.0));
        assert_close(eval_vec3(&format!("Vec3.sub({a}, {b})")), (9.0, 18.0, 27.0));

        assert_close(eval_vec3(&format!("Vec3.scale(2.0, {a})")), (2.0, 4.0, 6.0));
        // Negation is a scale by -1.
        assert_close(
            eval_vec3(&format!("Vec3.scale(0.0 - 1.0, {a})")),
            (-1.0, -2.0, -3.0),
        );

        assert_eq!(eval_number(&format!("Vec3.dot({b}, {a})")), 140.0);
        // Perpendicular vectors dot to zero.
        assert_eq!(
            eval_number("Vec3.dot(Vec3.make(0.0, 1.0, 0.0), Vec3.make(1.0, 0.0, 0.0))"),
            0.0
        );

        // cross(b, a) == a × b, right-handed: X × Y = Z.
        assert_close(
            eval_vec3("Vec3.cross(Vec3.make(0.0, 1.0, 0.0), Vec3.make(1.0, 0.0, 0.0))"),
            (0.0, 0.0, 1.0),
        );
        // …and is anti-commutative.
        assert_close(
            eval_vec3("Vec3.cross(Vec3.make(1.0, 0.0, 0.0), Vec3.make(0.0, 1.0, 0.0))"),
            (0.0, 0.0, -1.0),
        );

        assert_eq!(eval_number("Vec3.length(Vec3.make(3.0, 4.0, 0.0))"), 5.0);
        assert_eq!(eval_number("Vec3.length(Vec3.make(0.0, 0.0, 0.0))"), 0.0);

        // distance is symmetric.
        assert_eq!(
            eval_number("Vec3.distance(Vec3.make(0.0, 0.0, 0.0), Vec3.make(3.0, 4.0, 0.0))"),
            5.0
        );
        assert_eq!(
            eval_number("Vec3.distance(Vec3.make(3.0, 4.0, 0.0), Vec3.make(0.0, 0.0, 0.0))"),
            5.0
        );

        // lerp(target, t, from): t = 0 is `from`, t = 1 is `target`.
        assert_close(eval_vec3(&format!("Vec3.lerp({b}, 0.0, {a})")), (1.0, 2.0, 3.0));
        assert_close(
            eval_vec3(&format!("Vec3.lerp({b}, 1.0, {a})")),
            (10.0, 20.0, 30.0),
        );
        assert_close(eval_vec3(&format!("Vec3.lerp({b}, 0.5, {a})")), (5.5, 11.0, 16.5));
        // t is not clamped — it extrapolates.
        assert_close(eval_vec3(&format!("Vec3.lerp({b}, 2.0, {a})")), (19.0, 38.0, 57.0));
    }

    /// The strafe-axis identity the docs promise, pinned numerically: the
    /// world-space "right" of a gaze is `up × forward`, which in this
    /// argument order is `up |> Vec3.cross(forward)`. Getting this backwards
    /// is the predictable cross-product mistake, so both orders are asserted.
    #[test]
    fn vec3_cross_strafe_axis_matches_the_docs() {
        let forward = "Vec3.make(0.0, 0.0, 1.0)";
        let up = "Vec3.make(0.0, 1.0, 0.0)";
        // up × forward == +X — the documented strafe axis.
        assert_close(
            eval_vec3(&format!("{up} |> Vec3.cross({forward})")),
            (1.0, 0.0, 0.0),
        );
        // forward × up == -X — the other order.
        assert_close(
            eval_vec3(&format!("{forward} |> Vec3.cross({up})")),
            (-1.0, 0.0, 0.0),
        );
    }

    /// Accessors round to the brand's `f32` storage — documented, and pinned
    /// with a value that is NOT exactly representable so the rounding cannot
    /// be accidentally hidden by a well-chosen test constant.
    #[test]
    fn vec3_accessors_round_to_f32() {
        let x = eval_number("Vec3.x(Vec3.make(0.1, 0.0, 0.0))");
        assert_eq!(x, 0.1f32 as f64);
        assert_ne!(x, 0.1f64, "the f32 rounding is real and must stay documented");
    }

    /// Functor Lang has no exponent notation in float literals, so a huge
    /// constant has to be spelled out in full.
    fn lit(n: f64) -> String {
        format!("{n:.1}")
    }

    /// A length that IS representable must not overflow to infinity just
    /// because the intermediate squares would leave the `f32` range — and
    /// such a vector must normalize to a real unit vector, not to zero (which
    /// is indistinguishable from the documented zero-vector case).
    #[test]
    fn vec3_length_and_normalize_survive_huge_vectors() {
        let c = lit(2.0e19);
        let big = &format!("Vec3.make({c}, {c}, {c})");
        let len = eval_number(&format!("Vec3.length({big})"));
        assert!(len.is_finite(), "length overflowed to {len}");
        assert!(
            (len - 3.4641016e19).abs() / 3.4641016e19 < 1e-5,
            "unexpected length {len}"
        );
        let unit = eval_vec3(&format!("Vec3.normalize({big})"));
        assert_ne!(unit, (0.0, 0.0, 0.0), "a huge vector must not normalize to zero");
        let unit_len = eval_number(&format!("Vec3.length(Vec3.normalize({big}))"));
        assert!((unit_len - 1.0).abs() < 1e-6, "expected unit length, got {unit_len}");
    }

    /// An operation that overflows the `f32` range fails LOUDLY, naming the
    /// operation — it must never hand back an infinity that becomes a NaN and
    /// silently blanks the scene (a NaN transform also serializes as `null`,
    /// which the `Frame` wire form cannot round-trip).
    #[test]
    fn vec3_overflow_is_an_error_not_a_nan() {
        let (big, huge) = (lit(1.0e20), lit(1.0e19));
        let msg = fail_message(&format!(
            "let main = () => Vec3.make({big}, 0.0, 0.0) |> Vec3.scale({huge})"
        ));
        assert!(
            msg.starts_with("Vec3.scale: the result is not a finite vector"),
            "overflow should name the op, got: {msg}"
        );
        // …and the same guard covers the other producers.
        let (p, n) = (lit(3.0e38), lit(-3.0e38));
        for (src, path) in [
            (
                format!(
                    "let main = () => Vec3.make({p}, 0.0, 0.0) \
                     |> Vec3.add(Vec3.make({p}, 0.0, 0.0))"
                ),
                "Vec3.add",
            ),
            (
                format!(
                    "let main = () => Vec3.make({p}, 0.0, 0.0) \
                     |> Vec3.sub(Vec3.make({n}, 0.0, 0.0))"
                ),
                "Vec3.sub",
            ),
        ] {
            assert!(
                fail_message(&src).starts_with(path),
                "{path} overflow should be an error"
            );
        }
    }

    /// `lerp` is endpoint-exact even when `target - from` would overflow, so
    /// `t = 0` can never turn a legal vector into a NaN.
    #[test]
    fn vec3_lerp_endpoints_are_exact_at_extremes() {
        let (p, n) = (lit(3.0e38), lit(-3.0e38));
        let from = &format!("Vec3.make({p}, 0.0, 0.0)");
        let target = &format!("Vec3.make({n}, 0.0, 0.0)");
        // Endpoints are EXACT (the f32-rounded input, unchanged) — an
        // absolute tolerance is meaningless at this magnitude.
        assert_eq!(
            eval_vec3(&format!("{from} |> Vec3.lerp({target}, 0.0)")),
            (3.0e38f32 as f64, 0.0, 0.0),
        );
        assert_eq!(
            eval_vec3(&format!("{from} |> Vec3.lerp({target}, 1.0)")),
            (-3.0e38f32 as f64, 0.0, 0.0),
        );
        // The midpoint of those extremes is representable and is zero.
        assert_eq!(
            eval_vec3(&format!("{from} |> Vec3.lerp({target}, 0.5)")),
            (0.0, 0.0, 0.0),
        );
    }

    /// A normalized vector is unit-length and keeps its direction.
    #[test]
    fn vec3_normalize_yields_a_unit_vector() {
        assert_close(
            eval_vec3("Vec3.normalize(Vec3.make(0.0, 5.0, 0.0))"),
            (0.0, 1.0, 0.0),
        );
        let len = eval_number("Vec3.length(Vec3.normalize(Vec3.make(1.0, 2.0, 3.0)))");
        assert!((len - 1.0).abs() < 1e-6, "expected unit length, got {len}");
    }

    /// The documented zero policy: normalizing the zero vector yields the
    /// ZERO vector — not NaN, not an error. Per-frame code normalizes an idle
    /// velocity constantly, so this must not fault the frame or poison a
    /// transform with NaN.
    #[test]
    fn vec3_normalize_of_zero_is_zero() {
        let v = eval_vec3("Vec3.normalize(Vec3.make(0.0, 0.0, 0.0))");
        assert_eq!(v, (0.0, 0.0, 0.0));
        assert!(!v.0.is_nan() && !v.1.is_nan() && !v.2.is_nan());
        assert_eq!(
            eval_number("Vec3.length(Vec3.normalize(Vec3.make(0.0, 0.0, 0.0)))"),
            0.0
        );
    }

    /// Thread-last is the point of the argument order: a pipeline of the
    /// non-commutative ops must read left-to-right as the subject being
    /// acted on, and agree with the direct-call spelling.
    #[test]
    fn vec3_pipelines_read_subject_first() {
        // `v |> Vec3.sub(origin)` is "v minus origin".
        assert_close(
            eval_vec3("Vec3.make(5.0, 7.0, 9.0) |> Vec3.sub(Vec3.make(1.0, 2.0, 3.0))"),
            (4.0, 5.0, 6.0),
        );
        // `a |> Vec3.cross(b)` is "a cross b" (X × Y = Z).
        assert_close(
            eval_vec3("Vec3.make(1.0, 0.0, 0.0) |> Vec3.cross(Vec3.make(0.0, 1.0, 0.0))"),
            (0.0, 0.0, 1.0),
        );
        // `from |> Vec3.lerp(target, t)` moves `from` toward `target`.
        assert_close(
            eval_vec3("Vec3.make(0.0, 0.0, 0.0) |> Vec3.lerp(Vec3.make(4.0, 8.0, 12.0), 0.25)"),
            (1.0, 2.0, 3.0),
        );
        // A whole chain: (v - origin) doubled, then normalized.
        assert_close(
            eval_vec3(
                "Vec3.make(1.0, 2.0, 3.0) \
                 |> Vec3.sub(Vec3.make(1.0, 2.0, 0.0)) \
                 |> Vec3.scale(2.0) \
                 |> Vec3.normalize()",
            ),
            (0.0, 0.0, 1.0),
        );
        assert_eq!(
            eval_number("Vec3.make(3.0, 4.0, 0.0) |> Vec3.length()"),
            5.0
        );
    }

    /// The new functions are ordinary consumers of the brand, so they keep
    /// the Angle-rule teaching errors: a bare number (or a bare `{x, y, z}`
    /// record, the shape games hand-rolled before this existed) is refused.
    #[test]
    fn vec3_arithmetic_refuses_unbranded_values() {
        // The error names the function that rejected the value, as the other
        // branded consumers do.
        for (src, path) in [
            ("let main = () => Vec3.length(1.0)", "Vec3.length"),
            ("let main = () => Vec3.x(1.0)", "Vec3.x"),
            (
                "let main = () => Vec3.add(1.0, Vec3.make(1.0, 2.0, 3.0))",
                "Vec3.add",
            ),
        ] {
            assert_eq!(
                fail_message(src),
                format!(
                    "{path}: expected a Vec3, got a bare number — wrap the components: \
Vec3.make(x, y, z)"
                ),
                "in `{src}`"
            );
        }
        // A structural record is NOT a Vec3 — the brand is what makes an
        // axis transposition unrepresentable.
        let record = fail_message("let main = () => Vec3.length({ x: 1.0, y: 2.0, z: 3.0 })");
        assert!(
            record.contains("expected a Vec3"),
            "a bare record must be refused, got: {record}"
        );
        // Arity slips still teach the shape.
        assert_eq!(
            fail_message("let main = () => Vec3.scale(2.0)"),
            "usage: Vec3.scale(k, v) — v |> Vec3.scale(2.0)"
        );
    }

    /// A vector that round-trips through the arithmetic is still a plain
    /// three-f32 brand, so a velocity a game keeps in its model across
    /// frames does not lose hot-reload time-travel history.
    #[test]
    fn computed_vec3s_stay_reload_safe() {
        let v = eval(
            "let main = () => Vec3.make(1.0, 2.0, 3.0) \
             |> Vec3.add(Vec3.make(1.0, 0.0, 0.0)) |> Vec3.normalize()",
        );
        assert!(v.is_reload_safe_snapshot());
    }

    /// The whole point: a computed vector feeds the prelude directly, with
    /// no unpack-and-rebuild at the boundary.
    #[test]
    fn computed_vec3s_feed_the_prelude_directly() {
        let frame = frame_of(
            "let main = () =>\n\
             let eye = Vec3.make(0.0, 2.0, 0.0) |> Vec3.add(Vec3.make(0.0, 0.0, -6.0)) in\n\
             Frame.create(\n\
             Camera3D.lookAt(eye, Vec3.make(0.0, 0.0, 0.0)),\n\
             Scene.cube() |> Scene.translate(Vec3.make(1.0, 0.0, 0.0) |> Vec3.scale(3.0)))",
        );
        // The camera got the summed eye, and the scene the scaled offset.
        let json = serde_json::to_string(&frame).expect("frame serializes");
        assert!(json.contains("-6.0"), "camera eye z should be -6: {json}");
        assert!(json.contains("3.0"), "translate x should be 3: {json}");
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
    fn scene_opacity_wraps_the_subtree_and_nested_opacities_stay_nested() {
        let value = eval(
            "let main = () => Scene.cube() |> Scene.opacity(0.5) |> Scene.opacity(0.5)",
        );
        let scene = scene_of(&value).expect("a Scene");
        let SceneObject::Opacity(outer, outer_items) = &scene.obj else {
            panic!("expected an Opacity node, got {:?}", scene.obj);
        };
        assert_eq!(*outer, 0.5);
        let SceneObject::Opacity(inner, inner_items) = &outer_items[0].obj else {
            panic!("expected a nested Opacity node, got {:?}", outer_items[0].obj);
        };
        assert_eq!(*inner, 0.5);
        // The multiplication is the RENDERER's (0.25 at the leaf); the scene
        // value keeps both nodes so `Scene.equals` and `/scene` show what the
        // game actually said.
        assert!(matches!(
            inner_items[0].obj,
            SceneObject::Geometry(Shape::Cube)
        ));
    }

    /// Full opacity is the IDENTITY, not a wrapper: this is what makes the
    /// feature free for every scene that does not use it, and it is the
    /// property `Scene.equals` and the golden images both depend on.
    #[test]
    fn scene_opacity_of_one_is_the_identity() {
        let with = eval("let main = () => Scene.cube() |> Scene.color(Color.rgb(1.0, 0.0, 0.0)) |> Scene.opacity(1.0)");
        let without =
            eval("let main = () => Scene.cube() |> Scene.color(Color.rgb(1.0, 0.0, 0.0))");
        assert_eq!(
            scene_of(&with).expect("a Scene"),
            scene_of(&without).expect("a Scene")
        );
    }

    /// `Scene.billboard` builds the plain `Shape::Billboard` leaf — a
    /// camera-FREE scene value (the view-dependence is the renderer's, at
    /// draw time), so it stamps, compares, and replays like any other shape.
    #[test]
    fn scene_billboard_is_a_plain_camera_free_leaf() {
        let value = eval("let main = () => Scene.billboard()");
        let scene = scene_of(&value).expect("a Scene");
        assert!(matches!(
            scene.obj,
            SceneObject::Geometry(Shape::Billboard)
        ));

        // And `Scene.equals` sees it structurally, like every shape.
        let equal = eval("let main = () => Scene.equals(Scene.billboard(), Scene.billboard())");
        assert!(matches!(equal, functor_lang::Value::Bool(true)));
    }

    /// The identity test happens AFTER narrowing to 32-bit, so an alpha that
    /// rounds to 1 is the identity too — the `Opacity` variant's invariant is
    /// "alpha is strictly below 1", and nothing may build a counterexample.
    #[test]
    fn an_alpha_that_narrows_to_one_is_also_the_identity() {
        let value = eval("let main = () => Scene.cube() |> Scene.opacity(0.999999999)");
        assert_eq!(
            scene_of(&value).expect("a Scene"),
            scene_of(&eval("let main = () => Scene.cube()")).expect("a Scene")
        );
    }

    /// Zero is legal and distinct from "not drawn" — it is an invisible node,
    /// so a fade to nothing does not have to branch in game code.
    #[test]
    fn scene_opacity_of_zero_is_a_node() {
        let value = eval("let main = () => Scene.cube() |> Scene.opacity(0.0)");
        let scene = scene_of(&value).expect("a Scene");
        assert!(matches!(scene.obj, SceneObject::Opacity(a, _) if a == 0.0));
    }

    #[test]
    fn scene_animate_sets_clip_on_model() {
        let value = eval(
            "let main = () => Scene.model(Asset.model(\"Xbot.glb\")) |> Scene.animate(Anim.clip(\"walk\", 1.5))",
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
             Scene.group([Scene.model(Asset.model(\"Xbot.glb\")), Scene.cube()])\n\
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

    // --- Scene.instanced / Instance.t ---

    /// The builder chain decodes into the exact channel record: `at` sets
    /// position, `scaleXYZ` multiplies per-axis scale, `rotateY` composes the
    /// rotation quaternion, `tint` multiplies the tint.
    #[test]
    fn instance_builders_decode_into_channel_records() {
        let source = "let main = () => Scene.cube() \
             |> Scene.lit(Color.rgb(1.0, 1.0, 1.0)) \
             |> Scene.instanced([ \
                Instance.at(Vec3.make(1.0, 2.0, 3.0)) \
                  |> Instance.scaleXYZ(0.5, 1.0, 2.0) \
                  |> Instance.scale(2.0) \
                  |> Instance.rotateY(Angle.degrees(90.0)) \
                  |> Instance.tint(Color.rgb(0.2, 0.4, 0.8))])";
        let value = eval(source);
        let scene = scene_of(&value).expect("Scene.instanced returns a Scene");
        let SceneObject::Instanced {
            template,
            instances,
        } = &scene.obj
        else {
            panic!("expected Instanced, got {:?}", scene.obj);
        };
        assert!(matches!(&template.obj, SceneObject::Material(..)));
        assert_eq!(instances.len(), 1);
        let instance = &instances[0];
        assert_eq!(instance.position, [1.0, 2.0, 3.0]);
        assert_eq!(instance.scale, [1.0, 2.0, 4.0]);
        assert_eq!(instance.tint, [0.2, 0.4, 0.8]);
        // A 90° yaw: [0, sin 45°, 0, cos 45°].
        assert!((instance.rotation[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((instance.rotation[3] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(instance.rotation[0], 0.0);
        assert_eq!(instance.rotation[2], 0.0);
    }

    /// Non-finite instance data is refused at the host boundary, and a
    /// template containing `Scene.opacity` is a teaching error naming the
    /// supported spelling.
    #[test]
    fn instanced_rejects_non_finite_data_and_opacity_templates() {
        let non_finite = "let main = () => Instance.at(Vec3.make(1.0 / 0.0, 0.0, 0.0))";
        let module = functor_lang::lower(functor_lang::parse(non_finite).unwrap()).unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("non-finite instance data must fail at the host boundary");
        assert_eq!(failure.error.message, "expected a finite number, got inf");

        let opacity_template = "let main = () => Scene.cube() \
             |> Scene.lit(Color.rgb(1.0, 0.0, 0.0)) \
             |> Scene.opacity(0.5) \
             |> Scene.instanced([Instance.at(Vec3.make(0.0, 0.0, 0.0))])";
        let module = functor_lang::lower(functor_lang::parse(opacity_template).unwrap()).unwrap();
        let failure = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("an opacity-bearing template must be a teaching error");
        assert!(
            failure.error.message.contains("wrap the whole instanced node"),
            "unexpected message: {}",
            failure.error.message
        );
    }

    /// The SAME Functor Lang list converts once: both nodes share one
    /// instance `Arc` (the renderer's upload-once identity). An equal but
    /// separately built list gets its own `Arc` yet compares equal.
    #[test]
    fn instanced_lists_convert_once_per_list_identity() {
        let source = "
let xs = [Instance.at(Vec3.make(1.0, 0.0, 0.0)), Instance.at(Vec3.make(2.0, 0.0, 0.0))]
let ys = [Instance.at(Vec3.make(1.0, 0.0, 0.0)), Instance.at(Vec3.make(2.0, 0.0, 0.0))]
let t = Scene.cube() |> Scene.lit(Color.rgb(1.0, 1.0, 1.0))
let main = () => (Scene.instanced(xs, t), Scene.instanced(xs, t), Scene.instanced(ys, t))
";
        let value = eval(source);
        let Value::Tuple(items) = &value else {
            panic!("expected a tuple");
        };
        let lists: Vec<&InstanceList> = items
            .iter()
            .map(|item| match &scene_of(item).expect("a Scene").obj {
                SceneObject::Instanced { instances, .. } => instances,
                other => panic!("expected Instanced, got {other:?}"),
            })
            .collect();
        assert!(lists[0].ptr_eq(lists[1]));
        assert!(!lists[0].ptr_eq(lists[2]));
        assert_eq!(lists[0], lists[2]);
    }

    fn memo_session(src: &str) -> functor_lang::Session {
        let module = functor_lang::lower(functor_lang::parse(src).expect("parse")).expect("lower");
        functor_lang::Session::load(&module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("load failed: {}", f.error.message))
    }

    /// Call `frame(n)` — one draw's `Scene.memo` call.
    fn memo_frame(session: &functor_lang::Session, n: f64) -> Result<Value, String> {
        session
            .call("frame", vec![Value::Number(n)], &mut FunctorHost)
            .map_err(|e| e.message)
    }

    /// Identity of a returned scene value: equal means the builder did not run.
    fn scene_ptr(value: &Value) -> *const () {
        match value {
            Value::HostData(data) => Rc::as_ptr(data) as *const (),
            other => panic!("expected a scene, got {other}"),
        }
    }

    const MEMO_SOURCE: &str = "
let build = (n) => Scene.cube() |> Scene.scale(n)
let frame = (n) => Scene.memo(\"memo-test\", { size: n, tags: [\"a\"] }, (d) => build(d.size))
";

    /// Equal deps — even a freshly built record — return the SAME scene;
    /// changed deps rebuild.
    #[test]
    fn scene_memo_rebuilds_only_when_deps_change() {
        let session = memo_session(MEMO_SOURCE);
        let a = memo_frame(&session, 1.0).unwrap();
        let again = memo_frame(&session, 1.0).unwrap();
        assert_eq!(scene_ptr(&a), scene_ptr(&again));
        let b = memo_frame(&session, 2.0).unwrap();
        assert_ne!(scene_ptr(&a), scene_ptr(&b));
        assert!(scene_of(&a).unwrap() != scene_of(&b).unwrap());
        assert_eq!(scene_ptr(&b), scene_ptr(&memo_frame(&session, 2.0).unwrap()));
    }

    /// A reloaded program rebuilds even with equal deps, so an edit to what
    /// the builder calls takes effect.
    #[test]
    fn scene_memo_rebuilds_after_a_reload() {
        let before = memo_frame(&memo_session(MEMO_SOURCE), 1.0).unwrap();
        let edited = MEMO_SOURCE.replace("Scene.scale(n)", "Scene.scale(n * 3.0)");
        let after = memo_frame(&memo_session(&edited), 1.0).unwrap();
        assert_ne!(scene_ptr(&before), scene_ptr(&after));
        let expected = eval("let main = () => Scene.cube() |> Scene.scale(3.0)");
        assert!(scene_of(&after).unwrap() == scene_of(&expected).unwrap());
    }

    /// A site unused for more than the idle window is forgotten; one used
    /// within it is kept.
    #[test]
    fn scene_memo_evicts_idle_sites() {
        let session = memo_session(MEMO_SOURCE);
        let first = memo_frame(&session, 1.0).unwrap();
        for _ in 0..SCENE_MEMO_IDLE_FRAMES {
            sweep_scene_memos();
        }
        let kept = memo_frame(&session, 1.0).unwrap();
        assert_eq!(scene_ptr(&first), scene_ptr(&kept));
        for _ in 0..=SCENE_MEMO_IDLE_FRAMES {
            sweep_scene_memos();
        }
        assert_ne!(scene_ptr(&kept), scene_ptr(&memo_frame(&session, 1.0).unwrap()));
    }

    const PHYSICS_MEMO_SOURCE: &str = "
let tile = (i) => Physics.fixed(Physics.tag(Text.concat(\"s\", Text.fromFloat(i))), Physics.box(1.0, 1.0, 1.0))
  |> Physics.at(Vec3.make(Math.mod(i, 30.0) - 15.0, 0.0, (i - Math.mod(i, 30.0)) / 30.0 - 25.0))
let statics = (n) => List.map(tile, List.range(n))
let door = (y) => Physics.kinematic(Physics.tag(\"door\"), Physics.sphere(0.5)) |> Physics.at(Vec3.make(0.0, y, 9.0))
let physics = (doorY) => Physics.scene(Vec3.make(0.0, -9.81, 0.0), [
  Physics.memo(\"pm-statics\", 1500.0, statics),
  Physics.memo(\"pm-doors\", doorY, (y) => [door(y)]),
  Physics.dynamic(Physics.tag(\"ball\"), Physics.sphere(0.25)) |> Physics.at(Vec3.make(0.5, 2.0, 0.5)),
])
";

    /// A 1,500-body static group through the real drive: built once, skipped
    /// by reconcile every frame after, rebuilt alone when its deps change,
    /// kept (no rapier work) across a reload, and bit-exact under seek,
    /// rewind and replay.
    #[test]
    fn physics_memo_builds_a_static_group_once_and_reconcile_skips_it() {
        use crate::physics::{
            remove_world, with_world, ReconcileStats, SteppedPhysics, DEFAULT_WORLD, FIXED_DT,
        };
        let declare = |session: &functor_lang::Session, door_y: f64| {
            let value = session
                .call("physics", vec![Value::Number(door_y)], &mut FunctorHost)
                .unwrap_or_else(|e| panic!("physics: {}", e.message));
            physics_scene_value(&value).expect("a PhysicsScene").clone()
        };
        let door_y = |frame: u32| if frame < 150 { 1.0 } else { 2.0 };
        let stats = || with_world(DEFAULT_WORLD, |w| w.reconcile_stats()).unwrap();
        let snapshot = || with_world(DEFAULT_WORLD, |w| w.snapshot()).unwrap();
        let skipped = ReconcileStats {
            compared: 1,
            written: 0,
            groups_skipped: 2,
        };

        remove_world(DEFAULT_WORLD);
        let session = memo_session(PHYSICS_MEMO_SOURCE);
        let mut sp = SteppedPhysics::new();
        let first = declare(&session, 1.0);
        sp.advance(&first, FIXED_DT);
        assert_eq!(
            stats(),
            ReconcileStats {
                compared: 1502,
                written: 1502,
                groups_skipped: 0
            }
        );
        let (mut at_100, mut at_250) = (Vec::new(), Vec::new());
        for frame in 1..300 {
            let scene = declare(&session, door_y(frame));
            assert!(std::sync::Arc::ptr_eq(
                &scene.groups[0].bodies,
                &first.groups[0].bodies
            ));
            sp.advance(&scene, FIXED_DT);
            let expected = if frame == 150 {
                // Only the doors group changed.
                ReconcileStats {
                    compared: 2,
                    written: 1,
                    groups_skipped: 1,
                }
            } else {
                skipped
            };
            assert_eq!(stats(), expected, "frame {frame}");
            match frame {
                100 => at_100 = snapshot(),
                250 => at_250 = snapshot(),
                _ => {}
            }
        }

        // A rendered frame's end state is the next fixed frame's pre-step.
        sp.seek_to_frame(101);
        assert!(snapshot() == at_100, "seek before the doors edit diverged");
        sp.seek_to_frame(251);
        assert!(
            snapshot() == at_250,
            "replay across the doors edit diverged"
        );
        sp.rewind_to_frame(101);
        for frame in 101..=250 {
            sp.advance(&declare(&session, door_y(frame)), FIXED_DT);
        }
        assert!(snapshot() == at_250, "re-living after a rewind diverged");

        // A reload rebuilds both sites once into equal groups: no spawns.
        let reloaded = memo_session(PHYSICS_MEMO_SOURCE);
        let scene = declare(&reloaded, 2.0);
        assert!(!std::sync::Arc::ptr_eq(
            &scene.groups[0].bodies,
            &first.groups[0].bodies
        ));
        sp.advance(&scene, FIXED_DT);
        assert_eq!(
            stats(),
            ReconcileStats {
                compared: 1502,
                written: 0,
                groups_skipped: 0
            }
        );
        sp.advance(&declare(&reloaded, 2.0), FIXED_DT);
        assert_eq!(stats(), skipped);
        remove_world(DEFAULT_WORLD);
    }

    fn physics_memo_error(main: &str) -> String {
        let source = format!(
            "let box = Physics.box(1.0, 1.0, 1.0)\n\
             let fixed = (name) => Physics.fixed(Physics.tag(name), box)\n\
             let main = () => Physics.scene(Vec3.make(0.0, -9.81, 0.0), {main})\n"
        );
        eval_with_prelude(&source)
            .err()
            .unwrap_or_else(|| panic!("expected an error: {main}"))
    }

    #[test]
    fn physics_memo_teaching_errors() {
        let cases = [
            (
                "[Physics.memo(\"pm-e1\", 0.0, (d) => [Physics.dynamic(Physics.tag(\"a\"), box)])]",
                "\"a\" is dynamic",
            ),
            (
                "[Physics.memo(\"pm-e2\", Vec3.make(0.0, 0.0, 0.0), (d) => [fixed(\"a\")])]",
                "Physics.memo: deps must be plain data",
            ),
            (
                "[Physics.memo(\"pm-e3\", 0.0, (d) => [fixed(\"a\")]), fixed(\"a\")]",
                "tag \"a\" is declared both flat and by Physics.memo(\"pm-e3\")",
            ),
            (
                "[Physics.memo(\"pm-e4\", 0.0, (d) => [fixed(\"a\")]), \
                  Physics.memo(\"pm-e5\", 0.0, (d) => [fixed(\"b\"), fixed(\"a\")])]",
                "declared by both Physics.memo(\"pm-e4\") and Physics.memo(\"pm-e5\")",
            ),
            (
                "[Physics.memo(\"pm-e6\", 0.0, (d) => [fixed(\"a\")]), \
                  Physics.memo(\"pm-e6\", 0.0, (d) => [fixed(\"a\")])]",
                "Physics.memo(\"pm-e6\") appears twice",
            ),
            (
                "[Physics.memo(\"pm-e7\", 0.0, (d) => [fixed(\"a\"), fixed(\"a\")])]",
                "tag \"a\" is declared twice",
            ),
            (
                "[Physics.memo(\"pm-e8\", 0.0, (d) => [fixed(\"a\")]) \
                  |> Physics.at(Vec3.make(0.0, 1.0, 0.0))]",
                "Physics.at: Physics.memo(\"pm-e8\") is a group, not one body",
            ),
            (
                "[Physics.memo(\"pm-e9\", 0.0, (d) => [Physics.memo(\"pm-e10\", 0.0, (e) => [])])]",
                "a memo can't contain another memo",
            ),
            (
                "[Physics.memo(\"pm-e11\", 0.0, (d) => \
                  if Physics.cast(Vec3.make(0.0, 1.0, 0.0), Vec3.make(0.0, -1.0, 0.0), 2.0).hit \
                  then [] else [fixed(\"a\")])]",
                "Physics.memo: the builder read the live physics world",
            ),
            (
                "[Physics.memo(\"pm-e12\", 0.0, (d) => [Physics.heightfield(Physics.tag(\"t\"), \
                  Terrain.heightmap(Asset.texture(\"t.png\"), 10.0, 10.0, 0.0, 1.0))])]",
                "\"t\" is a heightfield",
            ),
        ];
        for (main, expected) in cases {
            let error = physics_memo_error(main);
            assert!(
                error.contains(expected),
                "{main}\n  expected: {expected}\n  got: {error}"
            );
        }
    }

    /// Deps that share structure (a 2^60-leaf tree in 60 nodes) validate and
    /// compare in linear time; a partially applied def is a valid builder.
    #[test]
    fn scene_memo_walks_shared_deps_once() {
        let source = "
type Tree =
  | Leaf
  | Fork(left: Tree, right: Tree)
let deep = (n) => List.range(n) |> List.fold((acc, i) => Fork(acc, acc), Leaf)
let build = (size, d) => Scene.cube() |> Scene.scale(size)
let frame = (n) => Scene.memo(\"memo-dag\", deep(60.0), build(n))
";
        let session = memo_session(source);
        let a = memo_frame(&session, 1.0).unwrap();
        assert_eq!(scene_ptr(&a), scene_ptr(&memo_frame(&session, 1.0).unwrap()));
    }

    #[test]
    fn scene_memo_teaches_bad_deps_builders_and_results() {
        let err = |src: &str| match memo_frame(&memo_session(src), 1.0) {
            Err(message) => message,
            Ok(value) => panic!("expected an error, got {value}"),
        };
        let closure = err("let frame = (n) => Scene.memo(\"k1\", { f: (x) => x }, (d) => Scene.cube())");
        assert!(closure.contains("deps must be plain data") && closure.contains("a function"), "{closure}");
        let engine =
            err("let frame = (n) => Scene.memo(\"k2\", [Vec3.make(n, 0.0, 0.0)], (d) => Scene.cube())");
        assert!(engine.contains("found a Vec3"), "{engine}");
        let nullary = err("let frame = (n) => Scene.memo(\"k3\", n, () => Scene.cube())");
        assert!(nullary.contains("one argument"), "{nullary}");
        // A builtin holds no code identity a reload could invalidate.
        let builtin = err("let frame = (n) => Scene.memo(\"k5\", [], List.fold((s, x) => s, Scene.cube()))");
        assert!(builtin.contains("must be a lambda or def"), "{builtin}");
        let world = err("let frame = (n) => Scene.memo(\"k6\", n, (d) =>
  let hit = Physics.cast(Vec3.make(0.0, 0.0, 0.0), Vec3.make(0.0, -1.0, 0.0), 1.0) in
  Scene.cube())");
        assert!(world.contains("read the live physics world"), "{world}");
        let result = err("let frame = (n) => Scene.memo(\"k4\", n, (d) => d)");
        assert!(result.contains("must return a Scene, got a number"), "{result}");
    }

    const SHARED_SOURCE: &str = "
let world = (d) => Scene.group([
  Scene.model(Asset.model(\"Xbot.glb\")) |> Scene.animate(Anim.clip(\"walk\", 0.5)),
  Scene.cube() |> Scene.lit(Color.rgb(1.0, 0.5, 0.25)) |> Scene.opacity(0.5)
    |> Scene.translate(Vec3.make(2.0, 0.0, 0.0))
])
let wrap = (s) => Scene.group([s, Scene.cube()]) |> Scene.translate(Vec3.make(1.0, 2.0, 3.0))
let frame = (n) => wrap(Scene.memo(\"shared-test\", 0.0, world))
let inline = (n) => wrap(world(0.0))
let animated = (n) => frame(n) |> Scene.animate(Anim.clip(\"run\", 1.0))
let animatedInline = (n) => inline(n) |> Scene.animate(Anim.clip(\"run\", 1.0))
";

    /// A memo result is held by reference, so wrapping it each frame copies a
    /// pointer; and the wrapped scene is indistinguishable from the same scene
    /// built inline: equality, `/scene` JSON, the transparent pass, animation.
    #[test]
    fn scene_memo_is_shared_and_transparent_under_wrappers() {
        let session = memo_session(SHARED_SOURCE);
        let call = |name: &str| {
            session
                .call(name, vec![Value::Number(0.0)], &mut FunctorHost)
                .unwrap_or_else(|e| panic!("{name}: {}", e.message))
        };
        // wrap(s) = Group(T)[Group[s, cube]]; `s` is the memo node.
        let memo_node = |scene: &Value| -> std::sync::Arc<SceneObject> {
            let SceneObject::Group(outer) = &scene_of(scene).unwrap().obj else {
                panic!("expected the translate group")
            };
            let SceneObject::Group(inner) = &outer[0].obj else {
                panic!("expected the group")
            };
            match &inner[0].obj {
                SceneObject::Shared(shared) => shared.clone(),
                other => panic!("expected a Shared memo node, got {other:?}"),
            }
        };
        let (a, b, inline) = (call("frame"), call("frame"), call("inline"));
        assert!(std::sync::Arc::ptr_eq(&memo_node(&a), &memo_node(&b)));

        let (a, inline) = (scene_of(&a).unwrap(), scene_of(&inline).unwrap());
        assert!(a == inline && inline == a);
        assert_eq!(
            serde_json::to_string(a).unwrap(),
            serde_json::to_string(inline).unwrap()
        );
        let back: Scene3D = serde_json::from_str(&serde_json::to_string(a).unwrap()).unwrap();
        assert!(back == *a);

        assert!(a.has_opacity());
        let transparent = |scene: &Scene3D| {
            let mut out = Vec::new();
            scene.collect_transparent(&Matrix4::from_scale(1.0), None, &mut out);
            out.iter()
                .map(|draw| (draw.node.clone(), draw.parent_world, draw.centroid, draw.material.cloned()))
                .collect::<Vec<_>>()
        };
        let (shared_draws, inline_draws) = (transparent(a), transparent(inline));
        assert_eq!(shared_draws.len(), 1);
        assert!(shared_draws == inline_draws);

        let (animated, animated_inline) = (call("animated"), call("animatedInline"));
        assert!(scene_of(&animated).unwrap() == scene_of(&animated_inline).unwrap());
        assert!(scene_of(&animated).unwrap() != a, "the pose changed");
        assert!(*scene_of(&call("frame")).unwrap() == *a, "the memo is untouched");
    }

    /// `cargo test --release -p functor_runtime_common scene_memo_wrap_bench -- --ignored --nocapture`
    ///
    /// A memoized N-node world under a group + translate costs the same per
    /// frame at 1,500 nodes as at 15.
    #[test]
    #[ignore]
    fn scene_memo_wrap_bench() {
        let per_frame_ms = |n: usize| {
            let source = format!(
                "
let node = (i) => Scene.cube() |> Scene.lit(Color.rgb(0.5, 0.5, 0.5))
  |> Scene.translate(Vec3.make(i, 0.0, Math.mod(i, 37.0)))
let world = (d) => Scene.group(List.map(node, List.range({n}.0)))
let frame = (k) => Scene.group([Scene.memo(\"bench-{n}\", 0.0, world), Scene.cube()])
  |> Scene.translate(Vec3.make(k, 0.0, 0.0))
"
            );
            let session = memo_session(&source);
            memo_frame(&session, 0.0).unwrap();
            // Best of 5 batches of 200 frames.
            (0..5)
                .map(|_| {
                    let start = std::time::Instant::now();
                    for k in 0..200 {
                        memo_frame(&session, k as f64).unwrap();
                    }
                    start.elapsed().as_secs_f64() * 1e3 / 200.0
                })
                .fold(f64::INFINITY, f64::min)
        };
        let (small, large) = (per_frame_ms(15), per_frame_ms(1500));
        println!("memo under group + translate: {small:.4} ms/frame at 15 nodes, {large:.4} at 1,500");
        assert!(large < small * 3.0, "per-frame cost grows with the memoized subtree");
    }

    /// `Scene.equals` sees the instanced node structurally — template AND
    /// per-instance channels.
    #[test]
    fn instanced_nodes_compare_structurally() {
        let node = |tint: &str| {
            format!(
                "Scene.cube() |> Scene.lit(Color.rgb(1.0, 1.0, 1.0)) \
                 |> Scene.instanced([Instance.at(Vec3.make(1.0, 2.0, 3.0)) \
                 |> Instance.tint(Color.rgb({tint}))])"
            )
        };
        let same = node("0.2, 0.4, 0.8");
        assert!(equality(&format!("Scene.equals({same}, {same})")));
        assert!(!equality(&format!(
            "Scene.equals({}, {})",
            node("0.2, 0.4, 0.8"),
            node("0.8, 0.4, 0.2")
        )));
    }

    // --- Scene.equals / Frame.equals (structural equality for draw tests) ---

    /// Evaluate a `Scene.equals`/`Frame.equals` expression to its bool.
    fn equality(expr: &str) -> bool {
        match eval(&format!("let main = () => {expr}")) {
            Value::Bool(b) => b,
            other => panic!("expected a bool from `{expr}`, got {other}"),
        }
    }

    /// The shape of the walk: identical constructions are equal, and each
    /// axis of a scene node — transform, material, children, and their ORDER
    /// — makes it unequal on its own.
    #[test]
    fn scene_equals_compares_structurally() {
        assert!(equality("Scene.equals(Scene.cube(), Scene.cube())"));
        assert!(!equality("Scene.equals(Scene.cube(), Scene.sphere())"));
        // Transform.
        assert!(equality(
            "Scene.equals(Scene.cube() |> Scene.translate(Vec3.make(1.0, 0.0, 0.0)),\n\
                          Scene.cube() |> Scene.translate(Vec3.make(1.0, 0.0, 0.0)))"
        ));
        assert!(!equality(
            "Scene.equals(Scene.cube() |> Scene.translate(Vec3.make(1.0, 0.0, 0.0)),\n\
                          Scene.cube() |> Scene.translate(Vec3.make(2.0, 0.0, 0.0)))"
        ));
        // Material.
        assert!(!equality(
            "Scene.equals(Scene.cube() |> Scene.color(Color.rgb(1.0, 0.0, 0.0)),\n\
                          Scene.cube() |> Scene.color(Color.rgb(0.0, 1.0, 0.0)))"
        ));
        // Children, and their order.
        assert!(equality(
            "Scene.equals(Scene.group([Scene.cube(), Scene.sphere()]),\n\
                          Scene.group([Scene.cube(), Scene.sphere()]))"
        ));
        assert!(!equality(
            "Scene.equals(Scene.group([Scene.cube(), Scene.sphere()]),\n\
                          Scene.group([Scene.sphere(), Scene.cube()]))"
        ));
    }

    /// Floats compare EXACTLY — there is no epsilon, so a difference far
    /// below any visible one is still a difference. (The comparison happens
    /// after the boundary's f64 → f32 narrowing, so it is f32-exact: a
    /// difference the cast erases, like `0.1 + 0.2` against `0.3`, is gone
    /// before equality ever sees it.)
    #[test]
    fn scene_equals_compares_floats_exactly() {
        assert!(!equality(
            "Scene.equals(Scene.cube() |> Scene.scale(1.0),\n\
                          Scene.cube() |> Scene.scale(1.000001))"
        ));
        assert!(equality(
            "Scene.equals(Scene.cube() |> Scene.scale(0.1 + 0.2),\n\
                          Scene.cube() |> Scene.scale(0.3))"
        ));
    }

    /// An asset compares by LOCATOR, not by loaded content — nothing has been
    /// loaded here, so two nodes naming the same model are equal and two
    /// naming different files are not.
    #[test]
    fn scene_equals_compares_assets_by_locator() {
        assert!(equality(
            "Scene.equals(Scene.model(Asset.model(\"Xbot.glb\")),\n\
                          Scene.model(Asset.model(\"Xbot.glb\")))"
        ));
        assert!(!equality(
            "Scene.equals(Scene.model(Asset.model(\"Xbot.glb\")),\n\
                          Scene.model(Asset.model(\"Ybot.glb\")))"
        ));
    }

    /// Animation compares as DECLARED: the clip name plus the playhead
    /// seconds, so the same model at two times is two different scenes.
    #[test]
    fn scene_equals_compares_animation_as_declared() {
        assert!(equality(
            "Scene.equals(\n\
               Scene.model(Asset.model(\"Xbot.glb\")) |> Scene.animate(Anim.clip(\"walk\", 0.5)),\n\
               Scene.model(Asset.model(\"Xbot.glb\")) |> Scene.animate(Anim.clip(\"walk\", 0.5)))"
        ));
        assert!(!equality(
            "Scene.equals(\n\
               Scene.model(Asset.model(\"Xbot.glb\")) |> Scene.animate(Anim.clip(\"walk\", 0.5)),\n\
               Scene.model(Asset.model(\"Xbot.glb\")) |> Scene.animate(Anim.clip(\"walk\", 0.75)))"
        ));
        assert!(!equality(
            "Scene.equals(\n\
               Scene.model(Asset.model(\"Xbot.glb\")) |> Scene.animate(Anim.clip(\"walk\", 0.5)),\n\
               Scene.model(Asset.model(\"Xbot.glb\")) |> Scene.animate(Anim.clip(\"run\", 0.5)))"
        ));
    }

    /// `Frame.equals` is the same walk one level up: it compares the camera,
    /// the scene, the lights, and the frame's optional decorations.
    #[test]
    fn frame_equals_compares_the_whole_frame() {
        const CAMERA: &str =
            "Camera3D.lookAt(Vec3.make(0.0, 1.0, -4.0), Vec3.make(0.0, 0.0, 0.0))";
        assert!(equality(&format!(
            "Frame.equals(Frame.create({CAMERA}, Scene.cube()),\n\
                          Frame.create({CAMERA}, Scene.cube()))"
        )));
        // The scene differs.
        assert!(!equality(&format!(
            "Frame.equals(Frame.create({CAMERA}, Scene.cube()),\n\
                          Frame.create({CAMERA}, Scene.cube() |> Scene.rotateY(Angle.degrees(90.0))))"
        )));
        // The camera differs.
        assert!(!equality(&format!(
            "Frame.equals(Frame.create({CAMERA}, Scene.cube()),\n\
             Frame.create(Camera3D.lookAt(Vec3.make(0.0, 2.0, -4.0), Vec3.make(0.0, 0.0, 0.0)), \
Scene.cube()))"
        )));
        // The lights differ.
        assert!(!equality(&format!(
            "Frame.equals(Frame.createLit({CAMERA}, Scene.cube(), [Light.ambient(Color.rgb(1.0, 1.0, 1.0))]),\n\
                          Frame.createLit({CAMERA}, Scene.cube(), [Light.ambient(Color.rgb(0.5, 0.5, 0.5))]))"
        )));
        // A decoration only one side carries.
        assert!(!equality(&format!(
            "Frame.equals(Frame.create({CAMERA}, Scene.cube()),\n\
                          Frame.create({CAMERA}, Scene.cube()) \
|> Frame.withClearColor(Color.rgb(0.0, 0.0, 0.0)))"
        )));
    }

    /// The rest of the frame's fields, one assertion each — the ones a future
    /// refactor that stops deriving would silently drop.
    /// [xreview: Claude Low]
    #[test]
    fn frame_equals_covers_every_decoration() {
        const CAMERA: &str =
            "Camera3D.lookAt(Vec3.make(0.0, 1.0, -4.0), Vec3.make(0.0, 0.0, 0.0))";
        const CAM2D: &str = "Camera2D.create(16.0, 9.0)";
        let differs = |a: &str, b: &str| {
            assert!(
                !equality(&format!("Frame.equals({a}, {b})")),
                "expected {a} != {b}"
            );
        };
        let base = format!("Frame.create({CAMERA}, Scene.cube())");
        // Fog.
        differs(
            &base,
            &format!(
                "{base} |> Frame.withFog(Fog.linear(5.0, 50.0, Color.rgb(0.1, 0.1, 0.2)))"
            ),
        );
        // Skybox.
        differs(
            &base,
            &format!(
                "{base} |> Frame.withSkybox(Skybox.files(\"px.png\", \"nx.png\", \"py.png\", \
\"ny.png\", \"pz.png\", \"nz.png\"))"
            ),
        );
        // Render-target passes.
        let target = "RenderTarget.named(\"feed\") |> RenderTarget.sized(64.0, 64.0)";
        differs(
            &base,
            &format!("{base} |> Frame.withRenderTarget({target}, {base})"),
        );
        // 2D layers — present vs absent, and their content.
        let dot = "Sprite.circle(Color.rgb(1.0, 1.0, 1.0), 1.0)";
        differs(&base, &format!("{base} |> Frame.with2D({CAM2D}, {dot})"));
        differs(
            &format!("{base} |> Frame.with2D({CAM2D}, {dot})"),
            &format!("{base} |> Frame.with2D({CAM2D}, Sprite.blank())"),
        );
        // The `Frame.create2D` marker: a pure 2D frame is not the 3D frame
        // that happens to carry the same layer.
        differs(
            &format!("Frame.create2D({CAM2D}, {dot})"),
            &format!(
                "Frame.create({CAMERA}, Scene.group([])) |> Frame.with2D({CAM2D}, {dot})"
            ),
        );
        // …and each of those still equals itself.
        assert!(equality(&format!(
            "Frame.equals(Frame.create2D({CAM2D}, {dot}), Frame.create2D({CAM2D}, {dot}))"
        )));
    }

    // The Angle rule for animations: a bare clip name teaches Anim.clip.
    #[test]
    fn bare_strings_are_not_anims() {
        let module = functor_lang::lower(
            functor_lang::parse(
                "let main = () => Scene.model(Asset.model(\"Xbot.glb\")) |> Scene.animate(\"walk\")",
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

    /// A file that is only a `part` gets the PartViewer: a valid game whose
    /// turntable turns when idle, orbits on drag, and zooms on the wheel.
    #[test]
    fn part_only_file_runs_in_the_part_viewer() {
        let sources = vec![(
            std::path::PathBuf::from("game.fun"),
            "let part = Solid.box(10.0, 20.0, 30.0)\n\
             let idle = () => tick(init, 1.0, 1.0)\n\
             let turns = () => idle().orbit.yaw > init.orbit.yaw\n\
             let grab = () => mouseMove(mouseButton(idle(), Mouse.Left, true), 100.0, 50.0)\n\
             let anchors = () => grab().orbit.yaw == idle().orbit.yaw\n\
             let dragged = () => mouseMove(grab(), 140.0, 50.0)\n\
             let orbits = () => dragged().orbit.yaw < grab().orbit.yaw\n\
             let holds = () => tick(dragged(), 5.0, 6.0).orbit.yaw == dragged().orbit.yaw\n\
             let zooms = () => mouseWheel(init, 1.0).orbit.zoom > init.orbit.zoom\n"
                .to_string(),
        )];
        let project = functor_lang::project::load_sources_with_bundled_modules(
            sources,
            &functor_prelude::bundled_modules(),
        )
        .unwrap_or_else(|error| panic!("bundle loads: {}", error.render()));
        let diagnostics = project.check();
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|failure| panic!("session loads: {}", failure.error.message));
        crate::functor_lang_producer::validate_contract(
            "game.fun",
            &session,
            &crate::functor_lang_producer::EntryNames::UNPREFIXED,
        )
        .unwrap_or_else(|e| panic!("part viewer contract: {e}"));
        for check in ["turns", "anchors", "orbits", "holds", "zooms"] {
            assert!(
                matches!(
                    session.call(check, vec![], &mut FunctorHost),
                    Ok(Value::Bool(true))
                ),
                "{check}"
            );
        }
        let init = session.global("init").unwrap();
        session
            .call("draw", vec![init, Value::Number(1.0)], &mut FunctorHost)
            .unwrap_or_else(|e| panic!("draw: {}", e.message));
    }

    #[test]
    fn bundled_animator_executes_against_anim_host() {
        let sources = vec![(
            std::path::PathBuf::from("game.fun"),
            "let main =\n\
             Animator.pose(\n\
               Animator.play(\"run\", 2.0, Animator.start(\"idle\", 0.0)),\n\
               1.0,\n\
               2.5)"
                .to_string(),
        )];
        let project = functor_lang::project::load_sources_with_bundled_modules(
            sources,
            &functor_prelude::bundled_modules(),
        )
        .unwrap_or_else(|error| panic!("bundle loads: {}", error.render()));
        let diagnostics = project.check();
        assert!(
            diagnostics.is_empty(),
            "Animator should typecheck against Anim: {diagnostics:#?}"
        );
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|failure| panic!("session loads: {}", failure.error.message));
        let value = session.global("main").expect("entry value");
        let AnimExpr::Blend(entries) = anim_of(&value, "test", Span::new(0, 0)).expect("Anim") else {
            panic!("mid-transition Animator pose should be a blend");
        };
        assert_eq!(entries.len(), 2);
        assert!(
            matches!(&entries[0], (AnimExpr::Clip { name, playhead }, weight)
                if name == "idle" && (*playhead - 2.5).abs() < 1e-6 && (*weight - 0.5).abs() < 1e-6)
        );
        assert!(
            matches!(&entries[1], (AnimExpr::Clip { name, playhead }, weight)
                if name == "run" && (*playhead - 0.5).abs() < 1e-6 && (*weight - 0.5).abs() < 1e-6)
        );
    }

    // The full pose algebra composes through pipes and lands on the Model
    // node as nested AnimExpr data: rest |> rotate |> lookAt |> reach,
    // masked blends, additive layers.
    #[test]
    fn anim_algebra_composes_and_serializes() {
        let value = eval(
            "let main = () =>\n\
             Scene.model(Asset.model(\"glove.glb\")) |> Scene.animate(\n\
               Anim.rest()\n\
                 |> Anim.rotate(\"finger_index_0_r\", Angle.degrees(45.0), Angle.degrees(0.0), Angle.degrees(0.0))\n\
                 |> Anim.lookAt(\"finger_index_0_r\", Vec3.make(1.0, 2.0, 3.0), Angle.degrees(80.0), 0.75)\n\
                 |> Anim.reach(\"arm_r\", \"forearm_r\", \"wrist_r\", Vec3.make(3.0, 2.0, 1.0), 0.6)\n\
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
        let AnimExpr::Reach {
            root,
            middle,
            end,
            target,
            weight,
            expr,
        } = &**expr
        else {
            panic!("expected a Reach inside the mask, got {expr:?}");
        };
        assert_eq!(root, "arm_r");
        assert_eq!(middle, "forearm_r");
        assert_eq!(end, "wrist_r");
        assert_eq!(target, &[3.0, 2.0, 1.0]);
        assert!((*weight - 0.6).abs() < 1e-6);
        let AnimExpr::LookAt {
            joint,
            target,
            max_angle,
            weight,
            expr,
        } = &**expr
        else {
            panic!("expected a LookAt inside the mask, got {expr:?}");
        };
        assert_eq!(joint, "finger_index_0_r");
        assert_eq!(target, &[1.0, 2.0, 3.0]);
        assert!((*max_angle - 80.0_f32.to_radians()).abs() < 1e-6);
        assert!((*weight - 0.75).abs() < 1e-6);
        let AnimExpr::Rotate { joint, euler, expr } = &**expr else {
            panic!("expected a Rotate inside LookAt, got {expr:?}");
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

    #[test]
    fn anim_look_at_enforces_spatial_and_angle_brands() {
        for source in [
            "let main = () => Anim.rest() |> Anim.lookAt(\"head\", { x: 0.0, y: 1.0, z: 2.0 }, Angle.degrees(80.0), 1.0)",
            "let main = () => Anim.rest() |> Anim.lookAt(\"head\", Vec3.make(0.0, 1.0, 2.0), 1.4, 1.0)",
        ] {
            let module =
                functor_lang::lower(functor_lang::parse(source).unwrap()).unwrap();
            let failure =
                functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
                    .err()
                    .expect("should fail");
            assert!(
                failure.error.message.contains("Vec3")
                    || failure.error.message.contains("Angle"),
                "message: {}",
                failure.error.message
            );
        }
    }

    #[test]
    fn anim_look_at_rejects_an_invalid_deflection_limit() {
        assert_eq!(
            fail_message(
                "let main = () => Anim.rest() |> Anim.lookAt(\"head\", Vec3.make(0.0, 1.0, 2.0), Angle.degrees(181.0), 1.0)"
            ),
            "usage: Anim.lookAt(\"jointName\", target, maxDeflection, weight, anim) — a non-empty joint name, model-space Vec3 target, Angle limit from 0 to 180 degrees, and weight"
        );
    }

    #[test]
    fn anim_reach_enforces_spatial_brand_and_distinct_joint_names() {
        let bare_target = fail_message(
            "let main = () => Anim.rest() |> Anim.reach(\"arm\", \"forearm\", \"hand\", { x: 0.0, y: 1.0, z: 2.0 }, 1.0)"
        );
        assert!(bare_target.contains("Vec3"), "message: {bare_target}");

        let duplicate_name = fail_message(
            "let main = () => Anim.rest() |> Anim.reach(\"arm\", \"arm\", \"hand\", Vec3.make(0.0, 1.0, 2.0), 1.0)"
        );
        assert!(
            duplicate_name.contains("three distinct non-empty joint names"),
            "message: {duplicate_name}"
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
            "Effect.play: expected a sound Asset value (the generated manifest's \
Assets.<name>, or Asset.sound(…)), got a number"
        );
        assert_eq!(
            fail_message("let main = () => AudioSource.ambient(\"bed\", 3.0)"),
            "AudioSource.ambient: expected a sound Asset value (the generated \
manifest's Assets.<name>, or Asset.sound(…)), got a number"
        );
    }

    // Degrees and radians agree where they should: 90° == τ/4 rad.
    #[test]
    fn degrees_and_radians_agree() {
        let deg = frame_of(
            "let main = () => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), \
Scene.cube() |> Scene.rotateY(Angle.degrees(90.0)))",
        );
        let rad = frame_of(
            "let main = () => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), \
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
            // Most paths reject the garbage `true` arg with a usage/type
            // error; a path whose contract accepts ANY value
            // (`Attr.onClick(msg)`) may legitimately succeed — an Ok proves
            // dispatch just as well.
            if let Err(error) = result {
                assert!(
                    !error.message.starts_with("internal:"),
                    "`{path}` fell through to the internal fallback: {}",
                    error.message
                );
            }
        }
        // An any-msg path (Attr.onClick) registered a handler above — drop it
        // so the table never leaks into other tests on this thread.
        let _ = take_ui_handlers();
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
               Camera3D.lookAt(Vec3.make(0.0, 2.0, -8.0), Vec3.make(0.0, 1.0, 0.0)),\n\
               Scene.group([\n\
                 Scene.plane() |> Scene.lit(Color.rgb(0.6, 0.6, 0.6)),\n\
                 Scene.quad() |> Scene.screen(feed),\n\
               ]),\n\
               [Light.ambient(Color.rgb(0.1, 0.1, 0.1))])\n\
             |> Frame.withRenderTarget(feed, Frame.createLit(\n\
                  Camera3D.lookAt(Vec3.make(0.0, 4.0, -6.0), Vec3.make(0.0, 0.5, 0.0)),\n\
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

    // The ui-target vocabulary: a branded target declared once, written by a
    // Ui.* tree (Frame.withUiTarget) and read by Scene.screen — the frame
    // carries the (target, view) pass and it all speaks the protocol.
    #[test]
    fn functor_lang_snippet_declares_a_ui_target_frame() {
        let frame = frame_of(
            "let hud = RenderTarget.named(\"hud\") |> RenderTarget.sized(256.0, 128.0)\n\
             let main = () =>\n\
             Frame.create(\n\
               Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
               Scene.quad() |> Scene.screen(hud))\n\
             |> Frame.withUiTarget(hud, Ui.column([Ui.text(\"score 3\")]))",
        );
        assert_eq!(frame.ui_targets.len(), 1);
        let pass = &frame.ui_targets[0];
        assert_eq!(pass.target.id, "hud");
        assert_eq!((pass.target.width, pass.target.height), (256, 128));
        assert!(matches!(&pass.view, View::Column(items) if items.len() == 1));
        // The whole frame round-trips through the protocol.
        let json = serde_json::to_string(&frame).expect("serialize");
        let back: Frame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    // Interactive widgets in a ui-target tree are display-only: the draw eval
    // pushes their handlers into the thread-local table, and the producers
    // drop that table right after the draw eval (with a one-time warning) so
    // it neither leaks across frames nor pollutes the `ui` hook's table.
    // This pins WHERE those handlers land — the draw eval's table.
    #[test]
    fn ui_target_button_handlers_land_in_the_draw_eval_table() {
        let _ = take_ui_handlers(); // isolate from other tests on this thread
        let frame = frame_of(
            "let hud = RenderTarget.named(\"hud\")\n\
             let main = () =>\n\
             Frame.create(\n\
               Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
               Scene.quad() |> Scene.screen(hud))\n\
             |> Frame.withUiTarget(hud, Ui.button(\"go\", 1))",
        );
        assert!(matches!(
            &frame.ui_targets[0].view,
            View::Button { slot: 0, .. }
        ));
        let handlers = take_ui_handlers();
        assert_eq!(handlers.len(), 1, "the button registered one handler");
    }

    // Scene.screen is an emissive (fullbright) white surface over the target's
    // texture — screens glow, unaffected by scene lighting.
    #[test]
    fn screen_material_is_emissive_white_over_the_target_texture() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(\n\
               Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)),\n\
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
                "let main = () => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), \
Scene.cube()) |> Frame.withRenderTarget(\"feed\", Frame.create(\
Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube()))"
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
             Frame.create(Camera3D.lookAt(Vec3.make(0.0, 2.0, -8.0), Vec3.make(0.0, 1.0, 0.0)), Scene.cube())\n\
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
             Frame.create(Camera3D.lookAt(Vec3.make(0.0, 2.0, -8.0), Vec3.make(0.0, 1.0, 0.0)), Scene.cube())\n\
             |> Frame.withClearColor(Color.rgb(0.2, 0.4, 0.6))",
        );
        assert_eq!(frame.clear_color, Some([0.2, 0.4, 0.6]));
        assert_eq!(frame.resolved_clear_color(), [0.2, 0.4, 0.6]);

        // Without the override, the resolved clear color is the engine default;
        // with fog it's the fog color; withClearColor overrides even that.
        let plain = frame_of(
            "let main = () => \
             Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())",
        );
        assert_eq!(plain.clear_color, None);
        assert_eq!(plain.resolved_clear_color(), [0.1, 0.2, 0.3]);

        let both = frame_of(
            "let main = () => \
             Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube()) \
             |> Frame.withFog(Fog.linear(4.0, 30.0, Color.rgb(0.5, 0.6, 0.7))) \
             |> Frame.withClearColor(Color.rgb(0.0, 0.0, 0.0))",
        );
        assert_eq!(both.resolved_clear_color(), [0.0, 0.0, 0.0]);
    }

    // Frame.withStandInRoom / withSurfaceGrid record their room and grid; a
    // non-positive cell is a teaching error.
    #[test]
    fn functor_lang_snippet_declares_stand_in_room_and_surface_grid() {
        let frame = frame_of(
            "let main = () =>\n\
             Frame.create(Camera3D.lookAt(Vec3.make(0.0, 1.6, 0.0), Vec3.make(0.0, 1.6, 1.0)), Scene.group([]))\n\
             |> Frame.withPassthrough\n\
             |> Frame.withStandInRoom(Scene.cube())\n\
             |> Frame.withSurfaceGrid(0.25, Color.rgb(0.0, 1.0, 1.0))\n\
             |> Frame.withFusedRoom(0.5, Color.rgb(1.0, 0.0, 1.0))",
        );
        assert_eq!(frame.stand_in_room.as_deref(), Some(&Scene3D::cube()));
        assert_eq!(
            frame.fused_room,
            Some(crate::surface_grid::SurfaceGrid {
                cell: 0.5,
                color: [1.0, 0.0, 1.0],
            })
        );
        assert_eq!(
            frame.surface_grid,
            Some(crate::surface_grid::SurfaceGrid {
                cell: 0.25,
                color: [0.0, 1.0, 1.0],
            })
        );
        let src = "let main = () =>\n\
             Frame.create(Camera3D.lookAt(Vec3.make(0.0, 1.6, 0.0), Vec3.make(0.0, 1.6, 1.0)), Scene.group([]))\n\
             |> Frame.withSurfaceGrid(0.0, Color.rgb(0.0, 1.0, 1.0))";
        let module = functor_lang::lower(functor_lang::parse(src).unwrap()).unwrap();
        let message = functor_lang::run_with_host(&module, Tracing::Off, &mut FunctorHost)
            .err()
            .expect("a zero cell should fail")
            .error
            .message;
        assert!(message.contains("cell size must be positive"), "{message}");
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
                "let main = () => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), \
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
             Frame.create(Camera3D.lookAt(Vec3.make(0.0, 2.0, -8.0), Vec3.make(0.0, 1.0, 0.0)), Scene.cube())\n\
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
                "let main = () => Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -5.0), Vec3.make(0.0, 0.0, 0.0)), \
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
             |> Physics.linearDamping(0.55)\n\
             |> Physics.angularDamping(1.25)\n\
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
        assert_eq!(scene.bodies[1].linear_damping, 0.55);
        assert_eq!(scene.bodies[1].angular_damping, 1.25);
        // Undeclared damping is 0.0 — no drag, the pre-existing behavior.
        assert_eq!(scene.bodies[0].linear_damping, 0.0);
        assert_eq!(scene.bodies[0].angular_damping, 0.0);
        assert!(scene.bodies[2].sensor);
    }

    #[test]
    fn physics_body_axis_rotations_are_canonical_and_body_centered() {
        let value = eval(
            "let shape = Physics.box(4.0, 1.0, 2.0)\n\
             let main = () => Physics.scene(Vec3.make(0.0, 0.0, 0.0), [\n\
               Physics.fixed(Physics.tag(\"x\"), shape)\n\
                 |> Physics.at(Vec3.make(3.0, 4.0, 5.0))\n\
                 |> Physics.rotateX(Angle.degrees(90.0)),\n\
               Physics.fixed(Physics.tag(\"y\"), shape)\n\
                 |> Physics.rotateY(Angle.degrees(90.0)),\n\
               Physics.fixed(Physics.tag(\"z\"), shape)\n\
                 |> Physics.rotateZ(Angle.degrees(90.0)),\n\
               Physics.fixed(Physics.tag(\"y-plus-turn\"), shape)\n\
                 |> Physics.rotateY(Angle.degrees(450.0)),\n\
             ])",
        );
        let scene = physics_scene_value(&value).expect("a PhysicsScene");
        let half_sqrt = std::f32::consts::FRAC_1_SQRT_2;
        let assert_close = |actual: [f32; 4], expected: [f32; 4]| {
            for (actual, expected) in actual.into_iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 1e-6,
                    "quaternion component: expected {expected}, got {actual}"
                );
            }
        };

        assert_close(scene.bodies[0].rotation, [half_sqrt, 0.0, 0.0, half_sqrt]);
        assert_close(scene.bodies[1].rotation, [0.0, half_sqrt, 0.0, half_sqrt]);
        assert_close(scene.bodies[2].rotation, [0.0, 0.0, half_sqrt, half_sqrt]);
        assert_close(scene.bodies[3].rotation, scene.bodies[1].rotation);
        assert_eq!(scene.bodies[0].position, [3.0, 4.0, 5.0]);
        for body in &scene.bodies {
            let norm = body.rotation.iter().map(|component| component * component).sum::<f32>();
            assert!((norm - 1.0).abs() < 1e-6, "rotation was not normalized: {norm}");
            assert!(body.rotation[3] >= 0.0, "rotation was not canonical: {:?}", body.rotation);
        }
    }

    #[test]
    fn physics_body_rotation_pipeline_applies_the_outer_modifier_last() {
        let value = eval(
            "let shape = Physics.box(1.0, 2.0, 3.0)\n\
             let main = () => Physics.scene(Vec3.make(0.0, 0.0, 0.0), [\n\
               Physics.fixed(Physics.tag(\"xy\"), shape)\n\
                 |> Physics.rotateX(Angle.degrees(90.0))\n\
                 |> Physics.rotateY(Angle.degrees(90.0)),\n\
               Physics.fixed(Physics.tag(\"yx\"), shape)\n\
                 |> Physics.rotateY(Angle.degrees(90.0))\n\
                 |> Physics.rotateX(Angle.degrees(90.0)),\n\
             ])",
        );
        let scene = physics_scene_value(&value).expect("a PhysicsScene");
        let assert_close = |actual: [f32; 4], expected: [f32; 4]| {
            for (actual, expected) in actual.into_iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 1e-6,
                    "quaternion component: expected {expected}, got {actual}"
                );
            }
        };

        assert_close(scene.bodies[0].rotation, [0.5, 0.5, -0.5, 0.5]);
        assert_close(scene.bodies[1].rotation, [0.5, 0.5, 0.5, 0.5]);
    }

    #[test]
    fn physics_body_rotation_requires_angles_and_rejects_heightfields() {
        assert_eq!(
            fail_message(
                "let main = () => Physics.fixed(Physics.tag(\"ramp\"), \
Physics.box(4.0, 1.0, 2.0)) |> Physics.rotateY(1.57)"
            ),
            "Physics.rotateY: expected an Angle, got a bare number — say which unit: \
`90deg` / `1.5rad`, or Angle.degrees(…) / Angle.radians(…)"
        );
        assert_eq!(
            fail_message(
                "let terrain = Terrain.heightmap(Asset.texture(\"terrain.png\"), \
20.0, 20.0, -1.0, 4.0)\n\
                 let main = () => Physics.heightfield(Physics.tag(\"terrain\"), terrain) \
|> Physics.rotateX(Angle.degrees(5.0))"
            ),
            "Physics.rotateX: heightfield bodies cannot be rotated — terrain rendering is \
translation-only; use Physics.at with an unrotated Scene.terrain"
        );
    }

    // `Physics.upright` locks a body's rotation — the character-capsule
    // attribute. Default is off, so existing bodies keep tumbling as before.
    #[test]
    fn physics_upright_locks_rotation_and_defaults_off() {
        let value = eval(
            "let main = () => Physics.scene(Vec3.make(0.0, -9.81, 0.0), [\n\
               Physics.dynamic(\"player\", Physics.capsule(0.5, 0.4)) |> Physics.upright,\n\
               Physics.dynamic(\"barrel\", Physics.capsule(0.5, 0.4)),\n\
             ])",
        );
        let scene = physics_scene_value(&value).expect("a PhysicsScene");
        assert!(scene.bodies[0].rotation_locked);
        assert!(!scene.bodies[1].rotation_locked);
    }

    #[test]
    fn physics_heightfield_consumes_the_shared_terrain_descriptor() {
        let value = eval(
            "let terrain = Terrain.heightmap(Asset.texture(\"physics-terrain.png\"), 4000.0, 4000.0, -80.0, 520.0)\n\
             let main = () => Physics.heightfield(Physics.tag(\"world\"), terrain)",
        );
        let Value::HostData(data) = value else {
            panic!("expected Body host data");
        };
        let body = data
            .as_any()
            .downcast_ref::<FunctorLangBody>()
            .expect("Body");
        assert_eq!(body.0.kind, physics::BodyKind::Fixed);
        let physics::Shape::Heightfield { geometry, data } = &body.0.shape else {
            panic!("expected a heightfield");
        };
        assert_eq!((geometry.width, geometry.depth), (4000.0, 4000.0));
        assert!(data.is_none(), "hydration belongs to World::reconcile");

        let json = serde_json::to_string(&body.0).expect("serialize");
        let back: physics::Body = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, body.0);
    }

    // `Physics.trimesh` reads `Model.skinned`'s byte layout. One value is one
    // shared mesh; bad data and a dynamic trimesh are teaching errors.
    #[test]
    fn physics_trimesh_decodes_bytes_and_refuses_bad_data_and_dynamic_bodies() {
        let mesh = |positions: &str, triangles: &str| {
            format!(
                "let floor = Physics.trimesh({{ positions: Bytes.fromF32le([{positions}]), \
triangles: Bytes.fromU32le([{triangles}]) }})\n"
            )
        };
        let triangle = mesh("0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0", "0.0, 1.0, 2.0");
        let value = eval(&format!(
            "{triangle}let main = () => Physics.scene(Vec3.make(0.0, 0.0, 0.0), [\n\
               Physics.fixed(Physics.tag(\"a\"), floor),\n\
               Physics.kinematic(Physics.tag(\"b\"), floor) |> Physics.at(Vec3.make(0.0, 1.0, 0.0)),\n\
             ])"
        ));
        let scene = physics_scene_value(&value).expect("a PhysicsScene");
        let [a, b] = [0, 1].map(|i| match &scene.bodies[i].shape {
            physics::Shape::TriMesh { mesh } => mesh.clone(),
            other => panic!("expected a trimesh, got {other:?}"),
        });
        assert!(std::sync::Arc::ptr_eq(&a, &b), "one value, one mesh");
        assert_eq!(scene.bodies[1].kind, physics::BodyKind::Kinematic);

        let fails = |src: String| fail_message(&format!("{src}let main = () => floor"));
        assert_eq!(
            fail_message(&format!(
                "{triangle}let main = () => Physics.dynamic(Physics.tag(\"a\"), floor)"
            )),
            "Physics.dynamic: a trimesh is a surface with no volume or mass, so it can't be \
dynamic — use Physics.fixed or Physics.kinematic for it, and a box, sphere or capsule for the \
moving body"
        );
        assert_eq!(
            fails(mesh("0.0, 0.0", "0.0, 1.0, 2.0")),
            "Physics.trimesh: positions: 8 bytes is not a whole number of 12-byte entries"
        );
        assert_eq!(
            fails(mesh("0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0", "0.0, 1.0, 7.0")),
            "Physics.trimesh: triangles: triangle 0 uses vertex 7, but there are only 3 vertices"
        );
        assert_eq!(
            fails(mesh("0.0, 0.0, 0.0, 0.0 / 0.0, 0.0, 0.0, 0.0, 0.0, 1.0", "0.0, 1.0, 2.0")),
            "Physics.trimesh: positions: vertex 1 is not finite ([NaN, 0.0, 0.0])"
        );
        assert_eq!(
            fails(mesh("0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0", "0.0, 1.0, 2.0")),
            "Physics.trimesh: triangles: no triangle has any area (1 given)"
        );
        assert_eq!(
            fail_message(
                "let main = () => Physics.trimesh({ positions: 1.0, \
triangles: Bytes.fromU32le([0.0, 1.0, 2.0]) })"
            ),
            "Physics.trimesh: `positions` must be Bytes"
        );
    }

    #[test]
    fn physics_fused_room_is_a_fixed_body_that_stays_put() {
        let value = eval(
            "let main = () => Physics.scene(Vec3.make(0.0, -9.81, 0.0), [\n\
               Physics.fusedRoom(Physics.tag(\"room\")) |> Physics.restitution(0.7),\n\
             ])",
        );
        let scene = physics_scene_value(&value).expect("a PhysicsScene");
        let room = &scene.bodies[0];
        assert_eq!((room.kind, room.restitution), (physics::BodyKind::Fixed, 0.7));
        let physics::Shape::FusedRoom { room } = &room.shape else {
            panic!("expected a fused room");
        };
        assert!(room.chunks.is_empty(), "the shell's room resolves at the step");

        let moved = "Physics.at: the fused room is already in world space — declare \
Physics.fusedRoom(tag) without moving or rotating it";
        assert_eq!(
            fail_message(
                "let main = () => Physics.fusedRoom(Physics.tag(\"room\")) \
|> Physics.at(Vec3.make(0.0, 1.0, 0.0))"
            ),
            moved
        );
        assert_eq!(
            fail_message(
                "let main = () => Physics.fusedRoom(Physics.tag(\"room\")) \
|> Physics.rotateY(Angle.degrees(5.0))"
            ),
            moved.replace("Physics.at", "Physics.rotateY")
        );
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

    /// A field of a record `Value`, for the synchronous-query tests.
    fn field(value: &Value, name: &str) -> Value {
        let Value::Record(fields) = value else {
            panic!("expected a record, got {}", value.kind_name());
        };
        fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("no field `{name}`"))
    }

    fn num(value: &Value, name: &str) -> f64 {
        match field(value, name) {
            Value::Number(n) => n,
            other => panic!("field `{name}` is {}", other.kind_name()),
        }
    }

    /// Declare a floor at y = 0 and a ball, reconcile and step the singleton
    /// world the way the driver does, and hand back nothing — the world is the
    /// thread-local the prelude reads.
    fn step_floor_and_ball(ball_y: f32, steps: usize) {
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        let scene = crate::physics::PhysicsScene::create(
            [0.0, -9.81, 0.0],
            vec![
                crate::physics::Body::fixed(
                    "floor".to_string(),
                    crate::physics::Shape::Cuboid {
                        extents: [20.0, 0.2, 20.0],
                    },
                )
                .at([0.0, -0.1, 0.0]),
                crate::physics::Body::dynamic(
                    "ball".to_string(),
                    crate::physics::Shape::Sphere { radius: 0.5 },
                )
                .at([0.0, ball_y, 0.0]),
            ],
        );
        crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| {
            w.reconcile(&scene);
            for _ in 0..steps {
                w.step_fixed();
            }
        });
    }

    // The synchronous character-controller queries: `Physics.linearVelocity`
    // and `Physics.cast` are plain reads of the stepped world, answering in
    // place rather than through a tagger — so `tick` can branch on them.
    #[test]
    fn sync_queries_read_the_stepped_world() {
        step_floor_and_ball(5.0, 30);

        // The falling ball has picked up downward velocity.
        let vel = eval("let main = () => Physics.linearVelocity(\"ball\")");
        let vy = num(&vel, "y");
        assert!(vy < -1.0, "ball should be falling, vy = {vy}");
        // Tolerance rather than exact equality: free fall under [0,-9.81,0]
        // happens to leave these bit-zero today, but any contact would make an
        // exact assertion flake.
        assert!(num(&vel, "x").abs() < 1e-6);
        assert!(num(&vel, "z").abs() < 1e-6);

        // A downward ray from well above hits the nearest body along it — the
        // still-falling ball, whose top surface faces +Y. Same record shape the
        // deferred effect hands its tagger.
        let hit = eval(
            "let main = () => Physics.cast(Vec3.make(0.0, 10.0, 0.0), \
             Vec3.make(0.0, -1.0, 0.0), 100.0)",
        );
        assert!(matches!(field(&hit, "hit"), Value::Bool(true)));
        assert!(matches!(field(&hit, "tag"), Value::String(s) if &*s == "ball"));
        assert!((num(&hit, "ny") - 1.0).abs() < 1e-5, "sphere top normal is +Y");
        assert!(num(&hit, "distance") > 0.0);

        // Excluding the ball lets the same ray through to the floor behind it.
        let floor = eval(
            "let main = () => Physics.castExcluding(Physics.tag(\"ball\"), \
             Vec3.make(0.0, 10.0, 0.0), Vec3.make(0.0, -1.0, 0.0), 100.0)",
        );
        assert!(matches!(field(&floor, "tag"), Value::String(s) if &*s == "floor"));
        assert!(
            num(&floor, "distance") > num(&hit, "distance"),
            "the floor is further than the ball in front of it"
        );

        // A ray into empty space is a miss, not an error.
        let miss = eval(
            "let main = () => Physics.cast(Vec3.make(0.0, 10.0, 0.0), \
             Vec3.make(0.0, 1.0, 0.0), 100.0)",
        );
        assert!(matches!(field(&miss, "hit"), Value::Bool(false)));
        assert_eq!(num(&miss, "distance"), 0.0);
        assert!(matches!(field(&miss, "tag"), Value::String(s) if s.is_empty()));

        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
    }

    // The grounding probe: cast from inside the character's own collider. The
    // plain cast hits the character itself; excluding it finds the ground.
    #[test]
    fn cast_excluding_skips_the_probing_body() {
        // Rest the ball on the floor so a probe from its centre has both its
        // own collider and the floor beneath it.
        step_floor_and_ball(0.5, 240);

        let down = "Vec3.make(0.0, -1.0, 0.0)";
        let at = "let origin = (p) => Vec3.make(p.x, p.y, p.z)\n";

        let selfhit = eval(&format!(
            "{at}let probe = (p) => Physics.cast(origin(p), {down}, 10.0)\n\
             let main = () => probe(Physics.position(\"ball\"))"
        ));
        assert!(matches!(field(&selfhit, "tag"), Value::String(s) if &*s == "ball"));

        let ground = eval(&format!(
            "{at}let probe = (p) => \
               Physics.castExcluding(Physics.tag(\"ball\"), origin(p), {down}, 10.0)\n\
             let main = () => probe(Physics.position(\"ball\"))"
        ));
        assert!(matches!(field(&ground, "tag"), Value::String(s) if &*s == "floor"));
        assert!(matches!(field(&ground, "hit"), Value::Bool(true)));
        // Standing on the floor: the probe reaches it within the ball's radius.
        let d = num(&ground, "distance");
        assert!(d > 0.0 && d < 1.0, "grounded distance = {d}");

        // Excluding a tag that isn't in the world excludes nothing: the ray
        // still hits the nearest body along it (the ball resting on the
        // floor), exactly as a plain `Physics.cast` would. This is what lets a
        // not-yet-spawned character probe cleanly on its first frame instead
        // of erroring.
        let unknown = eval(
            "let main = () => Physics.castExcluding(Physics.tag(\"nobody\"), \
             Vec3.make(0.0, 10.0, 0.0), Vec3.make(0.0, -1.0, 0.0), 100.0)",
        );
        assert!(matches!(field(&unknown, "tag"), Value::String(s) if &*s == "ball"));

        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
    }

    #[test]
    fn physics_character_builds_a_kinematic_capsule_and_validates_its_settings() {
        let ok = "{ stepHeight: 0.35, maxSlope: Angle.degrees(45.0), snap: 0.3, offset: 0.02 }";
        let character = |shape: &str, settings: &str| {
            format!(
                "let main = () => Physics.character(Physics.tag(\"p\"), {shape}, {settings})"
            )
        };
        let value = eval(&format!(
            "let main = () => Physics.scene(Vec3.make(0.0, 0.0, 0.0), \
             [Physics.character(Physics.tag(\"p\"), Physics.capsule(0.6, 0.3), {ok})])"
        ));
        let body = &physics_scene_value(&value).expect("a scene").bodies[0];
        assert_eq!(body.kind, physics::BodyKind::Kinematic);
        let settings = body.character.expect("character settings");
        assert!((settings.max_slope - std::f32::consts::FRAC_PI_4).abs() < 1e-6);
        assert_eq!((settings.step_height, settings.snap, settings.offset), (0.35, 0.3, 0.02));

        let fails = |shape: &str, settings: &str| fail_message(&character(shape, settings));
        assert_eq!(
            fails("Physics.sphere(0.3)", ok),
            "Physics.character: the shape must be a Physics.capsule"
        );
        assert_eq!(
            fails(
                "Physics.capsule(0.6, 0.3)",
                "{ stepHeight: 0.35, maxSlope: Angle.degrees(90.0), snap: 0.3, offset: 0.02 }"
            ),
            "Physics.character: `maxSlope` must be between 0deg and 90deg"
        );
        assert_eq!(
            fails(
                "Physics.capsule(0.6, 0.3)",
                "{ stepHeight: -1.0, maxSlope: Angle.degrees(45.0), snap: 0.3, offset: 0.02 }"
            ),
            "Physics.character: `stepHeight` must be a non-negative number of metres"
        );
        assert_eq!(
            fails(
                "Physics.capsule(0.6, 0.3)",
                "{ stepHeight: 0.35, maxSlope: Angle.degrees(45.0), snap: 0.3, offset: 0.0 }"
            ),
            "Physics.character: `offset` must be positive"
        );

        // `grounded` reads a character; anything else is an error.
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        assert_eq!(
            fail_message("let main = () => Physics.grounded(Physics.tag(\"p\"))"),
            "no Physics.character tagged \"p\" in the physics world"
        );
        let scene = crate::physics::PhysicsScene::create([0.0; 3], vec![body.clone()]);
        crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| w.reconcile(&scene));
        let grounded = eval("let main = () => Physics.grounded(Physics.tag(\"p\"))");
        assert!(matches!(grounded, Value::Bool(false)), "no move yet");
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
    }

    // `rayHit.triangle` is the trimesh source index (-1 elsewhere), and
    // `Physics.shapeCast` sweeps a primitive into the same record.
    #[test]
    fn shape_casts_and_triangle_indices_reach_functor_lang() {
        // The floor y = 0 as source triangles 1 and 2 (0 is degenerate).
        let floor = crate::physics::TriMeshData::new(
            vec![[-5.0, 0.0, -5.0], [5.0, 0.0, -5.0], [5.0, 0.0, 5.0], [-5.0, 0.0, 5.0]],
            vec![[0, 0, 1], [0, 2, 1], [0, 3, 2]],
        )
        .unwrap();
        let scene = crate::physics::PhysicsScene::create(
            [0.0; 3],
            vec![crate::physics::Body::fixed(
                "floor".to_string(),
                crate::physics::Shape::TriMesh {
                    mesh: std::sync::Arc::new(floor),
                },
            )],
        );
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| {
            w.reconcile(&scene);
            w.step_fixed();
        });
        let down = "Vec3.make(0.0, -1.0, 0.0)";
        let hit = eval(&format!(
            "let main = () => Physics.shapeCast(Physics.sphere(0.5), \
             Vec3.make(2.0, 3.0, -1.0), {down}, 10.0)"
        ));
        assert!(matches!(field(&hit, "tag"), Value::String(s) if &*s == "floor"));
        assert!((num(&hit, "distance") - 2.5).abs() < 1e-3);
        assert_eq!(num(&hit, "triangle"), 1.0);
        let ray = eval(&format!(
            "let main = () => Physics.cast(Vec3.make(-2.0, 3.0, 1.0), {down}, 10.0)"
        ));
        assert_eq!(num(&ray, "triangle"), 2.0);
        let miss = eval(&format!(
            "let main = () => Physics.cast(Vec3.make(9.0, 3.0, 0.0), {down}, 10.0)"
        ));
        assert_eq!(num(&miss, "triangle"), -1.0);
        // Excluding the floor sweeps through it.
        let through = eval(&format!(
            "let main = () => Physics.shapeCastExcluding(Physics.tag(\"floor\"), \
             Physics.sphere(0.5), Vec3.make(2.0, 3.0, -1.0), {down}, 10.0)"
        ));
        assert!(matches!(field(&through, "hit"), Value::Bool(false)));
        let err = fail_message(
            "let main = () => Physics.shapeCast(Physics.box(1.0, 1.0, 1.0), \
             Vec3.make(0.0, 3.0, 0.0), Vec3.make(0.0, 0.0, 0.0), 10.0)",
        );
        assert!(err.starts_with("Physics.shapeCast: the direction"), "{err}");
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
    }

    // The control loop a character controller needs closes inside ONE frame:
    // `tick` reads the world synchronously (position / velocity / grounded),
    // decides, and returns a command effect; draining that effect queues it,
    // and the frame's step applies it. No `update` round trip, no second frame
    // — this is the ordering `FrameCtx::before_physics` → `physics_phase`
    // performs, spelled out.
    #[test]
    fn a_synchronous_read_decide_write_loop_closes_in_one_frame() {
        step_floor_and_ball(0.5, 240);

        // The "tick": read grounded-ness synchronously, then jump.
        let src = "let origin = (p) => Vec3.make(p.x, p.y, p.z)\n\
                   let hitOf = (h) => h.hit\n\
                   let grounded = (p) => \
                     hitOf(Physics.castExcluding(Physics.tag(\"ball\"), origin(p), \
                       Vec3.make(0.0, -1.0, 0.0), 1.0))\n\
                   let main = () => \
                     if grounded(Physics.position(\"ball\")) then \
                       Physics.setVelocity(\"ball\", Vec3.make(0.0, 6.0, 0.0)) \
                     else \
                       Effect.none()";
        let effect = eval(src);
        let Value::HostData(data) = &effect else {
            panic!("the grounded read should have produced a command effect");
        };
        let tree = &data
            .as_any()
            .downcast_ref::<FunctorLangEffect>()
            .expect("Effect")
            .0;

        // Draining needs no `update` hook — a command effect carries no tagger.
        let module = functor_lang::lower(functor_lang::parse("let main = () => 0.0").unwrap())
            .unwrap();
        let session = functor_lang::Session::load(&module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("load failed: {}", f.error.message));
        let mut model = Value::Number(0.0);
        let mut log: Vec<Entry> = Vec::new();
        let mut runner = FakeEffects::new(0.0, vec![]);
        let deferred = drain_effects(
            &session,
            "update",
            &mut model,
            tree.clone(),
            &mut runner,
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            false,
        );
        assert!(
            deferred.is_empty(),
            "a command effect must not defer past the step"
        );

        // The same frame's step applies it: the ball is now rising.
        crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| {
            w.step_frame(1.0 / 60.0);
            let v = w.body_velocity("ball").unwrap();
            assert!(v[1] > 0.0, "the jump should have landed this frame: {v:?}");
        });

        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
    }

    // Boundary errors mirror the deferred `Physics.raycast` arm, and an
    // undeclared tag is loud like `Physics.position`.
    #[test]
    fn sync_query_boundaries_are_rejected() {
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        assert!(
            fail_message("let main = () => Physics.linearVelocity(\"ghost\")")
                .contains("no body tagged \"ghost\""),
            "an undeclared tag should be a loud error"
        );
        // The docs promise an empty world answers a cast as an ordinary miss
        // rather than erroring — the asymmetry with the read above is
        // deliberate: a tag read needs an identity that exists, a spatial
        // query does not, and "nothing there" is what a controller branches on.
        let empty = eval(
            "let main = () => Physics.cast(Vec3.make(0.0, 10.0, 0.0), \
             Vec3.make(0.0, -1.0, 0.0), 100.0)",
        );
        assert!(matches!(field(&empty, "hit"), Value::Bool(false)));
        assert_eq!(
            fail_message(
                "let main = () => Physics.cast(Vec3.make(0.0, 0.0, 0.0), \
                 Vec3.make(0.0, 0.0, 0.0), 10.0)"
            ),
            "Physics.cast: the direction must be finite and non-zero, got (0, 0, 0)"
        );
        assert_eq!(
            fail_message(
                "let main = () => Physics.castExcluding(Physics.tag(\"a\"), \
                 Vec3.make(0.0, 0.0, 0.0), Vec3.make(0.0, 0.0, 0.0), 10.0)"
            ),
            "Physics.castExcluding: the direction must be finite and non-zero, got (0, 0, 0)"
        );
        // A DEGENERATE but nonzero direction is rejected too, rather than
        // silently answered as a miss. 1e-27 (built by multiplication —
        // Functor Lang has no exponent literal) is finite in f64, but its f32
        // square underflows to zero, so the normalization inside the query
        // would bail and report `hit: false` — indistinguishable from open
        // sky, and a grounding probe built on it would never see the floor.
        assert!(
            fail_message(
                "let tiny = 0.000000001 * 0.000000001 * 0.000000001\n\
                 let main = () => Physics.cast(Vec3.make(0.0, 0.0, 0.0), \
                 Vec3.make(tiny, 0.0, 0.0), 10.0)"
            )
            .contains("must be finite and non-zero"),
            "a direction whose f32 norm underflows should be a loud error"
        );
        // The same at the other end: squaring 1e30 in f32 overflows to
        // infinity, which the query also treats as unusable.
        assert!(
            fail_message(
                "let huge = 10000000000.0 * 10000000000.0 * 10000000000.0\n\
                 let main = () => Physics.cast(Vec3.make(0.0, 0.0, 0.0), \
                 Vec3.make(huge, 0.0, 0.0), 10.0)"
            )
            .contains("must be finite and non-zero"),
            "a direction whose f32 square overflows should be a loud error"
        );
        assert!(
            fail_message(
                "let main = () => Physics.cast(Vec3.make(0.0, 0.0, 0.0), \
                 Vec3.make(0.0, -1.0, 0.0), 0.0)"
            )
            .contains("Physics.cast maxDist must be positive"),
            "a non-positive maxDist should be rejected"
        );
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
        crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| {
            w.reconcile(&ball_at(5.0))
        });
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
            let mut log: Vec<Entry> = Vec::new();
            let mut runner = FakeEffects::new(0.0, vec![]);
            let deferred = drain_effects(
                &session,
                "update",
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
            (
                "Physics.dynamic(\"x\", Physics.sphere(0.5)) |> Physics.linearDamping(-0.1)",
                "Physics.linearDamping must not be negative",
            ),
            (
                "Physics.dynamic(\"x\", Physics.sphere(0.5)) |> Physics.angularDamping(-0.1)",
                "Physics.angularDamping must not be negative",
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
        let mut log: Vec<Entry> = Vec::new();
        let mut runner = FakeEffects::new(0.0, vec![]);
        let deferred = drain_effects(
            &session,
            "update",
            &mut model,
            tree.clone(),
            &mut runner,
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            false,
        );
        assert!(deferred.is_empty(), "nothing should defer here");
        // A world command, not a frame-log op: the physics Timeline records it.
        assert!(log.is_empty());

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

    // The per-axis commands queue their OWN kind (not the whole-vector one), and
    // the masked write reaches the world preserving the axis it does not name.
    #[test]
    fn per_axis_velocity_commands_log_their_kind_and_preserve_the_other_axes() {
        for (src, kind) in [
            (
                "let main = () => Physics.setVelocityXZ(\"ball\", 3.0, 4.0)",
                "setVelocityXZ",
            ),
            (
                "let main = () => Physics.setVelocityY(\"ball\", 9.0)",
                "setVelocityY",
            ),
        ] {
            crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
            let effect = eval(src);
            let Value::HostData(data) = &effect else {
                panic!("expected an Effect from `{src}`");
            };
            let tree = &data.as_any().downcast_ref::<FunctorLangEffect>().expect("Effect").0;

            let module =
                functor_lang::lower(functor_lang::parse("let update = (m, msg) => m").unwrap()).unwrap();
            let session = functor_lang::Session::load(&module, &mut FunctorHost)
                .unwrap_or_else(|f| panic!("load failed: {}", f.error.message));
            let mut model = Value::Number(0.0);
            let mut log: Vec<Entry> = Vec::new();
            let mut runner = FakeEffects::new(0.0, vec![]);
            let deferred = drain_effects(
                &session,
                "update",
                &mut model,
                tree.clone(),
                &mut runner,
                &mut log,
                &mut |m| panic!("unexpected report: {m}"),
                false,
            );
            assert!(deferred.is_empty(), "nothing should defer here");
            let queued = crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| {
                w.pending_commands()
                    .iter()
                    .map(|c| c.tag_and_kind().1)
                    .collect::<Vec<_>>()
            });
            assert_eq!(queued, Some(vec![kind]), "wrong command for `{src}`");
        }

        // And the masked write really is masked once it reaches the world: a
        // body drifting on all three axes keeps its `y` through a setVelocityXZ.
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        let effect = eval("let main = () => Physics.setVelocityXZ(\"ball\", 3.0, 4.0)");
        let Value::HostData(data) = &effect else {
            panic!("expected an Effect");
        };
        let tree = &data.as_any().downcast_ref::<FunctorLangEffect>().expect("Effect").0;
        let module =
            functor_lang::lower(functor_lang::parse("let update = (m, msg) => m").unwrap()).unwrap();
        let session = functor_lang::Session::load(&module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("load failed: {}", f.error.message));
        let mut model = Value::Number(0.0);
        let mut log: Vec<Entry> = Vec::new();
        let mut runner = FakeEffects::new(0.0, vec![]);
        let _ = drain_effects(
            &session,
            "update",
            &mut model,
            tree.clone(),
            &mut runner,
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            false,
        );
        crate::physics::with_world(crate::physics::DEFAULT_WORLD, |w| {
            w.reconcile(&crate::physics::PhysicsScene::create(
                [0.0, 0.0, 0.0],
                vec![crate::physics::Body::dynamic(
                    "ball".to_string(),
                    crate::physics::Shape::Sphere { radius: 0.5 },
                )
                .with_velocity([1.0, -5.0, 2.0])],
            ));
            w.step_frame(1.0 / 60.0);
            assert_eq!(w.body_velocity("ball").unwrap(), [3.0, -5.0, 4.0]);
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

    /// The whole `Time` family lands on the same canonical seconds — the
    /// constructors the `s`/`ms`/`us`/`min`/`hr` literal suffixes call.
    #[test]
    fn every_time_constructor_is_canonical_seconds() {
        let seconds = |src: &str| {
            let value = eval(&format!("let main = () => {src}"));
            match &value {
                Value::HostData(data) => {
                    data.as_any()
                        .downcast_ref::<FunctorLangDuration>()
                        .expect("a Duration")
                        .0
                }
                other => panic!("expected a Duration, got {other}"),
            }
        };
        assert_eq!(seconds("Time.seconds(0.5)"), 0.5);
        assert_eq!(seconds("Time.millis(500.0)"), 0.5);
        assert_eq!(seconds("Time.micros(250.0)"), 0.00025);
        assert_eq!(seconds("Time.minutes(2.0)"), 120.0);
        assert_eq!(seconds("Time.hours(1.5)"), 5400.0);
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

    /// `(kind, answer)` per logged op — `None` for an outbound intent.
    fn ops(log: &[Entry]) -> Vec<(&'static str, Option<EffectValue>)> {
        log.iter()
            .map(|entry| match entry {
                Entry::Answer { kind, value, .. } => (*kind, Some(value.clone())),
                Entry::Issued { kind, .. } => (*kind, None),
                Entry::Input(input) => panic!("a drain logs no input: {input:?}"),
            })
            .collect()
    }

    /// The B6 broker contract: the same program under fake and replay
    /// runners produces the SAME model — the fake run's frame log IS
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
            let mut log: Vec<Entry> = Vec::new();
            let returned = session
                .call("roll", vec![model.clone()], &mut FunctorHost)
                .expect("roll");
            let (m, fx) = split_model_effect(returned);
            model = m;
            let deferred = drain_effects(
                &session,
                "update",
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
            ops(&fake_log),
            vec![
                ("random", Some(EffectValue::Number(0.25))),
                ("now", Some(EffectValue::Number(99.5))),
            ]
        );
        // Replay the log: same model, no world consulted.
        let mut replay = ReplayEffects::from_log(&fake_log);
        let (replay_model, replay_log) = run(&mut replay);
        assert_eq!(replay.finish(), None);
        assert_eq!(replay_model, fake_model);
        assert_eq!(ops(&replay_log), ops(&fake_log));
        // The real world: same SHAPE (both effects performed, chain drained),
        // world-supplied values.
        let (real_model, real_log) = run(&mut RealEffects::new());
        let real = ops(&real_log);
        assert_eq!(real.len(), 2);
        assert_eq!(real[0].0, "random");
        let Some(EffectValue::Number(r0)) = real[0].1 else {
            panic!("random should log a number");
        };
        assert!((0.0..1.0).contains(&r0));
        assert_eq!(real[1].0, "now");
        assert!(real_model.starts_with("{ rolls: 0."));
    }

    /// The Phase 4 query path end to end: a raycast effect DEFERS through the
    /// pre-step drain, then answers post-step against the live world, its
    /// record folding through `update` — a replay recomputes it (an ask), and
    /// the fake runner answers without a world at all.
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
        let mut log: Vec<Entry> = Vec::new();
        let mut runner = RealEffects::new();
        let mut fail = |m: String| panic!("unexpected report: {m}");

        // Pre-step drain: DEFERRED — nothing performed, nothing logged.
        let deferred =
            drain_effects(&session, "update", &mut model, tree, &mut runner, &mut log, &mut fail, false);
        assert_eq!(deferred.len(), 1);
        assert!(log.is_empty());
        assert!(matches!(model, Value::Number(_)), "model must be untouched");

        // Post-step drain: performed against the live world.
        perform_deferred_queries(
            &session,
            "update",
            &mut model,
            deferred,
            &mut runner,
            &mut log,
            &mut fail,
            false,
        );
        // An ask, not a frame-log op: replay recomputes it.
        assert!(log.is_empty());
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

        // Replay: recomputed against the active world, same answer.
        let mut replay_model = Value::Number(0.0);
        let mut replay_log: Vec<Entry> = Vec::new();
        let mut replay = ReplayEffects::from_log(&log);
        let deferred = drain_effects(
            &session,
            "update",
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
            "update",
            &mut replay_model,
            deferred,
            &mut replay,
            &mut replay_log,
            &mut fail,
            false,
        );
        assert!(replay_log.is_empty());
        assert_eq!(replay.finish(), None);
        assert_eq!(replay_model.to_string(), model.to_string());
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);

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
        let mut log: Vec<Entry> = Vec::new();
        let mut runner = FakeEffects::new(0.0, vec![]);
        deliver_physics_events(
            &session,
            "update",
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
            (
                "scores".to_string(),
                EffectValue::Map(vec![
                    (
                        EffectMapKey::Number(1.0),
                        EffectValue::Text("one".to_string()),
                    ),
                    (
                        EffectMapKey::Text("two".to_string()),
                        EffectValue::Number(2.0),
                    ),
                ]),
            ),
        ]);
        let functor_lang = value.to_functor_lang().unwrap();
        let Value::Record(fields) = &functor_lang else {
            panic!("expected a record, got {}", functor_lang.kind_name());
        };
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[0].0, "hit");
        assert!(matches!(fields[0].1, Value::Bool(true)));
        assert!(matches!(fields[1].1, Value::Number(n) if n == 4.25));
        assert!(matches!(&fields[3].1, Value::Map(entries) if entries.len() == 2));

        let json = serde_json::to_string(&value).expect("serialize");
        let back: EffectValue = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, value);
        assert_eq!(effect_value_from_value(&functor_lang).unwrap(), value);
    }

    #[test]
    fn effect_map_values_restore_canonical_order_at_the_replay_seam() {
        let value = EffectValue::Map(vec![
            (
                EffectMapKey::Text("z".to_string()),
                EffectValue::Number(1.0),
            ),
            (
                EffectMapKey::Number(-0.0),
                EffectValue::Text("old".to_string()),
            ),
            (EffectMapKey::Bool(false), EffectValue::Number(2.0)),
            (
                EffectMapKey::Number(0.0),
                EffectValue::Text("new".to_string()),
            ),
        ])
        .to_functor_lang()
        .unwrap();

        assert_eq!(
            value.to_string(),
            r#"Map.fromList([(false, 2), (0, "new"), ("z", 1)])"#
        );
    }

    #[test]
    fn effect_map_values_reject_non_finite_keys_at_the_replay_seam() {
        for key in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let value = EffectValue::List(vec![EffectValue::Map(vec![(
                EffectMapKey::Number(key),
                EffectValue::Bool(true),
            )])]);
            let error = value
                .to_functor_lang()
                .err()
                .expect("a non-finite map key must be rejected");
            assert!(error.contains("effect map keys must be finite"), "{error}");
        }
    }

    /// Past 16 entries a map is a multi-level tree. Every runtime walker
    /// still sees canonical key order, and maps holding equal entries compare
    /// equal whatever their tree shape (edit history).
    #[test]
    fn multi_leaf_maps_keep_key_order_through_runtime_walkers() {
        let built = eval(
            "let main = () => List.range(40.0) |> List.map((i) => (i, i * 2.0)) |> Map.fromList",
        );
        let edited = eval(
            "let main = () => List.range(80.0) |> List.reverse\n\
               |> List.fold((m, k) => Map.insert(k, k * 2.0, m), Map.empty())\n\
               |> (m) => List.range(40.0) |> List.fold((acc, k) => Map.remove(k + 40.0, acc), m)",
        );
        assert_eq!(built.to_string(), edited.to_string());
        assert!(plain_data(&built).is_ok());
        assert!(same_plain_data(&built, &edited));
        let changed = eval(
            "let main = () => List.range(40.0) |> List.map((i) => (i, i * 2.0)) |> Map.fromList\n\
               |> Map.insert(39.0, 0.0)",
        );
        assert!(!same_plain_data(&built, &changed));

        let ascending: Vec<f64> = (0..40).map(f64::from).collect();
        let effect = effect_value_from_value(&built).unwrap();
        let EffectValue::Map(entries) = &effect else {
            panic!("expected an effect map");
        };
        let keys: Vec<f64> = entries
            .iter()
            .map(|(key, _)| match key {
                EffectMapKey::Number(n) => *n,
                other => panic!("unexpected key {other:?}"),
            })
            .collect();
        assert_eq!(keys, ascending);
        assert_eq!(
            effect.to_functor_lang().unwrap().to_string(),
            built.to_string()
        );

        let json = value_to_json(&built);
        let keys: Vec<f64> = json["$map"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pair| pair[0].as_f64().unwrap())
            .collect();
        assert_eq!(keys, ascending);

        let closures = eval(
            "let main = () => List.range(40.0) |> List.map((i) => (i, (x) => x + i)) |> Map.fromList",
        );
        assert!(built.is_reload_safe_snapshot());
        assert!(!closures.is_reload_safe_snapshot());
    }

    /// The lossy model-JSON walker (`GET /state` `model`, v4): plain data
    /// maps structurally; callables, host values, and non-finite numbers
    /// become sigil placeholders instead of failing.
    #[test]
    fn value_to_json_is_total_and_lossy() {
        struct TestHost;
        impl HostData for TestHost {
            fn type_name(&self) -> &'static str {
                "TestHost"
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
        let model = Value::Record(Rc::new(vec![
            ("count".to_string(), Value::Number(3.0)),
            ("name".to_string(), Value::String(Rc::from("hello"))),
            ("alive".to_string(), Value::Bool(true)),
            (
                "items".to_string(),
                Value::List(Rc::new(vec![Value::Number(1.0), Value::Number(2.0)])),
            ),
            (
                "scores".to_string(),
                Value::Map(
                    functor_lang::pmap::PMap::from_sorted(vec![
                        (
                            functor_lang::value::MapKey::String(Rc::from("a")),
                            Value::Number(1.0),
                        ),
                        (
                            functor_lang::value::MapKey::String(Rc::from("b")),
                            Value::Number(2.0),
                        ),
                    ])
                    .0,
                ),
            ),
            (
                "pos".to_string(),
                Value::Tuple(Rc::new(vec![Value::Number(1.0), Value::Number(2.0)])),
            ),
            (
                "state".to_string(),
                Value::Variant {
                    ctor: Rc::from("Playing"),
                    args: Rc::new(vec![Value::Number(7.0)]),
                },
            ),
            ("spawn".to_string(), Value::HostFn(Rc::from("Scene.cube"))),
            ("handle".to_string(), Value::HostData(Rc::new(TestHost))),
            ("bad".to_string(), Value::Number(f64::NAN)),
            ("up".to_string(), Value::Number(f64::INFINITY)),
            ("down".to_string(), Value::Number(f64::NEG_INFINITY)),
        ]));
        assert_eq!(
            value_to_json(&model),
            serde_json::json!({
                "count": 3.0,
                "name": "hello",
                "alive": true,
                "items": [1.0, 2.0],
                "scores": { "$map": [["a", 1.0], ["b", 2.0]] },
                "pos": { "$tuple": [1.0, 2.0] },
                "state": { "$ctor": "Playing", "args": [7.0] },
                "spawn": { "$fn": "<host Scene.cube>" },
                "handle": { "$host": "TestHost" },
                "bad": { "$number": "NaN" },
                "up": { "$number": "Infinity" },
                "down": { "$number": "-Infinity" },
            })
        );
    }

    /// Nesting past the bound truncates with a marker instead of risking the
    /// host's stack (nesting depth is user-controlled — see `Value`'s
    /// `Display` doc).
    #[test]
    fn value_to_json_bounds_depth() {
        let mut value = Value::Number(0.0);
        for _ in 0..(VALUE_JSON_MAX_DEPTH + 10) {
            value = Value::List(Rc::new(vec![value]));
        }
        let mut node = &value_to_json(&value);
        while let Some(items) = node.as_array() {
            node = &items[0];
        }
        assert_eq!(node, &serde_json::json!({ "$truncated": "max depth" }));
    }

    /// The bound is charged in emitted JSON containers (a tuple emits TWO —
    /// sigil object + args array), so even the deepest producible
    /// `model`, wrapped in a full `RuntimeState`, stays inside
    /// serde_json's default 128-container parse limit and round-trips.
    #[test]
    fn deep_model_round_trips_through_runtime_state() {
        let mut value = Value::Number(0.0);
        for _ in 0..300 {
            value = Value::Tuple(Rc::new(vec![Value::Number(1.0), value]));
        }
        let state = crate::debug_protocol::RuntimeState {
            frame: 1,
            tts: 0.0,
            pending_steps: 0,
            model_revision: 0,
            pending_net: 0,
            viewport: crate::debug_protocol::RuntimeViewport::new(1, 1),
            views: vec![],
            model: value_to_json(&value),
            model_debug: String::new(),
            input: crate::InputSnapshot::default(),
            sim_thread: None,
            display_refresh_rate: None,
            instance_uploads: Default::default(),
            config: None,
            settings: None,
            terrain: Default::default(),
            passthrough: None,
            battery: None,
            skinned: Default::default(),
            cull: Default::default(),
            cells: Default::default(),
            stages: None,
            build: None,
            xr: None,
        };
        let back: crate::debug_protocol::RuntimeState = serde_json::from_str(&state.to_json())
            .expect("a deep model must stay parseable by stock serde_json");
        assert_eq!(back.model, state.model);
    }

    /// A diverged replay reports the op, answers with a stand-in, and stops
    /// consuming the log: a swapped call, an inserted call, a dropped call.
    #[test]
    fn replay_divergence_is_reported_at_the_op() {
        let now = || Entry::answer("now", args_digest(&()), EffectValue::Number(1.0));
        let mut swapped = ReplayEffects::from_log(&[now()]);
        assert_eq!(swapped.random(), 0.0);
        let message = swapped.finish().expect("diverged");
        assert!(
            message.contains("recorded now, program asked for random"),
            "{message}"
        );

        let mut inserted = ReplayEffects::from_log(&[now()]);
        assert_eq!(inserted.now(), 1.0);
        inserted.now();
        let message = inserted.finish().expect("diverged");
        assert!(
            message.contains("past its frame's recorded ops"),
            "{message}"
        );

        let mut dropped = ReplayEffects::from_log(&[now(), now()]);
        dropped.now();
        let message = dropped.finish().expect("diverged");
        assert!(
            message.contains("never asked for the recorded now"),
            "{message}"
        );

        // A changed intent diverges, but later answers still line up.
        let mut moved = ReplayEffects::from_log(&[
            Entry::issued(
                "storage.save",
                args_digest(&("slot", &EffectValue::Number(1.0))),
            ),
            now(),
        ]);
        moved.issued(
            "storage.save",
            args_digest(&("slot", &EffectValue::Number(2.0))),
        );
        assert_eq!(moved.now(), 1.0);
        let message = moved.finish().expect("diverged");
        assert!(message.contains("with different arguments"), "{message}");
        assert_eq!(moved.lost_at(), None);
        assert_eq!(swapped.lost_at(), Some(0));
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
        let mut log: Vec<Entry> = Vec::new();
        let mut reports = Vec::new();
        let deferred = drain_effects(
            &session,
            "update",
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
        // The cap fires before performing (or logging) the over-limit effect.
        assert_eq!(log.len(), 1000);
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
        let mut log: Vec<Entry> = Vec::new();
        let mut reports = Vec::new();
        let deferred = drain_effects(
            &session,
            "update",
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
        let in_map = Value::Map(
            functor_lang::pmap::PMap::from_sorted(vec![(
                functor_lang::value::MapKey::String(std::rc::Rc::from("command")),
                nested,
            )])
            .0,
        );
        assert!(contains_effect(&in_map));
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
`0.5s` / `500ms`, or Time.seconds(…) / Time.millis(…)"
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
               Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),\n\
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

    // `Solid.toScene`: a mm/Z-up solid lands in the scene as one shared mesh
    // under a metres/Y-up transform. Converting the same solid twice yields the
    // same mesh `Arc` — what lets the renderer skip re-uploading it per frame.
    #[test]
    fn solid_to_scene_shares_one_mesh_in_metres_y_up() {
        let frame = frame_of(
            "let part = Solid.box(10.0, 20.0, 30.0)\n\
             let main = () =>\n\
             Frame.create(\n\
               Camera3D.lookAt(Vec3.make(0.0, 0.0, -1.0), Vec3.make(0.0, 0.0, 0.0)),\n\
               Scene.group([Solid.toScene(part), Solid.toScene(part)]))",
        );
        let SceneObject::Group(children) = &frame.scene.obj else {
            panic!("expected a group, got {:?}", frame.scene.obj);
        };
        let meshes: Vec<_> = children
            .iter()
            .map(|child| {
                let SceneObject::Geometry(Shape::Mesh { mesh }) = &child.obj else {
                    panic!("expected a Mesh leaf, got {:?}", child.obj);
                };
                (child.xform, mesh)
            })
            .collect();
        assert!(std::sync::Arc::ptr_eq(meshes[0].1, meshes[1].1));
        // Bounds in world space: 10 x 20 x 30 mm (Z up) → 0.01 x 0.03 x 0.02 m (Y up).
        let (xform, mesh) = &meshes[0];
        let world: Vec<_> = mesh
            .positions
            .iter()
            .map(|&p| cgmath::Transform::transform_point(xform, cgmath::Point3::from(p)))
            .collect();
        let extent = |f: fn(&cgmath::Point3<f32>) -> f32| {
            let (lo, hi) = world.iter().map(f).fold((f32::MAX, f32::MIN), |(lo, hi), v| {
                (lo.min(v), hi.max(v))
            });
            hi - lo
        };
        assert!((extent(|p| p.x) - 0.01).abs() < 1e-6);
        assert!((extent(|p| p.y) - 0.03).abs() < 1e-6);
        assert!((extent(|p| p.z) - 0.02).abs() < 1e-6);
    }

    // The hello ripple shape: rows sampled with nested List builtins — F#'s
    // `heightmapFn`, in user space.
    #[test]
    fn heightmap_rows_sample_with_list_builtins() {
        let frame = frame_of(
            "let ripple = (r, c) => 0.05 * (Math.sin(c * 0.5) + Math.cos(r * 0.5))\n\
             let main = () =>\n\
             Frame.create(\n\
               Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),\n\
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

    #[test]
    fn terrain_descriptor_emits_the_protocol_leaf() {
        let frame = frame_of(
            "let world =\n\
               Terrain.heightmap(Asset.texture(\"world.png\"), 4000.0, 4000.0, -80.0, 520.0)\n\
               |> Terrain.maxPixelError(3.0)\n\
               |> Terrain.color(Color.rgb(0.2, 0.4, 0.1))\n\
             let main = () =>\n\
               Frame.create(\n\
                 Camera3D.lookAt(Vec3.make(0.0, 400.0, -900.0), Vec3.make(0.0, 0.0, 0.0)),\n\
                 Scene.terrain(world))",
        );
        let SceneObject::Terrain(terrain) = &frame.scene.obj else {
            panic!("expected a Terrain node, got {:?}", frame.scene.obj);
        };
        assert_eq!(terrain.heightmap, "world.png");
        assert_eq!((terrain.width, terrain.depth), (4000.0, 4000.0));
        assert_eq!((terrain.min_height, terrain.max_height), (-80.0, 520.0));
        assert_eq!(terrain.max_pixel_error, 3.0);
        assert_eq!(terrain.color, [0.2, 0.4, 0.1]);

        let json = serde_json::to_string(&frame).expect("serialize");
        assert!(json.contains(r#""Terrain""#), "json: {json}");
        let back: Frame = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn terrain_descriptor_validates_assets_and_dimensions() {
        let usage = "usage: Terrain.heightmap(asset, width, depth, minHeight, maxHeight) — \
a texture Asset, positive dimensions, and maxHeight greater than minHeight";
        assert_eq!(
            run_fail(
                "let main = () => Terrain.heightmap(Asset.texture(\"world.png\"), 0.0, 4000.0, -80.0, 520.0)"
            ),
            usage
        );
        assert_eq!(
            run_fail(
                "let main = () => Terrain.heightmap(Asset.texture(\"world.png\"), 4000.0, 4000.0, 10.0, 10.0)"
            ),
            usage
        );
        assert!(run_fail(
            "let main = () => Terrain.heightmap(\"world.png\", 4000.0, 4000.0, -80.0, 520.0)"
        )
        .contains("construct Asset.texture(\"world.png\") at the data boundary"));
        assert!(run_fail(
            "let main = () => Terrain.heightmap(Asset.model(\"world.glb\"), 4000.0, 4000.0, -80.0, 520.0)"
        )
        .contains("expected a texture asset, got a model asset"));
        assert_eq!(
            run_fail(
                "let main = () => Terrain.heightmap(Asset.texture(\"world.png\"), 4000.0, 4000.0, -80.0, 520.0) |> Terrain.maxPixelError(0.0)"
            ),
            "Terrain.maxPixelError pixels must be a positive finite number"
        );
        assert_eq!(
            run_fail(
                "let main = () => Terrain.heightmap(Asset.texture(\"world.png\"), 0.00000000000000000000000000000000000000000000001, 4000.0, -80.0, 520.0)"
            ),
            usage
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
               Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),\n\
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

    // Html/Attr (the `webview` hook): the Elm-style HTML tree. Attrs fold
    // into the element (class pairs, onClick slots), children must be nodes,
    // and the serialized HTML carries the handler slots as data attributes —
    // the exact string both shells render.
    #[test]
    fn html_builders_fold_attrs_and_stamp_handler_slots() {
        let _ = take_ui_handlers(); // isolate from other tests on this thread
        let value = eval(
            "type Msg = | Inc | Dec\n\
             let main = () =>\n\
             Html.div([Attr.class(\"hud\")], [\n\
               Html.button([Attr.onClick(Dec)], [Html.text(\"-\")]),\n\
               Html.button([Attr.onClick(Inc)], [Html.text(\"+\")]),\n\
               Html.input([Attr.value(\"hi\"), Attr.onInput((s) => s)]),\n\
             ])",
        );
        let node = html_node_value(&value).expect("main should return an Html node");
        assert_eq!(
            node.to_html(),
            r#"<div class="hud"><button data-fn-click="0">-</button><button data-fn-click="1">+</button><input value="hi" data-fn-input="2"></div>"#
        );
        let handlers = take_ui_handlers();
        assert_eq!(handlers.len(), 3);
        match (&handlers[0], &handlers[1], &handlers[2]) {
            (UiHandler::Msg(dec), UiHandler::Msg(inc), UiHandler::Tagger(_)) => {
                assert_eq!(dec.to_string(), "Dec");
                assert_eq!(inc.to_string(), "Inc");
            }
            _ => panic!("onClick registers msgs, onInput a tagger"),
        }
    }

    #[test]
    fn html_builders_reject_wrong_shapes_with_teaching_errors() {
        let _ = take_ui_handlers();
        assert_eq!(
            run_fail("let main = () => Html.div([42.0], [])"),
            "Html.div: expected an Attr, got a number"
        );
        let _ = take_ui_handlers();
        assert_eq!(
            run_fail("let main = () => Html.div([], [1.0])"),
            "Html.div: expected an Html node, got a number"
        );
        let _ = take_ui_handlers();
        assert_eq!(
            run_fail("let main = () => Attr.onInput(42.0)"),
            "Attr.onInput: the tagger must be a function of the result record, got a number"
        );
        let _ = take_ui_handlers();
    }

    // The wasm overlay is a REAL DOM, so script-capable tags and executable/
    // spoofing attribute names are refused at construction (game logic has no
    // DOM access by design — the webview must not become the escape hatch).
    #[test]
    fn html_rejects_injection_shaped_names() {
        let _ = take_ui_handlers();
        assert_eq!(
            run_fail("let main = () => Html.element(\"script\", [], [])"),
            "Html.element: `script` is not an allowed tag name \
(letters/digits/dashes; script-capable tags are refused)"
        );
        let _ = take_ui_handlers();
        assert_eq!(
            run_fail("let main = () => Html.element(\"div onload=x\", [], [])"),
            "Html.element: `div onload=x` is not an allowed tag name \
(letters/digits/dashes; script-capable tags are refused)"
        );
        let _ = take_ui_handlers();
        for bad in ["onerror", "href", "data-fn-click", "srcdoc"] {
            assert_eq!(
                run_fail(&format!("let main = () => Attr.attr(\"{bad}\", \"x\")")),
                format!(
                    "Attr.attr: `{bad}` is not an allowed attribute name \
(letters/digits/dashes; on*/href/data-fn-* are refused)"
                )
            );
            let _ = take_ui_handlers();
        }
        // Ordinary names still pass.
        let value =
            eval("let main = () => Html.element(\"section\", [Attr.attr(\"title\", \"hi\")], [])");
        let node = html_node_value(&value).expect("section node");
        assert_eq!(node.to_html(), r#"<section title="hi"></section>"#);
        let _ = take_ui_handlers();
    }

    #[test]
    fn html_style_emits_raw_css() {
        let _ = take_ui_handlers();
        let value = eval("let main = () => Html.style(\".a > .b { color: red; }\")");
        let node = html_node_value(&value).expect("style node");
        assert_eq!(node.to_html(), "<style>.a > .b { color: red; }</style>");
        let _ = take_ui_handlers();
    }

    // Attr.styles (the elm-css split — inline declarations typed, stylesheet
    // strings not): the Style list folds into ONE style="…" pair at
    // construction. Pinned as the exact serialized attribute, including the
    // px formatting and the ui::rgb_u8 color quantization.
    #[test]
    fn attr_styles_folds_into_one_style_attribute() {
        let _ = take_ui_handlers();
        let value = eval(
            "let main = () => Html.div([Attr.styles([\n\
               Style.flexRow(),\n\
               Style.gapPx(10.0),\n\
               Style.widthPx(300.0),\n\
               Style.background(Color.rgb(0.1, 0.1, 0.2)),\n\
               Style.borderPx(2.0, Color.rgb(1.0, 1.0, 1.0)),\n\
               Style.opacity(0.5),\n\
               Style.raw(\"letter-spacing\", \"0.1em\"),\n\
             ])], [])",
        );
        let node = html_node_value(&value).expect("div node");
        assert_eq!(
            node.to_html(),
            r#"<div style="display: flex; flex-direction: row; gap: 10px; width: 300px; background: rgb(26, 26, 51); border: 2px solid rgb(255, 255, 255); opacity: 0.5; letter-spacing: 0.1em"></div>"#
        );
        let _ = take_ui_handlers();
    }

    // The Angle rule applied to inline styles: bare values where a Style (or
    // the list) goes, an out-of-range opacity, and an injection-shaped
    // Style.raw property name are construction-time teaching errors.
    #[test]
    fn attr_styles_rejects_wrong_shapes_with_teaching_errors() {
        let _ = take_ui_handlers();
        assert_eq!(
            run_fail("let main = () => Attr.styles(42.0)"),
            "expected a list, got a number"
        );
        assert_eq!(
            run_fail("let main = () => Attr.styles([42.0])"),
            "Attr.styles: expected a Style, got a number"
        );
        assert_eq!(
            run_fail("let main = () => Attr.styles([\"width: 300px\"])"),
            "Attr.styles: expected a Style, got a string"
        );
        assert_eq!(
            run_fail("let main = () => Style.opacity(1.5)"),
            "Style.opacity: expected 0..1, got 1.5"
        );
        assert_eq!(
            run_fail("let main = () => Style.raw(\"width;height\", \"10px\")"),
            "Style.raw: `width;height` is not an allowed property name (letters/digits/dashes)"
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

    /// The outbound conn-command queue is PROCESS-GLOBAL (`net::CONN_OUT`),
    /// and cargo runs these tests as parallel threads — any two tests that
    /// push or drain it can steal each other's commands (a Windows-CI flake
    /// caught exactly that: a broadcast's second Send vanished into a
    /// concurrent test's clearing drain). Every test touching the queue takes
    /// this first.
    static CONN_QUEUE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn effect_send_queues_a_conn_command() {
        let _guard = CONN_QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let mut log: Vec<Entry> = Vec::new();
        drain_effects(
            &session,
            "update",
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
        let ev = net_event_value(NetEventKind::Message, 3, "yo")
            .to_functor_lang()
            .unwrap();
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

    /// The typed-message spine: a nested plain-data payload roundtrips
    /// Value → EffectValue → wire frame → EffectValue → Value, and an inbound
    /// frame decodes as `Net.Data(id, payload)` — while plain text stays
    /// `Net.Message` and a corrupt frame becomes a loud `Net.Error`.
    #[test]
    fn send_msg_payload_roundtrips_and_decodes_as_net_data() {
        let payload = EffectValue::Variant(
            "Protocol.Ping".into(),
            vec![EffectValue::Record(vec![
                ("n".into(), EffectValue::Number(2.0)),
                (
                    "pos".into(),
                    EffectValue::Tuple(vec![EffectValue::Number(1.0), EffectValue::Number(-3.5)]),
                ),
                ("tags".into(), EffectValue::List(vec![EffectValue::Text("a".into())])),
                ("ok".into(), EffectValue::Bool(true)),
            ])],
        );
        let wire = encode_typed_msg(&payload);
        assert!(wire.starts_with(TYPED_MSG_PREFIX));
        assert_eq!(decode_typed_msg(&wire).unwrap().unwrap(), payload);
        assert_eq!(
            effect_value_from_value(&payload.to_functor_lang().unwrap()).unwrap(),
            payload
        );

        assert_eq!(
            net_event_value(NetEventKind::Message, 7, &wire),
            EffectValue::Variant(
                "Net.Data".into(),
                vec![EffectValue::Number(7.0), payload]
            )
        );
        assert_eq!(
            net_event_value(NetEventKind::Message, 7, "plain text"),
            EffectValue::Variant(
                "Net.Message".into(),
                vec![EffectValue::Number(7.0), EffectValue::Text("plain text".into())]
            )
        );
        match net_event_value(NetEventKind::Message, 7, &format!("{TYPED_MSG_PREFIX}not json")) {
            EffectValue::Variant(ctor, args) => {
                assert_eq!(ctor, "Net.Error");
                assert!(
                    matches!(&args[1], EffectValue::Text(t) if t.contains("malformed typed message")),
                    "unexpected error payload: {args:?}"
                );
            }
            other => panic!("expected Net.Error, got {other:?}"),
        }
    }

    /// Effect.sendMsg performs like Send (a framed ConnCommand::Send) and its
    /// effect-log record keeps the STRUCTURED payload.
    #[test]
    fn send_msg_queues_a_framed_send_and_logs_the_payload() {
        let _guard = CONN_QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = crate::net::drain_conn_commands();
        let src = "type Wire = | Ping(n: float)\n\
                   let init = { n: 0.0 }\n\
                   let update = (m, msg) => m\n\
                   let tick = (m, dt: float, tts: float) => (m, Effect.sendMsg(7.0, Ping(2.0)))\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let tick = session.global("tick").unwrap();
        let model = session.global("init").unwrap();
        let (mut m, fx) = split_model_effect(
            session
                .apply(tick, vec![model, Value::Number(0.5), Value::Number(0.5)], "tick", &mut FunctorHost)
                .unwrap(),
        );
        let mut log: Vec<Entry> = Vec::new();
        let _ = drain_effects(
            &session,
            "update",
            &mut m,
            fx.expect("an effect"),
            &mut FakeEffects::new(0.0, vec![]),
            &mut log,
            &mut |r| panic!("unexpected report: {r}"),
            false,
        );
        let expected = EffectValue::Variant("Ping".into(), vec![EffectValue::Number(2.0)]);
        assert_eq!(ops(&log), vec![("net.sendMsg", None)]);
        let cmds = crate::net::drain_conn_commands();
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            crate::net::ConnCommand::Send { conn: 7, payload } => {
                assert_eq!(payload, encode_typed_msg(&expected).as_bytes());
            }
            other => panic!("expected a framed Send, got {other:?}"),
        }
    }

    /// A payload that cannot cross the wire — a function, or a non-finite
    /// number (JSON has no NaN/Infinity; serde_json would write `null` and
    /// only the PEER's decode would fail) — is a teaching error at the
    /// construction site, not a silent drop mid-drain or on the far end.
    #[test]
    fn send_msg_rejects_a_function_payload() {
        let msg = run_fail("let main = () => Effect.sendMsg(1.0, (x) => x)");
        assert!(
            msg.contains("not plain data") && msg.contains("a function"),
            "expected the plain-data teaching error, got: {msg}"
        );
        let msg = run_fail("let main = () => Effect.sendMsg(1.0, 1.0 / 0.0)");
        assert!(
            msg.contains("non-finite"),
            "expected the non-finite teaching error, got: {msg}"
        );
    }

    /// A save/load program, run under whichever runner a test hands it: the
    /// first tick saves the model, the second loads it into a FRESH model —
    /// the "quit and relaunch" shape, minus the process boundary.
    fn run_save_then_load(runner: &mut dyn EffectRunner) -> (String, Vec<Entry>) {
        let src = "let init = { score: 0.0, tags: [\"a\"] }\n\
                   let update = (m, msg) =>\n\
                     match msg with\n\
                     | Option.Some(saved) => saved\n\
                     | Option.None => m\n\
                   let save = (m) => (m, Persistence.save(\"autosave\", m))\n\
                   let restore = (m) => (m, Persistence.load(\"autosave\", (o) => o))\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let mut log: Vec<Entry> = Vec::new();
        let mut drain = |model: &mut Value, name: &str| {
            let (next, fx) = split_model_effect(
                session
                    .call(name, vec![model.clone()], &mut FunctorHost)
                    .unwrap_or_else(|e| panic!("{name}: {}", e.message)),
            );
            *model = next;
            let deferred = drain_effects(
                &session,
                "update",
                model,
                fx.expect("an effect"),
                runner,
                &mut log,
                &mut |m| panic!("unexpected report: {m}"),
                false,
            );
            assert!(deferred.is_empty(), "nothing should defer here");
        };
        // Play a little, then save.
        let mut model = Value::Record(std::rc::Rc::new(vec![
            ("score".to_string(), Value::Number(42.0)),
            (
                "tags".to_string(),
                Value::List(std::rc::Rc::new(vec![Value::String(Rc::from("a"))])),
            ),
        ]));
        drain(&mut model, "save");
        // Relaunch: a fresh model, restored from the slot.
        let mut restarted = session.global("init").unwrap();
        drain(&mut restarted, "restore");
        (restarted.to_string(), log)
    }

    /// `File.bytes` + `Bytes.*` readers + `Binary.*` from Functor code, over a
    /// file of known bytes in a temporary project.
    #[test]
    fn bytes_read_numbers_text_and_errors() {
        let _guard = crate::storage::SAVE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        crate::storage::set_project_root(dir.path().to_path_buf());
        // "LGMD", u32le 3, f32le 1.0, 0xFF, u16be 0x1234, i32le -2, f32le NaN,
        // f64be 2.5, invalid UTF-8 after "ok".
        let mut data = vec![
            b'L', b'G', b'M', b'D', 3, 0, 0, 0, 0, 0, 0x80, 0x3F, 0xFF, 0x12, 0x34,
        ];
        data.extend((-2i32).to_le_bytes());
        data.extend(f32::NAN.to_le_bytes());
        data.extend(2.5f64.to_be_bytes());
        data.extend([b'o', b'k', 0xFF]);
        std::fs::write(dir.path().join("data.bin"), data).unwrap();
        let checks = [
            "Bytes.length(d) == 34.0",
            "Bytes.i32le(15.0, d) == Result.Ok(-2.0) && Bytes.i16le(15.0, d) == Result.Ok(-2.0)",
            "Bytes.f64be(23.0, d) == Result.Ok(2.5) && Bytes.f32be(8.0, d) != Result.Ok(1.0)",
            "match Bytes.f32le(19.0, d) with | Result.Ok(x) => x != x | Result.Error(_) => false",
            "match Bytes.utf8(31.0, 3.0, d) with \
             | Result.Error(e) => e.offset == 33.0 | Result.Ok(_) => false",
            "(Bytes.slice(4.0, 30.0, d) |> Result.bind((s) => Bytes.slice(4.0, 4.0, s)) \
             |> Result.bind((s) => Bytes.f32le(0.0, s))) == Result.Ok(1.0)",
            "Bytes.latin1(0.0, 4.0, d) == Result.Ok(\"LGMD\")",
            "Bytes.u32le(4.0, d) == Result.Ok(3.0)",
            "Bytes.u32be(4.0, d) == Result.Ok(50331648.0)",
            "Bytes.f32le(8.0, d) == Result.Ok(1.0)",
            "Bytes.u8(12.0, d) == Result.Ok(255.0)",
            "Bytes.i8(12.0, d) == Result.Ok(-1.0)",
            "Bytes.u16be(13.0, d) == Result.Ok(4660.0)",
            "Binary.require(Bytes.u32le(4.0, d)) == 3.0",
            "Bytes.equals(d, d)",
            "match Bytes.u32le(32.0, d) with \
             | Result.Error(e) => Binary.describe(e) == \"at byte 32: needs 4 bytes, 2 left\" \
             | Result.Ok(_) => false",
            "Result.isError(Bytes.u8(1.5, d)) && Result.isError(Bytes.u8(-1.0, d))",
            "match Bytes.slice(4.0, 4.0, d) with \
             | Result.Ok(s) => Bytes.length(s) == 4.0 && Bytes.u32le(0.0, s) == Result.Ok(3.0) \
             | Result.Error(_) => false",
            "Result.isError(Bytes.slice(31.0, 4.0, d))",
            "(Bytes.slice(34.0, 0.0, d) |> Result.map(Bytes.length)) == Result.Ok(0.0)",
            "Result.isError(Bytes.slice(35.0, 0.0, d)) && Result.isError(Bytes.latin1(35.0, 0.0, d))",
            "Result.isError(Bytes.u8(0.0 / 0.0, d)) && Result.isError(Bytes.u8(1.0 / 0.0, d))",
            "Result.isError(Bytes.utf8(0.0, Math.pow(10.0, 40.0), d))",
            "Binary.describe({ offset: 1462.0, message: \"needs 2 bytes, 1 left\", \
             path: [Binary.Field(\"polygons\"), Binary.Index(17.0), Binary.Field(\"indices\"), \
             Binary.Index(2.0)] }) == \"polygons[17].indices[2] at byte 1462: needs 2 bytes, 1 left\"",
        ];
        for check in checks {
            let src = format!("let d = File.bytes(Path.of(\"data.bin\"))\nlet main = {check}\n");
            assert!(
                matches!(eval_with_prelude(&src), Ok(Value::Bool(true))),
                "{check}: {:?}",
                eval_with_prelude(&src).map(|v| v.to_string())
            );
        }
        let failure = |main: &str| {
            eval_with_prelude(&format!("let main = {main}\n"))
                .err()
                .expect("should fail")
        };
        let err = failure("Binary.require(Bytes.u32le(32.0, File.bytes(Path.of(\"data.bin\"))))");
        assert!(err.contains("at byte 32: needs 4 bytes, 2 left"), "{err}");
        let src = "let main = File.exists(Path.of(\"data.bin\")) && not File.exists(Path.of(\"missing.bin\"))\n";
        assert!(
            matches!(eval_with_prelude(src), Ok(Value::Bool(true))),
            "{:?}",
            eval_with_prelude(src).map(|v| v.to_string())
        );
        let err = failure("File.bytes(Path.of(\"missing.bin\"))");
        assert!(err.contains("can't read missing.bin"), "{err}");
        let err = failure("File.bytes(Path.of(\"../data.bin\"))");
        assert!(err.contains("inside the project"), "{err}");
        // A bare string through the gradual seam still gets the wrapper taught.
        let err = eval_with_prelude("let s : unknown = \"data.bin\"\nlet main = File.bytes(s)\n")
            .err()
            .expect("should fail");
        assert!(err.contains("wrap it: Path.of(\"data.bin\")"), "{err}");
        let err = failure("File.bytes(Path.of(\".\"))");
        assert!(err.contains("project root is a folder"), "{err}");
        let src = "let p = Path.of(\"./x/data.bin/\")\n\
                   let main = [Path.toString(p), Path.name(p), Path.ext(p), \
                   Path.toString(Path.parent(Path.parent(p))), \
                   Path.toString(p |> Path.parent |> Path.join(\"../data.bin\"))] \
                   == [\"x/data.bin\", \"data.bin\", \"bin\", \".\", \"data.bin\"] \
                   && File.exists(\"x\" |> Path.of |> Path.join(\"../data.bin\")) \
                   && not File.exists(Path.of(\".\"))\n";
        assert!(
            matches!(eval_with_prelude(src), Ok(Value::Bool(true))),
            "{:?}",
            eval_with_prelude(src).map(|v| v.to_string())
        );
        let err = failure("Path.of(\"x\") |> Path.join(\"/etc/passwd\")");
        assert!(err.contains("Path.join(\"/etc/passwd\"): an absolute path"), "{err}");
    }

    /// `Config.external` declares prefixes by source; `Config.xr` keeps them,
    /// and a change shows in the config's identity (the reload warning).
    #[test]
    fn config_declares_external_prefixes_by_source() {
        use crate::runtime_settings::ExternalPrefix;
        let config = |src: &str| eval_with_prelude(&format!("let main =\n  {src}\n"));
        let value = config(
            "Config.create() |> Config.external(\"ss2/\", Source.mount(\"ss2\") \
|> Source.device(\"/sdcard/s/x.kpf!data/res/\") |> Source.ignoreCase) |> Config.external(\"music/\", \
Source.mount(\"music\")) |> Config.xr(Xr.create() |> Xr.refreshRate(90))",
        )
        .unwrap();
        assert_eq!(
            externals_of(&value).unwrap(),
            vec![
                ExternalPrefix {
                    prefix: "ss2/".into(),
                    mount: "ss2".into(),
                    device: Some("/sdcard/s/x.kpf!data/res/".into()),
                    ignore_case: true,
                },
                ExternalPrefix {
                    prefix: "music/".into(),
                    mount: "music".into(),
                    device: None,
                    ignore_case: false,
                },
            ]
        );
        let plain = config("Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(90))");
        let key = |value: &Value| config_of(value).unwrap().external_key;
        assert_ne!(key(&value), key(&plain.unwrap()));
        assert_eq!(config_of(&value).unwrap().xr.refresh_rate, Some(90.0));

        for (src, error) in [
            (
                "Config.external(\"ss2\", Source.mount(\"ss2\"), Config.create())",
                "ending in `/`",
            ),
            (
                "Config.external(\"../x/\", Source.mount(\"x\"), Config.create())",
                "ending in `/`",
            ),
            (
                "Config.create() |> Config.external(\"a/\", Source.mount(\"a\")) \
|> Config.external(\"a/b/\", Source.mount(\"b\"))",
                "overlaps",
            ),
            (
                "Config.external(\"a/\", Source.mount(\"~/x\"), Config.create())",
                "mount name",
            ),
            (
                "Config.external(\"a/\", Source.mount(\"a\") |> Source.device(\"rel/x\"), \
Config.create())",
                "absolute path",
            ),
        ] {
            let err = config(src).err().unwrap_or_default();
            assert!(err.contains(error), "{src}: {err}");
        }
    }

    /// `File.bytes`/`File.exists` go through the installed project reader
    /// (the Quest shell's pushed-file cache), not the disk.
    #[test]
    fn file_reads_use_the_installed_project_reader() {
        let _guard = crate::storage::SAVE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Restore the default reader even if an assertion panics.
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                crate::storage::set_project_reader(None);
            }
        }
        let _reset = Reset;
        let cache = std::sync::Arc::new(crate::asset::AssetCache::new());
        cache.replace_uploaded("data/pushed.bin", vec![5, 6, 7]);
        crate::storage::set_project_reader(Some(cache));
        let result = eval_with_prelude(
            "let main = Bytes.length(File.bytes(Path.of(\"data/pushed.bin\"))) == 3.0 \
             && File.exists(Path.of(\"data/pushed.bin\")) && not File.exists(Path.of(\"other.bin\"))\n",
        );
        let missing = eval_with_prelude("let main = File.bytes(Path.of(\"other.bin\"))\n");
        assert!(
            matches!(result, Ok(Value::Bool(true))),
            "{:?}",
            result.map(|v| v.to_string())
        );
        let err = missing.err().expect("should fail");
        assert!(
            err.contains("can't read other.bin in the project (not pushed to this device"),
            "{err}"
        );
    }

    /// `Decode` (bundled Functor code) over a small made-up format: a header
    /// naming where a point list starts, read with `at`, `list`, `map3`.
    #[test]
    fn decode_composes_a_format_and_locates_failures() {
        let _guard = crate::storage::SAVE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        crate::storage::set_project_root(dir.path().to_path_buf());
        // "PTS!", count 2, offset 16, padding, then two f32le xyz points.
        let mut data = b"PTS!".to_vec();
        data.extend(2u32.to_le_bytes());
        data.extend(16u32.to_le_bytes());
        data.extend([0u8; 4]);
        for v in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0] {
            data.extend(v.to_le_bytes());
        }
        std::fs::write(dir.path().join("points.bin"), &data).unwrap();
        // 100,000 u8s: long lists loop without recursion.
        std::fs::write(dir.path().join("big.bin"), vec![7u8; 100_000]).unwrap();
        let prelude = "type header = { magic: string, count: float, start: float }\n\
             let vec3 = Decode.map3((x, y, z) => x + y + z, Decode.f32le, Decode.f32le, Decode.f32le)\n\
             let header = Decode.map3((magic, count, start): header => \
               { magic: magic, count: count, start: start }, \
               Decode.field(\"magic\", Decode.latin1(4.0)), \
               Decode.field(\"count\", Decode.u32le), Decode.field(\"start\", Decode.u32le))\n\
             let points = header |> Decode.andThen((h: header) => \
               Decode.field(\"points\", Decode.at(h.start, Decode.list(h.count, vec3))))\n\
             let data = File.bytes(Path.of(\"points.bin\"))\n";
        let checks = [
            "Decode.run(points, data) == Result.Ok([6.0, 15.0])",
            "Decode.run(Decode.map2((a, b) => a + b, Decode.u8, Decode.skipThen(3.0, Decode.u8)), data) \
             == Result.Ok(80.0 + 2.0)",
            "(Decode.run(Decode.list(100000.0, Decode.u8), File.bytes(Path.of(\"big.bin\"))) \
             |> Result.map(List.length)) == Result.Ok(100000.0)",
            "match Decode.run(Decode.field(\"head\", Decode.list(3.0, Decode.list(2.0, Decode.f64le))), data) with \
             | Result.Error(e) => Binary.describe(e) == \"head[2][1] at byte 40: needs 8 bytes, 0 left\" \
             | Result.Ok(_) => false",
            "Result.isError(Decode.run(Decode.list(1.5, Decode.u8), data)) \
             && Result.isError(Decode.run(Decode.fail(\"no\"), data)) \
             && Decode.run(Decode.succeed(1.0), data) == Result.Ok(1.0)",
            "Decode.run(Decode.bytes(4.0) |> Decode.map(Bytes.length), data) == Result.Ok(4.0)",
            "Decode.run(List.range(40.0) |> List.fold((d, i) => Decode.map2((a, b) => a + b, d, \
             Decode.u8), Decode.succeed(0.0)), File.bytes(Path.of(\"big.bin\"))) == Result.Ok(280.0)",
            "(Decode.run(Decode.sequence(List.range(5000.0) |> List.map((i) => Decode.u8)), \
             File.bytes(Path.of(\"big.bin\"))) |> Result.map(List.sum)) == Result.Ok(35000.0)",
            "Result.isError(Decode.run(Decode.skipThen(100.0, Decode.succeed(1.0)), data)) \
             && Result.isError(Decode.run(Decode.skipThen(-1.0, Decode.u8), data)) \
             && Result.isError(Decode.run(Decode.at(41.0, Decode.succeed(1.0)), data)) \
             && Decode.run(Decode.at(40.0, Decode.succeed(1.0)), data) == Result.Ok(1.0)",
            "Decode.run(Decode.map2((a, b) => a * 1000.0 + b, Decode.at(4.0, Decode.u8), Decode.u8), data) \
             == Result.Ok(2080.0)",
        ];
        for check in checks {
            let src = format!("{prelude}let main = {check}\n");
            assert!(
                matches!(eval_with_prelude(&src), Ok(Value::Bool(true))),
                "{check}: {:?}",
                eval_with_prelude(&src).map(|v| v.to_string())
            );
        }
    }

    /// `Hardware` cutters match their tables: radii and depths read back from
    /// thin cross-sections (`intersect` with a 0.02 mm slab), volumes from
    /// the analytic shapes.
    #[test]
    fn hardware_cuts_match_their_tables() {
        let prelude = "let near = (a: float, b: float) => Math.abs(a - b) < 0.0005\n\
             // Widest X of a cut in a 0.02 mm slab centered at height z.\n\
             let at = (z: float, cut) => Vec3.x(Solid.bounds(Solid.intersect(\
               Solid.box(100.0, 100.0, 0.02) |> Solid.translate(Vec3.make(0.0, 0.0, z)), cut)).max)\n\
             let low = (cut) => Vec3.z(Solid.bounds(cut).min)\n\
             let fit = Fit.clearance\n\
             let m3 = Hardware.m3\n";
        let checks = [
            // Tables.
            "m3.diameter == 3.0 && m3.headDiameter == 5.5 && m3.nutWidth == 5.5 \
             && Hardware.m8.sinkDiameter == 17.92 && Hardware.insertM2_5.hole == 3.6",
            "List.length(Hardware.screws) == 7.0 && List.all((s: Hardware.screw) => \
             s.headDiameter > s.diameter && s.sinkDiameter > s.headDiameter \
             && s.nutWidth > s.diameter, Hardware.screws)",
            "List.length(Hardware.inserts) == 7.0 \
             && List.all((i: Hardware.insert) => i.hole > 0.0 && i.length > 0.0, Hardware.inserts) \
             && { Hardware.insertM3 with hole: 4.1 }.hole == 4.1",
            "Fit.press.gap < Fit.slip.gap && Fit.slip.gap < Fit.clearance.gap && Fit.gap(0.2).gap == 0.2",
            // Clearance hole: Ø 3 + 2 × 0.3, 10 deep, 1 mm above the face.
            "near(at(-5.0, Hardware.clearanceHole(m3, fit, 10.0)), 1.8) \
             && near(low(Hardware.clearanceHole(m3, fit, 10.0)), -10.0) \
             // Inscribed facets: a 1.8 mm circle's 32-gon holds 0.64% less area.
             && Math.abs(Solid.volume(Hardware.clearanceHole(m3, fit, 10.0)) / (Math.pi * 1.8 * 1.8 * 11.0) - 1.0) < 0.01",
            // Counterbore: head pocket Ø 5.5 + 0.6 down to 3.3, then the hole.
            "near(at(-3.2, Hardware.counterbore(m3, fit, 10.0)), 3.05) \
             && near(at(-3.4, Hardware.counterbore(m3, fit, 10.0)), 1.8)",
            // Countersink: Ø 6.72 + 0.6 at the face, shrinking 1 mm per mm down
            // (read at the slab's top, 0.01 above z).
            "near(at(0.0, Hardware.countersink(m3, fit, 10.0)), 3.67) \
             && near(at(-1.0, Hardware.countersink(m3, fit, 10.0)), 2.67) \
             && near(at(-3.0, Hardware.countersink(m3, fit, 10.0)), 1.8)",
            // Nut trap: 5.5 + 0.6 across flats (±Y), corners at flats × 2/√3.
            "near(Vec3.y(Solid.bounds(Hardware.nutTrap(m3, fit, 3.0)).max), 3.05) \
             && near(Vec3.x(Solid.bounds(Hardware.nutTrap(m3, fit, 3.0)).max), 3.05 * 2.0 / Math.sqrt(3.0)) \
             && near(low(Hardware.nutTrap(m3, fit, 3.0)), -3.0) \
             && Math.abs(Solid.volume(Hardware.nutTrap(m3, fit, 3.0)) - 6.1 * 6.1 * Math.sqrt(3.0) / 2.0 * 4.0) < 0.001",
            // Insert pocket: the maker's hole, no fit added.
            "near(at(-3.0, Hardware.insertPocket(Hardware.insertM3, 6.7)), 2.0) \
             && near(low(Hardware.insertPocket(Hardware.insertM3, 6.7)), -6.7)",
            // Magnet pocket: 6 × 3 mm disc under a press fit.
            "near(at(-1.0, Hardware.magnetPocket(Fit.press, 6.0, 3.0)), 3.05) \
             && near(low(Hardware.magnetPocket(Fit.press, 6.0, 3.0)), -3.05)",
            // A negative gap tightens the sides but never makes a pocket shallow.
            "near(at(-1.0, Hardware.magnetPocket(Fit.gap(-0.1), 6.0, 3.0)), 2.9) \
             && near(low(Hardware.magnetPocket(Fit.gap(-0.1), 6.0, 3.0)), -3.0) \
             && near(at(-2.9, Hardware.counterbore(m3, Fit.gap(-0.1), 10.0)), 2.65)",
        ];
        for check in checks {
            let src = format!("{prelude}let main = {check}\n");
            assert!(
                matches!(eval_with_prelude(&src), Ok(Value::Bool(true))),
                "{check}: {:?}",
                eval_with_prelude(&src).map(|v| v.to_string())
            );
        }
    }

    /// `Thread` sizes, fits and engagement; `Solid.thread`'s refusals.
    #[test]
    fn threads_fit_and_engage() {
        let checks = [
            "List.length(Thread.sizes) == 8.0 && Thread.m8.pitch == 1.25 && Thread.m12.pitch == 1.75 \
             && Thread.unc1_4_20.diameter == 6.35 && Thread.unc1_4_20.pitch == 1.27",
            // Full ISO depth 5√3/16·P; the clearance fit closes the tap's
            // P/4 flat (gap > H/4): 2·gap over the 60° triangle's bottom.
            "Math.abs(Thread.engagement(Thread.m8, Fit.gap(0.0)) - 5.0 * Math.sqrt(3.0) / 16.0 * 1.25) < 0.000001",
            "Math.abs(Thread.engagement(Thread.m8, Fit.clearance) \
             - (7.0 / 8.0 * Math.sqrt(3.0) / 2.0 * 1.25 - 0.6)) < 0.000001",
            // Every size taps at a slip fit except M3; small ones refuse the clearance fit.
            "List.all((s: Thread.size) => s.name == \"M3\" || Thread.engagement(s, Fit.slip) \
             >= Thread.engagement(s, Fit.gap(0.0)) / 3.0, Thread.sizes)",
            "Vec3.z(Solid.bounds(Thread.tap(Thread.m3, Fit.press, 5.0)).max) > 4.999 \
             && Vec3.z(Solid.bounds(Thread.tap(Thread.m8, Fit.clearance, 5.0)).max) > 4.999",
            "Math.abs(Vec3.x(Solid.bounds(Thread.rod(Thread.m8, 5.0)).max) - 4.0) < 0.0001",
        ];
        for check in checks {
            let src = format!("let main = {check}\n");
            assert!(
                matches!(eval_with_prelude(&src), Ok(Value::Bool(true))),
                "{check}: {:?}",
                eval_with_prelude(&src).map(|v| v.to_string())
            );
        }
        let err = eval_with_prelude("let main = Solid.volume(Thread.tap(Thread.m5, Fit.clearance, 5.0))\n")
            .err()
            .expect("should fail");
        assert!(err.contains("leaves M5 only"), "{err}");
        let err = eval_with_prelude("let main = Solid.volume(Solid.thread(3.0, 0.5, 5.0, -1.0))\n")
            .err()
            .expect("should fail");
        assert!(err.contains("leaves no thread"), "{err}");
    }

    /// `Solid.split` keeps each side of a normalized plane; `Connector` pins,
    /// holes and keys mate with exactly the `Fit` gap; `layFlat` puts a cut
    /// face on the bed.
    #[test]
    fn split_halves_and_connectors_mate() {
        let prelude = "let near = (a: float, b: float) => Math.abs(a - b) < 0.0005\n\
             let same = (a: float, b: float) => Math.abs(a / b - 1.0) < 0.000001\n\
             let up = Vec3.make(0.0, 0.0, 1.0)\n\
             let box = Solid.box(40.0, 30.0, 60.0)\n\
             let halves = (p: Solid.plane, s) => Solid.split(p, s)\n\
             let first = (pair) => match pair with | (a, _) => a\n\
             let second = (pair) => match pair with | (_, b) => b\n\
             let pin = { diameter: 5.0, length: 16.0 }\n\
             let k = { width: 6.0, length: 10.0, height: 4.0 }\n\
             let spots = [{ x: -10.0, y: 0.0 }, { x: 10.0, y: 5.0 }]\n\
             let flat = { normal: up, offset: 10.0 }\n\
             // Oblique and unnormalized, through the box's center.\n\
             let tilted = { normal: Vec3.make(2.0, 0.0, 2.0), offset: 0.0 }\n\
             let dowelled = Connector.dowelled(tilted, Fit.slip, pin, spots, box)\n\
             let pinAt = (at: Input.point2) => Connector.onPlane(tilted, at, Connector.pin(pin) |> Solid.translate(Vec3.make(0.0, 0.0, -8.0)))\n\
             let keyed = Connector.keyed(flat, Fit.slip, k, { x: 5.0, y: 0.0 }, box)\n";
        let checks = [
            // Normal (0, 0, 2) with offset 20 is z = 10: 20 of the box's 60 mm above.
            "near(Solid.volume(first(halves({ normal: Vec3.make(0.0, 0.0, 2.0), offset: 20.0 }, box))), 40.0 * 30.0 * 20.0) \
             && near(Vec3.z(Solid.bounds(second(halves({ normal: Vec3.make(0.0, 0.0, 2.0), offset: 20.0 }, box))).max), 10.0)",
            // Pin: nominal, standing on the bed; hole: Ø + 2·gap, length / 2 + gap each side.
            "near(Vec3.x(Solid.bounds(Connector.pin(pin)).max), 2.5) && near(Vec3.z(Solid.bounds(Connector.pin(pin)).min), 0.0) \
             && near(Vec3.z(Solid.bounds(Connector.pin(pin)).max), 16.0) \
             && near(Vec3.x(Solid.bounds(Connector.hole(Fit.slip, pin)).max) - 2.5, Fit.slip.gap) \
             && near(Vec3.z(Solid.bounds(Connector.hole(Fit.slip, pin)).max), 8.0 + Fit.slip.gap)",
            // A negative gap tightens the hole, never its depth; a short pin still builds.
            "near(Vec3.x(Solid.bounds(Connector.hole(Fit.gap(-0.1), pin)).max), 2.4) \
             && near(Vec3.z(Solid.bounds(Connector.hole(Fit.gap(-0.1), pin)).max), 8.0) \
             && near(Vec3.z(Solid.bounds(Connector.slot(Fit.gap(-0.1), k)).max), 4.0) \
             && near(Vec3.z(Solid.bounds(Connector.pin({ diameter: 5.0, length: 1.0 })).max), 1.0)",
            // A normal a hair off +Z leaves spots where they are (no spin about Z).
            "near(Vec3.y(Solid.bounds(Connector.onPlane({ normal: Vec3.make(0.000001, 0.000001, 1.0), offset: 0.0 }, \
               { x: 10.0, y: 0.0 }, Solid.box(1.0, 1.0, 1.0))).max), 0.5)",
            // onPlane turns Z to the normal: a pin on the x = 10 plane runs from x = 10 out.
            "near(Vec3.x(Solid.bounds(Connector.onPlane({ normal: Vec3.make(3.0, 0.0, 0.0), offset: 30.0 }, { x: 0.0, y: 0.0 }, Connector.pin(pin))).min), 10.0) \
             && near(Vec3.x(Solid.bounds(Connector.onPlane({ normal: Vec3.make(3.0, 0.0, 0.0), offset: 30.0 }, { x: 0.0, y: 0.0 }, Connector.pin(pin))).max), 26.0)",
            // Dowelled on the oblique plane: halves sum to the box less two holes,
            // and the pins, seated, touch neither half.
            "same(Solid.volume(first(dowelled)) + Solid.volume(second(dowelled)), \
               Solid.volume(box) - 2.0 * Solid.volume(Connector.hole(Fit.slip, pin))) \
             && List.all((at: Input.point2) => Solid.volume(Solid.intersect(first(dowelled), pinAt(at))) < 0.000001 \
               && Solid.volume(Solid.intersect(second(dowelled), pinAt(at))) < 0.000001, spots)",
            // Key: the tab stands 4 mm proud of the cut, its slot is gap wider and
            // deeper, and the halves meet without touching.
            "near(Vec3.z(Solid.bounds(second(keyed)).max), 14.0) \
             && near(Vec3.x(Solid.bounds(Connector.slot(Fit.slip, k)).max) - 3.0, Fit.slip.gap) \
             && near(Vec3.z(Solid.bounds(Connector.slot(Fit.slip, k)).max) - 4.0, Fit.slip.gap) \
             && Solid.volume(Solid.intersect(first(keyed), second(keyed))) < 0.000001 \
             && same(Solid.volume(first(keyed)) + Solid.volume(second(keyed)), \
               Solid.volume(box) - 6.3 * 10.3 * 4.15 + 6.0 * 10.0 * 4.0)",
            // layFlat on the oblique cut: the 30 × 40√2 mm cut face lies on the
            // bed, so a slab 0.01 mm above it holds the whole face.
            "near(Vec3.z(Solid.bounds(Connector.layFlat(Vec3.make(-1.0, 0.0, -1.0), first(halves(tilted, box)))).min), 0.0) \
             && Math.abs(Solid.volume(Solid.intersect(Solid.box(200.0, 200.0, 0.02), \
                  Connector.layFlat(Vec3.make(-1.0, 0.0, -1.0), first(halves(tilted, box))))) \
                / (0.01 * 30.0 * 40.0 * Math.sqrt(2.0)) - 1.0) < 0.01",
        ];
        for check in checks {
            let src = format!("{prelude}let main = {check}\n");
            assert!(
                matches!(eval_with_prelude(&src), Ok(Value::Bool(true))),
                "{check}: {:?}",
                eval_with_prelude(&src).map(|v| v.to_string())
            );
        }
        for (src, want) in [
            ("first(Solid.split({ normal: Vec3.make(0.0, 0.0, 0.0), offset: 1.0 }, box))", "non-zero, finite normal"),
            ("first(halves({ normal: up, offset: 30.0 }, box))", "misses or only touches the solid"),
            ("first(halves({ normal: up, offset: 45.0 }, box))", "spans -30.000 to 30.000"),
            ("Connector.onPlane({ normal: Vec3.make(0.0, 0.0, 0.0), offset: 1.0 }, { x: 0.0, y: 0.0 }, box)", "normal can't be zero"),
        ] {
            let src = format!("{prelude}let main = Solid.volume({src})\n");
            let err = eval_with_prelude(&src).err().expect("should fail");
            assert!(err.contains(want), "{src}: {err}");
        }
    }

    /// `Mesh.*` from raw f32 positions + polygons, then `Solid.fromMesh`
    /// closes an open box and scales it to the asked height.
    #[test]
    fn meshes_from_decoded_data_print_closed() {
        let _guard = crate::storage::SAVE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        crate::storage::set_project_root(dir.path().to_path_buf());
        // A unit cube's corners: index = x + 2y + 4z.
        let corners: Vec<u8> = (0..8)
            .flat_map(|i| [i & 1, (i >> 1) & 1, (i >> 2) & 1])
            .flat_map(|c| (c as f32).to_le_bytes())
            .collect();
        std::fs::write(dir.path().join("cube.bin"), &corners).unwrap();
        // Five faces as quads — no top, so the box is open.
        let prelude = "let box = Mesh.fromPolygons(File.bytes(Path.of(\"cube.bin\")), \
             [[0.0, 2.0, 3.0, 1.0], [0.0, 1.0, 5.0, 4.0], [1.0, 3.0, 7.0, 5.0], \
             [3.0, 2.0, 6.0, 7.0], [2.0, 0.0, 4.0, 6.0]])\n\
             let moved = Mesh.transformVertices(0.0, 8.0, \
             [1.0, 0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0], box)\n";
        let checks = [
            "Mesh.triangleCount(box) == 10.0",
            "Vec3.x(Mesh.bounds(moved).min) == 10.0 && Vec3.x(Mesh.bounds(moved).max) == 11.0",
            "Mesh.triangleCount(Mesh.combine([box, moved])) == 20.0",
            "Vec3.z(Solid.bounds(Solid.fromMesh({ height: 20.0, nozzle: 0.8 }, box)).max) == 20.0",
            "Solid.volume(Solid.fromMesh({ height: 20.0, nozzle: 0.8 }, box)) > 1000.0",
            "Solid.volume(Solid.fromMesh({ height: 20.0, nozzle: 0.8 }, box) \
             |> Solid.subtract(Solid.box(40.0, 40.0, 20.0))) < 1000.0",
        ];
        for check in checks {
            let src = format!("{prelude}let main = {check}\n");
            assert!(
                matches!(eval_with_prelude(&src), Ok(Value::Bool(true))),
                "{check}: {:?}",
                eval_with_prelude(&src).map(|v| v.to_string())
            );
        }
        let failure = |main: &str| {
            eval_with_prelude(&format!("{prelude}let main = {main}\n"))
                .err()
                .expect("should fail")
        };
        // `toScene` draws the mesh as it is, Z up → Y up: one unit cube node.
        let src = format!("{prelude}let main = Mesh.toScene(box)\n");
        let scene = eval_with_prelude(&src).unwrap();
        let Value::HostData(data) = &scene else {
            panic!("expected a scene")
        };
        let node = data.as_any().downcast_ref::<FunctorLangScene>().unwrap();
        let crate::scene3d::SceneObject::Geometry(crate::scene3d::Shape::Mesh { mesh }) = &node.0.obj
        else {
            panic!("expected a mesh node")
        };
        assert_eq!(mesh.triangles.len(), 10);
        // The same mesh draws from one render copy (one GPU upload).
        let src = format!("{prelude}let main = [Mesh.toScene(box), Mesh.toScene(box)]\n");
        let Ok(Value::List(scenes)) = eval_with_prelude(&src) else {
            panic!("expected a list")
        };
        let render = |v: &Value| {
            let Value::HostData(data) = v else { panic!() };
            let node = data.as_any().downcast_ref::<FunctorLangScene>().unwrap();
            let crate::scene3d::SceneObject::Geometry(crate::scene3d::Shape::Mesh { mesh }) =
                &node.0.obj
            else {
                panic!()
            };
            std::sync::Arc::as_ptr(mesh) as usize
        };
        assert_eq!(render(&scenes[0]), render(&scenes[1]));
        let err = failure("Mesh.fromPolygons(File.bytes(Path.of(\"cube.bin\")), [[0.0, 1.0, 9.0]])");
        assert!(err.contains("polygon 0 uses vertex 9, but there are 8"), "{err}");
        let err = failure("Mesh.fromPolygons(File.bytes(Path.of(\"cube.bin\")), [[0.0, 1.0]])");
        assert!(err.contains("needs 3 or more"), "{err}");
        // A degenerate polygon (repeated corner) adds no triangles.
        let src = format!("{prelude}let main = Mesh.triangleCount(Mesh.fromPolygons(\
            File.bytes(Path.of(\"cube.bin\")), [[0.0, 1.0, 0.0], [0.0, 1.0, 3.0]]))\n");
        assert!(matches!(eval_with_prelude(&src), Ok(Value::Number(n)) if n == 1.0));
        let err = failure("Solid.fromMesh({ height: 20.0 }, box)");
        assert!(err.contains("missing field `nozzle`"), "{err}");
        let err = failure("Mesh.transformVertices(4.0, 8.0, [1.0], box)");
        assert!(err.contains("aren't within"), "{err}");
    }

    /// `Cells.*` from records (the `cells.funi` example): locate, line of
    /// sight through the doorway and not through the wall, and errors.
    #[test]
    fn cells_locate_and_see_through_doorways() {
        let prelude = "let room = (x0, x1) => { planes: [\n\
             { n: Vec3.make(1.0, 0.0, 0.0), d: 0.0 - x0 }, { n: Vec3.make(-1.0, 0.0, 0.0), d: x1 },\n\
             { n: Vec3.make(0.0, 1.0, 0.0), d: 0.0 },      { n: Vec3.make(0.0, -1.0, 0.0), d: 3.0 },\n\
             { n: Vec3.make(0.0, 0.0, 1.0), d: 2.0 },      { n: Vec3.make(0.0, 0.0, -1.0), d: 2.0 },\n\
             ] }\n\
             let door = [Vec3.make(4.0, 0.0, -0.5), Vec3.make(4.0, 0.0, 0.5),\n\
                         Vec3.make(4.0, 2.0, 0.5), Vec3.make(4.0, 2.0, -0.5)]\n\
             let world = Cells.make({\n\
               cells: [room(0.0, 4.0), room(4.0, 8.0)],\n\
               portals: [{ from: 0.0, to: 1.0, polygon: door },\n\
                         { from: 1.0, to: 0.0, polygon: List.reverse(door) }],\n\
             })\n";
        let checks = [
            "Cells.locate(Vec3.make(6.0, 1.0, 0.0), world) == Option.Some(1.0)",
            "Cells.locate(Vec3.make(4.0, 1.0, 0.0), world) == Option.Some(0.0)",
            "Cells.locate(Vec3.make(9.0, 1.0, 0.0), world) == Option.None",
            "Cells.segment(Vec3.make(1.0, 1.0, 0.0), Vec3.make(7.0, 1.0, 0.0), world)",
            "Cells.segment(Vec3.make(1.0, 1.0, 1.5), Vec3.make(7.0, 1.0, 1.5), world) == false",
        ];
        for check in checks {
            let src = format!("{prelude}let main = {check}\n");
            assert!(
                matches!(eval_with_prelude(&src), Ok(Value::Bool(true))),
                "{check}: {:?}",
                eval_with_prelude(&src).map(|v| v.to_string())
            );
        }
        let failure = |main: &str| {
            eval_with_prelude(&format!("{prelude}let main = {main}\n"))
                .err()
                .expect("should fail")
        };
        let err = failure(
            "Cells.make({ cells: [room(0.0, 4.0)], portals: [{ from: 0.0, to: 1.0, polygon: door }] })",
        );
        assert!(
            err.contains("Cells.make: portal 0 (0 → 1) names a cell that doesn't exist"),
            "{err}"
        );
        let err = failure(
            "Cells.make({ cells: [room(0.0, 4.0), room(4.0, 8.0)], portals: [{ from: 0.5, to: 1.0, polygon: door }] })",
        );
        assert!(err.contains("portal 0 `from` must be a cell id"), "{err}");
        let err = failure("Cells.make({ cells: [{ planes: [] }], portals: [] })");
        assert!(err.contains("cell 0 is unbounded"), "{err}");
        let err = failure("Scene.inCells([1.5], Scene.cube())");
        assert!(err.contains("Scene.inCells: an id must be a cell id"), "{err}");
        // `Scene.cells` wraps its scene; `Scene.inCells` tags its subtree.
        let src = format!(
            "{prelude}let main = Scene.cells(world, Scene.inCells([1.0], Scene.cube()))\n"
        );
        let Ok(Value::HostData(data)) = eval_with_prelude(&src) else {
            panic!("{:?}", eval_with_prelude(&src).map(|v| v.to_string()));
        };
        let scene = &data.as_any().downcast_ref::<FunctorLangScene>().unwrap().0;
        let SceneObject::Cells(crate::CellsNode::Walk(cells), items) = &scene.obj else {
            panic!("{scene:?}");
        };
        assert_eq!(cells.len(), 2);
        assert!(matches!(&items[0].obj, SceneObject::Cells(crate::CellsNode::In(ids), _) if ids == &[1]));
    }

    /// `Encode` writes what `Decode` reads; a `Codec` round-trips records,
    /// lists and mapped values; writers refuse numbers that don't fit.
    #[test]
    fn encode_and_codecs_round_trip() {
        let prelude = "type pair = { first: float, second: float }\n\
             let pairCodec = Codec.record2((a, b): pair => { first: a, second: b }, \
               (p: pair) => p.first, Codec.f32le, (p: pair) => p.second, Codec.f32le)\n\
             let flag = Codec.map((n) => n == 1.0, (b) => if b then 1.0 else 0.0, Codec.u8)\n\
             let header = Encode.sequence([Encode.latin1(\"PTS!\"), Encode.u32le(2.0), \
               Encode.bytes(Bytes.fromF32le([1.0, 2.0]))])\n";
        let checks = [
            "Bytes.length(Encode.run(header)) == 16.0",
            "Decode.run(Decode.map2((m, n) => m, Decode.latin1(4.0), Decode.u32le), \
             Encode.run(header)) == Result.Ok(\"PTS!\")",
            "Decode.run(Decode.skipThen(8.0, Decode.list(2.0, Decode.f32le)), Encode.run(header)) \
             == Result.Ok([1.0, 2.0])",
            "Codec.decode(pairCodec, Codec.encode(pairCodec, { first: 1.5, second: -2.0 })) \
             == Result.Ok({ first: 1.5, second: -2.0 })",
            "(Codec.decode(Codec.list(3.0, pairCodec), Codec.encode(Codec.list(3.0, pairCodec), \
             [{ first: 1.0, second: 2.0 }, { first: 3.0, second: 4.0 }, { first: 5.0, second: 6.0 }])) \
             |> Result.map(List.length)) == Result.Ok(3.0)",
            "Codec.decode(flag, Codec.encode(flag, true)) == Result.Ok(true)",
            "Bytes.equals(Encode.run(Encode.zeros(3.0)), Bytes.fromU8([0.0, 0.0, 0.0]))",
            "Bytes.equals(Bytes.fromI16be([-2.0]), Bytes.fromU8([255.0, 254.0]))",
            "Bytes.equals(Encode.run(Encode.utf8(\"é\")), Bytes.fromU8([195.0, 169.0]))",
            "Bytes.equals(Bytes.fromLatin1(\"é\"), Bytes.fromU8([233.0])) \
             && Bytes.latin1(0.0, 1.0, Bytes.fromLatin1(\"é\")) == Result.Ok(\"é\")",
            "Bytes.equals(Bytes.fromU32be([4294967295.0]), Bytes.fromU8([255.0, 255.0, 255.0, 255.0])) \
             && Bytes.equals(Bytes.fromI32le([-2147483648.0]), Bytes.fromU8([0.0, 0.0, 0.0, 128.0])) \
             && Bytes.equals(Bytes.fromI8([-128.0]), Bytes.fromU8([128.0]))",
            "Bytes.f64be(0.0, Bytes.fromF64be([2.5])) == Result.Ok(2.5) \
             && Bytes.f32be(0.0, Bytes.fromF32be([0.5])) == Result.Ok(0.5) \
             && Bytes.u16be(0.0, Bytes.fromU16be([258.0])) == Result.Ok(258.0)",
            "Bytes.length(Bytes.fromF32le([0.0 / 0.0, 1.0 / 0.0])) == 8.0 \
             && Bytes.length(Bytes.concat([])) == 0.0 && Bytes.length(Bytes.zeros(5.0)) == 5.0",
            "Codec.decode(Codec.latin1(4.0), Codec.encode(Codec.latin1(4.0), \"LGMD\")) \
             == Result.Ok(\"LGMD\")",
        ];
        for check in checks {
            let src = format!("{prelude}let main = {check}\n");
            assert!(
                matches!(eval_with_prelude(&src), Ok(Value::Bool(true))),
                "{check}: {:?}",
                eval_with_prelude(&src).map(|v| v.to_string())
            );
        }
        for (main, expected) in [
            ("Bytes.fromU8([256.0])", "doesn't fit a u8"),
            ("Encode.run(Encode.u16le(1.5))", "doesn't fit a u16"),
            ("Bytes.fromLatin1(\"€\")", "isn't a Latin-1 character"),
            ("Bytes.fromI8([-129.0])", "doesn't fit a i8"),
            ("Bytes.fromU32le([0.0 / 0.0])", "doesn't fit a u32"),
            ("Bytes.fromF32le([Math.pow(10.0, 300.0)])", "doesn't fit a f32"),
            ("Bytes.zeros(2.5)", "isn't a whole number"),
            ("Codec.encode(Codec.latin1(4.0), \"AB\")", "expected 4 characters"),
            ("Codec.encode(Codec.list(1.0, Codec.u8), [1.0, 2.0])", "expected 1 items, got 2"),
        ] {
            let err = eval_with_prelude(&format!("let main = {main}\n"))
                .err()
                .expect("should fail");
            assert!(err.contains(expected), "{main}: {err}");
        }
    }

    /// A two-bone arm: `upper` at the origin, `lower` 1 up. Vertices: the
    /// root (upper), the tip and a side point (lower), and a 50/50 blend at
    /// the tip. `bend` turns `lower` 90° about Z over two frames at 2 fps;
    /// `lift` also moves `upper` to z = 1.
    const ARM: &str = "let s = Math.sqrt(0.5)\n\
        let id = { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }\n\
        let none = Bytes.fromF32le([])\n\
        let armData = {\n\
          positions: Bytes.fromF32le([0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 1.0, 1.0, 0.0, 0.0, 2.0, 0.0]),\n\
          normals: Bytes.fromF32le([0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),\n\
          uvs: none,\n\
          joints: Bytes.fromU8([0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]),\n\
          weights: Bytes.fromF32le([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0]),\n\
          triangles: Bytes.fromU32le([0.0, 1.0, 2.0]),\n\
          skeleton: [\n\
            { name: \"upper\", parent: Option.None, translation: Vec3.make(0.0, 0.0, 0.0), rotation: id },\n\
            { name: \"lower\", parent: Option.Some(0.0), translation: Vec3.make(0.0, 1.0, 0.0), rotation: id }\n\
          ],\n\
          clips: [\n\
            { name: \"bend\", fps: 2.0, frames: 2.0, tracks: [\n\
              { joint: \"lower\", rotations: Bytes.fromF32le([0.0, 0.0, 0.0, 1.0, 0.0, 0.0, s, s]), translations: none }\n\
            ] },\n\
            { name: \"lift\", fps: 1.0, frames: 1.0, tracks: [\n\
              { joint: \"upper\", rotations: none, translations: Bytes.fromF32le([0.0, 0.0, 1.0]) },\n\
              { joint: \"lower\", rotations: Bytes.fromF32le([0.0, 0.0, s, s]), translations: none }\n\
            ] }\n\
          ],\n\
          texture: Option.None\n\
        }\n\
        let arm = Model.skinned(armData)\n";

    /// `Model.pose(anim, arm)`'s vertex positions.
    fn arm_pose(anim: &str) -> Vec<[f32; 3]> {
        let value = eval_with_prelude(&format!("{ARM}let main = Model.pose({anim}, arm)\n"))
            .unwrap_or_else(|e| panic!("{anim}: {e}"));
        let Value::HostData(data) = value else {
            panic!("{anim}: expected Bytes");
        };
        let bytes = data.as_any().downcast_ref::<FunctorLangBytes>().expect("Bytes");
        bytes
            .0
            .as_slice()
            .chunks_exact(12)
            .map(|p| std::array::from_fn(|k| f32::from_le_bytes(p[4 * k..4 * k + 4].try_into().unwrap())))
            .collect()
    }

    fn assert_near(anim: &str, vertex: usize, expected: [f32; 3]) {
        let actual = arm_pose(anim)[vertex];
        assert!(
            actual.iter().zip(expected).all(|(a, e)| (a - e).abs() < 1e-4),
            "{anim}: vertex {vertex} is {actual:?}, expected {expected:?}"
        );
    }

    /// `Model.pose` against hand-computed positions: rest, the clip (keys,
    /// interpolation, the wrap key), blend, mask and lookAt.
    #[test]
    fn skinned_model_poses_match_hand_computed_positions() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        // Rest: the bind pose, every vertex where it was authored.
        let rest = arm_pose("Anim.rest()");
        for (vertex, expected) in [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [1.0, 1.0, 0.0], [0.0, 2.0, 0.0]]
            .into_iter()
            .enumerate()
        {
            assert!(rest[vertex].iter().zip(expected).all(|(a, e)| (a - e).abs() < 1e-6));
        }
        // Frame 1 (t = 0.5): `lower` turned 90°; the blended tip is halfway.
        let bent = "Anim.clip(\"bend\", 0.5)";
        assert_near(bent, 0, [0.0, 0.0, 0.0]);
        assert_near(bent, 1, [-1.0, 1.0, 0.0]);
        assert_near(bent, 2, [0.0, 2.0, 0.0]);
        assert_near(bent, 3, [-0.5, 1.5, 0.0]);
        // Mid-way between keys interpolates: 45°.
        assert_near("Anim.clip(\"bend\", 0.25)", 1, [-h, 1.0 + h, 0.0]);
        // After the last key the wrap key interpolates back to frame 0
        // instead of holding frame 1.
        assert_near("Anim.clip(\"bend\", 0.75)", 1, [-h, 1.0 + h, 0.0]);
        let almost = arm_pose("Anim.clip(\"bend\", 0.999)")[1];
        assert!(almost[0].abs() < 0.01 && (almost[1] - 2.0).abs() < 0.01, "{almost:?}");
        assert_near("Anim.clip(\"bend\", 1.25)", 1, [-h, 1.0 + h, 0.0]);
        // Blend: half the bend is 45°.
        assert_near("Anim.blend([(Anim.clip(\"bend\", 0.5), 1.0), (Anim.rest(), 1.0)])", 1, [-h, 1.0 + h, 0.0]);
        // A translation key replaces the rest translation.
        let lift = "Anim.clip(\"lift\", 0.0)";
        assert_near(lift, 0, [0.0, 0.0, 1.0]);
        assert_near(lift, 1, [-1.0, 1.0, 1.0]);
        // Mask: only `lower`'s subtree follows the clip; `upper` stays put.
        let masked = "Anim.mask([\"lower\"], Anim.clip(\"lift\", 0.0))";
        assert_near(masked, 0, [0.0, 0.0, 0.0]);
        assert_near(masked, 1, [-1.0, 1.0, 0.0]);
        // lookAt: `lower`'s +Z turns to +X (90° about Y), so +X goes to -Z.
        let aimed = "Anim.lookAt(\"lower\", Vec3.make(1.0, 1.0, 0.0), 120deg, 1.0, Anim.rest())";
        assert_near(aimed, 2, [0.0, 1.0, -1.0]);
        assert_near(aimed, 1, [0.0, 2.0, 0.0]);
    }

    /// `Model.inspect`, and the data's wire form: its source, revalidated
    /// when read back.
    #[test]
    fn skinned_models_inspect_and_serialize() {
        let main = |expr: &str| eval_with_prelude(&format!("{ARM}let main = {expr}\n"));
        assert!(matches!(
            main("Model.inspect(arm).joints == [\"upper\", \"lower\"] \
                  && List.map((c) => c.duration, Model.inspect(arm).clips) == [1.0, 1.0] \
                  && Model.inspect(arm).vertices == 4.0"),
            Ok(Value::Bool(true))
        ));

        let Value::HostData(model) = main("arm").unwrap() else {
            panic!("expected a Model");
        };
        let model = &model.as_any().downcast_ref::<FunctorLangModel>().unwrap().0;
        let json = serde_json::to_string(&**model).unwrap();
        assert!(
            json.starts_with(r#"{"positions":[[0.0,0.0,0.0],[0.0,2.0,0.0],"#)
                && json.contains(r#""skeleton":[{"name":"upper","parent":null,"translation":[0.0,0.0,0.0],"rotation":[0.0,0.0,0.0,1.0]}"#),
            "{json}"
        );
        let back: crate::model::ModelData = serde_json::from_str(&json).unwrap();
        assert_eq!(model.source(), back.source());
        assert_ne!(model.id(), back.id());
        // A bad payload is refused with the validation error.
        let bad = json.replace(r#""joints":[[0,0,0,0]"#, r#""joints":[[9,0,0,0]"#);
        let err = serde_json::from_str::<crate::model::ModelData>(&bad)
            .unwrap_err()
            .to_string();
        assert!(err.contains("joints: vertex 0 uses joint 9"), "{err}");
    }

    /// `Scene.equals` compares models by identity; `GET /scene` carries the
    /// model inline.
    #[test]
    fn skinned_model_scenes_compare_and_serialize() {
        let main = |expr: &str| eval_with_prelude(&format!("{ARM}let main = {expr}\n"));
        // The same value is equal; an identical rebuild is a different model.
        assert!(matches!(
            main("Scene.equals(Scene.modelOf(arm), Scene.modelOf(arm))"),
            Ok(Value::Bool(true))
        ));
        assert!(matches!(
            main("Scene.equals(Scene.modelOf(arm), Scene.modelOf(Model.skinned(armData)))"),
            Ok(Value::Bool(false))
        ));

        let Value::HostData(scene) =
            main("Scene.modelOf(arm) |> Scene.animate(Anim.clip(\"bend\", 0.5))").unwrap()
        else {
            panic!("expected a Scene");
        };
        let scene = &scene.as_any().downcast_ref::<FunctorLangScene>().unwrap().0;
        let json = serde_json::to_string(scene).unwrap();
        assert!(
            json.contains(r#""handle":{"Data":{"positions":[[0.0,0.0,0.0],[0.0,2.0,0.0],"#),
            "{json}"
        );
        let back: Scene3D = serde_json::from_str(&json).unwrap();
        let (SceneObject::Model(a), SceneObject::Model(b)) = (&scene.obj, &back.obj) else {
            panic!("expected model nodes");
        };
        let (ModelHandle::Data(a), ModelHandle::Data(b)) = (&a.handle, &b.handle) else {
            panic!("expected data handles");
        };
        assert_eq!(a.source(), b.source());
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    /// A textured model carries its texture as a locator, through `GET
    /// /scene` and back.
    #[test]
    fn skinned_model_textures_serialize_as_locators() {
        let textured = |texture: &str| {
            ARM.replace("uvs: none", "uvs: Bytes.fromF32le([0.0, 1.0, 0.0, 0.0, 1.0, 0.5, 0.0, 0.0])")
                .replace("texture: Option.None", &format!("texture: Option.Some({texture})"))
        };
        {
            let src = format!("{}let main = Scene.modelOf(arm)\n", textured("Asset.texture(\"skin.gif\")"));
            let Value::HostData(scene) = eval_with_prelude(&src).unwrap() else {
                panic!("expected a Scene");
            };
            let scene = &scene.as_any().downcast_ref::<FunctorLangScene>().unwrap().0;
            let json = serde_json::to_string(scene).unwrap();
            assert!(json.contains(r#""texture":{"File":"skin.gif"}"#), "{json}");
            let back: Scene3D = serde_json::from_str(&json).unwrap();
            let SceneObject::Model(model) = &back.obj else {
                panic!("expected a model node");
            };
            let ModelHandle::Data(data) = &model.handle else {
                panic!("expected a data handle");
            };
            assert_eq!(data.source().uvs[1], [0.0, 0.0]);
            assert_eq!(serde_json::to_string(&back).unwrap(), json);
        }
        // Untextured models keep the v19 shape: no `texture` key.
        let plain = eval_with_prelude(&format!("{ARM}let main = Scene.modelOf(arm)\n")).unwrap();
        let Value::HostData(plain) = plain else { panic!("expected a Scene") };
        let plain = &plain.as_any().downcast_ref::<FunctorLangScene>().unwrap().0;
        assert!(!serde_json::to_string(plain).unwrap().contains("texture"));

        let err = eval_with_prelude(&format!(
            "{}let main = arm\n",
            textured("Asset.texture(\"skin.gif\")").replace("uvs: Bytes.fromF32le([0.0, 1.0, 0.0, 0.0, 1.0, 0.5, 0.0, 0.0])", "uvs: none")
        ))
        .err()
        .expect("should fail");
        assert_eq!(err, "Model.skinned: texture: needs uvs (one per vertex)");
        let err = eval_with_prelude(&format!("{}let main = arm\n", textured("\"skin.gif\"")))
            .err()
            .expect("should fail");
        assert!(err.contains("expected Option.t<Asset.Texture>, got Option.t<string>"), "{err}");
    }

    /// Bad data is an error naming the field.
    #[test]
    fn skinned_model_errors_name_the_field() {
        let fail = |replace: (&str, &str)| {
            let src = ARM.replace(replace.0, replace.1);
            assert_ne!(src, ARM, "{replace:?} did not apply");
            eval_with_prelude(&format!("{src}let main = arm\n")).err().expect("should fail")
        };
        let err = fail(("weights: Bytes.fromF32le([1.0,", "weights: Bytes.fromF32le([0.0,"));
        assert_eq!(err, "Model.skinned: weights: vertex 0's weights sum to 0");
        let err = fail(("parent: Option.Some(0.0)", "parent: Option.Some(1.0)"));
        assert!(err.starts_with("Model.skinned: skeleton[1] (\"lower\"): parent 1 must come before it"), "{err}");
        let err = fail(("triangles: Bytes.fromU32le([0.0, 1.0, 2.0])", "triangles: Bytes.fromU32le([0.0, 1.0, 7.0])"));
        assert_eq!(err, "Model.skinned: triangles: triangle 0 uses vertex 7, but there are 4");
        let err = fail(("{ joint: \"upper\",", "{ joint: \"elbow\","));
        assert_eq!(err, "Model.skinned: clips[1] (\"lift\").tracks[0]: no joint named \"elbow\"");
        let err = fail(("uvs: none", "uvs: Bytes.fromF32le([1.0])"));
        assert_eq!(err, "Model.skinned: uvs: 4 bytes is not a whole number of 8-byte entries");
    }

    /// A unit quad of two triangles, one group each: the first textured by
    /// pixels, the second unlit and untextured.
    const ROOM: &str = "let none = Bytes.fromF32le([])\n\
        let px = Texture.fromPixels({ width: 1.0, height: 1.0, format: Texture.Rgb8, \
colorSpace: Texture.Srgb, pixels: Bytes.fromU8([200.0, 100.0, 50.0]) })\n\
        let roomData = {\n\
          positions: Bytes.fromF32le([0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0]),\n\
          normals: Bytes.fromF32le([0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),\n\
          uvs: Bytes.fromF32le([0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]),\n\
          uvs2: Bytes.fromF32le([0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 0.0, 0.5]),\n\
          triangles: Bytes.fromU32le([0.0, 1.0, 2.0, 0.0, 2.0, 3.0]),\n\
          groups: [\n\
            { first: 0.0, count: 1.0, texture: Option.Some(px), unlit: false },\n\
            { first: 1.0, count: 1.0, texture: Option.None, unlit: true }\n\
          ],\n\
          lightmap: Option.Some({ texture: px, scale: 2.0, floor: Color.rgb(0.1, 0.0, 0.0) })\n\
        }\n\
        let room = Model.static(roomData)\n";

    /// A static model draws through `Scene.modelOf`, has no rig, and rides
    /// `GET /scene` inline (its pixel texture too) and back.
    #[test]
    fn static_models_build_and_serialize() {
        let main = |expr: &str| eval_with_prelude(&format!("{ROOM}let main = {expr}\n"));
        assert!(matches!(
            main("Model.inspect(room).joints == [] && Model.inspect(room).triangles == 2.0"),
            Ok(Value::Bool(true))
        ));
        assert_eq!(
            main("Model.pose(Anim.rest(), room)").err().unwrap(),
            "Model.pose: a static model has no skeleton to pose"
        );
        let Value::HostData(scene) = main("Scene.modelOf(room)").unwrap() else {
            panic!("expected a Scene");
        };
        let scene = &scene.as_any().downcast_ref::<FunctorLangScene>().unwrap().0;
        let json = serde_json::to_string(scene).unwrap();
        assert!(
            json.contains(r#""groups":[{"first":0,"count":1,"texture":{"Data":{"width":1,"#)
                && json.contains(r#"{"first":1,"count":1,"unlit":true}]"#)
                && !json.contains("joints"),
            "{json}"
        );
        let back: Scene3D = serde_json::from_str(&json).unwrap();
        let SceneObject::Model(model) = &back.obj else {
            panic!("expected a model node");
        };
        let ModelHandle::Data(data) = &model.handle else {
            panic!("expected a data handle");
        };
        assert!(data.is_static());
        assert_eq!(data.source().uvs2[2], [0.5, 0.5]);
        let lightmap = data.source().lightmap.as_ref().expect("a lightmap");
        assert_eq!((lightmap.scale, lightmap.floor), (2.0, [0.1, 0.0, 0.0]));
        assert!(json.contains(r#""lightmap":{"texture":{"Data":"#), "{json}");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    /// `Light.baked` marks a light on the frame's wire.
    #[test]
    fn baked_lights_reach_the_frame() {
        let src = "let main = Frame.createLit(Camera3D.lookAt(Vec3.make(0.0, 0.0, 1.0), \
Vec3.make(0.0, 0.0, 0.0)), Scene.cube(), [Light.ambient(Color.rgb(0.2, 0.2, 0.2)) |> Light.baked, \
Light.point(Vec3.make(0.0, 1.0, 0.0), Color.rgb(1.0, 1.0, 1.0), 1.0, 5.0)])\n";
        let Value::HostData(frame) = eval_with_prelude(src).unwrap() else {
            panic!("expected a Frame");
        };
        let frame = &frame.as_any().downcast_ref::<FunctorLangFrame>().unwrap().0;
        let baked: Vec<bool> = frame.lights.iter().map(Light::is_baked).collect();
        assert_eq!(baked, [true, false]);
    }

    /// Bad static data is an error naming the field.
    #[test]
    fn static_model_errors_name_the_field() {
        let fail = |replace: (&str, &str)| {
            let src = ROOM.replace(replace.0, replace.1);
            assert_ne!(src, ROOM, "{replace:?} did not apply");
            eval_with_prelude(&format!("{src}let main = room\n")).err().expect("should fail")
        };
        assert_eq!(
            fail(("{ first: 1.0, count: 1.0,", "{ first: 1.0, count: 2.0,")),
            "Model.static: groups: cover 3 triangles, but there are 2"
        );
        assert_eq!(
            fail(("{ first: 1.0, count: 1.0,", "{ first: 0.0, count: 1.0,")),
            "Model.static: groups[1]: first is 0, but the previous group ends at 1 \
(groups run in order, with no gaps)"
        );
        assert_eq!(
            fail(("uvs2: Bytes.fromF32le([0.0, 0.0, 0.5,", "uvs2: Bytes.fromF32le([0.5,")),
            "Model.static: uvs2: 3 vertices, but positions has 4 (or leave uvs2 empty)"
        );
        assert_eq!(
            fail(("uvs: Bytes.fromF32le([0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])", "uvs: none")),
            "Model.static: groups[0]: texture needs uvs (one per vertex)"
        );
        assert_eq!(
            fail(("{ first: 0.0, count: 1.0,", "{ first: 0.5, count: 1.0,")),
            "Model.static: groups[0].first: expected a whole number from 0, got 0.5"
        );
        assert_eq!(
            fail(("triangles: Bytes.fromU32le([0.0, 1.0, 2.0,", "triangles: Bytes.fromU32le([0.0, 1.0, 9.0,")),
            "Model.static: triangles: triangle 0 uses vertex 9, but there are 4"
        );
        // Untextured groups need no uvs; no groups draws everything.
        let src = ROOM
            .replace("texture: Option.Some(px)", "texture: Option.None")
            .replace("uvs: Bytes.fromF32le([0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])", "uvs: none");
        assert!(eval_with_prelude(&format!("{src}let main = room\n")).is_ok());
        assert_eq!(
            fail(("uvs2: Bytes.fromF32le([0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 0.0, 0.5])", "uvs2: none")),
            "Model.static: lightmap: needs uvs2 (one per vertex)"
        );
        assert_eq!(
            fail(("scale: 2.0", "scale: 0.0 - 1.0")),
            "Model.static: lightmap: scale must be a finite number from 0, got -1"
        );
        let ungrouped = "Model.static({ roomData with groups: [] })";
        assert!(eval_with_prelude(&format!("{ROOM}let main = {ungrouped}\n")).is_ok());
    }

    const UNWRAP: &str = "let u = Lightmap.unwrap(roomData, { texelsPerMeter: 16.0, padding: 1.0, \
maxSize: 64.0 })\n";

    /// `Lightmap.unwrap`'s mesh feeds `Model.static`: new `uvs2` in its
    /// chart, groups carried over, the lightmap cleared.
    #[test]
    fn unwrapped_data_builds_a_static_model() {
        let main = |expr: &str| eval_with_prelude(&format!("{ROOM}{UNWRAP}let main = {expr}\n"));
        assert!(matches!(
            main("u.width == 32.0 && u.height == 32.0 && Bytes.length(u.source) == 16.0"),
            Ok(Value::Bool(true))
        ));
        let Value::HostData(model) = main("Model.static(u.mesh)").unwrap() else {
            panic!("expected a Model");
        };
        let s = model.as_any().downcast_ref::<model::FunctorLangModel>().unwrap().0.source().clone();
        // A 1 m quad at 16 texels/m: one 16×16 chart, 1 texel in.
        assert_eq!(s.positions.len(), 4);
        for uv in &s.uvs2 {
            assert!(uv.iter().all(|&x| (1.0 / 32.0..=17.0 / 32.0).contains(&x)), "{uv:?}");
        }
        assert_eq!((s.groups.len(), s.groups[1].unlit), (2, true));
        // The old lightmap was baked for the old uvs2.
        assert!(s.lightmap.is_none());
    }

    #[test]
    fn unwrap_errors_name_the_field() {
        let fail = |replace: (&str, &str)| {
            // Without `room`, which would fail first.
            let room = ROOM.replace("let room = Model.static(roomData)\n", "");
            let src = format!("{room}{UNWRAP}").replace(replace.0, replace.1);
            eval_with_prelude(&format!("{src}let main = u\n")).err().expect("should fail")
        };
        assert_eq!(
            fail(("padding: 1.0", "padding: 0.5")),
            "Lightmap.unwrap: padding must be a whole number from 0, got 0.5"
        );
        assert_eq!(
            fail(("maxSize: 64.0", "maxSize: 48.0")),
            "Lightmap.unwrap: maxSize must be a power of two, got 48"
        );
        assert_eq!(
            fail(("maxSize: 64.0", "maxSize: 16.0")),
            "Lightmap.unwrap: the chart from triangle 0 is 16×16 texels plus padding, over \
maxSize 16; lower texelsPerMeter or padding"
        );
        assert_eq!(
            fail(("Bytes.fromU32le([0.0, 1.0, 2.0,", "Bytes.fromU32le([0.0, 1.0, 9.0,")),
            "Lightmap.unwrap: triangles: triangle 0 uses vertex 9, but there are 4"
        );
    }

    /// `bytes` as a Functor `Bytes.fromU8([…])` literal.
    fn bytes_literal(bytes: &[u8]) -> String {
        let list: Vec<String> = bytes.iter().map(|b| format!("{b}.0")).collect();
        format!("Bytes.fromU8([{}])", list.join(", "))
    }

    /// `Lightmap.readMesh` gives back `functor bake`'s mesh as
    /// `Model.staticData`; groups lose their textures, nothing else changes.
    #[test]
    fn read_mesh_round_trips_a_baked_mesh() {
        let Value::HostData(model) = eval_with_prelude(&format!("{ROOM}let main = room\n")).unwrap() else {
            panic!("expected a Model");
        };
        let mut mesh = model.as_any().downcast_ref::<model::FunctorLangModel>().unwrap().0.source().clone();
        let bin = bytes_literal(&crate::lightmap::write_mesh(&mesh));
        let read = |expr: &str| eval_with_prelude(&format!("let m = Lightmap.readMesh({bin})\nlet main = {expr}\n"));
        let Value::HostData(back) = read("Model.static(m)").unwrap() else {
            panic!("expected a Model");
        };
        mesh.lightmap = None;
        mesh.groups[0].texture = None;
        assert_eq!(back.as_any().downcast_ref::<model::FunctorLangModel>().unwrap().0.source(), &mesh);
        assert!(matches!(read("Option.isNone(m.lightmap) && List.length(m.groups) == 2.0"), Ok(Value::Bool(true))));
        assert_eq!(
            eval_with_prelude("let main = Lightmap.readMesh(Bytes.fromU8([1.0, 2.0]))").err().unwrap(),
            "Lightmap.readMesh: cut short: needs 4 bytes at byte 0, has 2"
        );
    }

    /// `Texture.fromPng` decodes into a pixel texture in the colour space
    /// asked for; bad bytes are an error.
    #[test]
    fn from_png_decodes_in_its_colour_space() {
        use crate::texture::ColorSpace;
        let image = image::RgbaImage::from_raw(2, 1, vec![10, 20, 30, 255, 40, 50, 60, 0]).unwrap();
        let mut png = Vec::new();
        image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let png = bytes_literal(&png);
        for (space, expected) in [("Linear", ColorSpace::Linear), ("Srgb", ColorSpace::Srgb)] {
            let src = format!("let main = Texture.fromPng({png}, Texture.{space})\n");
            let Value::HostData(data) = eval_with_prelude(&src).unwrap() else {
                panic!("expected a texture");
            };
            let TextureDescription::Data(pixels) = &data.as_any().downcast_ref::<FunctorLangTexture>().unwrap().0 else {
                panic!("expected a pixel texture");
            };
            assert_eq!((pixels.width(), pixels.height(), pixels.color_space()), (2, 1, expected));
            assert_eq!(pixels.rgba8(), [10, 20, 30, 255, 40, 50, 60, 0]);
        }
        let err = eval_with_prelude("let main = Texture.fromPng(Bytes.fromU8([1.0]), Texture.Linear)\n")
            .err()
            .unwrap();
        assert!(err.starts_with("Texture.fromPng: "), "{err}");
    }

    const BAKES: &str = "let bakes = [{ name: \"room\", mesh: roomData, lights: \
[Light.ambient(Color.rgb(0.2, 0.2, 0.2)), Light.point(Vec3.make(0.0, 1.0, 1.0), Color.rgb(1.0, 1.0, 1.0), 1.0, 4.0) \
|> Light.castShadows], albedo: [Color.rgb(0.8, 0.8, 0.8), Color.rgb(0.8, 0.2, 0.1)], scale: 2.0 }]\n";

    /// `functor bake`'s jobs: the mesh validated (its `uvs2` and lightmap
    /// dropped, since the bake replaces them), the lights and albedo as they are.
    #[test]
    fn bake_jobs_read_the_bakes_list() {
        let value = eval_with_prelude(&format!("{ROOM}{BAKES}let main = bakes\n")).unwrap();
        let jobs = lightmap::bake_jobs(&value).unwrap();
        assert_eq!(jobs.len(), 1);
        let job = &jobs[0];
        assert_eq!(job.name, "room");
        assert_eq!((job.mesh.positions.len(), job.mesh.groups.len()), (4, 2));
        assert!(job.mesh.uvs2.is_empty() && job.mesh.lightmap.is_none());
        assert!(matches!(job.lights[0], Light::Ambient { .. }));
        assert!(job.lights[1].casts_shadows());
        assert_eq!(job.albedo, [[0.8, 0.8, 0.8], [0.8, 0.2, 0.1]]);
        assert_eq!(job.scale, 2.0);
    }

    #[test]
    fn bake_job_errors_name_the_job_and_field() {
        let fail = |replace: (&str, &str)| {
            let src = format!("{ROOM}{BAKES}").replace(replace.0, replace.1);
            assert_ne!(src, format!("{ROOM}{BAKES}"), "{replace:?} did not apply");
            // `room` would fail first on a broken mesh.
            let src = src.replace("let room = Model.static(roomData)\n", "");
            let value = eval_with_prelude_unchecked(&format!("{src}let main = bakes\n")).unwrap();
            lightmap::bake_jobs(&value).unwrap_err()
        };
        for bad in ["../room", "a:b", ".room", ""] {
            assert_eq!(
                fail(("name: \"room\"", &format!("name: {bad:?}"))),
                format!("bakes[0].name: {bad:?} must be a file name of letters, digits, `-`, `_` and `.` (it names `<name>.png`)")
            );
        }
        assert_eq!(
            fail(("}]\n", "}, { name: \"Room\", mesh: roomData, lights: [] }]\n")),
            "bakes[1].name: \"Room\" is used twice"
        );
        assert_eq!(
            fail(("Bytes.fromU32le([0.0, 1.0, 2.0,", "Bytes.fromU32le([0.0, 1.0, 9.0,")),
            "bakes[0].mesh: triangles: triangle 0 uses vertex 9, but there are 4"
        );
        assert!(
            fail(("lights: [", "lights: [3.0, ")).starts_with("bakes[0].lights[0]"),
            "a non-light names its index"
        );
        assert!(
            fail(("albedo: [", "albedo: [3.0, ")).starts_with("bakes[0].albedo[0]"),
            "a non-colour names its index"
        );
        assert_eq!(
            lightmap::bake_jobs(&Value::Number(1.0)).unwrap_err(),
            "expected a list of Lightmap.bakeJob, got a number"
        );
    }

    const PIXELS: &str = "let px = { width: 2.0, height: 1.0, format: Texture.Rgb8, \
colorSpace: Texture.Linear, pixels: Bytes.fromU8([10.0, 20.0, 30.0, 40.0, 50.0, 60.0]) }\n";

    /// `Texture.fromPixels` builds a `Data` texture that 3D materials take,
    /// typed as `Asset.Texture`.
    #[test]
    fn from_pixels_feeds_texture_materials() {
        use crate::texture::ColorSpace;
        let src = format!(
            "{PIXELS}let tex : Asset.Texture = Texture.fromPixels(px)\n\
let main = Scene.cube() |> Scene.litTexture(tex)\n"
        );
        let value = eval_with_prelude(&src).unwrap();
        let Value::HostData(data) = value else {
            panic!("expected a scene");
        };
        let scene = &data.as_any().downcast_ref::<FunctorLangScene>().unwrap().0;
        let SceneObject::Material(MaterialDescription::Lit { texture, .. }, _) = &scene.obj else {
            panic!("expected a lit material, got {:?}", scene.obj);
        };
        let Some(TextureDescription::Data(pixels)) = texture else {
            panic!("expected a pixel texture, got {texture:?}");
        };
        assert_eq!((pixels.width(), pixels.height()), (2, 1));
        assert_eq!(pixels.color_space(), ColorSpace::Linear);
        assert_eq!(pixels.rgba8(), [10, 20, 30, 255, 40, 50, 60, 255]);
    }

    /// Bad pixels are an error naming the field.
    #[test]
    fn from_pixels_errors_name_the_field() {
        let fail = |replace: (&str, &str)| {
            let src = PIXELS.replace(replace.0, replace.1);
            assert_ne!(src, PIXELS, "{replace:?} did not apply");
            eval_with_prelude(&format!("{src}let main = Texture.fromPixels(px)\n"))
                .err()
                .expect("should fail")
        };
        assert_eq!(
            fail(("Texture.Rgb8", "Texture.Rgba8")),
            "Texture.fromPixels: pixels: 6 bytes, but 2×1 Rgba8 needs 8 (4 per pixel)"
        );
        assert_eq!(
            fail(("width: 2.0", "width: 0.0")),
            "Texture.fromPixels: width must be 1 to 4096 (the largest texture every target \
supports), got 0"
        );
        assert_eq!(
            fail(("height: 1.0", "height: 0.5")),
            "Texture.fromPixels: height must be a whole number, got 0.5"
        );
        assert_eq!(
            fail(("width: 2.0", "width: -1.0")),
            "Texture.fromPixels: width must be 1 to 4096 (the largest texture every target \
supports), got -1"
        );
        assert_eq!(
            fail(("width: 2.0", "width: 5000.0")),
            "Texture.fromPixels: width must be 1 to 4096 (the largest texture every target \
supports), got 5000"
        );
    }

    /// `Texture.atlas` packs pixels records into one `Data` texture; each
    /// rect's UVs map back to its image's texels.
    #[test]
    fn atlas_rects_map_back_to_source_texels() {
        let src = format!(
            "{PIXELS}let px2 = {{ width: 1.0, height: 3.0, format: Texture.Rgba8, \
colorSpace: Texture.Linear, pixels: Bytes.fromU8([1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0]) }}\n\
let main = Texture.atlas({{ images: [{{ pixels: px, layers: [] }}, {{ pixels: px2, layers: [] }}], \
padding: 1.0, maxSize: 64.0 }})\n"
        );
        let Value::Record(out) = eval_with_prelude(&src).unwrap() else {
            panic!("expected a record");
        };
        let get = |name: &str| &out.iter().find(|(f, _)| f == name).unwrap().1;
        let Value::HostData(texture) = get("texture") else {
            panic!("expected a texture");
        };
        let texture = &texture.as_any().downcast_ref::<FunctorLangTexture>().unwrap().0;
        let TextureDescription::Data(atlas) = texture else {
            panic!("expected pixels, got {texture:?}");
        };
        let side = atlas.width();
        let Value::List(rects) = get("rects") else {
            panic!("expected rects");
        };
        let sources: [&[[u8; 4]]; 2] = [
            &[[10, 20, 30, 255], [40, 50, 60, 255]],
            &[[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]],
        ];
        let sizes = [(2, 1), (1, 3)];
        for ((rect, texels), (w, h)) in rects.iter().zip(sources).zip(sizes) {
            let Value::Record(rect) = rect else { panic!("expected a rect") };
            let n = |name: &str| match &rect.iter().find(|(f, _)| f == name).unwrap().1 {
                Value::Number(n) => *n,
                other => panic!("{name}: {}", other.kind_name()),
            };
            // Texel (i, j) sits at u0 + i/(w-1) of the way to u1 (centres).
            for j in 0..h {
                for i in 0..w {
                    let lerp = |a: f64, b: f64, k: u32, n: u32| {
                        if n == 1 { a } else { a + (b - a) * k as f64 / (n - 1) as f64 }
                    };
                    let u = lerp(n("u0"), n("u1"), i, w);
                    let v = lerp(n("v0"), n("v1"), j, h);
                    let (x, y) = ((u * side as f64) as u32, (v * side as f64) as u32);
                    assert_eq!((x, y), (n("x") as u32 + i, n("y") as u32 + j));
                    let at = ((y * side + x) * 4) as usize;
                    assert_eq!(atlas.rgba8()[at..at + 4], texels[(j * w + i) as usize]);
                }
            }
        }
    }

    #[test]
    fn atlas_errors_name_the_image() {
        let atlas = |images: &str| {
            eval_with_prelude(&format!(
                "{PIXELS}let lit = {{ width: 1.0, height: 1.0, format: Texture.Rgb8, \
colorSpace: Texture.Srgb, pixels: Bytes.fromU8([0.0, 0.0, 0.0]) }}\n\
let main = Texture.atlas({{ images: {images}, padding: 1.0, maxSize: 64.0 }})\n"
            ))
            .err()
            .expect("should fail")
        };
        assert_eq!(
            atlas("[{ pixels: px, layers: [] }, { pixels: lit, layers: [] }]"),
            "Texture.atlas: images[1] is Srgb, but images[0] is Linear; an atlas has one \
colour space"
        );
        assert_eq!(
            atlas("[{ pixels: { px with width: 3.0 }, layers: [] }]"),
            "Texture.atlas: images[0]: pixels: 6 bytes, but 3×1 Rgb8 needs 9 (3 per pixel)"
        );
    }

    const LAYERED: &str = "let layer = (id, v) => { id: id, pixels: Bytes.fromU8([v, v, v, v, v, v]) }\n\
let lm = Texture.atlas({ images: [{ pixels: px, layers: [] }, { pixels: px, layers: [layer(3.0, 1.0), layer(1.0, 2.0)] }], \
padding: 1.0, maxSize: 64.0 })\n";

    /// `Texture.atlas` keeps each image's layers where it landed;
    /// `Texture.withLayers` pairs the atlas with intensities, replacing any it
    /// already has.
    #[test]
    fn with_layers_pairs_an_atlas_with_intensities() {
        let src = format!(
            "{PIXELS}{LAYERED}let main = Texture.withLayers(Map.fromList([(3.0, 0.5), (1.0, 1.0)]), \
Texture.withLayers(Map.fromList([(1.0, 0.0)]), lm.texture))\n"
        );
        let Value::HostData(texture) = eval_with_prelude(&src).unwrap() else {
            panic!("expected a texture");
        };
        let texture = &texture.as_any().downcast_ref::<FunctorLangTexture>().unwrap().0;
        let TextureDescription::Layered { base, intensities } = texture else {
            panic!("expected a layered texture, got {texture:?}");
        };
        assert_eq!(intensities, &[(1.0, 1.0), (3.0, 0.5)]);
        let [image] = base.layers() else {
            panic!("one layered image, got {:?}", base.layers());
        };
        let ids: Vec<f64> = image.layers.iter().map(|l| l.id).collect();
        assert_eq!((image.width, image.height, image.padding, ids), (2, 1, 1, vec![3.0, 1.0]));
        assert_eq!(image.layers[1].rgba8, [2, 2, 2, 255, 2, 2, 2, 255]);
    }

    #[test]
    fn layer_errors_name_the_layer() {
        let run = |main: &str| {
            eval_with_prelude(&format!("{PIXELS}{LAYERED}let main = {main}\n"))
                .err()
                .expect("should fail")
        };
        let atlas = |layers: &str| {
            run(&format!(
                "Texture.atlas({{ images: [{{ pixels: px, layers: {layers} }}], padding: 0.0, maxSize: 64.0 }})"
            ))
        };
        assert_eq!(
            atlas("[{ id: 1.0, pixels: Bytes.fromU8([1.0, 2.0, 3.0]) }]"),
            "Texture.atlas: images[0]: layers[0]: pixels: 3 bytes, but 2×1 Rgb8 needs 6 (3 per pixel)"
        );
        assert_eq!(
            atlas("[layer(4.0, 1.0), layer(4.0, 2.0)]"),
            "Texture.atlas: images[0]: layers[1]: id 4 repeats layers[0]"
        );
        assert_eq!(
            run("Texture.withLayers(Map.fromList([(1.0, 0.0 - 1.0)]), lm.texture)"),
            "Texture.withLayers: layer 1: intensity must be a finite number from 0, got -1"
        );
        assert_eq!(
            run("Texture.withLayers(Map.fromList([(1.0, 1.0)]), Asset.texture(\"a.png\"))"),
            "Texture.withLayers: expected a Texture.atlas or Texture.fromPixels texture"
        );
    }

    /// `Scene.withLightmap` overrides only lightmapped static models' texture.
    #[test]
    fn with_lightmap_overrides_lightmapped_static_models() {
        use crate::scene3d::{MeshOverride, MeshSelector};
        let src = format!(
            "{ROOM}let bare = Model.static({{ roomData with lightmap: Option.None }})\n\
let main = Scene.group([Scene.modelOf(room), Scene.modelOf(bare)]) \
|> Scene.withLightmap(Texture.withLayers(Map.fromList([(1.0, 1.0)]), px))\n"
        );
        let Value::HostData(scene) = eval_with_prelude(&src).unwrap() else {
            panic!("expected a scene");
        };
        let scene = &scene.as_any().downcast_ref::<FunctorLangScene>().unwrap().0;
        let SceneObject::Group(items) = &scene.obj else {
            panic!("expected a group, got {:?}", scene.obj);
        };
        let overrides = |i: usize| match &items[i].obj {
            SceneObject::Model(model) => model.overrides.clone(),
            other => panic!("expected a model, got {other:?}"),
        };
        assert!(matches!(
            overrides(0).as_slice(),
            [(MeshSelector::All, MeshOverride::Lightmap(TextureDescription::Layered { .. }))]
        ));
        assert!(overrides(1).is_empty(), "no lightmap, no override");
    }

    /// The persistence spine on the REAL store: `Persistence.save` writes the
    /// project's slot file (atomically, as canonical plain-data JSON) and a
    /// later `Persistence.load` restores exactly the saved value into a fresh
    /// model — the quit-and-relaunch contract, plus the on-disk artifact.
    #[test]
    fn save_writes_the_slot_file_and_load_restores_it() {
        let _guard = crate::storage::SAVE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        crate::storage::set_project_root(dir.path().to_path_buf());
        let (restored, log) = run_save_then_load(&mut RealEffects::new());
        assert_eq!(restored, "{ score: 42, tags: [\"a\"] }");
        let expected = EffectValue::Record(vec![
            ("score".into(), EffectValue::Number(42.0)),
            ("tags".into(), EffectValue::List(vec![EffectValue::Text("a".into())])),
        ]);
        assert_eq!(
            ops(&log),
            vec![
                ("storage.save", None),
                (
                    "storage.load",
                    Some(save_slot_value(Some(expected.clone())))
                ),
            ]
        );
        // The artifact a player's save actually is: one JSON file under the
        // project, decodable back into the same plain data.
        let text = std::fs::read_to_string(dir.path().join(".functor/saves/autosave.json")).unwrap();
        assert_eq!(serde_json::from_str::<EffectValue>(&text).unwrap(), expected);
    }

    /// A slot that was never written — and one whose file is corrupt — both
    /// answer `Option.None` instead of failing the frame.
    #[test]
    fn an_absent_or_corrupt_slot_loads_as_none() {
        let _guard = crate::storage::SAVE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        crate::storage::set_project_root(dir.path().to_path_buf());
        assert_eq!(
            RealEffects::new().load("autosave"),
            save_slot_value(None),
            "an unwritten slot is None"
        );
        std::fs::create_dir_all(dir.path().join(".functor/saves")).unwrap();
        std::fs::write(dir.path().join(".functor/saves/autosave.json"), "{not json").unwrap();
        assert_eq!(
            RealEffects::new().load("autosave"),
            save_slot_value(None),
            "a corrupt slot is None, not a crash"
        );
    }

    /// `FakeEffects` is an in-memory store: it RECORDS what the game saved
    /// (so a test asserts persistence with no disk at all) and answers loads
    /// from it — the same program, no world.
    #[test]
    fn fake_effects_record_saves_and_answer_loads() {
        let mut fake = FakeEffects::new(0.0, vec![0.0]);
        let (restored, log) = run_save_then_load(&mut fake);
        assert_eq!(restored, "{ score: 42, tags: [\"a\"] }");
        assert_eq!(fake.saves.len(), 1);
        assert_eq!(fake.saves[0].0, "autosave");
        assert_eq!(ops(&log)[0], ("storage.save", None));
        // An untouched slot is absent, and a pre-seeded one boots restored.
        assert_eq!(
            FakeEffects::new(0.0, vec![0.0]).load("nothing-here"),
            save_slot_value(None)
        );
        let seeded = EffectValue::Number(7.0);
        assert_eq!(
            FakeEffects::new(0.0, vec![0.0])
                .with_saves(vec![("autosave".into(), seeded.clone())])
                .load("autosave"),
            save_slot_value(Some(seeded))
        );
    }

    /// Replay determinism: a recorded save/load log replays to the SAME model
    /// and the same log, with no store consulted — the replay does not rewrite
    /// the slot it is replaying, and past the log it never reads it either.
    #[test]
    fn a_recorded_save_load_sequence_replays_exactly() {
        let (fake_model, fake_log) = run_save_then_load(&mut FakeEffects::new(0.0, vec![0.0]));
        let _guard = crate::storage::SAVE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Point the real store at an empty directory: if replay touched it,
        // the assertions below (and this directory) would show it.
        let dir = tempfile::tempdir().unwrap();
        crate::storage::set_project_root(dir.path().to_path_buf());
        let mut replay = ReplayEffects::from_log(&fake_log);
        let (replay_model, replay_log) = run_save_then_load(&mut replay);
        assert_eq!(replay.finish(), None);
        assert_eq!(replay_model, fake_model);
        assert_eq!(ops(&replay_log), ops(&fake_log));
        assert!(
            !dir.path().join(".functor/saves").exists(),
            "a replay must not write the store it is replaying"
        );
        RealEffects::new()
            .save("autosave", &EffectValue::Number(1.0))
            .unwrap();
        assert_eq!(
            ReplayEffects::new().load("autosave"),
            save_slot_value(None),
            "past the log, a replay answers a stand-in, not the live store"
        );
    }

    /// The two call-site guards: a slot key is a NAME (no traversal, no
    /// separators, no empty), and a payload must be plain data — both taught
    /// at the construction site, not frames later.
    #[test]
    fn save_rejects_bad_slots_and_non_plain_values() {
        let msg = run_fail("let main = () => Persistence.save(\"../escape\", 1.0)");
        assert!(
            msg.contains("Persistence.save: slot key") && msg.contains("a slot is a NAME, not a path"),
            "expected the slot teaching error, got: {msg}"
        );
        let msg = run_fail("let main = () => Persistence.load(\"\", (o) => o)");
        assert!(
            msg.contains("Persistence.load: slot key must not be empty"),
            "expected the empty-slot teaching error, got: {msg}"
        );
        let msg = run_fail("let main = () => Persistence.save(\"autosave\", (x) => x)");
        assert!(
            msg.contains("Persistence.save: not plain data") && msg.contains("a function"),
            "expected the plain-data teaching error, got: {msg}"
        );
        let msg = run_fail("let main = () => Persistence.save(\"autosave\", Scene.cube())");
        assert!(
            msg.contains("not plain data") && msg.contains("opaque host value"),
            "expected the host-value teaching error, got: {msg}"
        );
        // A NaN would serialize as JSON `null` and fail to READ BACK — after
        // the rename had already replaced the last good save. Refuse it at the
        // call site, as a value and as a map key.
        let msg = run_fail("let main = () => Persistence.save(\"autosave\", 0.0 / 0.0)");
        assert!(
            msg.contains("non-finite"),
            "expected the non-finite teaching error, got: {msg}"
        );
        // The other NaN door — a map KEY — is already shut by the
        // interpreter, so a save can never carry one either.
        let msg = run_fail(
            "let main = () => Persistence.save(\"autosave\", Map.insert(0.0 / 0.0, 1.0, Map.empty()))",
        );
        assert!(
            msg.contains("keys must be finite"),
            "expected the map-key finiteness error, got: {msg}"
        );
        // A reserved Windows device name would write nowhere on that platform.
        let msg = run_fail("let main = () => Persistence.save(\"con\", 1.0)");
        assert!(
            msg.contains("reserved device name"),
            "expected the reserved-name teaching error, got: {msg}"
        );
    }

    /// The store is defence-in-depth, not just call-site validation: the Rust
    /// API refuses a traversing key too, so no future host caller can reach
    /// outside the saves directory.
    #[test]
    fn the_store_itself_refuses_a_traversing_slot() {
        let _guard = crate::storage::SAVE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        crate::storage::set_project_root(dir.path().to_path_buf());
        assert!(crate::storage::write_slot("../escape", "1").is_err());
        assert!(crate::storage::read_slot("../escape").is_err());
        assert!(!dir.path().parent().unwrap().join("escape.json").exists());
    }

    /// A dry-run projection writes to its OWN overlay: the player's store is
    /// untouched, but a save is still visible to a later load inside the
    /// projection (otherwise the projected model would diverge from the live
    /// one for exactly the program this exists to project).
    #[test]
    fn a_dry_run_save_stays_in_the_projection() {
        let _guard = crate::storage::SAVE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        crate::storage::set_project_root(dir.path().to_path_buf());
        let (restored, _) = run_save_then_load(&mut ReplayEffects::new());
        assert_eq!(restored, "{ score: 42, tags: [\"a\"] }");
        assert!(
            !dir.path().join(".functor/saves").exists(),
            "a projection must not write the player's store"
        );
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
        let mut log: Vec<Entry> = Vec::new();
        let _ = drain_effects(
            &session,
            "update",
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
        assert_eq!(
            log.last().and_then(Entry::op).map(|op| op.1),
            Some("net.http")
        );

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
            let shoot = Effect.play(Asset.sound(\"gunshot.wav\"))\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let mut model = session.global("init").unwrap();
        let mut log: Vec<Entry> = Vec::new();

        let fetch = effect_of(&session.global("fetch").unwrap()).unwrap().0.clone();
        let shoot = effect_of(&session.global("shoot").unwrap()).unwrap().0.clone();
        for effect in [fetch, shoot] {
            let _ = drain_effects(
                &session,
                "update",
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
            ops(&log).into_iter().map(|op| op.0).collect::<Vec<_>>(),
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
            let shoot = Effect.play(Asset.sound(\"gunshot.wav\"))\n\
            let blast = Effect.playAt(Asset.sound(\"explosion.wav\"), Vec3.make(5.0, 0.5, -2.0))\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let mut model = session.global("init").unwrap();
        let mut log: Vec<Entry> = Vec::new();

        let shoot = effect_of(&session.global("shoot").unwrap()).unwrap().0.clone();
        let _ = drain_effects(
            &session,
            "update",
            &mut model,
            shoot,
            &mut FakeEffects::new(0.0, vec![]),
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            false,
        );
        assert_eq!(
            log.last().and_then(Entry::op).map(|op| op.1),
            Some("audio.play")
        );

        let blast = effect_of(&session.global("blast").unwrap()).unwrap().0.clone();
        let _ = drain_effects(
            &session,
            "update",
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
            let ping = Effect.playThen(Asset.sound(\"chime.wav\"), Finished)\n\
            let update = (m: Model, msg: Model) => msg\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let mut model = session.global("init").unwrap();
        let mut log: Vec<Entry> = Vec::new();

        let ping = effect_of(&session.global("ping").unwrap()).unwrap().0.clone();
        let _ = drain_effects(
            &session,
            "update",
            &mut model,
            ping,
            &mut FakeEffects::new(0.0, vec![]),
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            false,
        );
        assert_eq!(
            log.last().and_then(Entry::op).map(|op| op.1),
            Some("audio.playThen")
        );

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

    // --- Effect.preload / preloadThen (Track B.5) ---

    /// `Effect.preload` queues a shell command for the right pipeline and
    /// logs; `preloadThen` additionally registers its completion message,
    /// which the settled hook folds through `update` verbatim — the playThen
    /// shape, driven end to end with no shell.
    #[test]
    fn preload_queues_commands_and_completes_through_update() {
        let _guard = crate::audio::OUTBOUND_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = crate::asset::preload::drain_commands(); // clear the shared queue
        clear_preload_completions();
        let src = "\
            type Model = | Cold | Warm\n\
            let init = Cold\n\
            let boss = Asset.model(\"boss.glb\")\n\
            let wood = Asset.texture(\"wood.png\")\n\
            let warm = Effect.batch([Effect.preload(wood), Effect.preloadThen(boss, Warm)])\n\
            let update = (m: Model, msg: Model) => msg\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let mut model = session.global("init").unwrap();
        let mut log: Vec<Entry> = Vec::new();

        let warm = effect_of(&session.global("warm").unwrap()).unwrap().0.clone();
        let _ = drain_effects(
            &session,
            "update",
            &mut model,
            warm,
            &mut FakeEffects::new(0.0, vec![]),
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            false,
        );
        let kinds: Vec<&str> = ops(&log).into_iter().map(|op| op.0).collect();
        assert!(kinds.contains(&"asset.preload"), "{kinds:?}");
        assert!(kinds.contains(&"asset.preloadThen"), "{kinds:?}");

        // Two commands queued: the tokenless texture warm, the tokened model.
        let commands = crate::asset::preload::drain_commands();
        let token = match commands.as_slice() {
            [
                crate::asset::preload::PreloadCommand {
                    kind: crate::asset::preload::PreloadKind::Texture,
                    locator: tex,
                    token: None,
                },
                crate::asset::preload::PreloadCommand {
                    kind: crate::asset::preload::PreloadKind::Model,
                    locator: model_loc,
                    token: Some(t),
                },
            ] => {
                assert_eq!(tex, "wood.png");
                assert_eq!(model_loc, "boss.glb");
                *t
            }
            other => panic!("expected texture+tokened model commands, got: {other:?}"),
        };

        // Whole-shell history snapshots the still-pending completion without
        // exposing the Functor Lang message value to the shell.
        let completion_snapshot = snapshot_preload_completions(&[token]);
        assert_eq!(completion_snapshot.len(), 1);

        // The load settles: take the registered message and fold it through
        // `update` (delivered verbatim — no tagger).
        let message = take_preload_completion(token).expect("a message for the token");
        let (model, _) = split_model_effect(
            session
                .call("update", vec![model, message], &mut FunctorHost)
                .unwrap(),
        );
        assert_eq!(model.to_string(), "Warm");

        // Consumed: a duplicate/late settle finds no message.
        assert!(take_preload_completion(token).is_none());

        // Restoring the shell snapshot re-arms exactly that historical token.
        restore_preload_completions(completion_snapshot);
        assert_eq!(
            take_preload_completion(token)
                .expect("restored message")
                .to_string(),
            "Warm"
        );
        assert!(take_preload_completion(token).is_none());
    }

    /// Under a suppressed-outbound runner (fake/dry-run/replay) preloads log
    /// but queue nothing and register nothing — the deterministic test seam.
    #[test]
    fn preload_suppressed_outbound_only_logs() {
        let _guard = crate::audio::OUTBOUND_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = crate::asset::preload::drain_commands();
        clear_preload_completions();
        let src = "\
            let init = 0.0\n\
            let warm = Effect.preloadThen(Asset.model(\"boss.glb\"), 1.0)\n\
            let update = (m, msg) => msg\n";
        let project = functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()));
        let session = functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message));
        let mut model = session.global("init").unwrap();
        let mut log: Vec<Entry> = Vec::new();
        let warm = effect_of(&session.global("warm").unwrap()).unwrap().0.clone();
        let _ = drain_effects(
            &session,
            "update",
            &mut model,
            warm,
            &mut FakeEffects::new(0.0, vec![]),
            &mut log,
            &mut |m| panic!("unexpected report: {m}"),
            true, // suppress_outbound
        );
        assert_eq!(
            log.last().and_then(Entry::op).map(|op| op.1),
            Some("asset.preloadThen")
        );
        assert!(crate::asset::preload::drain_commands().is_empty());
    }

    /// Preload teaches: sounds have nothing to warm, and bare strings must
    /// construct an Asset value first (no new string coercion post-B.3).
    #[test]
    fn preload_teaching_errors() {
        for (src, want) in [
            (
                "let main = () => Effect.preload(Asset.sound(\"boom.ogg\"))",
                "Effect.preload: sounds have nothing to preload (they decode at play time, \
outside the asset cache) — preload applies to model and texture assets",
            ),
            (
                "let main = () => Effect.preload(\"boss.glb\")",
                "Effect.preload: expected an Asset value, got a bare string (\"boss.glb\") — \
construct one with Asset.model/texture(…) first",
            ),
            (
                "let main = () => Effect.preloadThen(42.0, 1.0)",
                "Effect.preloadThen: expected an Asset value (Asset.model/texture(…)), got \
a number",
            ),
        ] {
            assert_eq!(fail_message(src), want, "for {src}");
        }
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
                AudioSource.ambient(\"wind\", Asset.sound(\"wind-loop.wav\")) |> AudioSource.gain(0.35),\n\
                AudioSource.at(\"fountain\", Asset.sound(\"water.wav\"), Vec3.make(5.0, 0.5, 0.0))\n\
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

    /// An orbs `Intent` record — the held controls a `Steer` carries.
    fn orbs_intent(turn: f64, thrust: bool, claim: bool) -> EffectValue {
        EffectValue::Record(vec![
            ("turn".into(), EffectValue::Number(turn)),
            ("thrust".into(), EffectValue::Bool(thrust)),
            ("claim".into(), EffectValue::Bool(claim)),
        ])
    }

    /// A framed `Steer(intent)` wire text, as an orbs client sends it. Both
    /// roles are inline `module` blocks of the ENTRY file and the protocol is
    /// declared above them at that file's top level, so its ctors carry BARE
    /// canonical tags (there is no sibling module to qualify through).
    fn orbs_steer_frame(turn: f64, thrust: bool, claim: bool) -> String {
        encode_typed_msg(&EffectValue::Variant(
            "Steer".into(),
            vec![orbs_intent(turn, thrust, claim)],
        ))
    }

    /// A framed `Join` — the handshake that asks the authority for a seat.
    fn orbs_join_frame() -> String {
        encode_typed_msg(&EffectValue::Variant("Join".into(), vec![]))
    }

    /// A framed `Welcome(pid)` — the authority's answer to a `Join`.
    fn orbs_welcome_frame(pid: f64) -> String {
        encode_typed_msg(&EffectValue::Variant(
            "Welcome".into(),
            vec![EffectValue::Number(pid)],
        ))
    }

    fn orbs_ship(pid: f64, x: f64, y: f64) -> EffectValue {
        EffectValue::Record(vec![
            ("pid".into(), EffectValue::Number(pid)),
            ("x".into(), EffectValue::Number(x)),
            ("y".into(), EffectValue::Number(y)),
            ("rot".into(), EffectValue::Number(0.0)),
        ])
    }

    fn orbs_orb(id: f64, x: f64, y: f64, owner: f64) -> EffectValue {
        EffectValue::Record(vec![
            ("id".into(), EffectValue::Number(id)),
            ("x".into(), EffectValue::Number(x)),
            ("y".into(), EffectValue::Number(y)),
            ("owner".into(), EffectValue::Number(owner)),
        ])
    }

    /// A framed `Snapshot(ships, orbs)` wire text, as the orbs server
    /// broadcasts it.
    fn orbs_snapshot_frame(ships: &[(f64, f64, f64)], orbs: &[(f64, f64, f64, f64)]) -> String {
        encode_typed_msg(&EffectValue::Variant(
            "Snapshot".into(),
            vec![
                EffectValue::List(
                    ships.iter().map(|(pid, x, y)| orbs_ship(*pid, *x, *y)).collect(),
                ),
                EffectValue::List(
                    orbs.iter()
                        .map(|(id, x, y, owner)| orbs_orb(*id, *x, *y, *owner))
                        .collect(),
                ),
            ],
        ))
    }

    /// Decode a payload back to its `Wire` value — fails loud on anything but
    /// a typed frame.
    fn orbs_decode(payload: &[u8]) -> EffectValue {
        let text = std::str::from_utf8(payload).expect("utf8 payload");
        decode_typed_msg(text).expect("a typed frame").expect("frame decodes")
    }

    /// A record's numeric fields, in the order asked for — fails loud on a
    /// missing or non-numeric field (a renamed field is a test failure, not a
    /// silently skipped assertion).
    fn orbs_fields(value: &EffectValue, names: &[&str]) -> Vec<f64> {
        match value {
            EffectValue::Record(fields) => names
                .iter()
                .map(|name| match fields.iter().find(|(k, _)| k == name) {
                    Some((_, EffectValue::Number(v))) => *v,
                    other => panic!("bad field {name}: {other:?}"),
                })
                .collect(),
            other => panic!("not a record: {other:?}"),
        }
    }

    /// A broadcast `Snapshot`, decoded WHOLE: its `(pid, x, y)` ships (lowest
    /// pid first — the world's pilot list is prepend-ordered, which is not a
    /// contract) and its `(id, x, y, owner)` orbs in wire order.
    #[allow(clippy::type_complexity)]
    fn orbs_snapshot(payload: &[u8]) -> (Vec<(f64, f64, f64)>, Vec<(f64, f64, f64, f64)>) {
        match orbs_decode(payload) {
            EffectValue::Variant(ctor, args) if ctor == "Snapshot" => match &args[..] {
                [EffectValue::List(ships), EffectValue::List(orbs)] => {
                    let mut ships: Vec<(f64, f64, f64)> = ships
                        .iter()
                        .map(|ship| {
                            let f = orbs_fields(ship, &["pid", "x", "y"]);
                            (f[0], f[1], f[2])
                        })
                        .collect();
                    ships.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let orbs = orbs
                        .iter()
                        .map(|orb| {
                            let f = orbs_fields(orb, &["id", "x", "y", "owner"]);
                            (f[0], f[1], f[2], f[3])
                        })
                        .collect();
                    (ships, orbs)
                }
                other => panic!("Snapshot args: {other:?}"),
            },
            other => panic!("not a Snapshot: {other:?}"),
        }
    }

    /// The SHIPPED `examples/orbs/game.fun`, loaded like the runner does:
    /// `project::load` walks the entry's siblings (file = module) and injects
    /// the built-in Net module. Orbs is ONE file — both roles are inline
    /// modules of it — so each role's contract is looked up under `Client.*` /
    /// `Server.*` (what functor.json's `{ "file": …, "module": … }` resolves).
    fn orbs_session() -> functor_lang::Session {
        let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/orbs/game.fun");
        let project = functor_lang::project::load(&entry)
            .unwrap_or_else(|e| panic!("load orbs: {}", e.render()));
        functor_lang::Session::load(&project.module, &mut FunctorHost)
            .unwrap_or_else(|f| panic!("session: {}", f.error.message))
    }

    fn orbs_call(session: &functor_lang::Session, name: &str, args: Vec<Value>) -> Value {
        let f = session.global(name).unwrap_or_else(|| panic!("no `{name}`"));
        session
            .apply(f, args, name, &mut FunctorHost)
            .unwrap_or_else(|e| panic!("{name}: {}", e.message))
    }

    /// Perform whatever an entry point returned, draining its effects into the
    /// settled model plus (conn, payload) Send pairs.
    fn orbs_drain_sends(
        session: &functor_lang::Session,
        label: &'static str,
        returned: Value,
    ) -> (Value, Vec<(u64, Vec<u8>)>) {
        crate::net::drain_conn_commands(); // clear
        let (mut m, fx) = split_model_effect(returned);
        if let Some(tree) = fx {
            let mut log: Vec<Entry> = Vec::new();
            let _ = drain_effects(
                session,
                label,
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

    /// Headless server-lifecycle test for the `examples/orbs` SERVER role,
    /// with no socket. Loads the SHIPPED `game.fun` so this tracks the
    /// example, and exercises the whole server spine — `toMsg` decoding typed
    /// `Net.Data` packets, the `Join` handshake seating one pilot per
    /// CONNECTION (and answering its `Welcome`), a `Steer` folded in against
    /// the connection it arrived on, the world step, and the
    /// broadcast-the-whole-world-to-every-seat `Effect.sendMsg` (the naive
    /// server's defining behavior), plus a plain-text packet ignored (the
    /// protocol is typed) and a disconnect releasing a seat.
    #[test]
    fn orbs_server_broadcasts_the_world_to_every_client() {
        let _guard = CONN_QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let session = orbs_session();

        fn feed(
            session: &functor_lang::Session,
            model: Value,
            ev: Value,
        ) -> (Value, Vec<(u64, Vec<u8>)>) {
            let msg = orbs_call(session, "toMsg", vec![ev]);
            orbs_drain_sends(
                session,
                "Server.update",
                orbs_call(session, "Server.update", vec![model, msg]),
            )
        }
        // One tick's broadcast.
        fn broadcast(
            session: &functor_lang::Session,
            model: Value,
        ) -> (Value, Vec<(u64, Vec<u8>)>) {
            orbs_drain_sends(
                session,
                "Server.update",
                orbs_call(
                    session,
                    "Server.tick",
                    vec![model, Value::Number(0.5), Value::Number(0.5)],
                ),
            )
        }
        let event = |kind, id: u64, text: &str| {
            net_event_value(kind, id, text).to_functor_lang().unwrap()
        };

        // The listener is declared on the arena address.
        let init = session.global("Server.init").expect("no `Server.init`");
        let subs = orbs_call(&session, "Server.subscriptions", vec![init.clone()]);
        let conns = net_conn_subs(&subs).expect("a Sub tree");
        assert!(conns.len() == 1 && conns[0].listen && conns[0].key == "127.0.0.1:9101");

        // A socket alone is not a pilot: the connection opens, and only the
        // `Join` that follows it is answered with a seat and a `Welcome`.
        let (model, sends) = feed(&session, init, event(NetEventKind::Connected, 1, ""));
        assert!(sends.is_empty(), "a bare connect answers nothing: {sends:?}");
        let (model, sends) = feed(
            &session,
            model,
            event(NetEventKind::Message, 1, &orbs_join_frame()),
        );
        assert_eq!(
            sends,
            vec![(1, orbs_welcome_frame(0.0).into_bytes())],
            "the first Join is welcomed as pid 0, to the connection it came from"
        );
        // A re-Join over the SAME connection is answered from the seat it
        // already has — one connection is one pilot, however many times it
        // asks.
        let (model, sends) = feed(
            &session,
            model,
            event(NetEventKind::Message, 1, &orbs_join_frame()),
        );
        assert_eq!(
            sends,
            vec![(1, orbs_welcome_frame(0.0).into_bytes())],
            "a re-Join is answered from the seat it already holds, not a fresh pid"
        );

        // A second connection joins as pid 1.
        let (model, _) = feed(&session, model, event(NetEventKind::Connected, 2, ""));
        let (model, sends) = feed(
            &session,
            model,
            event(NetEventKind::Message, 2, &orbs_join_frame()),
        );
        assert_eq!(sends, vec![(2, orbs_welcome_frame(1.0).into_bytes())]);

        // Both connections steer — cid 1 thrusts, cid 2 turns — so the fold is
        // exercised for more than one sender, and a plain-TEXT packet from
        // cid 1 is IGNORED (the protocol is the typed Wire ADT; its intent is
        // not reset).
        let (model, _) = feed(
            &session,
            model,
            event(NetEventKind::Message, 1, &orbs_steer_frame(0.0, true, false)),
        );
        let (model, _) = feed(
            &session,
            model,
            event(NetEventKind::Message, 2, &orbs_steer_frame(1.0, false, false)),
        );
        let (model, _) = feed(&session, model, event(NetEventKind::Message, 1, "junk"));

        // Tick (dt = 0.5). rot 0 is straight up, so pid 0 thrusts to
        // y = 9 * 0.5 = 4.5 on its spawn lane x = -11; pid 1 only turns, so it
        // holds its spawn (x = -11 + 7.3, y = 0) — proof the text packet did
        // not reset pid 0's intent, and that each Steer moved only the
        // connection it arrived on. Full float precision — the typed wire has
        // no fixed-point rounding.
        let (model, sends) = broadcast(&session, model);
        let ships = [(0.0, -11.0, 4.5), (1.0, 0.0 - 11.0 + 7.3, 0.0)];
        // Nobody claimed anything, so the whole opening board rides along —
        // a snapshot that dropped or mutated the orbs would be a live bug.
        let orbs = [
            (0.0, 0.0, 0.0, -1.0),
            (1.0, -10.0, 4.0, -1.0),
            (2.0, 10.0, 4.0, -1.0),
            (3.0, -6.0, -5.0, -1.0),
            (4.0, 6.0, -5.0, -1.0),
        ];
        // The WHOLE world goes to EVERY seat — two identical full snapshots.
        assert_eq!(sends.len(), 2, "one Send per seat, got: {sends:?}");
        let mut recipients: Vec<u64> = sends.iter().map(|(c, _)| *c).collect();
        recipients.sort();
        assert_eq!(recipients, vec![1, 2], "the broadcast reaches both clients");
        for (_, payload) in &sends {
            let (got_ships, got_orbs) = orbs_snapshot(payload);
            assert_eq!(got_ships.len(), ships.len(), "ships: {got_ships:?}");
            for (got, want) in got_ships.iter().zip(&ships) {
                assert_close(*got, *want);
            }
            assert_eq!(got_orbs, orbs, "every client receives the whole board");
        }

        // Client 1 disconnects -> its seat is released. The next tick
        // broadcasts only pid 1, and only to the client still connected.
        let (model, _) = feed(&session, model, event(NetEventKind::Disconnected, 1, ""));
        let (_, sends) = broadcast(&session, model);
        assert_eq!(sends.len(), 1, "only the remaining client is served: {sends:?}");
        assert_eq!(sends[0].0, 2);
        let (ships, _) = orbs_snapshot(&sends[0].1);
        assert!(
            ships.len() == 1 && ships[0].0 == 1.0,
            "the snapshot no longer contains pid 0, got: {ships:?}"
        );
    }

    /// Headless client-lifecycle test for the `examples/orbs` CLIENT role,
    /// with no socket. Loads the SHIPPED `game.fun` and drives connect (the
    /// `Join` handshake sent), the `Welcome` that hands over an identity, a
    /// typed `Snapshot` landing in the board, `sampledInput` held levels
    /// becoming the per-tick `Steer` (including the release back to neutral),
    /// the in-range `Claim` proposal riding along with it, and a disconnect.
    /// The snapshot fed in is built by the same helper the server entry's
    /// broadcast is decoded with, so this doubles as a wire round-trip check
    /// between the two blocks.
    #[test]
    fn orbs_client_decodes_snapshots_and_sends_input() {
        let _guard = CONN_QUEUE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let session = orbs_session();
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
        let sends_of =
            |session: &functor_lang::Session, returned: Value| -> Vec<(u64, Vec<u8>)> {
                orbs_drain_sends(session, "Client.update", returned).1
            };
        let event = |kind, id: u64, text: &str| {
            net_event_value(kind, id, text).to_functor_lang().unwrap()
        };
        // Keys are the built-in `Key` module's variants (`Key.W`), as the
        // producers deliver them; `sampledInput` reads the held LEVELS off the
        // snapshot it is handed.
        let key = |k: &str| Value::Variant {
            ctor: std::rc::Rc::from(format!("Key.{k}").as_str()),
            args: std::rc::Rc::new(Vec::new()),
        };
        let held = |keys: Vec<Value>| {
            Value::Record(std::rc::Rc::new(vec![(
                "heldKeys".to_string(),
                Value::List(std::rc::Rc::new(keys)),
            )]))
        };
        let tick = |session: &functor_lang::Session, model: Value| {
            orbs_call(
                session,
                "Client.tick",
                vec![model, Value::Number(0.016), Value::Number(0.016)],
            )
        };

        // subscriptions declares an OUTBOUND connection (not a listener).
        let init = session.global("Client.init").expect("no `Client.init`");
        let subs = orbs_call(&session, "Client.subscriptions", vec![init.clone()]);
        let conns = net_conn_subs(&subs).expect("a Sub tree");
        assert!(!conns[0].listen && conns[0].key == "ws://127.0.0.1:9101/orbs");

        // Nothing is sent before the socket is up.
        assert!(
            sends_of(&session, tick(&session, init.clone())).is_empty(),
            "a tick before connect sends nothing"
        );

        // The socket opens (id 5): store it and open the handshake with a Join.
        let opened = orbs_call(&session, "toMsg", vec![event(NetEventKind::Connected, 5, "")]);
        let joined = orbs_call(&session, "Client.update", vec![init, opened]);
        let (model, _) = split_model_effect(joined.clone());
        assert_eq!(field(&model, "conn").to_string(), "Option.Some(5)");
        assert_eq!(num(field(&model, "myPid")), -1.0, "no identity until the Welcome");
        assert_eq!(sends_of(&session, joined), vec![(5, orbs_join_frame().into_bytes())]);
        // …and until the server answers, a tick still sends nothing: the
        // client steers no one before it is seated.
        assert!(
            sends_of(&session, tick(&session, model.clone())).is_empty(),
            "a tick before the Welcome sends nothing"
        );

        // The Welcome hands over the identity.
        let welcome = orbs_call(
            &session,
            "toMsg",
            vec![event(NetEventKind::Message, 5, &orbs_welcome_frame(0.0))],
        );
        let (model, _) =
            split_model_effect(orbs_call(&session, "Client.update", vec![model, welcome]));
        assert_eq!(num(field(&model, "myPid")), 0.0);

        // A server snapshot lands in the board, binding each pid to its own
        // coordinates. Our ship sits just off orb 0 (at the origin), inside
        // the claim range.
        let msg = orbs_call(
            &session,
            "toMsg",
            vec![event(
                NetEventKind::Message,
                5,
                &orbs_snapshot_frame(
                    &[(0.0, 0.5, 0.0), (1.0, -3.7, 0.0)],
                    &[(0.0, 0.0, 0.0, -1.0), (1.0, -10.0, 4.0, -1.0)],
                ),
            )],
        );
        let (model, _) = split_model_effect(orbs_call(&session, "Client.update", vec![model, msg]));
        let ships = match field(&model, "ships") {
            Value::List(items) => items.clone(),
            other => panic!("ships is not a list: {other}"),
        };
        assert_eq!(ships.len(), 2, "two ships decoded, got: {}", field(&model, "ships"));
        let at = |pid: f64| -> (f64, f64) {
            let s = ships
                .iter()
                .find(|s| num(field(s, "pid")) == pid)
                .unwrap_or_else(|| panic!("no ship pid {pid}"));
            (num(field(s, "x")), num(field(s, "y")))
        };
        // Per-pid coordinates, full float precision (a swapped field order
        // would fail here).
        assert_eq!(at(0.0), (0.5, 0.0));
        assert_eq!(at(1.0), (-3.7, 0.0));

        // Held LEVELS become the per-tick Steer: `w` is thrust, `a` turns, an
        // unrelated key is neither, and releasing everything steers back to
        // neutral (a latched control would show up here).
        let steer = |model: &Value, keys: Vec<Value>| -> Vec<(u64, Vec<u8>)> {
            let (sampled, _) = split_model_effect(orbs_call(
                &session,
                "Client.sampledInput",
                vec![model.clone(), held(keys)],
            ));
            sends_of(&session, tick(&session, sampled))
        };
        assert_eq!(
            steer(&model, vec![key("W")]),
            vec![(5, orbs_steer_frame(0.0, true, false).into_bytes())],
            "a seated client streams its held intent"
        );
        assert_eq!(
            steer(&model, vec![key("A")]),
            vec![(5, orbs_steer_frame(1.0, false, false).into_bytes())]
        );
        let coast = vec![(5, orbs_steer_frame(0.0, false, false).into_bytes())];
        assert_eq!(steer(&model, vec![key("X")]), coast, "an unrelated key steers nothing");
        assert_eq!(steer(&model, vec![]), coast, "releasing everything coasts");

        // Holding SPACE over a free orb rides a Claim along with the Steer —
        // a PROPOSAL the server re-checks, not a fact.
        let claim_sends = steer(&model, vec![key("Space")]);
        assert_eq!(
            claim_sends
                .iter()
                .map(|(conn, payload)| (*conn, orbs_decode(payload)))
                .collect::<Vec<_>>(),
            vec![
                (5, EffectValue::Variant("Steer".into(), vec![orbs_intent(0.0, false, true)])),
                (5, EffectValue::Variant("Claim".into(), vec![EffectValue::Number(0.0)])),
            ],
            "the claim names the orb the last snapshot put us over"
        );

        // Disconnect drops the connection and clears the board.
        let dropped = orbs_call(&session, "toMsg", vec![event(NetEventKind::Disconnected, 5, "")]);
        let (model, _) =
            split_model_effect(orbs_call(&session, "Client.update", vec![model, dropped]));
        assert_eq!(field(&model, "conn").to_string(), "Option.None");
        assert_eq!(field(&model, "ships").to_string(), "[]");
        assert_eq!(field(&model, "orbs").to_string(), "[]");
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
