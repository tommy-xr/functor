//! The fused room as colliders (`Physics.fusedRoom`): the pure half.
//!
//! The shell fuses the environment depth into chunk meshes
//! ([`crate::fused_mesh`]) and publishes them to the producer's thread
//! ([`publish_fused_room`]). A `Physics.fusedRoom` body resolves to the latest
//! publish when its scene is recorded, and `World::reconcile` turns each chunk
//! into one static triangle-mesh collider, rebuilding only the chunks
//! [`dirty_chunks`] reports.
//!
//! Floor snap: the fused floor ripples by a few millimetres, enough to keep a
//! resting ball twitching. [`floor_height`] finds the floor (the lowest level
//! surface with real area) and [`chunk_collider`] flattens the level
//! triangles near it onto that one height.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::fused_mesh::ChunkKey;
use crate::scene3d::MeshData;

/// A triangle counts as level when its normal is within ~18° of straight up.
const LEVEL_NORMAL_Y: f32 = 0.95;
/// Level triangles within this of the floor height snap onto it, metres.
const FLOOR_BAND: f32 = 0.03;
/// Level area a height needs to be the floor, m²: more than fusion noise or
/// a desk top, less than the floor seen from one spot. (Taking a big table
/// for the floor only flattens the table.)
const FLOOR_MIN_AREA: f32 = 1.0;
/// A newly detected floor within this of the current one keeps the current
/// one, metres, so re-meshing doesn't rebuild every floor chunk for a
/// sub-millimetre change.
const FLOOR_HYSTERESIS: f32 = 0.01;
/// Triangles smaller than this (m²) are dropped: rapier rejects degenerate ones.
const MIN_TRIANGLE_AREA: f32 = 1e-8;

/// The fused room's chunk meshes, in world space.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoomMeshes {
    #[serde(with = "pairs")]
    pub chunks: BTreeMap<ChunkKey, Arc<MeshData>>,
}

/// Same chunks, each the same `Arc` or the same triangles: cheap for the
/// unchanged room a body re-declares every frame, exact after a restore.
impl PartialEq for RoomMeshes {
    fn eq(&self, other: &Self) -> bool {
        self.chunks.len() == other.chunks.len()
            && self
                .chunks
                .iter()
                .zip(&other.chunks)
                .all(|((ka, a), (kb, b))| ka == kb && same_mesh(a, b))
    }
}

fn same_mesh(a: &Arc<MeshData>, b: &Arc<MeshData>) -> bool {
    Arc::ptr_eq(a, b) || a == b
}

thread_local! {
    // The latest fused room on this (the producer's) thread.
    static FUSED_ROOM: RefCell<Arc<RoomMeshes>> = RefCell::default();
}

/// Hand the shell's fused room to the producer's thread; `Physics.fusedRoom`
/// bodies see it from the next physics step.
pub fn publish_fused_room(room: Arc<RoomMeshes>) {
    FUSED_ROOM.with(|slot| *slot.borrow_mut() = room);
}

pub(crate) fn fused_room() -> Arc<RoomMeshes> {
    FUSED_ROOM.with(|slot| slot.borrow().clone())
}

/// One chunk's collider geometry.
#[derive(Debug, PartialEq)]
pub(crate) struct ChunkGeometry {
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

/// The floor height: the lowest height where level, upward triangles within
/// [`FLOOR_BAND`] cover [`FLOOR_MIN_AREA`]; their area-weighted mean height.
/// `None` until the floor has been seen that much.
pub(crate) fn floor_height(room: &RoomMeshes) -> Option<f32> {
    // (height, area) of each level, upward triangle, lowest first.
    let mut level: Vec<(f32, f32)> = room
        .chunks
        .values()
        .flat_map(|mesh| {
            mesh.triangles.iter().filter_map(|t| {
                let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
                let (normal, area) = normal_and_area(a, b, c)?;
                (normal[1] > LEVEL_NORMAL_Y).then_some(((a[1] + b[1] + c[1]) / 3.0, area))
            })
        })
        .collect();
    level.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Slide a band up from the lowest; the first with enough area is it.
    let (mut start, mut area) = (0, 0.0);
    for end in 0..level.len() {
        area += level[end].1;
        while level[end].0 - level[start].0 > FLOOR_BAND {
            area -= level[start].1;
            start += 1;
        }
        if area >= FLOOR_MIN_AREA {
            let band = &level[start..=end];
            let total: f32 = band.iter().map(|(_, a)| a).sum();
            return Some(band.iter().map(|(y, a)| y * a).sum::<f32>() / total);
        }
    }
    None
}

/// Keep `current` while `detected` is within [`FLOOR_HYSTERESIS`] of it.
pub(crate) fn settle_floor(current: Option<f32>, detected: Option<f32>) -> Option<f32> {
    match (current, detected) {
        (Some(c), Some(d)) if (c - d).abs() < FLOOR_HYSTERESIS => Some(c),
        _ => detected,
    }
}

/// `mesh` as collider geometry, with the level triangles within
/// [`FLOOR_BAND`] of `floor` flattened onto it; `None` if nothing is left.
pub(crate) fn chunk_collider(mesh: &MeshData, floor: Option<f32>) -> Option<ChunkGeometry> {
    let mut positions = mesh.positions.clone();
    if let Some(floor) = floor {
        // Snap whole triangles (all three vertices), so a shared vertex moves
        // once and its other triangles stay attached.
        let mut snap = vec![false; positions.len()];
        for t in &mesh.triangles {
            let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
            let near = [a, b, c].iter().all(|p| (p[1] - floor).abs() <= FLOOR_BAND);
            if near && normal_and_area(a, b, c).is_some_and(|(n, _)| n[1] > LEVEL_NORMAL_Y) {
                for &i in t {
                    snap[i as usize] = true;
                }
            }
        }
        for (p, snap) in positions.iter_mut().zip(snap) {
            if snap {
                p[1] = floor;
            }
        }
    }
    let triangles: Vec<[u32; 3]> = mesh
        .triangles
        .iter()
        .copied()
        .filter(|t| {
            let [a, b, c] = t.map(|i| positions[i as usize]);
            normal_and_area(a, b, c).is_some()
        })
        .collect();
    (!triangles.is_empty()).then_some(ChunkGeometry {
        positions,
        triangles,
    })
}

/// The chunks whose colliders must be rebuilt going from `(prev, prev_floor)`
/// to `(next, next_floor)`: added, removed or re-meshed chunks, and — when
/// the floor moved — every chunk with a vertex near either floor height.
pub(crate) fn dirty_chunks(
    prev: &RoomMeshes,
    prev_floor: Option<f32>,
    next: &RoomMeshes,
    next_floor: Option<f32>,
) -> Vec<ChunkKey> {
    let near_floor = |mesh: &MeshData| {
        mesh.positions.iter().any(|p| {
            [prev_floor, next_floor]
                .iter()
                .flatten()
                .any(|f| (p[1] - f).abs() <= FLOOR_BAND)
        })
    };
    let floor_moved = prev_floor != next_floor;
    let mut keys: Vec<ChunkKey> = prev
        .chunks
        .keys()
        .chain(next.chunks.keys())
        .copied()
        .collect();
    keys.sort();
    keys.dedup();
    keys.retain(|key| match (prev.chunks.get(key), next.chunks.get(key)) {
        (Some(a), Some(b)) => !same_mesh(a, b) || (floor_moved && near_floor(b)),
        _ => true,
    });
    keys
}

/// A triangle's unit normal (counter-clockwise front) and area; `None` when
/// degenerate.
fn normal_and_area(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Option<([f32; 3], f32)> {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let area = len / 2.0;
    (area > MIN_TRIANGLE_AREA).then(|| ([n[0] / len, n[1] / len, n[2] / len], area))
}

/// A `BTreeMap` keyed by arrays as a list of pairs: JSON keys must be
/// strings.
pub(crate) mod pairs {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<K: Serialize, V: Serialize, S: Serializer>(
        map: &BTreeMap<K, V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(map.iter())
    }

    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Ok(Vec::<(K, V)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A level `w` × `d` grid of 10 cm cells at height `y(x, z)`, facing up.
    fn level(x0: f32, z0: f32, w: f32, d: f32, y: impl Fn(f32, f32) -> f32) -> MeshData {
        let (nx, nz) = ((w / 0.1).round() as u32, (d / 0.1).round() as u32);
        let mut mesh = MeshData {
            positions: Vec::new(),
            triangles: Vec::new(),
        };
        for j in 0..=nz {
            for i in 0..=nx {
                let (x, z) = (x0 + i as f32 * 0.1, z0 + j as f32 * 0.1);
                mesh.positions.push([x, y(x, z), z]);
            }
        }
        let at = |i: u32, j: u32| j * (nx + 1) + i;
        for j in 0..nz {
            for i in 0..nx {
                // Counter-clockwise seen from above (+Y).
                mesh.triangles.push([at(i, j), at(i, j + 1), at(i + 1, j)]);
                mesh.triangles
                    .push([at(i + 1, j), at(i, j + 1), at(i + 1, j + 1)]);
            }
        }
        mesh
    }

    /// A vertical wall at `z`, 1 m wide, from `y0` up 1 m.
    fn wall(z: f32, y0: f32) -> MeshData {
        MeshData {
            positions: vec![
                [0.0, y0, z],
                [1.0, y0, z],
                [0.0, y0 + 1.0, z],
                [1.0, y0 + 1.0, z],
            ],
            triangles: vec![[0, 2, 1], [1, 2, 3]],
        }
    }

    fn room(chunks: Vec<MeshData>) -> RoomMeshes {
        RoomMeshes {
            chunks: chunks
                .into_iter()
                .enumerate()
                .map(|(i, mesh)| ([i as i32, 0, 0], Arc::new(mesh)))
                .collect(),
        }
    }

    /// A few millimetres of ripple, like the fused floor.
    fn ripple(x: f32, z: f32) -> f32 {
        0.004 * (x * 37.0).sin() * (z * 23.0).cos()
    }

    #[test]
    fn the_floor_is_the_lowest_level_surface_with_real_area() {
        let floor = level(-1.0, -1.0, 2.0, 2.0, |x, z| 0.01 + ripple(x, z));
        let desk = level(0.0, 0.0, 1.0, 0.6, |_, _| 0.75);
        // A speck of noise below the floor is not enough area.
        let speck = level(2.0, 2.0, 0.2, 0.2, |_, _| -0.3);
        let found = floor_height(&room(vec![desk.clone(), floor, speck, wall(1.0, 0.0)]))
            .expect("a floor");
        assert!((found - 0.01).abs() < 0.002, "{found}");
        // A desk alone is too small to be a floor.
        assert_eq!(floor_height(&room(vec![desk])), None);
    }

    #[test]
    fn floor_triangles_snap_flat_and_the_rest_stays() {
        let floor = level(-1.0, -1.0, 2.0, 2.0, ripple);
        let snapped = chunk_collider(&floor, Some(0.0)).unwrap();
        assert!(snapped.positions.iter().all(|p| p[1] == 0.0));
        assert_eq!(snapped.triangles, floor.triangles);

        let desk = level(0.0, 0.0, 1.0, 0.6, |x, z| 0.75 + ripple(x, z));
        assert_eq!(
            chunk_collider(&desk, Some(0.0)).unwrap().positions,
            desk.positions
        );
        // A wall standing on the floor keeps its shape: its triangles are not
        // level, so none of its vertices move.
        let w = wall(1.0, 0.002);
        assert_eq!(chunk_collider(&w, Some(0.0)).unwrap().positions, w.positions);
        assert_eq!(chunk_collider(&w, None).unwrap().positions, w.positions);
    }

    #[test]
    fn degenerate_triangles_are_dropped() {
        let mesh = MeshData {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            triangles: vec![[0, 1, 2]],
        };
        assert_eq!(chunk_collider(&mesh, None), None);
    }

    #[test]
    fn a_small_floor_change_keeps_the_floor() {
        assert_eq!(settle_floor(Some(0.0), Some(0.004)), Some(0.0));
        assert_eq!(settle_floor(Some(0.0), Some(0.02)), Some(0.02));
        assert_eq!(settle_floor(None, Some(0.004)), Some(0.004));
        assert_eq!(settle_floor(Some(0.0), None), None);
    }

    #[test]
    fn only_changed_chunks_are_dirty() {
        let floor = Arc::new(level(-1.0, -1.0, 2.0, 2.0, |_, _| 0.0));
        let high = Arc::new(wall(1.0, 1.5));
        let low = Arc::new(wall(2.0, 0.0));
        let prev = RoomMeshes {
            chunks: BTreeMap::from([
                ([0, 0, 0], floor.clone()),
                ([1, 0, 0], high.clone()),
                ([2, 0, 0], low.clone()),
            ]),
        };
        let at_floor = Some(0.0);
        assert!(dirty_chunks(&prev, at_floor, &prev, at_floor).is_empty());
        // The same triangles in a new allocation (a restored snapshot) are
        // not a change.
        let copied = RoomMeshes {
            chunks: BTreeMap::from([
                ([0, 0, 0], Arc::new((*floor).clone())),
                ([1, 0, 0], high.clone()),
                ([2, 0, 0], low.clone()),
            ]),
        };
        assert_eq!(copied, prev);
        assert!(dirty_chunks(&prev, at_floor, &copied, at_floor).is_empty());

        // Re-meshed, added and removed chunks are dirty.
        let mut next = prev.clone();
        next.chunks.insert([1, 0, 0], Arc::new(wall(1.1, 1.5)));
        next.chunks.insert([3, 0, 0], Arc::new(wall(3.0, 1.5)));
        next.chunks.remove(&[2, 0, 0]);
        assert_eq!(
            dirty_chunks(&prev, at_floor, &next, at_floor),
            vec![[1, 0, 0], [2, 0, 0], [3, 0, 0]]
        );

        // A moved floor rebuilds the chunks near it, not the high wall.
        assert_eq!(
            dirty_chunks(&prev, at_floor, &prev, Some(0.02)),
            vec![[0, 0, 0], [2, 0, 0]]
        );
    }

    #[test]
    fn room_meshes_serialize_as_pairs() {
        let room = room(vec![wall(1.0, 0.0)]);
        let json = serde_json::to_string(&room).unwrap();
        assert_eq!(serde_json::from_str::<RoomMeshes>(&json).unwrap(), room);
    }
}
