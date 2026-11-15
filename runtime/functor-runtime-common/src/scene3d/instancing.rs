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

use std::sync::Arc;

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

/// The instance records of one [`SceneObject::Instanced`] node, shared by
/// reference so cloning a scene never copies them.
///
/// Equality, `Debug`, and serde are by CONTENT — the wire shape is the plain
/// array — so the node stays plain data for `Scene.equals`, `/scene`, and
/// replay. The `Arc` is ALSO an identity token: the prelude hands back the
/// same `Arc` for the same Functor Lang list, and the renderer's retained
/// buffer for an identical `Arc` needs no compare or upload, so a list kept in
/// the model converts and uploads once.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstanceList(Arc<[InstanceData]>);

impl InstanceList {
    /// Whether both lists are the SAME allocation (not merely equal content).
    pub fn ptr_eq(&self, other: &InstanceList) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::ops::Deref for InstanceList {
    type Target = [InstanceData];
    fn deref(&self) -> &[InstanceData] {
        &self.0
    }
}

impl From<Vec<InstanceData>> for InstanceList {
    fn from(instances: Vec<InstanceData>) -> Self {
        InstanceList(instances.into())
    }
}

impl std::fmt::Debug for InstanceList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self[..].fmt(f)
    }
}

/// One copy's paint for `Scene.instancedPaint` — `repr(C)` so the decoded
/// slice uploads directly as the second instance stream (4 floats, 16 bytes
/// per copy). Kept apart from [`InstanceData`] so a paint that changes every
/// frame re-uploads 16 bytes per copy, never the static transforms.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PaintData {
    /// Multiplied into the copy's tint (white is the identity).
    pub tint: [f32; 3],
    /// How far a lit copy's shading mixes toward its unlit colour, 0–1
    /// (clamped). No effect on unlit materials.
    pub glow: f32,
}

impl PaintData {
    pub const IDENTITY: PaintData = PaintData {
        tint: [1.0, 1.0, 1.0],
        glow: 0.0,
    };
}

/// The paint records of one instanced node, shared by reference; content
/// equality and serde, `Arc` identity for the renderer's retained buffer
/// (see [`InstanceList`]).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaintList(Arc<[PaintData]>);

impl PaintList {
    pub fn ptr_eq(&self, other: &PaintList) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::ops::Deref for PaintList {
    type Target = [PaintData];
    fn deref(&self) -> &[PaintData] {
        &self.0
    }
}

impl From<Vec<PaintData>> for PaintList {
    fn from(paints: Vec<PaintData>) -> Self {
        PaintList(paints.into())
    }
}

impl FromIterator<PaintData> for PaintList {
    fn from_iter<I: IntoIterator<Item = PaintData>>(iter: I) -> Self {
        PaintList(iter.into_iter().collect())
    }
}

impl std::fmt::Debug for PaintList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self[..].fmt(f)
    }
}

/// Two glows applied in turn: `mix(mix(s, a, g1), a, g2)` is
/// `mix(s, a, 1 - (1 - g1)(1 - g2))`.
pub(crate) fn compose_glow(g1: f32, g2: f32) -> f32 {
    1.0 - (1.0 - g1.clamp(0.0, 1.0)) * (1.0 - g2.clamp(0.0, 1.0))
}

/// The stamp expansion — the DEFINITION of `Scene.instanced`'s semantics, and
/// the renderer's fallback for templates [`recognize`] does not accept.
///
/// Each copy is the template wrapped in a `Group` carrying the instance
/// transform (transforms never sit on `Material` nodes, which the renderer
/// ignores), with the copy's tint (times its paint's) multiplied into every
/// material color in the stamped subtree, and its paint's glow composed into
/// every lit material.
pub fn expand_instanced(
    template: &Scene3D,
    instances: &[InstanceData],
    paints: Option<&PaintList>,
) -> Scene3D {
    Scene3D {
        obj: SceneObject::Group(
            instances
                .iter()
                .enumerate()
                .map(|(index, instance)| {
                    let paint = paints.and_then(|p| p.get(index)).copied();
                    let paint = paint.unwrap_or(PaintData::IDENTITY);
                    let tint = std::array::from_fn(|k| instance.tint[k] * paint.tint[k]);
                    Scene3D {
                        obj: SceneObject::Group(vec![paint_scene(
                            template.clone(),
                            tint,
                            paint.glow,
                        )]),
                        xform: instance.matrix(),
                    }
                })
                .collect(),
        ),
        xform: Matrix4::from_scale(1.0),
    }
}

/// Multiply `tint` into every material color of a subtree and compose
/// `glow` into every lit material. White and 0 are the identity (returns the
/// scene untouched). A `Texture` material has no color channel and is
/// unchanged; nested `Instanced` nodes fold both into their own channels.
fn paint_scene(scene: Scene3D, tint: [f32; 3], glow: f32) -> Scene3D {
    if tint == [1.0, 1.0, 1.0] && glow <= 0.0 {
        return scene;
    }
    let map = |items: Vec<Scene3D>| {
        items
            .into_iter()
            .map(|item| paint_scene(item, tint, glow))
            .collect()
    };
    let obj = match scene.obj.into_resolved() {
        SceneObject::Material(material, items) => {
            let material = match tint_material(material, tint) {
                MaterialDescription::Lit {
                    color,
                    texture,
                    normal_map,
                    world_tile,
                    glow: own,
                } => MaterialDescription::Lit {
                    color,
                    texture,
                    normal_map,
                    world_tile,
                    glow: compose_glow(own, glow),
                },
                other => other,
            };
            SceneObject::Material(material, map(items))
        }
        SceneObject::Group(items) => SceneObject::Group(map(items)),
        SceneObject::Opacity(alpha, items) => SceneObject::Opacity(alpha, map(items)),
        SceneObject::Cells(node, items) => SceneObject::Cells(node, map(items)),
        SceneObject::CastShadows(casts, items) => SceneObject::CastShadows(casts, map(items)),
        SceneObject::Resident(items) => SceneObject::Resident(map(items)),
        SceneObject::Instanced {
            template,
            instances,
            paints,
        } => {
            let paints = if glow > 0.0 {
                let own = |index| {
                    let own = paints.as_ref().and_then(|p| p.get(index)).copied();
                    own.unwrap_or(PaintData::IDENTITY)
                };
                let glowing = (0..instances.len()).map(|index| PaintData {
                    glow: compose_glow(own(index).glow, glow),
                    ..own(index)
                });
                Some(glowing.collect())
            } else {
                paints
            };
            SceneObject::Instanced {
                template,
                instances: instances
                    .iter()
                    .map(|instance| instance.clone().tinted(tint))
                    .collect::<Vec<_>>()
                    .into(),
                paints,
            }
        }
        leaf @ (SceneObject::Geometry(_) | SceneObject::Model(_) | SceneObject::Terrain(_)) => leaf,
        SceneObject::Shared(_) => unreachable!("resolved"),
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
            world_tile,
            glow,
        } => MaterialDescription::Lit {
            color: scale(color),
            texture,
            normal_map,
            world_tile,
            glow,
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
        MaterialDescription::Multiply { color, texture } => MaterialDescription::Multiply {
            color: scale(color),
            texture,
        },
        MaterialDescription::Additive { color, fresnel } => MaterialDescription::Additive {
            color: scale(color),
            fresnel,
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
    /// The quad mesh, billboarded per instance at draw time (see
    /// [`Shape::Billboard`]).
    Billboard,
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
    /// The innermost `Scene.castShadows` in the template, if any.
    pub casts_shadows: Option<bool>,
}

pub(crate) struct RecognizedModel<'a> {
    pub description: &'a super::ModelDescription,
    /// Group wrappers plus the leaf's own xform; each mesh primitive further
    /// composes its glTF node transform on the right.
    pub local: Matrix4<f32>,
    /// The innermost `Scene.castShadows` in the template, if any.
    pub casts_shadows: Option<bool>,
}

impl RecognizedTemplate<'_> {
    pub(crate) fn casts_shadows(&self) -> Option<bool> {
        match self {
            RecognizedTemplate::Primitive(primitive) => primitive.casts_shadows,
            RecognizedTemplate::Model(model) => model.casts_shadows,
        }
    }
}

/// Decide whether `template` is drawable by the instanced hardware path.
///
/// Recognized: `Group` (or one-child `Scene.castShadows`) chains with exactly
/// one child (their transforms compose into `local`) ending in either a
/// `Cube`/`Sphere`/`Cylinder`/`Quad`/`Billboard`/`Plane` leaf under at most ONE material
/// node — a solid `Color`, `Emissive` (no texture), `Lit` (no normal map;
/// a texture only when world-tiled), `Multiply` or `Additive` — or a
/// `Scene.model` leaf with no overrides, with or without
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
        casts_shadows: Option<bool>,
    ) -> Option<RecognizedTemplate<'a>> {
        match node.obj.resolved() {
            SceneObject::Geometry(shape) => {
                let primitive = match shape {
                    Shape::Cube => InstancedPrimitive::Cube,
                    Shape::Sphere => InstancedPrimitive::Sphere,
                    Shape::Cylinder => InstancedPrimitive::Cylinder,
                    Shape::Quad => InstancedPrimitive::Quad,
                    Shape::Billboard => InstancedPrimitive::Billboard,
                    Shape::Plane => InstancedPrimitive::Plane,
                    Shape::Heightmap { .. } | Shape::ConvexPolygon { .. } | Shape::Mesh { .. } => {
                        return None
                    }
                };
                Some(RecognizedTemplate::Primitive(RecognizedPrimitive {
                    primitive,
                    material: material?,
                    local: local * node.xform,
                    casts_shadows,
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
                // Static data models draw per group; they stamp.
                if matches!(&description.handle, super::ModelHandle::Data(d) if d.is_static()) {
                    return None;
                }
                Some(RecognizedTemplate::Model(RecognizedModel {
                    description,
                    local: local * node.xform,
                    casts_shadows,
                }))
            }
            SceneObject::Group(items) => match items.as_slice() {
                [only] => walk(only, local * node.xform, material, casts_shadows),
                _ => None,
            },
            SceneObject::CastShadows(casts, items) => match items.as_slice() {
                [only] => walk(only, local * node.xform, material, Some(*casts)),
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
                    // A world-tiled texture reads no mesh UVs, so the
                    // copies' scales can't stretch it.
                    MaterialDescription::Lit {
                        texture,
                        normal_map,
                        world_tile,
                        ..
                    } => texture.is_some() == world_tile.is_some() && normal_map.is_none(),
                    // Blend materials: a multiply's texture reads the mesh's
                    // UVs as each stamped copy's does (scaled with it).
                    MaterialDescription::Multiply { .. } | MaterialDescription::Additive { .. } => {
                        true
                    }
                    MaterialDescription::Texture(_) | MaterialDescription::SpriteTexture { .. } => {
                        false
                    }
                };
                if !recognized {
                    return None;
                }
                match items.as_slice() {
                    [only] => walk(only, local, Some(description), casts_shadows),
                    _ => None,
                }
            }
            SceneObject::Terrain(_)
            | SceneObject::Opacity(..)
            | SceneObject::Instanced { .. }
            | SceneObject::Cells(..)
            | SceneObject::Resident(_) => None,
            SceneObject::Shared(_) => unreachable!("resolved"),
        }
    }
    walk(template, Matrix4::from_scale(1.0), None, None)
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
        match node.obj.resolved() {
            SceneObject::Geometry(shape) => out.push_str(match shape {
                Shape::Cube => "cube",
                Shape::Sphere => "sphere",
                Shape::Cylinder => "cylinder",
                Shape::Quad => "quad",
                Shape::Billboard => "billboard",
                Shape::Plane => "plane",
                Shape::Heightmap { .. } => "heightmap",
                Shape::ConvexPolygon { .. } => "polygon",
                Shape::Mesh { .. } => "mesh",
            }),
            SceneObject::Model(model) => {
                out.push_str("model(");
                out.push_str(match &model.handle {
                    super::ModelHandle::File(file) => file,
                    super::ModelHandle::Data(data) => data.label(),
                });
                out.push(')');
            }
            SceneObject::Terrain(_) => out.push_str("terrain"),
            SceneObject::Instanced { .. } => out.push_str("instanced"),
            SceneObject::Cells(..) => out.push_str("cells"),
            SceneObject::Resident(_) => out.push_str("resident"),
            SceneObject::CastShadows(_, items) => {
                out.push_str("castShadows[");
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(' ');
                    }
                    describe(item, depth + 1, out);
                }
                out.push(']');
            }
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
                    MaterialDescription::Multiply { .. } => "multiply[",
                    MaterialDescription::Additive { .. } => "additive[",
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
            SceneObject::Shared(_) => unreachable!("resolved"),
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
        let expanded = expand_instanced(&lit_cube(), &instances, None);

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

    /// A paint multiplies into the copy's tint and composes its glow into
    /// the lit material; an unlit material keeps no glow.
    #[test]
    fn paints_expand_into_tint_and_glow() {
        let instances = vec![
            InstanceData::at([0.0; 3]).tinted([0.5, 1.0, 1.0]),
            InstanceData::at([1.0, 0.0, 0.0]),
        ];
        let paints: PaintList = vec![
            PaintData { tint: [1.0, 0.5, 1.0], glow: 0.5 },
            PaintData::IDENTITY,
        ]
        .into();
        let expanded = expand_instanced(&lit_cube(), &instances, Some(&paints));
        let SceneObject::Group(copies) = &expanded.obj else { panic!() };
        let material = |copy: &Scene3D| {
            let SceneObject::Group(inner) = &copy.obj else { panic!() };
            let SceneObject::Material(material, _) = &inner[0].obj else { panic!() };
            material.clone()
        };
        let mut glowing = MaterialDescription::lit(0.4, 0.1, 0.1, 1.0);
        if let MaterialDescription::Lit { glow, .. } = &mut glowing {
            *glow = 0.5;
        }
        assert_eq!(material(&copies[0]), glowing);
        assert_eq!(material(&copies[1]), MaterialDescription::lit(0.8, 0.2, 0.1, 1.0));

        let unlit = Scene3D {
            obj: SceneObject::Material(
                MaterialDescription::Color(cgmath::vec4(1.0, 1.0, 1.0, 1.0)),
                vec![Scene3D::cube()],
            ),
            xform: Matrix4::from_scale(1.0),
        };
        let expanded = expand_instanced(&unlit, &instances, Some(&paints));
        let SceneObject::Group(copies) = &expanded.obj else { panic!() };
        assert_eq!(
            material(&copies[0]),
            MaterialDescription::Color(cgmath::vec4(0.5, 0.5, 1.0, 1.0))
        );
    }

    #[test]
    fn glows_compose_like_two_mixes() {
        // mix(mix(s, a, 0.5), a, 0.5) = mix(s, a, 0.75).
        assert_eq!(compose_glow(0.5, 0.5), 0.75);
        assert_eq!(compose_glow(0.0, 0.3), 0.3);
        assert_eq!(compose_glow(2.0, 0.0), 1.0);
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

        // A world-tiled lit texture: no mesh UVs for the copies to stretch.
        let world_tiled = Scene3D {
            obj: SceneObject::Material(
                MaterialDescription::lit_world_texture(
                    crate::scene3d::TextureDescription::File("t.png".into()),
                    2.0,
                ),
                vec![Scene3D::cube()],
            ),
            xform: Matrix4::from_scale(1.0),
        };
        assert!(matches!(
            recognize(&world_tiled),
            Some(RecognizedTemplate::Primitive(_))
        ));

        for (scene, primitive) in [
            (Scene3D::sphere(), InstancedPrimitive::Sphere),
            (Scene3D::cylinder(), InstancedPrimitive::Cylinder),
            (Scene3D::quad(), InstancedPrimitive::Quad),
            (Scene3D::billboard(), InstancedPrimitive::Billboard),
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
                parts: Vec::new(),
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

    /// A one-child `Scene.castShadows` is a transparent wrapper; the
    /// innermost flag is reported. Without one, the template has no say.
    #[test]
    fn recognizer_sees_through_cast_shadows() {
        let lit = |inner: Scene3D| Scene3D {
            obj: SceneObject::Material(MaterialDescription::color(1.0, 0.0, 0.0, 1.0), vec![inner]),
            xform: Matrix4::from_scale(1.0),
        };
        let cast = |casts: bool, inner: Scene3D| Scene3D {
            obj: SceneObject::CastShadows(casts, vec![inner]),
            xform: Matrix4::from_scale(2.0),
        };
        let flag = |template: &Scene3D| {
            let RecognizedTemplate::Primitive(recognized) =
                recognize(template).expect("recognized")
            else {
                panic!("expected a primitive template");
            };
            (recognized.casts_shadows, recognized.local)
        };
        assert_eq!(flag(&lit(Scene3D::cube())).0, None);
        assert_eq!(
            flag(&cast(false, lit(Scene3D::cube()))),
            (Some(false), Matrix4::from_scale(2.0))
        );
        assert_eq!(
            flag(&lit(cast(true, cast(false, Scene3D::cube())))).0,
            Some(false)
        );
        assert_eq!(
            flag(&cast(false, lit(cast(true, Scene3D::cube())))).0,
            Some(true)
        );
        let pair = Scene3D {
            obj: SceneObject::CastShadows(false, vec![Scene3D::cube(), Scene3D::cube()]),
            xform: Matrix4::from_scale(1.0),
        };
        assert!(recognize(&lit(pair)).is_none());
    }

    #[test]
    fn recognizer_rejects_what_must_fall_back() {
        use crate::scene3d::TextureDescription;

        // A bare primitive: correct only through the stamp (pass material).
        assert!(recognize(&Scene3D::cube()).is_none());
        // A bare billboard behaves exactly like a bare quad — fall back.
        assert!(recognize(&Scene3D::billboard()).is_none());
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

    /// Blend templates instance as one draw: a multiply's texture reads the
    /// mesh's UVs as each stamped copy does, and a tint scales the colour.
    #[test]
    fn recognizer_accepts_blend_templates_and_tints_them() {
        use crate::scene3d::TextureDescription;

        let multiply = MaterialDescription::Multiply {
            color: cgmath::vec4(1.0, 1.0, 1.0, 1.0),
            texture: Some(TextureDescription::File("dirt.png".into())),
        };
        let reflection = MaterialDescription::Additive {
            color: cgmath::vec4(1.0, 1.0, 1.0, 1.0),
            fresnel: Some(0.04),
        };
        for material in [multiply, reflection] {
            let template = Scene3D {
                obj: SceneObject::Material(material.clone(), vec![Scene3D::quad()]),
                xform: Matrix4::from_scale(1.0),
            };
            let Some(RecognizedTemplate::Primitive(recognized)) = recognize(&template) else {
                panic!("expected a primitive template for {material:?}");
            };
            assert_eq!(recognized.material, &material);
            let tinted = tint_material(material, [0.5, 0.25, 1.0]);
            let (MaterialDescription::Multiply { color, .. }
            | MaterialDescription::Additive { color, .. }) = tinted
            else {
                panic!("tinting keeps the blend material");
            };
            assert_eq!(color, cgmath::vec4(0.5, 0.25, 1.0, 1.0));
        }
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
