//! glTF models as triangle soup for printing: flatten the default scene,
//! optionally posing skinned meshes at a moment of an animation, then move it
//! into the print frame (mm, Z up, on the bed).

use crate::Mesh;

type M4 = [[f64; 4]; 4]; // column-major: m[col][row]

const IDENTITY: M4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// A moment of an animation to pose skinned meshes at.
pub struct Pose<'a> {
    /// Animation name; `None` is the first animation.
    pub animation: Option<&'a str>,
    pub seconds: f32,
}

/// The model's triangles in its own units and axes (glTF: Y up). Only
/// self-contained files load (`.glb`, or `.gltf` with embedded buffers).
pub fn load_gltf(bytes: &[u8], pose: Option<Pose>) -> Result<Mesh, String> {
    let (doc, buffers, _) =
        gltf::import_slice(bytes).map_err(|e| format!("not a loadable glTF: {e}"))?;
    let animation = match &pose {
        None => None,
        Some(p) => Some(match p.animation {
            None => doc
                .animations()
                .next()
                .ok_or("the model has no animations to pose")?,
            Some(name) => doc
                .animations()
                .find(|a| a.name() == Some(name))
                .ok_or_else(|| format!("the model has no animation named {name:?}"))?,
        }),
    };
    let local = local_transforms(&doc, &buffers, animation.zip(pose.map(|p| p.seconds)));
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or("the model has no scene")?;

    // World transform per node, in scene order.
    let mut world = vec![IDENTITY; doc.nodes().len()];
    let mut order = Vec::new();
    let mut stack: Vec<(gltf::Node, M4)> = scene.nodes().map(|n| (n, IDENTITY)).collect();
    while let Some((node, parent)) = stack.pop() {
        let w = mul(&parent, &local[node.index()]);
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
                Some(it) => it.map(|m| m.map(|c| c.map(f64::from))).collect(),
                None => vec![IDENTITY; skin.joints().count()],
            };
            skin.joints()
                .zip(inverse_bind)
                .map(|(j, ib)| mul(&world[j.index()], &ib))
                .collect::<Vec<M4>>()
        });
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = prim.reader(|b| Some(&buffers[b.index()]));
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let positions: Vec<[f64; 3]> = positions.map(|p| p.map(f64::from)).collect();
            let base = out.vertices.len() as u32;
            match (&joints, reader.read_joints(0), reader.read_weights(0)) {
                (Some(jm), Some(js), Some(ws)) => {
                    for ((p, j), w) in positions.iter().zip(js.into_u16()).zip(ws.into_f32()) {
                        let total: f32 = w.iter().sum();
                        let mut acc = [0.0; 3];
                        for k in 0..4 {
                            if w[k] > 0.0 {
                                let q = apply(&jm[j[k] as usize], *p);
                                for (a, q) in acc.iter_mut().zip(q) {
                                    *a += q * f64::from(w[k] / total);
                                }
                            }
                        }
                        out.vertices.push(acc);
                    }
                }
                _ => {
                    let w = &world[node.index()];
                    out.vertices.extend(positions.iter().map(|&p| apply(w, p)));
                }
            }
            let indices: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            out.triangles.extend(
                indices
                    .chunks_exact(3)
                    .map(|t| [t[0] + base, t[1] + base, t[2] + base]),
            );
        }
    }
    if out.triangles.is_empty() {
        return Err("the model has no triangle meshes".to_string());
    }
    Ok(out)
}

/// Move a Y-up model into the print frame: Z up, scaled so it is `height` mm
/// tall, centered over the origin, resting on z = 0.
pub fn to_print_frame(mesh: &mut Mesh, height: f64) {
    for v in &mut mesh.vertices {
        *v = [v[0], -v[2], v[1]];
    }
    let Some(b) = mesh.bounds() else { return };
    let scale = height / (b.max[2] - b.min[2]);
    let center = [
        (b.min[0] + b.max[0]) / 2.0,
        (b.min[1] + b.max[1]) / 2.0,
        b.min[2],
    ];
    for v in &mut mesh.vertices {
        *v = std::array::from_fn(|i| (v[i] - center[i]) * scale);
    }
}

/// Local transform per node; nodes driven by `animation` are sampled at the
/// given time (linear interpolation; step and cubic keys treated as linear).
fn local_transforms(
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    animation: Option<(gltf::Animation, f32)>,
) -> Vec<M4> {
    use gltf::animation::util::ReadOutputs;
    let mut trs: Vec<([f32; 3], [f32; 4], [f32; 3])> =
        doc.nodes().map(|n| n.transform().decomposed()).collect();
    if let Some((animation, t)) = animation {
        for channel in animation.channels() {
            let reader = channel.reader(|b| Some(&buffers[b.index()]));
            let Some(times) = reader.read_inputs() else {
                continue;
            };
            let times: Vec<f32> = times.collect();
            if times.is_empty() {
                continue;
            }
            let i = times
                .partition_point(|&x| x <= t)
                .clamp(1, times.len().max(2) - 1);
            let (i0, i1) = (i - 1, i.min(times.len() - 1));
            let a = if times[i1] > times[i0] {
                ((t - times[i0]) / (times[i1] - times[i0])).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let lerp = |v: &[[f32; 3]]| -> [f32; 3] {
                std::array::from_fn(|k| v[i0][k] * (1.0 - a) + v[i1][k] * a)
            };
            let node = channel.target().node().index();
            match reader.read_outputs() {
                Some(ReadOutputs::Translations(it)) => trs[node].0 = lerp(&it.collect::<Vec<_>>()),
                Some(ReadOutputs::Scales(it)) => trs[node].2 = lerp(&it.collect::<Vec<_>>()),
                Some(ReadOutputs::Rotations(it)) => {
                    let q: Vec<[f32; 4]> = it.into_f32().collect();
                    // Shortest-arc normalized lerp.
                    let sign = if (0..4).map(|k| q[i0][k] * q[i1][k]).sum::<f32>() < 0.0 {
                        -1.0
                    } else {
                        1.0
                    };
                    let r: [f32; 4] =
                        std::array::from_fn(|k| q[i0][k] * (1.0 - a) + sign * q[i1][k] * a);
                    let len = r.iter().map(|x| x * x).sum::<f32>().sqrt();
                    trs[node].1 = r.map(|x| x / len);
                }
                _ => {}
            }
        }
    }
    trs.into_iter()
        .map(|(t, q, s)| {
            gltf::scene::Transform::Decomposed {
                translation: t,
                rotation: q,
                scale: s,
            }
            .matrix()
            .map(|c| c.map(f64::from))
        })
        .collect()
}

fn mul(a: &M4, b: &M4) -> M4 {
    std::array::from_fn(|c| std::array::from_fn(|r| (0..4).map(|k| a[k][r] * b[c][k]).sum()))
}

fn apply(m: &M4, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
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
        to_print_frame(&mut mesh, 80.0);
        let b = mesh.bounds().unwrap();
        // Y up became Z up, scaled to 80 mm tall, centered, on the bed.
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
}
