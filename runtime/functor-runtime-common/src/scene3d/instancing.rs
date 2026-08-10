//! The GL-free half of `Scene.instanced`: per-instance data, the stamp
//! expansion that DEFINES the node's semantics, and the template recognizer
//! that decides whether the renderer may take the hardware-instanced path.
//!
//! The contract: `scene |> Scene.instanced(xs)` is semantically equivalent to
//! stamping — a group of per-instance transformed copies of the template, with
//! each copy's tint multiplied into the template's material colors. Hardware
//! instancing is an OPTIMIZATION of that semantics, applied when
//! [`recognize`] accepts the template shape; everything else renders through
//! [`expand_instanced`], which is the semantics itself.

use cgmath::{Matrix4, Quaternion};
use serde::{Deserialize, Serialize};

use super::{MaterialDescription, Scene3D, SceneObject, Shape};

/// One copy's channels for [`SceneObject::Instanced`] — plain data, `repr(C)`
/// so the decoded slice uploads directly as the GPU instance buffer
/// (13 floats, 52 bytes per copy).
///
/// The channels apply in a FIXED order — the instance transform is always
/// `translate(position) * rotate(rotation) * scale(scale)` — so the builder
/// combinators compose within their own channel (rotations multiply, scales
/// multiply, tints multiply) rather than composing arbitrary matrices. That
/// keeps the wire shape a channel record with room for the later animate
/// (clip + playhead) and UV-offset channels.
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceData {
    /// The copy's center, in the instanced node's space.
    pub position: [f32; 3],
    /// Unit quaternion `[x, y, z, w]`; identity is `[0, 0, 0, 1]`.
    pub rotation: [f32; 4],
    /// Per-axis scale applied before the rotation.
    pub scale: [f32; 3],
    /// Multiplier over the template material's RGB; white is untinted.
    pub tint: [f32; 3],
}

impl Default for InstanceData {
    fn default() -> Self {
        InstanceData {
            position: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0],
        }
    }
}

impl InstanceData {
    /// `Instance.at` — a fresh instance at `position` with identity rotation,
    /// unit scale, and white tint.
    pub fn at(position: [f32; 3]) -> Self {
        InstanceData {
            position,
            ..InstanceData::default()
        }
    }

    /// `Instance.scale` — multiply every axis by `factor`.
    pub fn scaled(mut self, factor: f32) -> Self {
        for axis in &mut self.scale {
            *axis *= factor;
        }
        self
    }

    /// `Instance.scaleXYZ` — multiply the per-axis scales componentwise.
    pub fn scaled_xyz(mut self, x: f32, y: f32, z: f32) -> Self {
        self.scale[0] *= x;
        self.scale[1] *= y;
        self.scale[2] *= z;
        self
    }

    /// `Instance.rotateX/Y/Z` — compose `by` AFTER the rotation so far (the
    /// outer pipe applies last, like `Scene.rotate*`).
    ///
    /// The result is re-normalized and sign-canonicalized (`w >= 0`) so
    /// repeated composition neither drifts off unit length nor flips a wire
    /// representation between `q` and `-q` (the same rotation) — instances are
    /// pinned on the v13 `/scene` wire and compared by `Scene.equals`.
    pub fn rotated(mut self, by: Quaternion<f32>) -> Self {
        use cgmath::InnerSpace;
        let current = Quaternion::new(
            self.rotation[3],
            self.rotation[0],
            self.rotation[1],
            self.rotation[2],
        );
        let mut next = (by * current).normalize();
        if next.s < 0.0 {
            next = -next;
        }
        self.rotation = [next.v.x, next.v.y, next.v.z, next.s];
        self
    }

    /// `Instance.tint` — multiply the tint componentwise (tints compose).
    pub fn tinted(mut self, rgb: [f32; 3]) -> Self {
        for (channel, factor) in self.tint.iter_mut().zip(rgb) {
            *channel *= factor;
        }
        self
    }

    /// The copy's transform: `translate * rotate * scale`.
    pub fn matrix(&self) -> Matrix4<f32> {
        let rotation = Quaternion::new(
            self.rotation[3],
            self.rotation[0],
            self.rotation[1],
            self.rotation[2],
        );
        Matrix4::from_translation(cgmath::vec3(
            self.position[0],
            self.position[1],
            self.position[2],
        )) * Matrix4::from(rotation)
            * Matrix4::from_nonuniform_scale(self.scale[0], self.scale[1], self.scale[2])
    }
}

/// The stamp expansion — the DEFINITION of `Scene.instanced`'s semantics, and
/// the renderer's fallback for templates [`recognize`] does not accept.
///
/// Each copy is the template wrapped in a `Group` carrying the instance
/// transform (transforms never sit on `Material` nodes, which the renderer
/// ignores), with the copy's tint multiplied into every material color in the
/// stamped subtree.
pub fn expand_instanced(template: &Scene3D, instances: &[InstanceData]) -> Scene3D {
    Scene3D {
        obj: SceneObject::Group(
            instances
                .iter()
                .map(|instance| Scene3D {
                    obj: SceneObject::Group(vec![tint_scene(template.clone(), instance.tint)]),
                    xform: instance.matrix(),
                })
                .collect(),
        ),
        xform: Matrix4::from_scale(1.0),
    }
}

/// Multiply `tint` into every material color of a subtree. White is the
/// identity (returns the scene untouched). A `Texture` material has no color
/// channel and is unchanged; nested `Instanced` nodes fold the tint into
/// their own instances' tint channel.
fn tint_scene(scene: Scene3D, tint: [f32; 3]) -> Scene3D {
    if tint == [1.0, 1.0, 1.0] {
        return scene;
    }
    let obj = match scene.obj {
        SceneObject::Material(material, items) => SceneObject::Material(
            tint_material(material, tint),
            items
                .into_iter()
                .map(|item| tint_scene(item, tint))
                .collect(),
        ),
        SceneObject::Group(items) => SceneObject::Group(
            items
                .into_iter()
                .map(|item| tint_scene(item, tint))
                .collect(),
        ),
        SceneObject::Opacity(alpha, items) => SceneObject::Opacity(
            alpha,
            items
                .into_iter()
                .map(|item| tint_scene(item, tint))
                .collect(),
        ),
        SceneObject::Instanced {
            template,
            mut instances,
        } => {
            for instance in &mut instances {
                instance.tint[0] *= tint[0];
                instance.tint[1] *= tint[1];
                instance.tint[2] *= tint[2];
            }
            SceneObject::Instanced {
                template,
                instances,
            }
        }
        leaf @ (SceneObject::Geometry(_) | SceneObject::Model(_) | SceneObject::Terrain(_)) => leaf,
    };
    Scene3D { obj, ..scene }
}

fn tint_material(material: MaterialDescription, tint: [f32; 3]) -> MaterialDescription {
    let scale = |mut color: cgmath::Vector4<f32>| {
        color.x *= tint[0];
        color.y *= tint[1];
        color.z *= tint[2];
        color
    };
    match material {
        MaterialDescription::Color(color) => MaterialDescription::Color(scale(color)),
        MaterialDescription::Emissive { color, texture } => MaterialDescription::Emissive {
            color: scale(color),
            texture,
        },
        MaterialDescription::Lit {
            color,
            texture,
            normal_map,
        } => MaterialDescription::Lit {
            color: scale(color),
            texture,
            normal_map,
        },
        MaterialDescription::SpriteTexture {
            color,
            texture,
            source_pixels,
            sampling,
        } => MaterialDescription::SpriteTexture {
            color: scale(color),
            texture,
            source_pixels,
            sampling,
        },
        // No color channel to multiply.
        texture @ MaterialDescription::Texture(_) => texture,
    }
}

/// The primitive shapes the hardware path has singleton meshes for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum InstancedPrimitive {
    Cube,
    Sphere,
    Cylinder,
    Quad,
    Plane,
}

/// A template the renderer can draw with instanced calls.
pub(crate) enum RecognizedTemplate<'a> {
    /// A single primitive leaf, optionally under `Group` transform wrappers
    /// (folded into `local`) and at most one solid-color
    /// `Color`/`Emissive`/`Lit` material node.
    Primitive(RecognizedPrimitive<'a>),
    /// A `Scene.model` leaf (no overrides; an attached `Scene.animate` pose
    /// is allowed — every copy shows the same sampled pose), optionally under
    /// `Group` transform wrappers. Whether the LOADED model is rigid or
    /// skinned is only known at render time; each gets its own instanced
    /// draw path there.
    Model(RecognizedModel<'a>),
}

pub(crate) struct RecognizedPrimitive<'a> {
    pub primitive: InstancedPrimitive,
    /// The material enclosing the leaf. `None` never reaches the hardware
    /// path — a bare-primitive template CPU-expands so it keeps rendering
    /// exactly like its stamped equivalent (the pass's ambient material).
    pub material: &'a MaterialDescription,
    /// The template-internal transform accumulated between the instanced node
    /// and the leaf (Group wrappers plus the leaf's own xform).
    pub local: Matrix4<f32>,
}

pub(crate) struct RecognizedModel<'a> {
    pub description: &'a super::ModelDescription,
    /// Group wrappers plus the leaf's own xform; each mesh primitive further
    /// composes its glTF node transform on the right.
    pub local: Matrix4<f32>,
}

/// Decide whether `template` is drawable by the instanced hardware path.
///
/// Recognized: `Group` chains with exactly one child (their transforms
/// compose into `local`) ending in either a
/// `Cube`/`Sphere`/`Cylinder`/`Quad`/`Plane` leaf under at most ONE material
/// node — a solid `Color`, `Emissive` (no texture), or `Lit` (no texture, no
/// normal map) — or a `Scene.model` leaf with no overrides, with or without
/// an attached `Scene.animate` pose (every copy shows the same sampled pose;
/// enclosing materials are ignored by the ordinary model draw, so a
/// material-wrapped model falls back rather than pretending the wrapper
/// does something). Everything else (terrain, multi-child groups, textured
/// materials, bare primitive leaves, nested instancing, opacity) answers
/// `None` and renders through [`expand_instanced`].
pub(crate) fn recognize(template: &Scene3D) -> Option<RecognizedTemplate<'_>> {
    fn walk<'a>(
        node: &'a Scene3D,
        local: Matrix4<f32>,
        material: Option<&'a MaterialDescription>,
    ) -> Option<RecognizedTemplate<'a>> {
        match &node.obj {
            SceneObject::Geometry(shape) => {
                let primitive = match shape {
                    Shape::Cube => InstancedPrimitive::Cube,
                    Shape::Sphere => InstancedPrimitive::Sphere,
                    Shape::Cylinder => InstancedPrimitive::Cylinder,
                    Shape::Quad => InstancedPrimitive::Quad,
                    Shape::Plane => InstancedPrimitive::Plane,
                    Shape::Heightmap { .. } | Shape::ConvexPolygon { .. } => return None,
                };
                Some(RecognizedTemplate::Primitive(RecognizedPrimitive {
                    primitive,
                    material: material?,
                    local: local * node.xform,
                }))
            }
            SceneObject::Model(description) => {
                // A pose-attached (`Scene.animate`) model is recognized too —
                // every copy shows the SAME sampled pose (the stamp of a
                // shared-tts group is exactly that), drawn by the skinned
                // instanced path. Overrides are legacy and untypical. An
                // enclosing material would be IGNORED by the ordinary model
                // draw — fall back so the stamp stays authoritative rather
                // than encoding that quirk here too.
                if material.is_some() || !description.overrides.is_empty() {
                    return None;
                }
                Some(RecognizedTemplate::Model(RecognizedModel {
                    description,
                    local: local * node.xform,
                }))
            }
            SceneObject::Group(items) => match items.as_slice() {
                [only] => walk(only, local * node.xform, material),
                _ => None,
            },
            // A `Material` node's own xform is ignored by the renderer, so it
            // does not compose here either.
            SceneObject::Material(description, items) => {
                if material.is_some() {
                    return None;
                }
                let recognized = match description {
                    MaterialDescription::Color(_) => true,
                    MaterialDescription::Emissive { texture, .. } => texture.is_none(),
                    MaterialDescription::Lit {
                        texture,
                        normal_map,
                        ..
                    } => texture.is_none() && normal_map.is_none(),
                    MaterialDescription::Texture(_) | MaterialDescription::SpriteTexture { .. } => {
                        false
                    }
                };
                if !recognized {
                    return None;
                }
                match items.as_slice() {
                    [only] => walk(only, local, Some(description)),
                    _ => None,
                }
            }
            SceneObject::Terrain(_)
            | SceneObject::Opacity(..)
            | SceneObject::Instanced { .. } => None,
        }
    }
    walk(template, Matrix4::from_scale(1.0), None)
}

/// A short structural signature of a template — the once-per-topology key and
/// human-readable shape for the CPU-expansion perf note. Depth-limited so a
/// pathological tree cannot make the KEY itself expensive.
pub(crate) fn template_summary(template: &Scene3D) -> String {
    fn describe(node: &Scene3D, depth: usize, out: &mut String) {
        if depth > 4 {
            out.push('…');
            return;
        }
        match &node.obj {
            SceneObject::Geometry(shape) => out.push_str(match shape {
                Shape::Cube => "cube",
                Shape::Sphere => "sphere",
                Shape::Cylinder => "cylinder",
                Shape::Quad => "quad",
                Shape::Plane => "plane",
                Shape::Heightmap { .. } => "heightmap",
                Shape::ConvexPolygon { .. } => "polygon",
            }),
            SceneObject::Model(model) => {
                out.push_str("model(");
                let super::ModelHandle::File(file) = &model.handle;
                out.push_str(file);
                out.push(')');
            }
            SceneObject::Terrain(_) => out.push_str("terrain"),
            SceneObject::Instanced { .. } => out.push_str("instanced"),
            SceneObject::Opacity(_, items) => {
                out.push_str("opacity[");
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(' ');
                    }
                    describe(item, depth + 1, out);
                }
                out.push(']');
            }
            SceneObject::Material(material, items) => {
                out.push_str(match material {
                    MaterialDescription::Color(_) => "color[",
                    MaterialDescription::Texture(_) => "texture[",
                    MaterialDescription::Emissive { .. } => "emissive[",
                    MaterialDescription::Lit { .. } => "lit[",
                    MaterialDescription::SpriteTexture { .. } => "sprite[",
                });
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(' ');
                    }
                    describe(item, depth + 1, out);
                }
                out.push(']');
            }
            SceneObject::Group(items) => {
                out.push_str("group[");
                for (index, item) in items.iter().enumerate().take(4) {
                    if index > 0 {
                        out.push(' ');
                    }
                    describe(item, depth + 1, out);
                }
                if items.len() > 4 {
                    out.push('…');
                }
                out.push(']');
            }
        }
    }
    let mut out = String::new();
    describe(template, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{Deg, Matrix4, Quaternion, Rotation3};

    fn lit_cube() -> Scene3D {
        Scene3D {
            obj: SceneObject::Material(
                MaterialDescription::lit(0.8, 0.2, 0.1, 1.0),
                vec![Scene3D::cube()],
            ),
            xform: Matrix4::from_scale(1.0),
        }
    }

    /// The load-bearing pin: an `Instanced` node's CPU expansion IS the
    /// hand-written group of transformed, tinted copies.
    #[test]
    fn expansion_matches_the_hand_written_stamp() {
        let instances = vec![
            InstanceData::at([1.0, 2.0, 3.0])
                .scaled_xyz(0.5, 1.0, 2.0)
                .tinted([0.5, 1.0, 1.0]),
            InstanceData::at([-4.0, 0.0, 0.0]).rotated(Quaternion::from_angle_y(Deg(90.0))),
        ];
        let expanded = expand_instanced(&lit_cube(), &instances);

        let stamp = |instance: &InstanceData, r: f32, g: f32, b: f32| Scene3D {
            obj: SceneObject::Group(vec![Scene3D {
                obj: SceneObject::Material(
                    MaterialDescription::lit(r, g, b, 1.0),
                    vec![Scene3D::cube()],
                ),
                xform: Matrix4::from_scale(1.0),
            }]),
            xform: instance.matrix(),
        };
        let by_hand = Scene3D {
            obj: SceneObject::Group(vec![
                stamp(&instances[0], 0.4, 0.2, 0.1),
                stamp(&instances[1], 0.8, 0.2, 0.1),
            ]),
            xform: Matrix4::from_scale(1.0),
        };
        assert_eq!(expanded, by_hand);
    }

    #[test]
    fn channels_compose_within_themselves() {
        let instance = InstanceData::at([1.0, 0.0, 0.0])
            .scaled(2.0)
            .scaled_xyz(1.0, 3.0, 1.0)
            .tinted([0.5, 1.0, 1.0])
            .tinted([0.5, 0.5, 1.0]);
        assert_eq!(instance.scale, [2.0, 6.0, 2.0]);
        assert_eq!(instance.tint, [0.25, 0.5, 1.0]);

        // Rotations compose with the outer pipe applied LAST: X(90) after
        // Y(90) maps +Z -> +X (via Y) and leaves +X alone under the later X
        // rotation only after Y has already run.
        let rotated = InstanceData::default()
            .rotated(Quaternion::from_angle_y(Deg(90.0)))
            .rotated(Quaternion::from_angle_x(Deg(90.0)));
        let by_hand = Matrix4::from_angle_x(Deg(90.0f32)) * Matrix4::from_angle_y(Deg(90.0f32));
        let delta: Matrix4<f32> = rotated.matrix() - by_hand;
        for column in [delta.x, delta.y, delta.z, delta.w] {
            for value in [column.x, column.y, column.z, column.w] {
                assert!(value.abs() < 1e-6, "rotation composition drifted: {value}");
            }
        }
    }

    #[test]
    fn recognizer_accepts_the_hardware_shapes() {
        // Solid-material primitive, through Group transform wrappers.
        let template = Scene3D {
            obj: SceneObject::Group(vec![lit_cube()]),
            xform: Matrix4::from_nonuniform_scale(2.0, 1.0, 1.0),
        };
        let RecognizedTemplate::Primitive(recognized) =
            recognize(&template).expect("lit cube under a transform")
        else {
            panic!("expected a primitive template");
        };
        assert_eq!(recognized.primitive, InstancedPrimitive::Cube);
        assert!(matches!(
            recognized.material,
            MaterialDescription::Lit { .. }
        ));
        assert_eq!(
            recognized.local,
            Matrix4::from_nonuniform_scale(2.0, 1.0, 1.0)
        );

        for (scene, primitive) in [
            (Scene3D::sphere(), InstancedPrimitive::Sphere),
            (Scene3D::cylinder(), InstancedPrimitive::Cylinder),
            (Scene3D::quad(), InstancedPrimitive::Quad),
            (Scene3D::plane(), InstancedPrimitive::Plane),
        ] {
            let template = Scene3D {
                obj: SceneObject::Material(
                    MaterialDescription::color(1.0, 0.0, 0.0, 1.0),
                    vec![scene],
                ),
                xform: Matrix4::from_scale(1.0),
            };
            let RecognizedTemplate::Primitive(recognized) =
                recognize(&template).expect("solid primitive")
            else {
                panic!("expected a primitive template");
            };
            assert_eq!(recognized.primitive, primitive);
        }
    }

    /// Rigid model templates are recognized (transform wrappers fold into
    /// `local`); animated, override-carrying, or material-wrapped models fall
    /// back to the stamp.
    #[test]
    fn recognizer_accepts_rigid_model_templates() {
        use crate::scene3d::{ModelDescription, ModelHandle};

        let model = |animation| {
            Scene3D::model(ModelDescription {
                handle: ModelHandle::File("barrel.glb".into()),
                overrides: vec![],
                animation,
                while_pending: vec![],
            })
        };
        let template = Scene3D {
            obj: SceneObject::Group(vec![model(None)]),
            xform: Matrix4::from_scale(0.02),
        };
        let RecognizedTemplate::Model(recognized) =
            recognize(&template).expect("rigid model under a transform")
        else {
            panic!("expected a model template");
        };
        assert!(matches!(
            recognized.description.handle,
            ModelHandle::File(ref f) if f == "barrel.glb"
        ));
        assert_eq!(recognized.local, Matrix4::from_scale(0.02));

        // A pose-attached template is recognized too — the shared-pose rung:
        // every copy renders the same sampled pose.
        let animated = model(Some(crate::anim::AnimExpr::Rest));
        let RecognizedTemplate::Model(recognized) =
            recognize(&animated).expect("pose-attached model template")
        else {
            panic!("expected a model template");
        };
        assert!(recognized.description.animation.is_some());

        // A material wrapper is IGNORED by the ordinary model draw; fall
        // back so the stamp stays authoritative.
        let wrapped = Scene3D {
            obj: SceneObject::Material(
                MaterialDescription::color(1.0, 0.0, 0.0, 1.0),
                vec![model(None)],
            ),
            xform: Matrix4::from_scale(1.0),
        };
        assert!(recognize(&wrapped).is_none());
    }

    #[test]
    fn recognizer_rejects_what_must_fall_back() {
        use crate::scene3d::TextureDescription;

        // A bare primitive: correct only through the stamp (pass material).
        assert!(recognize(&Scene3D::cube()).is_none());
        // A textured material.
        let textured = Scene3D {
            obj: SceneObject::Material(
                MaterialDescription::lit_texture(TextureDescription::File("t.png".into())),
                vec![Scene3D::cube()],
            ),
            xform: Matrix4::from_scale(1.0),
        };
        assert!(recognize(&textured).is_none());
        // A multi-child group.
        let pair = Scene3D {
            obj: SceneObject::Group(vec![Scene3D::cube(), Scene3D::sphere()]),
            xform: Matrix4::from_scale(1.0),
        };
        assert!(recognize(&pair).is_none());
        // Nested materials.
        let nested = Scene3D {
            obj: SceneObject::Material(
                MaterialDescription::color(1.0, 0.0, 0.0, 1.0),
                vec![lit_cube()],
            ),
            xform: Matrix4::from_scale(1.0),
        };
        assert!(recognize(&nested).is_none());
    }

    #[test]
    fn template_summary_is_short_and_structural() {
        assert_eq!(template_summary(&lit_cube()), "lit[cube]");
        let pair = Scene3D {
            obj: SceneObject::Group(vec![Scene3D::cube(), Scene3D::sphere()]),
            xform: Matrix4::from_scale(1.0),
        };
        assert_eq!(template_summary(&pair), "group[cube sphere]");
    }
}
