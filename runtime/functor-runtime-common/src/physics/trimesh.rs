//! Triangle-mesh collision geometry (`Physics.trimesh`): validation, and the
//! rapier shape built once per mesh value.

use std::sync::OnceLock;

use rapier3d::prelude::*;
use serde::{Deserialize, Serialize};

/// Validated triangle-mesh data, shared as `Arc<TriMeshData>`. The compiled
/// BVH is built on first spawn and lives here, so every re-declaration,
/// keyframe and rewind holding this `Arc` reuses it.
// Deserialized only from the engine's own world snapshots, which hold data
// `new` already validated.
#[derive(Serialize, Deserialize)]
pub struct TriMeshData {
    positions: Vec<[f32; 3]>,
    /// Non-degenerate triangles only.
    triangles: Vec<[u32; 3]>,
    #[serde(skip)]
    compiled: OnceLock<SharedShape>,
}

impl TriMeshData {
    /// Validate and keep the non-degenerate triangles. Errors name the field.
    pub fn new(positions: Vec<[f32; 3]>, triangles: Vec<[u32; 3]>) -> Result<TriMeshData, String> {
        if let Some(v) = positions.iter().position(|p| !p.iter().all(|c| c.is_finite())) {
            return Err(format!(
                "positions: vertex {v} is not finite ({:?})",
                positions[v]
            ));
        }
        let mut kept = Vec::with_capacity(triangles.len());
        for (t, tri) in triangles.iter().enumerate() {
            if let Some(i) = tri.iter().find(|&&i| i as usize >= positions.len()) {
                return Err(format!(
                    "triangles: triangle {t} uses vertex {i}, but there are only {} vertices",
                    positions.len()
                ));
            }
            if !degenerate(tri.map(|i| positions[i as usize])) {
                kept.push(*tri);
            }
        }
        if kept.is_empty() {
            return Err(if triangles.is_empty() {
                "triangles: no triangles".to_string()
            } else {
                format!(
                    "triangles: no triangle has any area ({} given)",
                    triangles.len()
                )
            });
        }
        Ok(TriMeshData {
            positions,
            triangles: kept,
            compiled: OnceLock::new(),
        })
    }

    /// The compiled shape, built on first use.
    pub(crate) fn shape(&self) -> SharedShape {
        self.compiled
            .get_or_init(|| {
                shape(self.positions.clone(), self.triangles.clone())
                    .expect("TriMeshData::new rejects meshes rapier can't build")
            })
            .clone()
    }
}

/// Zero-area: a repeated vertex, collinear vertices, or a normal f32 can't
/// hold. Such a face has no normal, so it is dropped rather than left to
/// produce contacts. In f64 the cross product of collinear points is exactly
/// zero in any vertex order.
fn degenerate([a, b, c]: [[f32; 3]; 3]) -> bool {
    let edge = |p: [f32; 3]| [0, 1, 2].map(|i| p[i] as f64 - a[i] as f64);
    let (u, v) = (edge(b), edge(c));
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    !((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]) as f32).is_normal()
}

/// A trimesh shape with internal edges fixed, so a ball rolls over shared
/// edges without hopping. That also makes contacts one-sided (a face's back
/// lets bodies through); rays still hit both sides. `None` if rapier rejects
/// the mesh.
pub(crate) fn shape(positions: Vec<[f32; 3]>, triangles: Vec<[u32; 3]>) -> Option<SharedShape> {
    #[cfg(test)]
    BUILDS.with(|b| b.set(b.get() + 1));
    SharedShape::trimesh_with_flags(
        positions.into_iter().map(|[x, y, z]| Vector::new(x, y, z)).collect(),
        triangles,
        TriMeshFlags::FIX_INTERNAL_EDGES,
    )
    .ok()
}

// Per thread, so parallel tests count only their own builds.
#[cfg(test)]
thread_local! {
    static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Trimesh shapes built on this thread so far.
#[cfg(test)]
pub(crate) fn builds() -> usize {
    BUILDS.with(|b| b.get())
}

/// Identity first, so re-declaring the same value is one pointer compare.
impl PartialEq for TriMeshData {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
            || (self.positions == other.positions && self.triangles == other.triangles)
    }
}

impl std::fmt::Debug for TriMeshData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriMeshData")
            .field("vertices", &self.positions.len())
            .field("triangles", &self.triangles.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::physics::{
        remove_world, with_world, Body, PhysicsScene, Shape, SteppedPhysics, World,
        DEFAULT_WORLD, FIXED_DT,
    };

    /// A 10 m square at y = 0, two triangles.
    fn floor() -> Arc<TriMeshData> {
        let positions = vec![
            [-5.0, 0.0, -5.0],
            [5.0, 0.0, -5.0],
            [5.0, 0.0, 5.0],
            [-5.0, 0.0, 5.0],
        ];
        Arc::new(TriMeshData::new(positions, vec![[0, 2, 1], [0, 3, 2]]).unwrap())
    }

    /// A closed box of half-extent `h`, 12 triangles facing outward.
    fn closed_box(h: f32) -> Arc<TriMeshData> {
        let positions = (0..8)
            .map(|i| {
                let s = |bit| if i & bit == 0 { -h } else { h };
                [s(1), s(2), s(4)]
            })
            .collect();
        let quads = [
            [0, 1, 3, 2], // -z
            [4, 6, 7, 5], // +z
            [0, 4, 5, 1], // -y
            [2, 3, 7, 6], // +y
            [0, 2, 6, 4], // -x
            [1, 5, 7, 3], // +x
        ];
        let triangles = quads
            .iter()
            .flat_map(|[a, b, c, d]| [[*a, *c, *b], [*a, *d, *c]])
            .collect();
        Arc::new(TriMeshData::new(positions, triangles).unwrap())
    }

    fn trimesh(tag: &str, mesh: &Arc<TriMeshData>) -> Body {
        Body::fixed(tag.to_string(), Shape::TriMesh { mesh: mesh.clone() })
    }

    fn ball(y: f32) -> Body {
        Body::dynamic("ball".to_string(), Shape::Sphere { radius: 0.5 }).at([0.0, y, 0.0])
    }

    fn stepped(gravity: [f32; 3], bodies: Vec<Body>, steps: usize) -> World {
        let mut w = World::new(gravity);
        w.reconcile(&PhysicsScene::create(gravity, bodies));
        for _ in 0..steps {
            w.step_fixed();
        }
        w
    }

    #[test]
    fn a_ball_comes_to_rest_on_a_trimesh_floor() {
        let mut w = stepped([0.0, -9.81, 0.0], vec![trimesh("floor", &floor()), ball(3.0)], 240);
        let y = w.body_transform("ball").unwrap().0[1];
        let vy = w.body_velocity("ball").unwrap()[1];
        assert!((y - 0.5).abs() < 0.02, "rests on the floor: y={y}");
        assert!(vy.abs() < 0.05, "at rest: vy={vy}");
        // The contact reports the body's tag.
        let landed = w.take_events().into_iter().find(|e| e.started).expect("a contact");
        let mut pair = [landed.a, landed.b];
        pair.sort();
        assert_eq!(pair, ["ball", "floor"]);
    }

    #[test]
    fn a_ray_hits_the_trimesh_with_its_tag_point_and_normal() {
        let w = stepped([0.0; 3], vec![trimesh("floor", &floor())], 1);
        let hit = w.raycast([1.0, 4.0, 2.0], [0.0, -1.0, 0.0], 10.0).unwrap();
        assert_eq!(hit.tag, "floor");
        assert!((hit.position[1]).abs() < 1e-5, "{:?}", hit.position);
        assert!((hit.distance - 4.0).abs() < 1e-5, "{}", hit.distance);
        assert!((hit.normal[1].abs() - 1.0).abs() < 1e-5, "{:?}", hit.normal);
        // Rays hit both sides.
        let below = w.raycast([1.0, -4.0, 2.0], [0.0, 1.0, 0.0], 10.0).unwrap();
        assert!((below.distance - 4.0).abs() < 1e-5, "{}", below.distance);
        // Off the edge: a miss.
        assert!(w.raycast([6.0, 4.0, 0.0], [0.0, -1.0, 0.0], 10.0).is_none());
    }

    #[test]
    fn inside_a_closed_trimesh_rays_hit_the_far_wall_and_nothing_touches() {
        let w = stepped(
            [0.0; 3],
            vec![
                trimesh("room", &closed_box(1.0)),
                Body::dynamic("ball".to_string(), Shape::Sphere { radius: 0.3 }),
            ],
            0,
        );
        let mut w = w;
        for _ in 0..10 {
            w.step_fixed();
        }
        // A surface, not a solid: an interior ray reaches the wall 1 m away.
        let hit = w
            .raycast_excluding([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 10.0, Some("ball"))
            .unwrap();
        assert_eq!(hit.tag, "room");
        assert!((hit.distance - 1.0).abs() < 1e-5, "{}", hit.distance);
        // The ball floats untouched: no contact with walls it isn't touching.
        assert!(w.take_events().is_empty());
        let p = w.body_transform("ball").unwrap().0;
        assert!(p.iter().all(|c| c.abs() < 1e-6), "{p:?}");
    }

    #[test]
    fn at_and_rotation_move_the_collider() {
        // The floor turned 90° about Z is the wall x = 2.
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let wall = trimesh("wall", &floor())
            .at([2.0, 0.0, 0.0])
            .facing([0.0, 0.0, half, half]);
        let w = stepped([0.0; 3], vec![wall], 1);
        let hit = w.raycast([-3.0, 1.0, 1.0], [1.0, 0.0, 0.0], 10.0).unwrap();
        assert!((hit.position[0] - 2.0).abs() < 1e-5, "{:?}", hit.position);
        assert!(w.raycast([0.0, 4.0, 0.0], [0.0, -1.0, 0.0], 10.0).is_none());
    }

    #[test]
    fn a_ball_rolls_across_shared_edges_without_hopping() {
        // A 20 m strip of 1 m quads: 40 internal edges to cross.
        let positions = (0..=20)
            .flat_map(|i| [[i as f32 - 10.0, 0.0, -1.0], [i as f32 - 10.0, 0.0, 1.0]])
            .collect();
        let triangles = (0..20u32)
            .flat_map(|i| {
                let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
                [[a, b, d], [a, d, c]]
            })
            .collect();
        let strip = Arc::new(TriMeshData::new(positions, triangles).unwrap());
        let ball = Body::dynamic("ball".to_string(), Shape::Sphere { radius: 0.25 })
            .at([-9.0, 0.25, 0.0])
            .with_velocity([6.0, 0.0, 0.0]);
        let mut w = stepped([0.0, -9.81, 0.0], vec![trimesh("strip", &strip), ball], 0);
        let mut highest = 0.0f32;
        for _ in 0..150 {
            w.step_fixed();
            highest = highest.max(w.body_transform("ball").unwrap().0[1]);
        }
        let x = w.body_transform("ball").unwrap().0[0];
        assert!(x > 0.0, "the ball rolled along the strip: x={x}");
        assert!(highest < 0.25 + 0.01, "hopped on an internal edge: y={highest}");
    }

    /// The BVH is built once per mesh value: re-declaring it every frame, a
    /// keyframe, a rewind, a seek and a content-equal copy (a hot reload)
    /// reuse it. A rewind also lands bit-exact.
    #[test]
    fn redeclaring_keyframes_and_rewinds_build_the_collider_once() {
        remove_world(DEFAULT_WORLD);
        let mut sp = SteppedPhysics::new();
        let mesh = floor();
        let scene = |mesh: &Arc<TriMeshData>| {
            PhysicsScene::create([0.0, -9.81, 0.0], vec![trimesh("floor", mesh), ball(3.0)])
        };
        let snapshot = || with_world(DEFAULT_WORLD, |w| w.snapshot()).unwrap();
        let before = builds();

        let mut snap_20 = Vec::new();
        for t in 0..90 {
            if t == 20 {
                snap_20 = snapshot();
            }
            sp.advance(&scene(&mesh), FIXED_DT);
        }
        let end = snapshot();
        assert_eq!(builds() - before, 1, "90 frames, 3 keyframes");

        assert!(sp.seek_to_frame(5).is_empty());
        assert!(sp.seek_to_frame(60).is_empty());
        assert!(sp.rewind_to_frame(20).is_empty());
        assert!(snapshot() == snap_20, "a rewind restores bit-exact");
        for _ in 20..90 {
            sp.advance(&scene(&mesh), FIXED_DT);
        }
        assert!(snapshot() == end, "the branch replays bit-exact");

        // A hot reload re-evaluates the data into a new, equal value.
        let reloaded = floor();
        for _ in 0..5 {
            sp.advance(&scene(&reloaded), FIXED_DT);
        }
        assert_eq!(builds() - before, 1, "rewind, seek and reload reuse the build");
        remove_world(DEFAULT_WORLD);
    }

    /// Edge fixing makes contacts one-sided: a body meets a face from its
    /// front (counter-clockwise) side and passes through from behind.
    #[test]
    fn contacts_are_one_sided_and_rays_two_sided() {
        let rest = |gravity: f32, y: f32| {
            let w = stepped([0.0, gravity, 0.0], vec![trimesh("floor", &floor()), ball(y)], 240);
            w.body_transform("ball").unwrap().0[1]
        };
        assert!((rest(-9.81, 3.0) - 0.5).abs() < 0.02, "the front side holds");
        assert!(rest(9.81, -3.0) > 5.0, "the back side lets the ball through");
        // An outward-wound closed box holds a ball on its top.
        let w = stepped([0.0, -9.81, 0.0], vec![trimesh("box", &closed_box(1.0)), ball(3.0)], 240);
        let y = w.body_transform("ball").unwrap().0[1];
        assert!((y - 1.5).abs() < 0.02, "rests on the box: y={y}");
    }

    #[test]
    fn invalid_meshes_are_errors_naming_the_field() {
        let tri = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let err = |p: Vec<[f32; 3]>, t: Vec<[u32; 3]>| TriMeshData::new(p, t).unwrap_err();
        assert_eq!(
            err(tri.clone(), vec![[0, 1, 3]]),
            "triangles: triangle 0 uses vertex 3, but there are only 3 vertices"
        );
        assert_eq!(
            err(vec![[0.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0], [0.0, 0.0, 1.0]], vec![[0, 1, 2]]),
            "positions: vertex 1 is not finite ([NaN, 0.0, 0.0])"
        );
        assert_eq!(err(tri.clone(), vec![]), "triangles: no triangles");
        // A repeated index, and three collinear points.
        let line = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        assert_eq!(
            err(line, vec![[0, 1, 2], [0, 0, 1]]),
            "triangles: no triangle has any area (2 given)"
        );
        // Degenerate triangles beside a good one are dropped.
        let mesh = TriMeshData::new(tri, vec![[0, 1, 2], [0, 0, 1]]).unwrap();
        assert_eq!(mesh.triangles, vec![[0, 1, 2]]);
        // A sliver is kept in every vertex order.
        let sliver = vec![[0.0, 0.0, 0.0], [1000.0, 0.0, 0.0], [1000.0, 0.0, 0.0005]];
        for order in [[0, 1, 2], [1, 2, 0], [2, 0, 1]] {
            assert!(TriMeshData::new(sliver.clone(), vec![order]).is_ok(), "{order:?}");
        }
    }
}
