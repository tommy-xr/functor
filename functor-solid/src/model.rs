//! glTF models as triangle soup for printing: flatten the default scene,
//! optionally posing skinned meshes at a moment of an animation, then move it
//! into the print frame (mm, Z up, on the bed). Morph targets are not applied.

use cgmath::{Matrix4, Quaternion, SquareMatrix, Vector4};
use gltf::animation::{util::ReadOutputs, Interpolation, Property};

use crate::Mesh;

type M4 = Matrix4<f64>;

/// A moment of an animation to pose skinned meshes at.
pub struct Pose<'a> {
    /// Animation name or index; `None` is the first animation.
    pub animation: Option<&'a str>,
    /// Wrapped by the animation's length, as a looping clip plays in a game.
    pub seconds: f32,
}

/// The model's triangles in its own units, turned Z up (glTF is Y up). Only
/// self-contained files load (`.glb`, or `.gltf` with embedded buffers);
/// textures are never decoded.
pub fn load_gltf(bytes: &[u8], pose: Option<Pose>) -> Result<Mesh, String> {
    let gltf::Gltf {
        document: doc,
        blob,
    } = gltf::Gltf::from_slice(bytes).map_err(|e| format!("not a loadable glTF: {e}"))?;
    let buffers = gltf::import_buffers(&doc, None, blob)
        .map_err(|e| format!("cannot load the model's buffers: {e}"))?;
    let posed = match pose {
        None => None,
        Some(p) => Some((find_animation(&doc, p.animation)?, p.seconds)),
    };
    let local = local_transforms(&doc, &buffers, posed);
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or("the model has no scene")?;

    // World transform per node, in scene order.
    let mut world = vec![M4::identity(); doc.nodes().len()];
    let mut order = Vec::new();
    let mut stack: Vec<(gltf::Node, M4)> = scene.nodes().map(|n| (n, M4::identity())).collect();
    while let Some((node, parent)) = stack.pop() {
        let w = parent * local[node.index()];
        world[node.index()] = w;
        stack.extend(node.children().map(|c| (c, w)));
        order.push(node);
    }

    let mut out = Mesh::default();
    for node in order {
        let Some(mesh) = node.mesh() else { continue };
        // Joint matrices for skinned meshes (rest pose unless posed).
        let joints = node.skin().map(|skin| {
            let reader = skin.reader(|b| Some(&buffers[b.index()]));
            let inverse_bind: Vec<M4> = match reader.read_inverse_bind_matrices() {
                Some(it) => it.map(matrix).collect(),
                None => vec![M4::identity(); skin.joints().count()],
            };
            skin.joints()
                .zip(inverse_bind)
                .map(|(j, ib)| world[j.index()] * ib)
                .collect::<Vec<M4>>()
        });
        for prim in mesh.primitives() {
            use gltf::mesh::Mode;
            let reader = prim.reader(|b| Some(&buffers[b.index()]));
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let positions: Vec<[f64; 3]> = positions.map(|p| p.map(f64::from)).collect();
            let n = positions.len() as u32;
            let indices: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..n).collect(),
            };
            if indices.iter().any(|&i| i >= n) {
                return Err("a mesh indexes past its vertices".to_string());
            }
            let mut triangles: Vec<[u32; 3]> = match prim.mode() {
                Mode::Triangles => indices
                    .chunks_exact(3)
                    .map(|t| [t[0], t[1], t[2]])
                    .collect(),
                // Every other strip triangle is flipped to keep the winding.
                Mode::TriangleStrip => indices
                    .windows(3)
                    .enumerate()
                    .map(|(i, t)| {
                        if i % 2 == 0 {
                            [t[0], t[1], t[2]]
                        } else {
                            [t[1], t[0], t[2]]
                        }
                    })
                    .collect(),
                Mode::TriangleFan => indices
                    .windows(2)
                    .skip(1)
                    .map(|t| [indices[0], t[0], t[1]])
                    .collect(),
                // Points and lines have no surface.
                _ => continue,
            };
            // Repeated indices (strip joins) have no area, but would thicken
            // into rods between pieces.
            triangles.retain(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
            let sets: Vec<(Vec<[u16; 4]>, Vec<[f32; 4]>)> = (0..)
                .map_while(|s| {
                    Some((
                        reader.read_joints(s)?.into_u16().collect(),
                        reader.read_weights(s)?.into_f32().collect(),
                    ))
                })
                .collect();
            let base = out.vertices.len() as u32;
            match &joints {
                Some(jm) if !sets.is_empty() => out.vertices.extend(skin(jm, &sets, &positions)?),
                _ => {
                    let w = &world[node.index()];
                    out.vertices.extend(positions.iter().map(|&p| apply(w, p)));
                }
            }
            out.triangles
                .extend(triangles.into_iter().map(|t| t.map(|i| i + base)));
        }
    }
    if out.triangles.is_empty() {
        return Err("the model has no triangle meshes".to_string());
    }
    for v in &mut out.vertices {
        *v = [v[0], -v[2], v[1]];
    }
    Ok(out)
}

/// Scale `mesh` so it is `height` mm tall, centered over the origin, resting
/// on z = 0.
pub fn to_print_frame(mesh: &mut Mesh, height: f64) -> Result<(), String> {
    let b = mesh.bounds().ok_or("the model has no geometry")?;
    let extent = b.max[2] - b.min[2];
    if !(extent > 0.0 && mesh.vertices.iter().flatten().all(|x| x.is_finite())) {
        return Err(
            "the model is flat or has invalid coordinates, so it has no height to scale"
                .to_string(),
        );
    }
    let scale = height / extent;
    let center = [
        (b.min[0] + b.max[0]) / 2.0,
        (b.min[1] + b.max[1]) / 2.0,
        b.min[2],
    ];
    for v in &mut mesh.vertices {
        *v = std::array::from_fn(|i| (v[i] - center[i]) * scale);
    }
    Ok(())
}

/// An animation by name, else by index; `None` is the first.
fn find_animation<'a>(
    doc: &'a gltf::Document,
    selector: Option<&str>,
) -> Result<gltf::Animation<'a>, String> {
    let found = match selector {
        None => doc.animations().next(),
        Some(s) => doc
            .animations()
            .find(|a| a.name() == Some(s))
            .or_else(|| s.parse().ok().and_then(|i| doc.animations().nth(i))),
    };
    found.ok_or_else(|| {
        let names: Vec<String> = doc
            .animations()
            .map(|a| a.name().map_or(a.index().to_string(), str::to_string))
            .collect();
        if names.is_empty() {
            "the model has no animations to pose".to_string()
        } else {
            format!(
                "no animation {:?} (available: {})",
                selector.unwrap_or_default(),
                names.join(", ")
            )
        }
    })
}

/// Local transform per node; nodes driven by the animation are sampled at
/// the given time, wrapped by the clip's length.
fn local_transforms(
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    posed: Option<(gltf::Animation, f32)>,
) -> Vec<M4> {
    let mut local: Vec<M4> = doc
        .nodes()
        .map(|n| matrix(n.transform().matrix()))
        .collect();
    if let Some((animation, seconds)) = posed {
        let channels: Vec<_> = animation
            .channels()
            .filter_map(|c| {
                let reader = c.reader(|b| Some(&buffers[b.index()]));
                let times: Vec<f32> = reader.read_inputs()?.collect();
                let values: Vec<[f32; 4]> = match reader.read_outputs()? {
                    ReadOutputs::Translations(it) | ReadOutputs::Scales(it) => {
                        it.map(|v| [v[0], v[1], v[2], 0.0]).collect()
                    }
                    ReadOutputs::Rotations(it) => it.into_f32().collect(),
                    ReadOutputs::MorphTargetWeights(_) => return None,
                };
                let target = c.target();
                Some((
                    target.node().index(),
                    target.property(),
                    c.sampler().interpolation(),
                    times,
                    values,
                ))
            })
            .collect();
        // The clip's length spans every channel, morph weights included.
        let duration = animation
            .channels()
            .filter_map(|c| {
                c.reader(|b| Some(&buffers[b.index()]))
                    .read_inputs()?
                    .last()
            })
            .fold(0.0, f32::max);
        let t = if duration > 0.0 {
            seconds.rem_euclid(duration)
        } else {
            0.0
        };
        // Only animated nodes are decomposed: a static matrix with a zero
        // scale (a hidden prop) has no TRS form.
        let mut trs = vec![None; local.len()];
        for (node, property, interpolation, times, values) in channels {
            let rotation = property == Property::Rotation;
            let Some(v) = sample(interpolation, &times, &values, t, rotation) else {
                continue;
            };
            let (t, q, s) = trs[node].get_or_insert_with(|| {
                doc.nodes()
                    .nth(node)
                    .expect("channel targets a node")
                    .transform()
                    .decomposed()
            });
            match property {
                Property::Translation => *t = [v[0], v[1], v[2]],
                Property::Rotation => *q = v,
                Property::Scale => *s = [v[0], v[1], v[2]],
                Property::MorphTargetWeights => {}
            }
        }
        for (m, trs) in local.iter_mut().zip(trs) {
            if let Some((translation, rotation, scale)) = trs {
                *m = matrix(
                    gltf::scene::Transform::Decomposed {
                        translation,
                        rotation,
                        scale,
                    }
                    .matrix(),
                );
            }
        }
    }
    local
}

/// A channel's value at `t` (vec3s padded to four components), or `None` if
/// its keys are malformed. Cubic keys are `[in-tangent, value, out-tangent]`.
fn sample(
    interpolation: Interpolation,
    times: &[f32],
    values: &[[f32; 4]],
    t: f32,
    rotation: bool,
) -> Option<[f32; 4]> {
    let cubic = interpolation == Interpolation::CubicSpline;
    if times.is_empty() || values.len() != times.len() * if cubic { 3 } else { 1 } {
        return None;
    }
    let key = |k: usize| values[if cubic { 3 * k + 1 } else { k }];
    let i = times.partition_point(|&x| x <= t);
    let v = if i == 0 {
        key(0)
    } else if i == times.len() {
        key(i - 1)
    } else {
        // times[i0] <= t < times[i1], so dt > 0.
        let (i0, i1) = (i - 1, i);
        let dt = times[i1] - times[i0];
        let s = (t - times[i0]) / dt;
        match interpolation {
            Interpolation::Step => key(i0),
            Interpolation::Linear if rotation => slerp(key(i0), key(i1), s),
            Interpolation::Linear => {
                std::array::from_fn(|k| key(i0)[k] * (1.0 - s) + key(i1)[k] * s)
            }
            Interpolation::CubicSpline => {
                let (s2, s3) = (s * s, s * s * s);
                let (out0, in1) = (values[3 * i0 + 2], values[3 * i1]);
                std::array::from_fn(|k| {
                    (2.0 * s3 - 3.0 * s2 + 1.0) * key(i0)[k]
                        + (s3 - 2.0 * s2 + s) * dt * out0[k]
                        + (-2.0 * s3 + 3.0 * s2) * key(i1)[k]
                        + (s3 - s2) * dt * in1[k]
                })
            }
        }
    };
    if !rotation {
        return Some(v);
    }
    let len = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    (len > 0.0).then(|| v.map(|x| x / len))
}

/// Slerp of glTF `[x, y, z, w]` quaternions (cgmath takes the short arc).
fn slerp(a: [f32; 4], b: [f32; 4], s: f32) -> [f32; 4] {
    let q0 = Quaternion::new(a[3], a[0], a[1], a[2]);
    let q1 = Quaternion::new(b[3], b[0], b[1], b[2]);
    let q = q0.slerp(q1, s);
    [q.v.x, q.v.y, q.v.z, q.s]
}

/// Linear-blend skinning over every JOINTS_n/WEIGHTS_n set.
fn skin(
    joints: &[M4],
    sets: &[(Vec<[u16; 4]>, Vec<[f32; 4]>)],
    positions: &[[f64; 3]],
) -> Result<Vec<[f64; 3]>, String> {
    if sets
        .iter()
        .any(|(j, w)| j.len() != positions.len() || w.len() != positions.len())
    {
        return Err("a skinned mesh's joints and weights don't match its vertices".to_string());
    }
    positions
        .iter()
        .enumerate()
        .map(|(v, &p)| {
            let (mut acc, mut total) = ([0.0; 3], 0.0);
            for (js, ws) in sets {
                for k in 0..4 {
                    let w = f64::from(ws[v][k]);
                    if w <= 0.0 {
                        continue;
                    }
                    let m = joints
                        .get(js[v][k] as usize)
                        .ok_or("a skinned vertex names a joint its skin doesn't have")?;
                    for (a, q) in acc.iter_mut().zip(apply(m, p)) {
                        *a += q * w;
                    }
                    total += w;
                }
            }
            // Unweighted vertices stay where they were authored.
            Ok(if total > 0.0 {
                acc.map(|a| a / total)
            } else {
                p
            })
        })
        .collect()
}

fn matrix(m: [[f32; 4]; 4]) -> M4 {
    M4::from(m.map(|c| c.map(f64::from)))
}

fn apply(m: &M4, p: [f64; 3]) -> [f64; 3] {
    let q = *m * Vector4::new(p[0], p[1], p[2], 1.0);
    [q.x, q.y, q.z]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal GLB: one triangle mesh under a node with `node_extra` JSON
    /// (e.g. a translation), positions in metres.
    fn glb(positions: &[[f32; 3]], indices: &[u16], node_extra: &str) -> Vec<u8> {
        let mut bin: Vec<u8> = positions
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let index_offset = bin.len();
        bin.extend(indices.iter().flat_map(|i| i.to_le_bytes()));
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let lo: [f32; 3] =
            std::array::from_fn(|k| positions.iter().map(|p| p[k]).fold(f32::MAX, f32::min));
        let hi: [f32; 3] =
            std::array::from_fn(|k| positions.iter().map(|p| p[k]).fold(f32::MIN, f32::max));
        let mut json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
"nodes":[{{"mesh":0{node_extra}}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1}}]}}],
"buffers":[{{"byteLength":{len}}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":{pos_len}}},{{"buffer":0,"byteOffset":{index_offset},"byteLength":{idx_len}}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":{count},"type":"VEC3","min":{lo:?},"max":{hi:?}}},
{{"bufferView":1,"componentType":5123,"count":{icount},"type":"SCALAR"}}]}}"#,
            len = bin.len(),
            pos_len = positions.len() * 12,
            idx_len = indices.len() * 2,
            count = positions.len(),
            icount = indices.len(),
        );
        while json.len() % 4 != 0 {
            json.push(' ');
        }
        let mut out = Vec::new();
        out.extend(b"glTF");
        out.extend(2u32.to_le_bytes());
        out.extend(((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(json.as_bytes());
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(&bin);
        out
    }

    #[test]
    fn loads_node_transforms_and_moves_into_the_print_frame() {
        // A 1 m tall upright triangle (Y up), its node moved 5 m along X.
        let bytes = glb(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[0, 1, 2],
            r#","translation":[5.0,0.0,0.0]"#,
        );
        let mut mesh = load_gltf(&bytes, None).unwrap();
        assert_eq!(mesh.triangles.len(), 1);
        assert_eq!(mesh.vertices[1], [6.0, 0.0, 0.0]);
        to_print_frame(&mut mesh, 80.0).unwrap();
        let b = mesh.bounds().unwrap();
        // Y up turned Z up, scaled to 80 mm tall, centered, on the bed.
        assert!((b.max[2] - 80.0).abs() < 1e-9 && b.min[2] == 0.0);
        assert!((b.min[0] + b.max[0]).abs() < 1e-9);
    }

    #[test]
    fn rejects_files_it_cannot_use() {
        assert!(load_gltf(b"not a model", None)
            .unwrap_err()
            .contains("glTF"));
        let bytes = glb(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[0, 1, 2],
            "",
        );
        let pose = Pose {
            animation: None,
            seconds: 0.0,
        };
        assert!(load_gltf(&bytes, Some(pose))
            .unwrap_err()
            .contains("no animations"));
    }

    #[test]
    fn drops_triangles_with_repeated_indices() {
        // The second triangle is a strip-style join with no area.
        let bytes = glb(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [9.0, 9.0, 0.0],
            ],
            &[0, 1, 2, 2, 2, 3],
            "",
        );
        assert_eq!(load_gltf(&bytes, None).unwrap().triangles.len(), 1);
    }

    #[test]
    fn a_flat_model_has_no_height_to_scale() {
        // A card lying in the glTF ground plane (all Y = 0).
        let bytes = glb(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            &[0, 1, 2],
            "",
        );
        let mut mesh = load_gltf(&bytes, None).unwrap();
        assert!(to_print_frame(&mut mesh, 80.0)
            .unwrap_err()
            .contains("flat"));
    }

    #[test]
    fn samples_each_interpolation() {
        let times = [0.0, 2.0];
        let lin = [[0.0, 0.0, 0.0, 0.0], [4.0, 0.0, 0.0, 0.0]];
        assert_eq!(
            sample(Interpolation::Linear, &times, &lin, 1.0, false).unwrap()[0],
            2.0
        );
        assert_eq!(
            sample(Interpolation::Step, &times, &lin, 1.9, false).unwrap()[0],
            0.0
        );
        // Past the last key holds it.
        assert_eq!(
            sample(Interpolation::Linear, &times, &lin, 5.0, false).unwrap()[0],
            4.0
        );
        // Cubic keys [in, value, out]: zero tangents ease, so midway is the midpoint.
        let zero = [0.0; 4];
        let cubic = [
            zero,
            [0.0, 0.0, 0.0, 0.0],
            zero,
            zero,
            [4.0, 0.0, 0.0, 0.0],
            zero,
        ];
        assert_eq!(
            sample(Interpolation::CubicSpline, &times, &cubic, 1.0, false).unwrap()[0],
            2.0
        );
        // ...but a quarter of the way it lags linear (ease-in).
        let quarter = sample(Interpolation::CubicSpline, &times, &cubic, 0.5, false).unwrap()[0];
        assert!(quarter < 1.0, "{quarter}");
        assert!(sample(Interpolation::CubicSpline, &times, &lin, 1.0, false).is_none());
        // Rotations slerp along the short arc: identity to 180° about Z, halfway is 90°.
        let rot = [[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]];
        let q = sample(Interpolation::Linear, &times, &rot, 1.0, true).unwrap();
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!(
            (q[2] - half).abs() < 1e-6 && (q[3] - half).abs() < 1e-6,
            "{q:?}"
        );
    }
}
