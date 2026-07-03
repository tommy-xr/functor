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
//! Scene.rotateX/rotateY/rotateZ(scene, radians)             -> Scene
//! Scene.scale(scene, k)                                     -> Scene
//! Camera.lookAt(ex, ey, ez, tx, ty, tz)                     -> Camera
//!   (up is +Y; vertical fov pinned at 45°, near/far at protocol defaults)
//! Frame.create(camera, scene)                               -> Frame
//! ```
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
    "Camera.lookAt",
    "Camera.firstPerson",
    "Light.ambient",
    "Light.directional",
    "Light.point",
    "Light.castShadows",
    "Frame.create",
    "Frame.createLit",
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
            "Scene.rotateX" | "Scene.rotateY" | "Scene.rotateZ" => match args.as_slice() {
                [scene, radians] => {
                    let angle: cgmath::Rad<f32> =
                        Angle::from_radians(num(radians, span)? as f32).into();
                    let xform = match path {
                        "Scene.rotateX" => Matrix4::from_angle_x(angle),
                        "Scene.rotateY" => Matrix4::from_angle_y(angle),
                        _ => Matrix4::from_angle_z(angle),
                    };
                    wrap_transform(scene, xform, &format!("{path}(scene, radians)"), span)
                }
                _ => return usage(&format!("{path}(scene, radians)")),
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
                [ex, ey, ez, yaw, pitch, fov_deg] => Ok(host(MleCamera(Camera::first_person(
                    [
                        num(ex, span)? as f32,
                        num(ey, span)? as f32,
                        num(ez, span)? as f32,
                    ],
                    Angle::from_radians(num(yaw, span)? as f32),
                    Angle::from_radians(num(pitch, span)? as f32),
                    Angle::from_degrees(num(fov_deg, span)? as f32),
                )))),
                _ => usage("Camera.firstPerson(ex, ey, ez, yawRad, pitchRad, fovDeg)"),
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
               Scene.translate(Scene.rotateY(Scene.cube(), 1.5707964), 3.0, 0.0, 0.0))",
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
               Camera.firstPerson(0.0, 3.5, -8.0, 0.0, -0.3, 60.0),
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
