//! Scene-diff trajectory preview (docs/time-travel.md T6, the scene-diff
//! variant). Given a game's live frame plus its forward-simulated future frames,
//! find the scene nodes whose WORLD position changes across the sequence and
//! emit a trail of dots tracing each mover's path.
//!
//! The point is that this needs NO game cooperation: the runtime derives the
//! trajectory purely from what `draw` already renders. It diffs the rendered
//! *scene* (which carries concrete world transforms), not the opaque model — so
//! "which numbers are positions" is unambiguous (a node's world translation) and
//! "what moved" falls out of comparing those across the forward-sim.
//!
//! Pure and testable — no GPU, no interpreter needed (see the unit tests).

use std::collections::HashMap;

use cgmath::{InnerSpace, Matrix4, SquareMatrix, Vector3};

use crate::{MaterialDescription, Scene3D, SceneObject};

/// Walk `scene`, accumulating world transforms, and record each LEAF node's
/// world-space position keyed by its child-index path from the root. Node
/// identity across frames = this path — stable while `draw` keeps the same tree
/// shape (the common case, where only transforms vary frame to frame). A game
/// that conditionally changes the tree structure between frames will simply not
/// match those nodes across frames (they fall out of the trail), which is safe.
fn collect_positions(
    scene: &Scene3D,
    world: Matrix4<f32>,
    path: &mut Vec<usize>,
    out: &mut Vec<(Vec<usize>, Vector3<f32>)>,
) {
    let w = world * scene.xform;
    match &scene.obj {
        SceneObject::Group(children) | SceneObject::Material(_, children) => {
            for (i, child) in children.iter().enumerate() {
                path.push(i);
                collect_positions(child, w, path, out);
                path.pop();
            }
        }
        SceneObject::Geometry(_) | SceneObject::Model(_) => {
            // The 4th column of the accumulated matrix is the node origin's
            // world position.
            out.push((path.clone(), w.w.truncate()));
        }
    }
}

fn positions_by_path(scene: &Scene3D) -> HashMap<Vec<usize>, Vector3<f32>> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    collect_positions(scene, Matrix4::identity(), &mut path, &mut out);
    out.into_iter().collect()
}

/// A single dim emissive marker at a world position. The renderer applies a
/// node's `xform` on `Group`/`Geometry` but NOT on `Material` (the prelude only
/// ever puts transforms on Groups), so the world translation goes on an
/// enclosing Group — the size lives on the geometry leaf.
fn trail_dot(p: Vector3<f32>) -> Scene3D {
    let sphere = Scene3D::sphere().transform(Matrix4::from_scale(0.07));
    let material = Scene3D {
        obj: SceneObject::Material(
            MaterialDescription::emissive(0.25, 0.85, 1.0, 1.0),
            vec![sphere],
        ),
        xform: Matrix4::identity(),
    };
    Scene3D {
        obj: SceneObject::Group(vec![material]),
        xform: Matrix4::from_translation(p),
    }
}

/// Build a trail scene from a sequence of scenes (index 0 = current, the rest =
/// forward-simulated futures). A node earns a trail only if its world position
/// varies by more than `eps` across the sequence — so static geometry stays
/// clean and only movers get ghost dots. Returns `None` when nothing moved.
///
/// `max_step` guards against TELEPORTS: a forward-sim can reset/respawn a node
/// (e.g. a platformer character falling off the level snaps back to spawn), and
/// that discontinuity is not a trajectory. Each polyline is cut at the first
/// per-sample jump larger than `max_step`, so the preview traces the smooth path
/// up to the reset instead of drawing a straight streak across the snap-back.
pub fn trajectory_trail(scenes: &[&Scene3D], eps: f32, max_step: f32) -> Option<Scene3D> {
    if scenes.len() < 2 {
        return None;
    }
    let per_scene: Vec<_> = scenes.iter().map(|s| positions_by_path(s)).collect();
    let eps2 = eps * eps;
    let mut dots = Vec::new();
    // Identity set = the paths present in the current (index 0) scene.
    for (path, p0) in &per_scene[0] {
        let mut poly = vec![*p0];
        let mut present_everywhere = true;
        for m in &per_scene[1..] {
            match m.get(path) {
                Some(p) => poly.push(*p),
                None => {
                    present_everywhere = false;
                    break;
                }
            }
        }
        if !present_everywhere {
            continue;
        }
        // Cut at the first teleport (respawn/reset) — a trajectory is continuous.
        if let Some(cut) = (1..poly.len()).find(|&i| (poly[i] - poly[i - 1]).magnitude() > max_step)
        {
            poly.truncate(cut);
        }
        let moved = poly.iter().any(|p| (p - poly[0]).magnitude2() > eps2);
        if !moved {
            continue;
        }
        if std::env::var("TRAJ_DEBUG").is_ok() {
            let last = poly.len() - 1;
            let span = poly.iter().map(|p| (p - poly[0]).magnitude()).fold(0.0f32, f32::max);
            eprintln!(
                "[traj] path={:?} span={:.2} p0=({:.2},{:.2},{:.2}) plast=({:.2},{:.2},{:.2})",
                path, span,
                poly[0].x, poly[0].y, poly[0].z,
                poly[last].x, poly[last].y, poly[last].z,
            );
        }
        for p in &poly {
            dots.push(trail_dot(*p));
        }
    }
    if dots.is_empty() {
        None
    } else {
        Some(Scene3D {
            obj: SceneObject::Group(dots),
            xform: Matrix4::identity(),
        })
    }
}

/// Composite a derived trail onto a scene (the runtime's trajectory overlay).
pub fn overlay(scene: Scene3D, trail: Scene3D) -> Scene3D {
    Scene3D {
        obj: SceneObject::Group(vec![scene, trail]),
        xform: Matrix4::identity(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::vec3;

    fn ball_at(x: f32, y: f32) -> Scene3D {
        Scene3D::sphere().transform(Matrix4::from_translation(vec3(x, y, 0.0)))
    }

    // A group holding a mover (sphere 0) and a static sphere (sphere 1).
    fn frame(x: f32, y: f32) -> Scene3D {
        Scene3D {
            obj: SceneObject::Group(vec![ball_at(x, y), ball_at(5.0, 0.0)]),
            xform: Matrix4::identity(),
        }
    }

    #[test]
    fn moving_node_gets_a_trail_static_does_not() {
        let f0 = frame(0.0, 0.0);
        let f1 = frame(1.0, 1.0);
        let f2 = frame(2.0, 1.5);
        let trail = trajectory_trail(&[&f0, &f1, &f2], 0.05, 3.0).expect("a trail");
        // Only the mover contributes dots — one per frame (3); the static ball none.
        match trail.obj {
            SceneObject::Group(dots) => assert_eq!(dots.len(), 3),
            _ => panic!("expected a group of dots"),
        }
    }

    #[test]
    fn nothing_moving_yields_no_trail() {
        let s = frame(1.0, 1.0);
        assert!(trajectory_trail(&[&s, &s, &s], 0.05, 3.0).is_none());
    }

    #[test]
    fn dot_lands_at_the_movers_world_position() {
        // A mover nested under a translated group: world position must fold in
        // the parent transform (2 + 3 = 5 on x).
        let nested = |x: f32| Scene3D {
            obj: SceneObject::Group(vec![ball_at(x, 0.0)]),
            xform: Matrix4::from_translation(vec3(2.0, 0.0, 0.0)),
        };
        let a = nested(0.0);
        let b = nested(3.0);
        let trail = trajectory_trail(&[&a, &b], 0.05, 3.0).expect("a trail");
        let dots = match trail.obj {
            SceneObject::Group(d) => d,
            _ => panic!(),
        };
        // Second dot is the mover at frame b: world x = 2 (group) + 3 (local) = 5.
        let x = dots[1].xform.w.x;
        assert!((x - 5.0).abs() < 1e-4, "expected world x=5, got {x}");
    }

    #[test]
    fn trail_stops_at_a_teleport() {
        // A node steps smoothly (0 → 0.5 → 1.0) then RESPAWNS to a far position
        // (a mario-style reset). The trail must cover the smooth run only — 3
        // dots — and NOT draw the snap-back streak.
        let step = |x: f32| Scene3D {
            obj: SceneObject::Group(vec![ball_at(x, 0.0)]),
            xform: Matrix4::identity(),
        };
        let frames = [step(0.0), step(0.5), step(1.0), step(-6.0)];
        let refs: Vec<&Scene3D> = frames.iter().collect();
        let trail = trajectory_trail(&refs, 0.05, 3.0).expect("a trail");
        match trail.obj {
            SceneObject::Group(dots) => assert_eq!(dots.len(), 3, "teleport sample dropped"),
            _ => panic!(),
        }
    }
}
