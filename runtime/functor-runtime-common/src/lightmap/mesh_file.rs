//! `functor bake`'s `.mesh.bin`: a static mesh's vertex data, triangles and
//! groups (without textures), read back by `Lightmap.readMesh`.
//!
//! Little-endian: `FLMB`, u32 version, vertex count, triangle count, group
//! count, flags (bit 0: has `uvs`); then positions and normals (3 × f32
//! each), uvs if flagged and uvs2 (2 × f32), triangles (3 × u32), and per
//! group `first`, `count`, `unlit` (u32).

use crate::model::{GroupSource, ModelSource};

const MAGIC: &[u8; 4] = b"FLMB";
const VERSION: u32 = 1;

pub fn write_mesh(mesh: &ModelSource) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    let u32s =
        |out: &mut Vec<u8>, words: &[u32]| words.iter().for_each(|w| out.extend(w.to_le_bytes()));
    let f32s =
        |out: &mut Vec<u8>, floats: &[f32]| floats.iter().for_each(|f| out.extend(f.to_le_bytes()));
    let counts = [
        mesh.positions.len(),
        mesh.triangles.len(),
        mesh.groups.len(),
    ]
    .map(|n| n as u32);
    u32s(
        &mut out,
        &[
            VERSION,
            counts[0],
            counts[1],
            counts[2],
            !mesh.uvs.is_empty() as u32,
        ],
    );
    f32s(&mut out, mesh.positions.as_flattened());
    f32s(&mut out, mesh.normals.as_flattened());
    f32s(&mut out, mesh.uvs.as_flattened());
    f32s(&mut out, mesh.uvs2.as_flattened());
    u32s(&mut out, mesh.triangles.as_flattened());
    for g in &mesh.groups {
        u32s(&mut out, &[g.first, g.count, g.unlit as u32]);
    }
    out
}

/// The mesh in `bytes`, or what is wrong with them. Not validated as a
/// model: `Model.static` does that.
pub fn read_mesh(bytes: &[u8]) -> Result<ModelSource, String> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let end = at
            .checked_add(n)
            .filter(|&end| end <= bytes.len())
            .ok_or_else(|| {
                format!(
                    "cut short: needs {n} bytes at byte {at}, has {}",
                    bytes.len() - at.min(bytes.len())
                )
            })?;
        let slice = &bytes[at..end];
        at = end;
        Ok(slice)
    };
    if take(4)? != MAGIC {
        return Err("not a baked mesh (no FLMB header)".to_string());
    }
    let mut word = || take(4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let version = word()?;
    if version != VERSION {
        return Err(format!(
            "version {version}; this engine reads version {VERSION}"
        ));
    }
    let [vertices, triangles, groups, flags] =
        [word()?, word()?, word()?, word()?].map(|n| n as usize);
    let mut f32s = |n: usize| -> Result<Vec<f32>, String> {
        let b = take(n.checked_mul(4).ok_or("too large")?)?;
        Ok(b.chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect())
    };
    let vec3s = |v: Vec<f32>| {
        v.chunks_exact(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect::<Vec<_>>()
    };
    let vec2s = |v: Vec<f32>| v.chunks_exact(2).map(|c| [c[0], c[1]]).collect::<Vec<_>>();
    let positions = vec3s(f32s(vertices * 3)?);
    let normals = vec3s(f32s(vertices * 3)?);
    let uvs = vec2s(f32s(if flags & 1 == 1 { vertices * 2 } else { 0 })?);
    let uvs2 = vec2s(f32s(vertices * 2)?);
    let mut u32s = |n: usize| -> Result<Vec<u32>, String> {
        let b = take(n.checked_mul(4).ok_or("too large")?)?;
        Ok(b.chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect())
    };
    let tris = u32s(triangles * 3)?
        .chunks_exact(3)
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    let groups = u32s(groups * 3)?
        .chunks_exact(3)
        .map(|g| GroupSource {
            first: g[0],
            count: g[1],
            texture: None,
            unlit: g[2] != 0,
        })
        .collect();
    if at != bytes.len() {
        return Err(format!(
            "{} bytes past the end of the mesh",
            bytes.len() - at
        ));
    }
    Ok(ModelSource {
        positions,
        normals,
        uvs,
        uvs2,
        joints: Vec::new(),
        weights: Vec::new(),
        triangles: tris,
        groups,
        lightmap: None,
        skeleton: Vec::new(),
        clips: Vec::new(),
        texture: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(uvs: bool) -> ModelSource {
        ModelSource {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.5, -2.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uvs: if uvs {
                vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
            } else {
                Vec::new()
            },
            uvs2: vec![[0.1, 0.2], [0.3, 0.4], [0.5, 0.75]],
            joints: Vec::new(),
            weights: Vec::new(),
            triangles: vec![[0, 1, 2], [2, 1, 0]],
            groups: vec![
                GroupSource {
                    first: 0,
                    count: 1,
                    texture: None,
                    unlit: false,
                },
                GroupSource {
                    first: 1,
                    count: 1,
                    texture: None,
                    unlit: true,
                },
            ],
            lightmap: None,
            skeleton: Vec::new(),
            clips: Vec::new(),
            texture: None,
        }
    }

    #[test]
    fn meshes_round_trip() {
        for uvs in [false, true] {
            let m = mesh(uvs);
            assert_eq!(read_mesh(&write_mesh(&m)).unwrap(), m);
        }
    }

    #[test]
    fn bad_bytes_say_what_is_wrong() {
        let bytes = write_mesh(&mesh(true));
        let err = |b: &[u8]| read_mesh(b).unwrap_err();
        assert_eq!(err(b"PNG!"), "not a baked mesh (no FLMB header)");
        assert!(
            err(&bytes[..bytes.len() - 1]).starts_with("cut short: needs 24 bytes at byte 168"),
            "{}",
            err(&bytes[..bytes.len() - 1])
        );
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(err(&longer), "1 bytes past the end of the mesh");
        let mut future = bytes;
        future[4] = 9;
        assert_eq!(err(&future), "version 9; this engine reads version 1");
    }
}
