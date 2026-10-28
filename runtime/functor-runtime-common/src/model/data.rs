//! Models built from plain data (`Model.skinned`, `Model.static`): validated
//! once, then shared by the sim side (posing, inspection) and the render
//! side, which uploads them on first draw. A model with no joints is static.

use std::sync::atomic::{AtomicU64, Ordering};

use cgmath::{vec2, vec3, vec4, InnerSpace, Matrix4, Quaternion, SquareMatrix, Vector3, Zero};
use serde::{Deserialize, Serialize};

use super::{Model, ModelMesh, Skeleton, SkeletonBuilder, MAX_JOINTS};
use crate::animation::{Animation, AnimationChannel, AnimationProperty, AnimationValue, Keyframe};
use crate::geometry::IndexedMesh;
use crate::render::{VertexPositionTextureLightmap, VertexPositionTextureSkinned};
use crate::scene3d::TextureDescription;
use crate::texture::{Texture2D, TextureData, TextureOptions};

/// A model's inputs, Y up. Quaternions are `[x, y, z, w]`. Skinned models
/// have joints, weights and a skeleton; static ones have none of the four
/// skin fields (omitted on the wire) and may have `uvs2` and `groups`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSource {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Empty, or one per vertex.
    pub uvs: Vec<[f32; 2]>,
    /// Static only: a second UV set (lightmaps); empty, or one per vertex.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uvs2: Vec<[f32; 2]>,
    /// Four skeleton indices per vertex; every slot must name a joint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joints: Vec<[u8; 4]>,
    /// Four weights per vertex, normalized to sum to 1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weights: Vec<[f32; 4]>,
    pub triangles: Vec<[u32; 3]>,
    /// Static only: consecutive triangle ranges, each drawn with its own
    /// base texture. Empty draws every triangle untextured.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupSource>,
    /// Parents before children.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skeleton: Vec<JointSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<ClipSource>,
    /// The base color, sampled at `uvs`; `None` draws white. A locator, so
    /// the image loads (and is shared) through the asset cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<TextureDescription>,
}

/// `count` triangles from `first`, drawn with one base texture (`None` is
/// white); `unlit` draws it fullbright.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupSource {
    pub first: u32,
    pub count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<TextureDescription>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unlit: bool,
}

/// A joint's rest pose, local to its parent (unit scale).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JointSource {
    pub name: String,
    pub parent: Option<u32>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
}

/// A looping clip: `frames` keys per track, `1 / fps` seconds apart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipSource {
    pub name: String,
    pub fps: f32,
    pub frames: u32,
    pub tracks: Vec<TrackSource>,
}

/// One joint's keys; each list is empty or one key per frame. A key
/// REPLACES the joint's rest rotation/translation, as a glTF channel does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackSource {
    pub joint: String,
    pub rotations: Vec<[f32; 4]>,
    pub translations: Vec<[f32; 3]>,
}

/// A validated model plus the engine skeleton and clips built from it.
/// Immutable; equality is identity (the same `Model.skinned`/`Model.static`
/// value).
pub struct ModelData {
    id: u64,
    label: String,
    source: ModelSource,
    skeleton: Skeleton,
    animations: Vec<Animation>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl ModelData {
    /// Validate `source` (errors name the field) and build the rig.
    pub fn new(mut source: ModelSource) -> Result<ModelData, String> {
        validate(&mut source)?;
        let skeleton = build_skeleton(&source.skeleton);
        let animations = source
            .clips
            .iter()
            .map(|clip| build_animation(clip, &source.skeleton))
            .collect();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let kind = if source.skeleton.is_empty() { "static" } else { "skinned" };
        Ok(ModelData {
            id,
            label: format!("Model.{kind}#{id}"),
            source,
            skeleton,
            animations,
        })
    }

    /// Unique per built model, never reused: the render-side cache key.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// How warnings and summaries name this model.
    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn source(&self) -> &ModelSource {
        &self.source
    }

    /// No skeleton: drawn by the static path, in groups.
    pub fn is_static(&self) -> bool {
        self.source.skeleton.is_empty()
    }

    pub fn skeleton(&self) -> &Skeleton {
        &self.skeleton
    }

    pub fn animations(&self) -> &[Animation] {
        &self.animations
    }

    /// A drawable model; its GPU buffers hydrate on first draw.
    pub(crate) fn to_model(&self) -> Model {
        let s = &self.source;
        let vertices = (0..s.positions.len())
            .map(|i| {
                let [x, y, z] = s.positions[i];
                let [nx, ny, nz] = s.normals[i];
                let [u, v] = s.uvs.get(i).copied().unwrap_or([0.0, 0.0]);
                let j = s.joints[i].map(f32::from);
                let w = s.weights[i];
                VertexPositionTextureSkinned {
                    position: vec3(x, y, z),
                    uv: vec2(u, v),
                    normal: vec3(nx, ny, nz),
                    // No skinned normal-map path reads it.
                    tangent: vec4(1.0, 0.0, 0.0, 1.0),
                    joint_indices: vec4(j[0], j[1], j[2], j[3]),
                    weights: vec4(w[0], w[1], w[2], w[3]),
                }
            })
            .collect();
        let indices = s.triangles.iter().flatten().copied().collect();
        // White, so lighting alone shades it; a `texture` binds over it at
        // draw time (see `scene3d::bind_model_base_color`).
        let white = Texture2D::init_from_data(
            TextureData::solid_color([255; 4]),
            TextureOptions {
                srgb: true,
                ..TextureOptions::default()
            },
        );
        Model {
            meshes: vec![ModelMesh {
                base_color_texture: white,
                mesh: IndexedMesh::create(vertices, indices),
                transform: Matrix4::identity(),
            }],
            skeleton: self.skeleton.clone(),
            animations: self.animations.clone(),
        }
    }

    /// A static model's mesh, one vertex per source vertex (white base
    /// textures bind per group at draw time).
    pub(crate) fn to_static_mesh(&self) -> IndexedMesh<VertexPositionTextureLightmap> {
        let s = &self.source;
        let vertices = (0..s.positions.len())
            .map(|i| VertexPositionTextureLightmap {
                position: s.positions[i].into(),
                uv: s.uvs.get(i).copied().unwrap_or([0.0, 0.0]).into(),
                normal: s.normals[i].into(),
                uv2: s.uvs2.get(i).copied().unwrap_or([0.0, 0.0]).into(),
            })
            .collect();
        IndexedMesh::create(vertices, s.triangles.iter().flatten().copied().collect())
    }

    /// Bind-space positions skinned by `palette` (one matrix per joint), as
    /// the vertex shader blends them.
    pub fn skin_positions(&self, palette: &[Matrix4<f32>]) -> Vec<[f32; 3]> {
        let s = &self.source;
        s.positions
            .iter()
            .zip(&s.joints)
            .zip(&s.weights)
            .map(|((p, joints), weights)| {
                let mut skin = Matrix4::zero();
                for (joint, weight) in joints.iter().zip(weights) {
                    skin = skin + palette[*joint as usize] * *weight;
                }
                let out = skin * vec4(p[0], p[1], p[2], 1.0);
                [out.x, out.y, out.z]
            })
            .collect()
    }
}

impl PartialEq for ModelData {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl std::fmt::Debug for ModelData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelData")
            .field("id", &self.id)
            .field("vertices", &self.source.positions.len())
            .field("joints", &self.source.skeleton.len())
            .field("clips", &self.source.clips.len())
            .finish()
    }
}

// The wire form is the source; reading it back validates and rebuilds (with
// a fresh identity).
impl Serialize for ModelData {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.source.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ModelData {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let source = ModelSource::deserialize(deserializer)?;
        ModelData::new(source).map_err(serde::de::Error::custom)
    }
}

fn finite<const N: usize>(values: &[[f32; N]]) -> bool {
    values.iter().flatten().all(|x| x.is_finite())
}

/// `q` normalized, or an error for a zero or non-finite quaternion.
fn unit(q: &mut [f32; 4], what: impl Fn() -> String) -> Result<(), String> {
    let length = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    if !(length.is_finite() && length > 0.0) {
        return Err(format!("{}: not a rotation (zero or non-finite)", what()));
    }
    q.iter_mut().for_each(|c| *c /= length);
    Ok(())
}

/// Check every invariant the evaluator and shaders rely on; normalizes
/// weights and quaternions in place.
fn validate(s: &mut ModelSource) -> Result<(), String> {
    let count = s.positions.len();
    if s.normals.len() != count {
        return Err(format!(
            "normals: {} vertices, but positions has {count}",
            s.normals.len()
        ));
    }
    for (field, uvs) in [("uvs", &s.uvs), ("uvs2", &s.uvs2)] {
        if !uvs.is_empty() && uvs.len() != count {
            return Err(format!(
                "{field}: {} vertices, but positions has {count} (or leave {field} empty)",
                uvs.len()
            ));
        }
    }
    if s.texture.is_some() && s.uvs.is_empty() {
        return Err("texture: needs uvs (one per vertex)".to_string());
    }
    for (field, ok) in [
        ("positions", finite(&s.positions)),
        ("normals", finite(&s.normals)),
        ("uvs", finite(&s.uvs)),
        ("uvs2", finite(&s.uvs2)),
    ] {
        if !ok {
            return Err(format!("{field}: a value is not a finite number"));
        }
    }
    for (i, t) in s.triangles.iter().enumerate() {
        if let Some(v) = t.iter().find(|&&v| v as usize >= count) {
            return Err(format!(
                "triangles: triangle {i} uses vertex {v}, but there are {count}"
            ));
        }
    }
    if s.skeleton.is_empty() && s.joints.is_empty() {
        validate_static(s)
    } else {
        validate_skin(s)
    }
}

/// A static model: no skin, and groups that tile the triangles in order.
fn validate_static(s: &ModelSource) -> Result<(), String> {
    for (field, empty) in [
        ("weights", s.weights.is_empty()),
        ("clips", s.clips.is_empty()),
        ("texture", s.texture.is_none()),
    ] {
        if !empty {
            return Err(format!("{field}: a static model has none (it has no joints)"));
        }
    }
    let mut next = 0u64;
    for (i, group) in s.groups.iter().enumerate() {
        if group.first as u64 != next {
            return Err(format!(
                "groups[{i}]: first is {}, but the previous group ends at {next} \
(groups run in order, with no gaps)",
                group.first
            ));
        }
        if group.count == 0 {
            return Err(format!("groups[{i}]: count must be at least 1"));
        }
        if group.texture.is_some() && s.uvs.is_empty() {
            return Err(format!("groups[{i}]: texture needs uvs (one per vertex)"));
        }
        next += group.count as u64;
    }
    let triangles = s.triangles.len() as u64;
    if !s.groups.is_empty() && next != triangles {
        return Err(format!(
            "groups: cover {next} triangles, but there are {triangles}"
        ));
    }
    Ok(())
}

/// A skinned model: per-vertex joints and weights, a skeleton and clips.
fn validate_skin(s: &mut ModelSource) -> Result<(), String> {
    let count = s.positions.len();
    for (field, len) in [("joints", s.joints.len()), ("weights", s.weights.len())] {
        if len != count {
            return Err(format!(
                "{field}: {len} vertices, but positions has {count}"
            ));
        }
    }
    for (field, empty) in [("uvs2", s.uvs2.is_empty()), ("groups", s.groups.is_empty())] {
        if !empty {
            return Err(format!("{field}: only static models have them"));
        }
    }

    let joints = s.skeleton.len();
    if joints == 0 {
        return Err("skeleton: needs at least one joint".to_string());
    }
    if joints > MAX_JOINTS {
        return Err(format!(
            "skeleton: {joints} joints, but skinning supports at most {MAX_JOINTS}"
        ));
    }
    for (i, joint) in s.skeleton.iter_mut().enumerate() {
        let what = || format!("skeleton[{i}] ({:?})", joint.name);
        if joint.name.is_empty() {
            return Err(format!("skeleton[{i}]: name is empty"));
        }
        if let Some(parent) = joint.parent {
            if parent as usize >= i {
                return Err(format!(
                    "{}: parent {parent} must come before it (parents first)",
                    what()
                ));
            }
        }
        if !joint.translation.iter().all(|x| x.is_finite()) {
            return Err(format!("{}: translation is not finite", what()));
        }
        let name = joint.name.clone();
        unit(&mut joint.rotation, || {
            format!("skeleton[{i}] ({name:?}) rotation")
        })?;
    }
    for (i, joint) in s.skeleton.iter().enumerate() {
        if s.skeleton[..i].iter().any(|other| other.name == joint.name) {
            return Err(format!(
                "skeleton[{i}]: joint name {:?} repeats",
                joint.name
            ));
        }
    }

    // Shaders read all four slots, so even a zero-weight slot must be valid.
    for (v, (slots, weights)) in s.joints.iter().zip(s.weights.iter_mut()).enumerate() {
        if let Some(j) = slots.iter().find(|&&j| j as usize >= joints) {
            return Err(format!(
                "joints: vertex {v} uses joint {j}, but the skeleton has {joints}"
            ));
        }
        if !weights.iter().all(|w| w.is_finite() && *w >= 0.0) {
            return Err(format!(
                "weights: vertex {v} has a negative or non-finite weight"
            ));
        }
        let sum: f32 = weights.iter().sum();
        if !(sum.is_finite() && sum > 0.0) {
            return Err(format!("weights: vertex {v}'s weights sum to {sum}"));
        }
        weights.iter_mut().for_each(|w| *w /= sum);
    }

    for (c, clip) in s.clips.iter_mut().enumerate() {
        let what = format!("clips[{c}] ({:?})", clip.name);
        if !(clip.fps.is_finite() && clip.fps > 0.0) {
            return Err(format!("{what}: fps must be positive, got {}", clip.fps));
        }
        if clip.frames == 0 {
            return Err(format!("{what}: frames must be at least 1"));
        }
        if !(clip.frames as f32 / clip.fps).is_finite() {
            return Err(format!("{what}: fps {} is too small", clip.fps));
        }
        let frames = clip.frames as usize;
        for (t, track) in clip.tracks.iter_mut().enumerate() {
            let what = format!("{what}.tracks[{t}]");
            if !s.skeleton.iter().any(|j| j.name == track.joint) {
                return Err(format!("{what}: no joint named {:?}", track.joint));
            }
            for (field, len) in [
                ("rotations", track.rotations.len()),
                ("translations", track.translations.len()),
            ] {
                if len != 0 && len != frames {
                    return Err(format!(
                        "{what}: {field} has {len} keys, but the clip has {frames} frames"
                    ));
                }
            }
            if track.rotations.is_empty() && track.translations.is_empty() {
                return Err(format!("{what}: has no keys"));
            }
            if !finite(&track.translations) {
                return Err(format!("{what}: a translation is not finite"));
            }
            for (k, q) in track.rotations.iter_mut().enumerate() {
                unit(q, || format!("{what}: rotation key {k}"))?;
            }
        }
        for (t, track) in clip.tracks.iter().enumerate() {
            if clip.tracks[..t]
                .iter()
                .any(|other| other.joint == track.joint)
            {
                return Err(format!(
                    "{what}.tracks[{t}]: joint {:?} already has a track",
                    track.joint
                ));
            }
        }
    }
    for (c, clip) in s.clips.iter().enumerate() {
        if s.clips[..c].iter().any(|other| other.name == clip.name) {
            return Err(format!("clips[{c}]: clip name {:?} repeats", clip.name));
        }
    }
    Ok(())
}

fn rotation(q: [f32; 4]) -> Quaternion<f32> {
    Quaternion::new(q[3], q[0], q[1], q[2])
}

/// Joint id = palette index = list index; inverse binds from the rest pose.
fn build_skeleton(joints: &[JointSource]) -> Skeleton {
    let locals: Vec<Matrix4<f32>> = joints
        .iter()
        .map(|j| {
            Matrix4::from_translation(Vector3::from(j.translation))
                * Matrix4::from(rotation(j.rotation))
        })
        .collect();
    let mut globals: Vec<Matrix4<f32>> = Vec::with_capacity(joints.len());
    for (joint, local) in joints.iter().zip(&locals) {
        let global = match joint.parent {
            Some(parent) => globals[parent as usize] * local,
            None => *local,
        };
        globals.push(global);
    }
    let inverse_binds = globals
        .iter()
        .map(|g| g.invert().expect("a rigid transform inverts"))
        .collect();
    let mut builder = SkeletonBuilder::create(inverse_binds);
    for (i, (joint, local)) in joints.iter().zip(locals).enumerate() {
        builder.add_joint(
            i,
            i as i32,
            joint.name.clone(),
            joint.parent.map(|p| p as i32),
            local,
        );
    }
    builder.build()
}

/// Keys at `k / fps`, plus frame 0 again at `frames / fps`: the sampler holds
/// the last key, so the wrap key is what interpolates the final frame back
/// to the first and makes the loop seamless.
fn build_animation(clip: &ClipSource, joints: &[JointSource]) -> Animation {
    let duration = clip.frames as f32 / clip.fps;
    let keys = |values: Vec<AnimationValue>| -> Vec<Keyframe> {
        let first = values[0].clone();
        values
            .into_iter()
            .chain(std::iter::once(first))
            .enumerate()
            .map(|(k, value)| Keyframe {
                time: k as f32 / clip.fps,
                value,
            })
            .collect()
    };
    let mut channels = Vec::new();
    for track in &clip.tracks {
        let target = joints
            .iter()
            .position(|j| j.name == track.joint)
            .expect("validated");
        if !track.rotations.is_empty() {
            channels.push(AnimationChannel {
                target_node_index: target,
                target_property: AnimationProperty::Rotation,
                keyframes: keys(
                    track
                        .rotations
                        .iter()
                        .map(|q| AnimationValue::Rotation(rotation(*q).normalize()))
                        .collect(),
                ),
            });
        }
        if !track.translations.is_empty() {
            channels.push(AnimationChannel {
                target_node_index: target,
                target_property: AnimationProperty::Translation,
                keyframes: keys(
                    track
                        .translations
                        .iter()
                        .map(|t| AnimationValue::Translation(Vector3::from(*t)))
                        .collect(),
                ),
            });
        }
    }
    Animation {
        name: clip.name.clone(),
        channels,
        duration,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Crosses the pipelined sim → render boundary inside `Arc<Frame>`.
    const _: fn() = || {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<ModelData>();
        send_sync::<crate::Frame>();
    };

    /// Two joints, two vertices, one two-frame clip.
    fn source() -> ModelSource {
        let joint = |name: &str, parent| JointSource {
            name: name.to_string(),
            parent,
            translation: [0.0, 1.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
        };
        ModelSource {
            positions: vec![[0.0; 3], [0.0, 2.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 2],
            uvs: vec![],
            uvs2: vec![],
            joints: vec![[0, 0, 0, 0], [1, 0, 0, 0]],
            weights: vec![[2.0, 0.0, 0.0, 0.0], [1.0, 1.0, 0.0, 0.0]],
            triangles: vec![],
            groups: vec![],
            skeleton: vec![joint("a", None), joint("b", Some(0))],
            clips: vec![ClipSource {
                name: "c".to_string(),
                fps: 4.0,
                frames: 2,
                tracks: vec![TrackSource {
                    joint: "b".to_string(),
                    rotations: vec![[0.0, 0.0, 0.0, 2.0], [0.0, 0.0, 1.0, 1.0]],
                    translations: vec![],
                }],
            }],
            texture: None,
        }
    }

    #[test]
    fn builds_bind_pose_and_wrapped_clips() {
        let data = ModelData::new(source()).unwrap();
        // Rest skins to identity: the inverse binds undo the rest globals.
        let skeleton = data.skeleton();
        for m in skeleton.skinning_transforms_for(&skeleton.base_pose()) {
            let identity = Matrix4::<f32>::identity();
            assert!(
                (0..4).all(|c| (m[c] - identity[c]).magnitude() < 1e-6),
                "{m:?}"
            );
        }
        // Weights and quaternions are normalized.
        assert_eq!(data.source().weights[0], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(data.source().weights[1], [0.5, 0.5, 0.0, 0.0]);
        assert_eq!(
            data.source().clips[0].tracks[0].rotations[0],
            [0.0, 0.0, 0.0, 1.0]
        );
        // Frame 0 repeats at frames / fps.
        let clip = &data.animations()[0];
        assert_eq!(clip.duration, 0.5);
        let times: Vec<f32> = clip.channels[0].keyframes.iter().map(|k| k.time).collect();
        assert_eq!(times, [0.0, 0.25, 0.5]);
        let AnimationValue::Rotation(last) = clip.channels[0].keyframes[2].value else {
            panic!("rotation keys");
        };
        assert_eq!(last, Quaternion::new(1.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn rejects_invalid_data_naming_the_field() {
        let cases: Vec<(fn(&mut ModelSource), &str)> = vec![
            (
                |s| s.normals.truncate(1),
                "normals: 1 vertices, but positions has 2",
            ),
            (
                |s| s.positions[0][1] = f32::NAN,
                "positions: a value is not a finite number",
            ),
            (
                |s| s.joints[0][3] = 2,
                "joints: vertex 0 uses joint 2, but the skeleton has 2",
            ),
            (
                |s| s.weights[1][2] = -1.0,
                "weights: vertex 1 has a negative or non-finite weight",
            ),
            (
                |s| s.weights[1] = [f32::MAX; 4],
                "weights: vertex 1's weights sum to inf",
            ),
            (|s| s.clips[0].fps = 1e-45, "clips[0] (\"c\"): fps 0.0"),
            (|s| s.skeleton.clear(), "skeleton: needs at least one joint"),
            (|s| s.uvs2 = vec![[0.0; 2]; 2], "uvs2: only static models have them"),
            (
                |s| s.texture = Some(TextureDescription::File("skin.png".into())),
                "texture: needs uvs (one per vertex)",
            ),
            (
                |s| s.skeleton[1].name = "a".into(),
                "skeleton[1]: joint name \"a\" repeats",
            ),
            (
                |s| s.skeleton[0].rotation = [0.0; 4],
                "skeleton[0] (\"a\") rotation: not a rotation",
            ),
            (
                |s| s.clips[0].fps = 0.0,
                "clips[0] (\"c\"): fps must be positive, got 0",
            ),
            (
                |s| s.clips[0].frames = 3,
                "clips[0] (\"c\").tracks[0]: rotations has 2 keys, but the clip has 3 frames",
            ),
            (
                |s| s.clips[0].tracks[0].rotations.clear(),
                "clips[0] (\"c\").tracks[0]: has no keys",
            ),
            (
                |s| {
                    let track = s.clips[0].tracks[0].clone();
                    s.clips[0].tracks.push(track);
                },
                "clips[0] (\"c\").tracks[1]: joint \"b\" already has a track",
            ),
            (
                |s| {
                    let clip = s.clips[0].clone();
                    s.clips.push(clip);
                },
                "clips[1]: clip name \"c\" repeats",
            ),
            (
                |s| {
                    let joint = s.skeleton[0].clone();
                    s.skeleton = (0..=MAX_JOINTS)
                        .map(|i| JointSource {
                            name: i.to_string(),
                            ..joint.clone()
                        })
                        .collect();
                },
                "skeleton: 201 joints, but skinning supports at most 200",
            ),
        ];
        for (mutate, expected) in cases {
            let mut s = source();
            mutate(&mut s);
            let err = ModelData::new(s).err().expect(expected);
            assert!(err.starts_with(expected), "{err} != {expected}");
        }
    }

    /// A quad (two triangles) with no skin.
    fn static_source() -> ModelSource {
        ModelSource {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0; 2]; 4],
            uvs2: vec![[0.5; 2]; 4],
            joints: vec![],
            weights: vec![],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            groups: vec![
                GroupSource { first: 0, count: 1, texture: None, unlit: false },
                GroupSource {
                    first: 1,
                    count: 1,
                    texture: Some(TextureDescription::File("wall.png".into())),
                    unlit: true,
                },
            ],
            skeleton: vec![],
            clips: vec![],
            texture: None,
        }
    }

    #[test]
    fn static_models_have_no_rig_and_round_trip() {
        let data = ModelData::new(static_source()).unwrap();
        assert!(data.is_static());
        assert!(data.label().starts_with("Model.static#"));
        assert_eq!(data.skeleton().get_joint_count(), 0);
        // The skin fields are omitted on the wire, and read back static.
        let json = serde_json::to_string(&data).unwrap();
        assert!(!json.contains("joints") && !json.contains("skeleton"), "{json}");
        assert!(json.contains(r#""groups":[{"first":0,"count":1},"#), "{json}");
        let back: ModelData = serde_json::from_str(&json).unwrap();
        assert!(back.is_static());
        assert_eq!(back.source(), data.source());
    }

    #[test]
    fn rejects_invalid_static_data_naming_the_field() {
        let cases: Vec<(fn(&mut ModelSource), &str)> = vec![
            (
                |s| s.uvs2.truncate(3),
                "uvs2: 3 vertices, but positions has 4 (or leave uvs2 empty)",
            ),
            (|s| s.uvs2[1][0] = f32::INFINITY, "uvs2: a value is not a finite number"),
            (
                |s| s.groups[1].first = 2,
                "groups[1]: first is 2, but the previous group ends at 1",
            ),
            (|s| s.groups[0].count = 0, "groups[0]: count must be at least 1"),
            (|s| s.groups.truncate(1), "groups: cover 1 triangles, but there are 2"),
            (
                |s| s.uvs.clear(),
                "groups[1]: texture needs uvs (one per vertex)",
            ),
            (
                |s| s.weights = vec![[1.0, 0.0, 0.0, 0.0]; 4],
                "weights: a static model has none",
            ),
            (
                |s| s.texture = Some(TextureDescription::File("x.png".into())),
                "texture: a static model has none",
            ),
        ];
        for (mutate, expected) in cases {
            let mut s = static_source();
            mutate(&mut s);
            let err = ModelData::new(s).err().expect(expected);
            assert!(err.starts_with(expected), "{err} != {expected}");
        }
        // No groups draws everything as one untextured group.
        let mut s = static_source();
        s.groups.clear();
        assert!(ModelData::new(s).is_ok());
    }
}
