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
//!
//! Physics.box(w, h, d) / sphere(r) / capsule(hh, r)         -> Shape
//! Physics.dynamic/kinematic/fixed(tag, shape)               -> Body
//! Physics.at/velocity(body, x, y, z)                        -> Body
//! Physics.mass/friction/restitution(body, n)                -> Body
//! Physics.sensor(body)                                      -> Body
//! Physics.scene(gx, gy, gz, [body, …])                      -> PhysicsScene
//! Physics.position(tag)                                     -> {x, y, z}
//! Physics.transformed(scene, tag)                           -> Scene
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
use crate::scene3d::MaterialDescription;
use crate::{Camera, Frame, Light, Scene3D, SceneObject};

/// A [`Scene3D`] as an opaque MLE value.
pub struct MleScene(pub Scene3D);

/// A [`Camera`] as an opaque MLE value.
pub struct MleCamera(pub Camera);

/// A [`Frame`] as an opaque MLE value — what an MLE `draw` returns.
pub struct MleFrame(pub Frame);

/// A [`Light`] as an opaque MLE value.
pub struct MleLight(pub Light);

/// An [`Angle`] as an opaque MLE value — `Angle.degrees(…)`/`Angle.radians(…)`.
/// Rotation/camera functions accept ONLY this, never a bare number, making
/// degree/radian confusion unrepresentable (the F# side's `Math.Angle`
/// discipline, carried across the boundary).
pub struct MleAngle(pub Angle);

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
            "Physics.transformed" => match args.as_slice() {
                [scene, Value::String(tag)] => {
                    let Some(inner) = scene_of(scene) else {
                        return usage("Physics.transformed(scene, tag)");
                    };
                    match live_transform(tag) {
                        Some((pos, rot)) => {
                            // cgmath's Quaternion::new is scalar-FIRST (w, x, y, z).
                            let rotation = cgmath::Quaternion::new(rot[3], rot[0], rot[1], rot[2]);
                            let xform = Matrix4::from_translation(cgmath::vec3(
                                pos[0], pos[1], pos[2],
                            )) * Matrix4::from(rotation);
                            scene_value(group(vec![inner.clone()], xform))
                        }
                        None => err(no_body(tag)),
                    }
                }
                _ => usage("Physics.transformed(scene, tag)"),
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
        let scene = physics_scene_value(&declare).expect("a PhysicsScene").clone();
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
        let drawn = eval(
            "let main = () => Scene.sphere() |> Physics.transformed(\"ball\")",
        );
        let scene3d = scene_of(&drawn).expect("a Scene");
        assert!((scene3d.xform.w.y as f64 - y).abs() < 1e-6);

        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
    }

    // Degenerate physical dimensions are boundary errors — Rapier would
    // silently build a broken collider, and MLE can't branch to notice.
    #[test]
    fn non_positive_dimensions_are_rejected() {
        for (src, needle) in [
            ("Physics.sphere(-0.5)", "Physics.sphere radius must be positive"),
            ("Physics.box(1.0, 0.0, 1.0)", "Physics.box height must be positive"),
            (
                "Physics.dynamic(\"x\", Physics.sphere(0.5)) |> Physics.mass(0.0)",
                "Physics.mass must be positive",
            ),
            (
                "Physics.dynamic(\"x\", Physics.sphere(0.5)) |> Physics.friction(-1.0)",
                "Physics.friction must not be negative",
            ),
        ] {
            let module = mle::lower(
                mle::parse(&format!("let main = () => {src}")).unwrap(),
            )
            .unwrap();
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

    // A missing tag is a loud spanned error, not a sentinel value.
    #[test]
    fn physics_read_of_unknown_tag_is_a_spanned_error() {
        crate::physics::remove_world(crate::physics::DEFAULT_WORLD);
        let module = mle::lower(
            mle::parse("let main = () => Physics.position(\"ghost\")").unwrap(),
        )
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
}
