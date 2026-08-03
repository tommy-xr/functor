//! Frame-diff preview (docs/time-travel.md T6): given a game's live frame plus
//! its forward-simulated future frames, find the 3D or 2D scene nodes whose
//! WORLD transform changes across the sequence ("movers") and render their
//! future two ways:
//!
//! - a **trail** of arrowheads tracing each mover's path, each pointing along
//!   the local direction of travel (the clean-lines view), and
//! - a **scene-space strobe**: real-geometry copies of each mover at its future
//!   poses, color- or alpha-faded by age (the chronophotography view). Copies
//!   use the normal render path: independently faded, with no division cap,
//!   while the camera stays live.
//!
//! The point is that this needs NO game cooperation: the runtime derives
//! everything purely from what `draw` already renders. It diffs the rendered
//! 3D scene and each anchor sprite layer (all of which carry concrete world
//! transforms), not the opaque model — so "which numbers are positions" is
//! unambiguous and "what moved" falls out of comparing world transforms across
//! the forward-sim.
//!
//! Pure and testable — no GPU, no interpreter needed (see the unit tests). The
//! one host-facing entry point is [`frame_preview`]: one forward-sim, both
//! consumers.

use std::collections::BTreeMap;

use cgmath::{vec4, Deg, InnerSpace, Matrix4, Quaternion, Rad, SquareMatrix, Vector3, Vector4};

use crate::protocol::GameProducer;
use crate::{Camera2D, Frame, MaterialDescription, RecordedInput, Scene3D, SceneObject, Shape};

const TRAIL_RADIUS_3D: f32 = 0.07;
const TRAIL_REFERENCE_HEIGHT_2D: f32 = 13.5;

/// The recorded PAST's color — cyan, matching the scrubber rail's accent
/// (`#41d8e6`). See [`PreviewSide`].
pub const PAST_COLOR: [f32; 3] = [0.25, 0.85, 1.0];

/// The projected FUTURE's color — pink, matching the scrubber rail's future
/// segment (`#e858b8`). See [`PreviewSide`].
pub const FUTURE_COLOR: [f32; 3] = [0.906, 0.345, 0.722];

/// How much of the DIRECTION's color a strobe copy keeps versus the mover's
/// own (0 = the mover's color untouched, 1 = pure direction color).
///
/// Deliberately high. A strobe copy is real geometry in the mover's own
/// material, so a light tint leaves past and future copies of the same object
/// nearly identical — the two directions read as one muddy cloud, which is
/// exactly the failure a UX prototype of this surfaced. At 0.8 the direction
/// dominates and only a fifth of the source color survives, so a copy still
/// hints at what it is a copy OF while being unambiguously past or future.
pub const STROBE_TINT_WEIGHT: f32 = 0.8;

/// Which side of the playhead a preview overlay describes — the recorded past
/// or the projected future. The two sides render in different colors so a
/// bidirectional preview reads as two directions rather than one cloud.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PreviewSide {
    /// Reconstructed from the recorder's rings — fact.
    Past,
    /// Forward-simulated — projection.
    Future,
}

impl PreviewSide {
    /// The side's flat mark color, and the color its strobe copies tint toward.
    pub fn color(self) -> [f32; 3] {
        match self {
            PreviewSide::Past => PAST_COLOR,
            PreviewSide::Future => FUTURE_COLOR,
        }
    }
}

/// The arrowhead outline in its own XY space, pointing along +X: tip, the two
/// base corners, and a notched tail between them (a kite, wound CCW). Every
/// mark shares these exact points and carries its placement in the transform,
/// so on the GPU they all resolve to ONE cached polygon mesh — `PolygonMesh`
/// keys on POINT COUNT and skips the re-upload when the points already match.
/// Scene-side each mark still allocates its own point `Vec`; that is fine here
/// (preview-only, bounded by the sample count).
///
/// The point count is deliberately FOUR, not three. Sharing a cache slot only
/// pays off while the points agree, and slot 3 is exactly where a user's own
/// `Sprite.polygon` triangle would land — alternating draws would then each
/// re-upload via `buffer_sub_data`. A kite cannot collide with user triangles.
const ARROW_POINTS: [[f32; 2]; 4] = [[1.0, 0.0], [-0.4, 0.6], [-0.8, 0.0], [-0.4, -0.6]];

/// World size of one arrowhead per unit of trail radius: an arrow 1.8 units
/// long in [`ARROW_POINTS`] space becomes ~4.0 radii long and ~2.6 wide, about
/// the visual weight of the sphere it replaces.
const ARROW_SCALE: f32 = 2.2;

/// Radius of the mark's sphere core, in the arrow's own (already-scaled) frame.
/// Both flat heads CONTAIN the direction of travel, so a mark heading straight
/// at or away from the camera collapses to a pair of hairline crosses. The
/// core is the floor on that: an edge-on mark degrades to a dot — what the
/// trail drew before arrows — instead of disappearing.
const ARROW_CORE: f32 = 0.35;

/// Minimum local displacement, as a fraction of the mark radius, for a heading
/// to be meaningful. Below it the mover is effectively stationary at this
/// sample (a ball at the apex of its arc, a paused entity) and would point
/// somewhere arbitrary, so the mark falls back to the direction-free dot.
/// Relative to the radius so it scales with the 2D camera the same way the
/// marks do.
const HEADING_EPS_RADII: f32 = 0.25;

/// One step of a node path: (child index, sibling count). Node identity across
/// frames = the path of these segments from the root. Including the sibling
/// count means a structural change that alters a group's SIZE (a child spawning
/// or despawning mid-window) changes every sibling's key in that frame, so a
/// current-frame path stops matching there instead of silently resolving to a
/// shifted neighbor — a plain despawn TRUNCATES a trail rather than cross-wiring
/// it onto a different entity. Positional identity still can't distinguish
/// size-preserving changes (a removal plus an insertion within one sample
/// interval, or two siblings swapping list positions) — those can alias, and
/// the real fix is stable node ids, a known limit.
type PathSeg = (usize, usize);

/// A leaf node of the anchor scene: its world transform, the innermost
/// enclosing material, and the leaf object itself.
struct AnchorLeaf {
    world: Matrix4<f32>,
    material: Option<MaterialDescription>,
    leaf: SceneObject,
}

/// Walk a scene, accumulating world transforms, and record each leaf's matrix.
fn collect_transforms(
    scene: &Scene3D,
    world: Matrix4<f32>,
    path: &mut Vec<PathSeg>,
    out: &mut BTreeMap<Vec<PathSeg>, Matrix4<f32>>,
) {
    let w = world * scene.xform;
    match &scene.obj {
        SceneObject::Group(children)
        | SceneObject::Material(_, children)
        | SceneObject::Opacity(_, children) => {
            let count = children.len();
            for (i, child) in children.iter().enumerate() {
                path.push((i, count));
                collect_transforms(child, w, path, out);
                path.pop();
            }
        }
        SceneObject::Geometry(_) | SceneObject::Model(_) | SceneObject::Terrain(_) => {
            out.insert(path.clone(), w);
        }
    }
}

fn transforms_by_path(scene: &Scene3D) -> BTreeMap<Vec<PathSeg>, Matrix4<f32>> {
    let mut out = BTreeMap::new();
    let mut path = Vec::new();
    collect_transforms(scene, Matrix4::identity(), &mut path, &mut out);
    out
}

/// Walk the anchor scene and retain owned leaf/material data for public
/// [`MoverTrack`] values and the legacy 3D strobe.
fn collect_anchor(
    scene: &Scene3D,
    world: Matrix4<f32>,
    material: Option<&MaterialDescription>,
    path: &mut Vec<PathSeg>,
    out: &mut BTreeMap<Vec<PathSeg>, AnchorLeaf>,
) {
    let w = world * scene.xform;
    match &scene.obj {
        SceneObject::Group(children) | SceneObject::Opacity(_, children) => {
            let count = children.len();
            for (i, child) in children.iter().enumerate() {
                path.push((i, count));
                collect_anchor(child, w, material, path, out);
                path.pop();
            }
        }
        SceneObject::Material(mat, children) => {
            let count = children.len();
            for (i, child) in children.iter().enumerate() {
                path.push((i, count));
                collect_anchor(child, w, Some(mat), path, out);
                path.pop();
            }
        }
        SceneObject::Geometry(_) | SceneObject::Model(_) | SceneObject::Terrain(_) => {
            out.insert(
                path.clone(),
                AnchorLeaf {
                    world: w,
                    material: material.cloned(),
                    leaf: scene.obj.clone(),
                },
            );
        }
    }
}

fn anchor_leaves(scene: &Scene3D) -> BTreeMap<Vec<PathSeg>, AnchorLeaf> {
    let mut out = BTreeMap::new();
    let mut path = Vec::new();
    collect_anchor(scene, Matrix4::identity(), None, &mut path, &mut out);
    out
}

/// A mover identified by the scene diff: the leaf object, the material the
/// renderer shades it with (from the anchor frame), and its world transform at
/// each sampled frame — index 0 = the anchor, truncated at the first structural
/// mismatch or teleport. `translated` distinguishes movers whose world POSITION
/// changes from pure in-place spinners (rotation/scale only): the strobe
/// depicts both, but a trail of a spinner would just pile marks on one
/// spot, so the trail consumer skips them.
pub struct MoverTrack {
    pub leaf: SceneObject,
    pub material: Option<MaterialDescription>,
    pub worlds: Vec<Matrix4<f32>>,
    pub translated: bool,
}

#[derive(Clone, Copy)]
struct SampleLeaf<'a> {
    world: Matrix4<f32>,
    material: Option<&'a MaterialDescription>,
    leaf: &'a SceneObject,
}

struct SampledMoverTrack<'a> {
    samples: Vec<SampleLeaf<'a>>,
}

fn world_pos(w: &Matrix4<f32>) -> Vector3<f32> {
    // The 4th column of the accumulated matrix is the node origin's world
    // position.
    w.w.truncate()
}

/// Largest squared per-column delta between two transforms: the translation
/// column plus the three (scaled) basis vectors, so rotation and scale changes
/// register as movement too — for a unit-scale object, `eps` on a basis column
/// is roughly the sine of the rotation angle.
fn columns_delta2(a: &Matrix4<f32>, b: &Matrix4<f32>) -> f32 {
    let dx = (a.x - b.x).magnitude2();
    let dy = (a.y - b.y).magnitude2();
    let dz = (a.z - b.z).magnitude2();
    let dw = (a.w - b.w).magnitude2();
    dx.max(dy).max(dz).max(dw)
}

/// Diff a scene sequence (index 0 = current, the rest = forward-simulated
/// futures) into mover tracks — the shared core both the trail and the strobe
/// consume. A node earns a track only if its world position varies by more than
/// `eps` across the sequence, so static geometry contributes nothing.
///
/// `max_step` guards against TELEPORTS: a forward-sim can reset/respawn a node
/// (a platformer character falling off the level snaps back to spawn), and that
/// discontinuity is not a trajectory. Each track is cut at the first per-sample
/// jump larger than `max_step`, so the preview traces the smooth path up to the
/// reset instead of streaking across the snap-back. A path that stops matching
/// mid-window (despawn, or its group changed shape — see [`PathSeg`]) keeps its
/// track up to that sample.
pub fn mover_tracks(scenes: &[&Scene3D], eps: f32, max_step: f32) -> Vec<MoverTrack> {
    if scenes.len() < 2 {
        return Vec::new();
    }
    let anchor = anchor_leaves(scenes[0]);
    let futures: Vec<_> = scenes[1..].iter().map(|scene| transforms_by_path(scene)).collect();
    let eps2 = eps * eps;
    let mut tracks = Vec::new();
    for (path, anchor_leaf) in &anchor {
        let mut worlds = vec![anchor_leaf.world];
        for future in &futures {
            match future.get(path) {
                Some(world) => worlds.push(*world),
                None => break,
            }
        }
        // Cut at the first teleport (respawn/reset) — a trajectory is continuous.
        if let Some(cut) = (1..worlds.len())
            .find(|&i| (world_pos(&worlds[i]) - world_pos(&worlds[i - 1])).magnitude() > max_step)
        {
            worlds.truncate(cut);
        }
        let p0 = world_pos(&worlds[0]);
        let translated = worlds
            .iter()
            .any(|w| (world_pos(w) - p0).magnitude2() > eps2);
        // A mover is anything whose world TRANSFORM changes — translation, or
        // an in-place rotation/scale (which only the strobe can depict).
        let moved = translated
            || worlds
                .iter()
                .any(|w| columns_delta2(w, &worlds[0]) > eps2);
        if !moved {
            continue;
        }
        tracks.push(MoverTrack {
            leaf: anchor_leaf.leaf.clone(),
            material: anchor_leaf.material.clone(),
            worlds,
            translated,
        });
    }
    tracks
}

fn collect_sample_leaves<'a>(
    scene: &'a Scene3D,
    world: Matrix4<f32>,
    material: Option<&'a MaterialDescription>,
    path: &mut Vec<PathSeg>,
    out: &mut BTreeMap<Vec<PathSeg>, SampleLeaf<'a>>,
) {
    let w = world * scene.xform;
    match &scene.obj {
        SceneObject::Group(children) | SceneObject::Opacity(_, children) => {
            let count = children.len();
            for (i, child) in children.iter().enumerate() {
                path.push((i, count));
                collect_sample_leaves(child, w, material, path, out);
                path.pop();
            }
        }
        SceneObject::Material(next_material, children) => {
            let count = children.len();
            for (i, child) in children.iter().enumerate() {
                path.push((i, count));
                collect_sample_leaves(child, w, Some(next_material), path, out);
                path.pop();
            }
        }
        SceneObject::Geometry(_) | SceneObject::Model(_) | SceneObject::Terrain(_) => {
            out.insert(
                path.clone(),
                SampleLeaf {
                    world: w,
                    material,
                    leaf: &scene.obj,
                },
            );
        }
    }
}

fn sample_leaves_by_path(scene: &Scene3D) -> BTreeMap<Vec<PathSeg>, SampleLeaf<'_>> {
    let mut out = BTreeMap::new();
    let mut path = Vec::new();
    collect_sample_leaves(scene, Matrix4::identity(), None, &mut path, &mut out);
    out
}

fn sampled_mover_tracks<'a>(
    scenes: &[&'a Scene3D],
    eps: f32,
    max_step: f32,
) -> Vec<SampledMoverTrack<'a>> {
    if scenes.len() < 2 {
        return Vec::new();
    }
    let samples_by_path: Vec<_> = scenes
        .iter()
        .map(|scene| sample_leaves_by_path(scene))
        .collect();
    let anchor = &samples_by_path[0];
    let eps2 = eps * eps;
    let mut tracks = Vec::new();
    for (path, anchor_leaf) in anchor {
        let mut samples = vec![*anchor_leaf];
        for future in &samples_by_path[1..] {
            match future.get(path) {
                Some(leaf) => samples.push(*leaf),
                None => break,
            }
        }
        if let Some(cut) = (1..samples.len()).find(|&i| {
            (world_pos(&samples[i].world) - world_pos(&samples[i - 1].world)).magnitude() > max_step
        }) {
            samples.truncate(cut);
        }
        let moved = samples.iter().any(|sample| {
            columns_delta2(&sample.world, &samples[0].world) > eps2
        });
        if moved {
            tracks.push(SampledMoverTrack { samples });
        }
    }
    tracks
}

/// Which space a trail is drawn in — the marks are the same arrowhead, but a
/// sprite layer is viewed straight down -Z, so its mark must stay in the XY
/// plane, while a 3D mark has to read from any camera angle.
#[derive(Clone, Copy)]
enum TrailSpace {
    Scene3D,
    Sprite2D,
}

/// The flat emissive wrapper every trail mark shares, in its side's color
/// ([`PreviewSide::color`]). The renderer applies a
/// node's `xform` on `Group`/`Geometry` but NOT on `Material`, so placement
/// goes on the enclosing Group and the mark's own shape on the leaves.
fn trail_mark(marks: Vec<Scene3D>, place: Matrix4<f32>, color: [f32; 3]) -> Scene3D {
    let material = Scene3D {
        obj: SceneObject::Material(
            MaterialDescription::emissive(color[0], color[1], color[2], 1.0),
            marks,
        ),
        xform: Matrix4::identity(),
    };
    Scene3D {
        obj: SceneObject::Group(vec![material]),
        xform: place,
    }
}

/// A single dim emissive marker at a world position — the direction-free
/// fallback for a sample with no meaningful heading.
fn trail_dot(p: Vector3<f32>, radius: f32, color: [f32; 3]) -> Scene3D {
    let sphere = Scene3D::sphere().transform(Matrix4::from_scale(radius));
    trail_mark(vec![sphere], Matrix4::from_translation(p), color)
}

/// Rotation taking +X onto the unit vector `dir` by the SHORTEST arc. The
/// arrowhead is symmetric about its axis, so only the axis is load-bearing —
/// but the roll must not jump: picking a transverse basis from a helper axis
/// chosen by a threshold on `dir` makes the crossed heads snap through 90° as
/// a smooth heading crosses that threshold. The shortest arc varies
/// continuously everywhere except the antiparallel pole, where `from_arc`
/// takes the explicit fallback axis (any unit vector perpendicular to +X).
fn heading_rotation(dir: Vector3<f32>) -> Matrix4<f32> {
    Matrix4::from(Quaternion::from_arc(
        Vector3::unit_x(),
        dir,
        Some(Vector3::unit_z()),
    ))
}

/// A marker pointing along `dir` (which the caller has established is longer
/// than the heading threshold). In 3D the arrowhead is two flat heads crossed
/// at 90° about the direction of travel plus a sphere core, so it reads as an
/// arrow from most angles and as a dot edge-on rather than vanishing; a sprite
/// layer is viewed from one fixed side, so one flat head in the XY plane is
/// exactly right there and needs no core.
///
/// `None` when this space cannot orient the mark from `dir` — see the
/// `Sprite2D` arm — leaving the caller to draw the direction-free dot.
fn trail_arrow(
    p: Vector3<f32>,
    dir: Vector3<f32>,
    radius: f32,
    space: TrailSpace,
    color: [f32; 3],
) -> Option<Scene3D> {
    let head = Scene3D {
        obj: SceneObject::Geometry(Shape::ConvexPolygon {
            points: ARROW_POINTS.to_vec(),
        }),
        xform: Matrix4::identity(),
    };
    let (heads, rotation) = match space {
        TrailSpace::Scene3D => (
            vec![
                head.clone(),
                head.transform(Matrix4::from_angle_x(Deg(90.0))),
                Scene3D::sphere().transform(Matrix4::from_scale(ARROW_CORE)),
            ],
            heading_rotation(dir.normalize()),
        ),
        // Motion is in the layer's XY plane; spin the head about Z so it always
        // faces the 2D camera. Only x/y orient that spin, so the IN-PLANE
        // displacement is what has to clear the threshold: a (currently
        // unreachable — sprite lowering pins z = 0) mostly-z displacement would
        // pass a 3D-magnitude gate and then leave `atan2(~0, ~0)` pointing +X
        // arbitrarily. Guard the invariant rather than rely on the lowering.
        TrailSpace::Sprite2D => {
            let planar = dir.truncate();
            if planar.magnitude() <= radius * HEADING_EPS_RADII {
                return None;
            }
            (vec![head], Matrix4::from_angle_z(Rad(planar.y.atan2(planar.x))))
        }
    };
    Some(trail_mark(
        heads,
        Matrix4::from_translation(p) * rotation * Matrix4::from_scale(radius * ARROW_SCALE),
        color,
    ))
}

/// The local direction of travel at sample `i`: the segment between its
/// neighbors (central difference) where both exist, and the one-sided segment
/// at each end of the track. `None` when the displacement is too small to be a
/// heading — see [`HEADING_EPS_RADII`].
///
/// The threshold is compared PER STEP, not against the raw segment. An
/// interior sample spans two steps while the endpoints span one, so testing
/// both against the same epsilon would apply a 2x stricter rule at the ends —
/// a track moving at just under 2x the threshold would draw dot endpoints
/// around arrow interiors. Dividing by the span makes the mark form depend
/// only on the SPEED, so it is consistent along a constant-velocity track.
fn heading_at(worlds: &[Matrix4<f32>], i: usize, radius: f32) -> Option<Vector3<f32>> {
    let lo = i.saturating_sub(1);
    let hi = (i + 1).min(worlds.len() - 1);
    let dir = world_pos(&worlds[hi]) - world_pos(&worlds[lo]);
    let steps = (hi - lo).max(1) as f32;
    let eps = radius * HEADING_EPS_RADII;
    (dir.magnitude() / steps > eps).then_some(dir)
}

/// The 1-based future-sample index the strobe's `c`-th copy stands on, for a
/// track with `n_future` future samples and `count` copies: evenly spread,
/// always including the window's end. Shared by the strobe (to place copies)
/// and the trail (to stay OFF the strobe's cadence).
fn strobe_idx(c: usize, count: usize, n_future: usize) -> usize {
    (((c + 1) as f32 * n_future as f32 / count as f32).round() as usize).clamp(1, n_future)
}

fn trail_from_tracks(
    tracks: &[MoverTrack],
    strobe: Option<&StrobeOptions>,
    radius: f32,
    space: TrailSpace,
    side: PreviewSide,
) -> Option<Scene3D> {
    let color = side.color();
    let mut marks = Vec::new();
    for track in tracks {
        // A pure in-place spinner has a track (for the strobe) but no path to
        // mark — its marks would all land on one spot.
        if !track.translated {
            continue;
        }
        // Off-cadence with the strobe: skip the samples where a copy stands,
        // so marks fill the gaps BETWEEN copies instead of hiding under them.
        let n_future = track.worlds.len() - 1;
        let skip: Vec<usize> = match strobe {
            Some(s) if n_future > 0 && s.copies > 0 => {
                let count = s.copies.min(n_future);
                (0..count).map(|c| strobe_idx(c, count, n_future)).collect()
            }
            _ => Vec::new(),
        };
        for (i, w) in track.worlds.iter().enumerate() {
            if skip.contains(&i) {
                continue;
            }
            // Sample 0 is the ANCHOR, shared by both sides. The future side
            // owns it; marking it on the past side too would stack two
            // coplanar emissive marks in different colors on one spot
            // (z-fighting), which is exactly where the eye is looking.
            if i == 0 && side == PreviewSide::Past {
                continue;
            }
            let p = world_pos(w);
            marks.push(
                heading_at(&track.worlds, i, radius)
                    // Sample order runs AWAY from the anchor on both sides, so
                    // a past track's samples are ordered backwards in time.
                    // Negating there keeps every arrow pointing along the
                    // actual direction of travel, so past and future read as
                    // one continuous flow THROUGH the playhead instead of two
                    // trails pointing away from each other.
                    .map(|dir| match side {
                        PreviewSide::Past => -dir,
                        PreviewSide::Future => dir,
                    })
                    .and_then(|dir| trail_arrow(p, dir, radius, space, color))
                    .unwrap_or_else(|| trail_dot(p, radius, color)),
            );
        }
    }
    if marks.is_empty() {
        None
    } else {
        Some(Scene3D {
            obj: SceneObject::Group(marks),
            xform: Matrix4::identity(),
        })
    }
}

/// Build a trail scene of direction-of-travel arrowheads from a scene
/// sequence. Returns `None` when nothing moved. (The trail consumer of
/// [`mover_tracks`].)
pub fn trajectory_trail(scenes: &[&Scene3D], eps: f32, max_step: f32) -> Option<Scene3D> {
    trail_from_tracks(
        &mover_tracks(scenes, eps, max_step),
        None,
        TRAIL_RADIUS_3D,
        TrailSpace::Scene3D,
        PreviewSide::Future,
    )
}

/// Scene-space strobe options.
#[derive(Clone)]
pub struct StrobeOptions {
    /// Strobe copies per mover across the window (evenly sampled from its track).
    pub copies: usize,
    /// The color copies fade toward with age — pick the scene's background so
    /// far-future copies read as receding into it.
    pub fade_to: [f32; 3],
    /// Color retention at (nearest, farthest) future — e.g. `(0.8, 0.2)` draws
    /// the next moment at 80% of the mover's own color and the window's end at
    /// 20%.
    pub fade: (f32, f32),
    /// The DIRECTION tint: which side of the playhead these copies belong to,
    /// and how strongly its color overrides the mover's own
    /// ([`STROBE_TINT_WEIGHT`]). `None` leaves copies in the mover's own color
    /// — the single-direction behavior.
    pub tint: Option<([f32; 3], f32)>,
}

impl StrobeOptions {
    /// This side's copies: same cadence and age-fade, tinted toward the side's
    /// color so past and future copies of the SAME mover stay distinguishable.
    fn for_side(&self, side: PreviewSide) -> StrobeOptions {
        StrobeOptions {
            tint: Some((side.color(), STROBE_TINT_WEIGHT)),
            ..self.clone()
        }
    }
}

impl Default for StrobeOptions {
    fn default() -> Self {
        StrobeOptions {
            copies: 8,
            // The runtime's clear color (run.rs / web lib.rs) — overridden by
            // hosts whose scene has its own backdrop.
            fade_to: [0.1, 0.2, 0.3],
            fade: (0.8, 0.2),
            tint: None,
        }
    }
}

/// Lerp a material's color toward `to` keeping `k` of the original (k=1 → the
/// original color, k=0 → fully `to`). Textures/normal maps are kept — the tint
/// darkens them toward the background. A `Texture` material (no color channel)
/// becomes an emissive-tinted texture so it can fade at all. `None` (a bare
/// leaf with no enclosing material — typically a `Model`, which carries its own
/// internal materials) stays `None`: the copy renders at full fidelity, and
/// fading it needs a render-path tint (a known follow-up).
fn faded_material(
    material: Option<&MaterialDescription>,
    to: [f32; 3],
    k: f32,
    tint: Option<([f32; 3], f32)>,
) -> Option<MaterialDescription> {
    let lerp = |c: Vector4<f32>| {
        // The DIRECTION tint lands FIRST, so age-fading still runs from the
        // copy's actual painted color toward the background. See
        // [`STROBE_TINT_WEIGHT`] for why the tint has to dominate.
        let c = match tint {
            Some((t, w)) => vec4(
                c.x + (t[0] - c.x) * w,
                c.y + (t[1] - c.y) * w,
                c.z + (t[2] - c.z) * w,
                c.w,
            ),
            None => c,
        };
        vec4(
            to[0] + (c.x - to[0]) * k,
            to[1] + (c.y - to[1]) * k,
            to[2] + (c.z - to[2]) * k,
            c.w,
        )
    };
    match material {
        Some(MaterialDescription::Color(c)) => Some(MaterialDescription::Color(lerp(*c))),
        Some(MaterialDescription::Emissive { color, texture }) => {
            Some(MaterialDescription::Emissive {
                color: lerp(*color),
                texture: texture.clone(),
            })
        }
        Some(MaterialDescription::Lit {
            color,
            texture,
            normal_map,
        }) => Some(MaterialDescription::Lit {
            color: lerp(*color),
            texture: texture.clone(),
            normal_map: normal_map.clone(),
        }),
        Some(MaterialDescription::Texture(t)) => Some(MaterialDescription::Emissive {
            color: lerp(vec4(1.0, 1.0, 1.0, 1.0)),
            texture: Some(t.clone()),
        }),
        Some(MaterialDescription::SpriteTexture {
            color,
            texture,
            source_pixels,
            sampling,
        }) => Some(MaterialDescription::SpriteTexture {
            color: lerp(*color),
            texture: texture.clone(),
            source_pixels: *source_pixels,
            sampling: *sampling,
        }),
        None => None,
    }
}

/// Sprite layers already blend with straight alpha, so their onion skins fade
/// by opacity instead of tinting toward the 3D pass's clear color.
fn alpha_faded_material(
    material: Option<&MaterialDescription>,
    k: f32,
    tint: Option<([f32; 3], f32)>,
) -> Option<MaterialDescription> {
    let fade = |mut color: Vector4<f32>| {
        // Sprite copies fade by OPACITY, but they still have to say which
        // direction they belong to — so the direction tint applies here too.
        if let Some((t, w)) = tint {
            color.x += (t[0] - color.x) * w;
            color.y += (t[1] - color.y) * w;
            color.z += (t[2] - color.z) * w;
        }
        color.w *= k;
        color
    };
    match material {
        Some(MaterialDescription::Color(color)) => Some(MaterialDescription::Color(fade(*color))),
        Some(MaterialDescription::Emissive { color, texture }) => {
            Some(MaterialDescription::Emissive {
                color: fade(*color),
                texture: texture.clone(),
            })
        }
        Some(MaterialDescription::Lit {
            color,
            texture,
            normal_map,
        }) => Some(MaterialDescription::Lit {
            color: fade(*color),
            texture: texture.clone(),
            normal_map: normal_map.clone(),
        }),
        Some(MaterialDescription::Texture(texture)) => Some(MaterialDescription::Emissive {
            color: vec4(1.0, 1.0, 1.0, k),
            texture: Some(texture.clone()),
        }),
        Some(MaterialDescription::SpriteTexture {
            color,
            texture,
            source_pixels,
            sampling,
        }) => Some(MaterialDescription::SpriteTexture {
            color: fade(*color),
            texture: texture.clone(),
            source_pixels: *source_pixels,
            sampling: *sampling,
        }),
        None => None,
    }
}

#[derive(Clone, Copy)]
enum StrobeFade {
    Color,
    Alpha,
}

/// One strobe copy: the mover's leaf at a future world pose, shaded by its
/// (age-faded) material. Transforms go on a Group / the leaf itself — never on
/// a Material node, which the renderer ignores (see [`trail_mark`]).
#[allow(clippy::too_many_arguments)]
fn strobe_copy(
    leaf: &SceneObject,
    material: Option<&MaterialDescription>,
    world: Matrix4<f32>,
    fade_to: [f32; 3],
    k: f32,
    fade: StrobeFade,
    tint: Option<([f32; 3], f32)>,
) -> Scene3D {
    let leaf = Scene3D {
        obj: leaf.clone(),
        xform: Matrix4::identity(),
    };
    let material = match fade {
        StrobeFade::Color => faded_material(material, fade_to, k, tint),
        StrobeFade::Alpha => alpha_faded_material(material, k, tint),
    };
    match material {
        Some(mat) => Scene3D {
            obj: SceneObject::Group(vec![Scene3D {
                obj: SceneObject::Material(mat, vec![leaf]),
                xform: Matrix4::identity(),
            }]),
            xform: world,
        },
        None => Scene3D {
            obj: leaf.obj,
            xform: world,
        },
    }
}

fn strobe_age(idx: usize, n_future: usize, fade: (f32, f32)) -> f32 {
    let age = if n_future <= 1 {
        0.0
    } else {
        (idx - 1) as f32 / (n_future - 1) as f32
    };
    fade.0 + (fade.1 - fade.0) * age
}

/// Scene-space strobe: real-geometry copies of each mover at its future poses,
/// color-faded by age. Returns `None` when nothing moved. (The strobe consumer
/// of [`mover_tracks`].)
pub fn strobe_overlay(tracks: &[MoverTrack], opts: &StrobeOptions) -> Option<Scene3D> {
    let mut copies = Vec::new();
    for track in tracks {
        // Future poses only — the live mover is already in the frame.
        let n_future = track.worlds.len() - 1;
        if n_future == 0 || opts.copies == 0 {
            continue;
        }
        let count = opts.copies.min(n_future);
        for c in 0..count {
            // Evenly sample the future, always including the window's end.
            let idx = strobe_idx(c, count, n_future);
            // Age by TIME along the track (not copy index), so sparse strobes
            // fade the same way dense ones do. Normalized over the inclusive
            // endpoints so the nearest possible future really gets `fade.0`; a
            // single-sample track counts as near.
            let k = strobe_age(idx, n_future, opts.fade);
            copies.push(strobe_copy(
                &track.leaf,
                track.material.as_ref(),
                track.worlds[idx],
                opts.fade_to,
                k,
                StrobeFade::Color,
                opts.tint,
            ));
        }
    }
    if copies.is_empty() {
        None
    } else {
        Some(Scene3D {
            obj: SceneObject::Group(copies),
            xform: Matrix4::identity(),
        })
    }
}

fn sampled_strobe_overlay(
    tracks: &[SampledMoverTrack<'_>],
    opts: &StrobeOptions,
    fade: StrobeFade,
) -> Option<Scene3D> {
    let mut copies = Vec::new();
    for sampled in tracks {
        let n_future = sampled.samples.len() - 1;
        if n_future == 0 || opts.copies == 0 {
            continue;
        }
        let count = opts.copies.min(n_future);
        for c in 0..count {
            let idx = strobe_idx(c, count, n_future);
            let sample = &sampled.samples[idx];
            copies.push(strobe_copy(
                sample.leaf,
                sample.material,
                sample.world,
                opts.fade_to,
                strobe_age(idx, n_future, opts.fade),
                fade,
                opts.tint,
            ));
        }
    }
    if copies.is_empty() {
        None
    } else {
        Some(Scene3D {
            obj: SceneObject::Group(copies),
            xform: Matrix4::identity(),
        })
    }
}

/// Composite a derived overlay onto a scene in place. In-place so callers
/// overlaying every frame don't deep-clone the scene tree just to regroup it.
pub fn overlay(scene: &mut Scene3D, trail: Scene3D) {
    let prev = std::mem::replace(
        scene,
        Scene3D {
            obj: SceneObject::Group(Vec::new()),
            xform: Matrix4::identity(),
        },
    );
    *scene = Scene3D {
        obj: SceneObject::Group(vec![prev, trail]),
        xform: Matrix4::identity(),
    };
}

/// The interactive future-preview mode — what the scrubber (and the
/// `--trajectory`/`--strobe` launch flags, which seed it) ask the shell to
/// overlay: scene-diff overlays (trail / strobe / both) drawn as geometry on
/// the normal render path. Shared by both shells so the wire encoding and
/// cycle order match.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PreviewMode {
    #[default]
    Off,
    Trail,
    Strobe,
    Both,
}

impl PreviewMode {
    pub fn wants_trail(self) -> bool {
        matches!(self, PreviewMode::Trail | PreviewMode::Both)
    }
    pub fn wants_strobe(self) -> bool {
        matches!(self, PreviewMode::Strobe | PreviewMode::Both)
    }
    /// Any mode that forward-simulates (i.e. everything but `Off`) — the
    /// timeline's future pseudo-bar shows exactly when this is true.
    pub fn is_on(self) -> bool {
        self != PreviewMode::Off
    }
    pub fn label(self) -> &'static str {
        match self {
            PreviewMode::Off => "off",
            PreviewMode::Trail => "trail",
            PreviewMode::Strobe => "strobe",
            PreviewMode::Both => "both",
        }
    }
    /// Stable wire encoding for the wasm scrubber bridge (`window.__scrub`'s
    /// `setPreview({ mode })`); anything unknown is `Off`. Index 4 was the
    /// removed screen-space ghost compositor and now falls through to `Off`.
    pub fn from_index(i: u32) -> PreviewMode {
        match i {
            1 => PreviewMode::Trail,
            2 => PreviewMode::Strobe,
            3 => PreviewMode::Both,
            _ => PreviewMode::Off,
        }
    }
}

/// The render work selected by an interactive extrapolation control. Pause is
/// deliberately not an input: playing advances the projection anchor while
/// pausing freezes it. Catch-up seeks suppress the expensive dry run until the
/// requested frame arrives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InteractivePreview {
    pub trail: bool,
    pub strobe: bool,
}

pub fn interactive_preview(
    mode: PreviewMode,
    enabled: bool,
    catching_up: bool,
) -> InteractivePreview {
    if !enabled || catching_up {
        return InteractivePreview::default();
    }
    InteractivePreview {
        trail: mode.wants_trail(),
        strobe: mode.wants_strobe(),
    }
}

/// What [`frame_preview`] should compute.
pub struct PreviewOptions {
    /// Forward-sim divisions (samples). Not bound by the screen-space
    /// compositor's 8-target cap — this only reads scenes — so sample finely.
    pub divisions: usize,
    /// Seconds of future to project.
    pub window: f32,
    /// Movement threshold: ignore world-position jitter below this.
    pub eps: f32,
    /// Teleport threshold: cut a track at a per-sample jump beyond this.
    pub max_step: f32,
    /// Emit the arrowhead trail?
    pub trail: bool,
    /// Emit the scene-space strobe?
    pub strobe: Option<StrobeOptions>,
    /// Also emit the BACKWARD overlays — the recorded past, reconstructed from
    /// the producer's history rings (docs/time-travel.md T6e). `window` is
    /// SYMMETRIC: the same number of seconds back as forward.
    pub backward: bool,
}

/// The overlays derived for one scene tree: either the frame's 3D scene or one
/// ordered 2D sprite layer.
#[derive(Clone, Default)]
pub struct SceneOverlays {
    pub trail: Option<crate::Scene3D>,
    pub strobe: Option<crate::Scene3D>,
}

/// Backward-compatible name for the 3D-only preview returned by
/// [`scene_preview`].
pub type ScenePreview = SceneOverlays;

impl SceneOverlays {
    fn is_empty(&self) -> bool {
        self.trail.is_none() && self.strobe.is_none()
    }

    /// Fold another side's overlays in, so one [`FramePreview`] carries both
    /// directions. Same-kind overlays group together — each side already
    /// carries its own color, so the merged tree needs no further distinction.
    fn merge(&mut self, other: SceneOverlays) {
        fn join(into: &mut Option<Scene3D>, add: Option<Scene3D>) {
            let Some(add) = add else { return };
            match into.take() {
                Some(existing) => {
                    *into = Some(Scene3D {
                        obj: SceneObject::Group(vec![existing, add]),
                        xform: Matrix4::identity(),
                    })
                }
                None => *into = Some(add),
            }
        }
        join(&mut self.trail, other.trail);
        join(&mut self.strobe, other.strobe);
    }

    fn apply(&self, scene: &mut Scene3D, presence: f32) {
        let mut add = |part: Option<&Scene3D>| {
            let Some(part) = part else { return };
            let mut part = part.clone();
            // Exactly 1.0 must leave the overlay byte-identical to the
            // pre-ramp tree (goldens, `--fixed-time` captures), so the walk
            // only runs while the fade is actually in flight.
            if presence < 1.0 {
                scale_presence(&mut part, presence);
            }
            overlay(scene, part);
        };
        add(self.trail.as_ref());
        add(self.strobe.as_ref());
    }
}

/// Walk an overlay subtree and multiply every material's alpha by `presence`.
///
/// This is deliberately an APPLY-TIME transform on the per-frame clone, never a
/// build-time one: both shells cache a BUILT [`FramePreview`] downstream of the
/// forward sim, so folding presence into the builders would make every frame of
/// the fade a cache miss and re-run `ghost_frames`/`history_frames`. Presence is
/// therefore not part of [`PreviewOptions`] nor of either shell's cache key.
///
/// A leaf with no enclosing material — typically a `Model`, which carries its
/// own internal glTF materials — cannot fade, exactly as it cannot age-fade
/// today (see [`faded_material`]); it pops instead of easing.
fn scale_presence(scene: &mut Scene3D, presence: f32) {
    match &mut scene.obj {
        SceneObject::Material(material, items) => {
            // Scaling every color's alpha by k is EXACTLY the sprite onion-skin
            // fade with no direction tint, so reuse it rather than growing a
            // third copy of the five-variant material rewrite.
            //
            // INVARIANT this relies on: no overlay node carries a bare
            // `MaterialDescription::Texture`. Every overlay material is freshly
            // produced by `faded_material` / `alpha_faded_material` /
            // `trail_mark`, none of which emit one. It matters because
            // `alpha_faded_material` rewrites a bare `Texture` to `Emissive`,
            // which changes the lighting model — and `apply` skips this walk
            // entirely at presence 1.0, so such a node would render lit at 1.0
            // and emissive at 0.999: a pop at exactly the boundary the ramp
            // exists to remove.
            let faded = alpha_faded_material(Some(material), presence, None);
            if let Some(faded) = faded {
                *material = faded;
            }
            for item in items {
                scale_presence(item, presence);
            }
        }
        SceneObject::Group(items) => {
            for item in items {
                scale_presence(item, presence);
            }
        }
        // KNOWN LIMITATION: an overlay subtree can never actually CONTAIN an
        // `Opacity` node — `collect_anchor` / `collect_sample_leaves` keep only
        // each leaf and its innermost material, so `strobe_leaf` rebuilds copies
        // out of `Group` / `Material` / leaf and the authored `Scene.opacity`
        // wrapper is dropped. The consequence is that the extrapolation ghost
        // of a TRANSLUCENT object renders at the overlay's own fade rather than
        // at the object's authored alpha. Carrying the accumulated opacity into
        // `AnchorLeaf` / `SampleLeaf` is the fix when that matters; this arm
        // exists only so the match stays exhaustive and honest if it ever does.
        SceneObject::Opacity(_, items) => {
            for item in items {
                scale_presence(item, presence);
            }
        }
        SceneObject::Geometry(_) | SceneObject::Model(_) | SceneObject::Terrain(_) => {}
    }
}

/// How long the preview overlay takes to fade fully in or out when the
/// extrapolation toggle (🔮) flips — short enough to feel immediate, long
/// enough that the overlay arrives rather than pops.
pub const PRESENCE_RAMP_SECONDS: f32 = 0.28;

/// Advance the preview's linear presence PHASE toward `target` (1 = the preview
/// selects something, 0 = it does not) by `dt` seconds, so a full fade takes
/// [`PRESENCE_RAMP_SECONDS`] regardless of frame rate. The result is clamped to
/// `[0, 1]` and never overshoots, so a hitch (or a `dt` longer than the ramp)
/// lands exactly ON the target rather than past it.
///
/// The phase is linear; [`presence_ease`] shapes it into the alpha multiplier.
/// Keeping the STATE linear is what makes the ramp symmetric — flipping the
/// target mid-fade retraces the same curve backwards.
pub fn presence_step(presence: f32, target: f32, dt: f32) -> f32 {
    let presence = if presence.is_finite() {
        presence.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let target = target.clamp(0.0, 1.0);
    if !dt.is_finite() || dt <= 0.0 {
        return presence;
    }
    let step = dt / PRESENCE_RAMP_SECONDS;
    if target >= presence {
        (presence + step).min(target)
    } else {
        (presence - step).max(target)
    }
}

/// Smoothstep shaping of the linear phase — the alpha multiplier the overlay is
/// actually drawn at. Ease-in-out, so the fade has no visible start/stop edge,
/// and symmetric (`presence_ease(1 - p) == 1 - presence_ease(p)`), so fading out
/// looks like fading in played backwards.
pub fn presence_ease(phase: f32) -> f32 {
    let p = phase.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

/// The inverse of [`presence_ease`]: the phase that renders at `alpha`.
///
/// A tooling PIN (`--preview-presence`, `setPreview({ presence })`) names a
/// final alpha, but the runtime's state is a linear phase. Seeding the phase
/// through this on every pinned frame is what makes RELEASING a pin continuous —
/// the ease picks up from exactly the opacity that was on screen instead of
/// jumping to wherever a free-running phase had drifted.
pub fn presence_phase(alpha: f32) -> f32 {
    if !alpha.is_finite() {
        return 0.0;
    }
    let a = alpha.clamp(0.0, 1.0);
    // Closed-form inverse of the smoothstep cubic 3a² - 2a³.
    0.5 - (((1.0 - 2.0 * a).clamp(-1.0, 1.0)).asin() / 3.0).sin()
}

/// A computed preview for the complete render frame. Sprite overlays use the
/// anchor frame's layer order and camera. A future layer-count change truncates
/// every layer conservatively; same-count reordering remains a positional-
/// identity limitation until sprite layers have stable ids.
#[derive(Clone, Default)]
pub struct FramePreview {
    pub scene: SceneOverlays,
    pub sprite_layers: Vec<SceneOverlays>,
}

impl FramePreview {
    /// Fold another side's preview in. Sprite layers merge positionally; a
    /// side that derived a different layer count (a structural change inside
    /// its window) contributes only its 3D overlays, matching how
    /// [`Self::apply_all`] already refuses a mismatched layer list.
    fn merge(&mut self, other: FramePreview) {
        self.scene.merge(other.scene);
        if self.sprite_layers.len() != other.sprite_layers.len() {
            return;
        }
        for (layer, add) in self.sprite_layers.iter_mut().zip(other.sprite_layers) {
            layer.merge(add);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.scene.is_empty() && self.sprite_layers.iter().all(SceneOverlays::is_empty)
    }

    /// Add every requested scene-space overlay to a normal display frame, at
    /// full presence.
    pub fn apply_all(&self, frame: &mut Frame) {
        self.apply_all_with_presence(frame, 1.0);
    }

    /// Add every requested scene-space overlay to a normal display frame, with
    /// every overlay material's alpha multiplied by `presence` (0 = absent,
    /// 1 = fully drawn — the presence ramp, see [`presence_step`]).
    ///
    /// At `presence <= 0` NOTHING is applied: an alpha-0 overlay would still
    /// write depth and occlude the scene behind it, so a fully faded-out
    /// preview has to be absent from the tree, not transparent in it.
    pub fn apply_all_with_presence(&self, frame: &mut Frame, presence: f32) {
        if !(presence > 0.0) {
            return;
        }
        let presence = presence.min(1.0);
        self.scene.apply(&mut frame.scene, presence);
        if frame.sprite_layers.len() != self.sprite_layers.len() {
            return;
        }
        for (layer, overlays) in frame.sprite_layers.iter_mut().zip(&self.sprite_layers) {
            overlays.apply(&mut layer.scene, presence);
        }
    }
}

fn scene_overlays(
    scenes: &[&Scene3D],
    opts: &PreviewOptions,
    trail_radius: f32,
    side: PreviewSide,
) -> SceneOverlays {
    let tracks = mover_tracks(scenes, opts.eps, opts.max_step);
    SceneOverlays {
        trail: if opts.trail {
            // When the strobe draws too, the trail stays off its cadence.
            trail_from_tracks(
                &tracks,
                opts.strobe.as_ref(),
                trail_radius,
                TrailSpace::Scene3D,
                side,
            )
        } else {
            None
        },
        strobe: opts
            .strobe
            .as_ref()
            .and_then(|strobe| strobe_overlay(&tracks, &strobe.for_side(side))),
    }
}

fn sprite_scene_overlays(
    scenes: &[&Scene3D],
    opts: &PreviewOptions,
    trail_radius: f32,
    side: PreviewSide,
) -> SceneOverlays {
    let trail = if opts.trail {
        let tracks = mover_tracks(scenes, opts.eps, opts.max_step);
        trail_from_tracks(
            &tracks,
            opts.strobe.as_ref(),
            trail_radius,
            TrailSpace::Sprite2D,
            side,
        )
    } else {
        None
    };
    let strobe = opts.strobe.as_ref().and_then(|strobe| {
        let sampled = sampled_mover_tracks(scenes, opts.eps, opts.max_step);
        sampled_strobe_overlay(&sampled, &strobe.for_side(side), StrobeFade::Alpha)
    });
    SceneOverlays {
        trail,
        strobe,
    }
}

fn sprite_trail_radius(camera: &Camera2D) -> f32 {
    let visible_height = (camera.height / camera.zoom).abs();
    if visible_height.is_finite() && visible_height > 0.0 {
        TRAIL_RADIUS_3D * visible_height / TRAIL_REFERENCE_HEIGHT_2D
    } else {
        TRAIL_RADIUS_3D
    }
}

/// Compute the original 3D-only scene preview.
///
/// New render shells should prefer [`frame_preview`], which applies the same
/// analysis to the frame's sprite layers without changing this API.
pub fn scene_preview(
    game: &dyn GameProducer,
    anchor_scene: &Scene3D,
    start_tts: f64,
    script_inputs: Option<&[Vec<RecordedInput>]>,
    opts: &PreviewOptions,
) -> ScenePreview {
    let divisions = opts.divisions.max(1);
    let dt = opts.window / divisions as f32;
    let futures = game.ghost_frames(divisions, dt, start_tts, script_inputs);
    let mut scenes: Vec<&Scene3D> = vec![anchor_scene];
    scenes.extend(futures.iter().map(|(frame, _)| &frame.scene));
    scene_overlays(&scenes, opts, TRAIL_RADIUS_3D, PreviewSide::Future)
}

/// One side's overlays: diff the anchor against `others` — the frames walking
/// AWAY from the playhead on that side (future ghosts, or past reconstructions
/// nearest-first) — and build whichever overlays `opts` asks for, in the
/// side's color.
fn side_overlays(
    anchor: &Frame,
    others: &[&Frame],
    opts: &PreviewOptions,
    side: PreviewSide,
) -> FramePreview {
    let mut scenes = Vec::with_capacity(others.len() + 1);
    scenes.push(&anchor.scene);
    scenes.extend(others.iter().map(|frame| &frame.scene));

    let matching_futures: Vec<_> = others
        .iter()
        .take_while(|frame| frame.sprite_layers.len() == anchor.sprite_layers.len())
        .copied()
        .collect();
    let sprite_layers = anchor
        .sprite_layers
        .iter()
        .enumerate()
        .map(|(index, anchor_layer)| {
            let mut scenes = Vec::with_capacity(others.len() + 1);
            scenes.push(&anchor_layer.scene);
            for future in &matching_futures {
                scenes.push(&future.sprite_layers[index].scene);
            }
            sprite_scene_overlays(
                &scenes,
                opts,
                sprite_trail_radius(&anchor_layer.camera),
                side,
            )
        })
        .collect();

    FramePreview {
        scene: scene_overlays(&scenes, opts, TRAIL_RADIUS_3D, side),
        sprite_layers,
    }
}

/// The SHARED composition step both shells call (desktop `run.rs`; web
/// `lib.rs`): run ONE forward-sim via the producer's `ghost_frames`, diff the
/// frame's 3D scene and sprite-layer scenes into mover tracks, and build
/// whichever overlays `opts` asks for.
/// `script_inputs` follows `ghost_frames`' contract (docs/time-travel.md F2) —
/// the caller builds the slice, since only the shell knows its script and
/// anchor convention.
pub fn frame_preview(
    game: &dyn GameProducer,
    anchor: &Frame,
    start_tts: f64,
    script_inputs: Option<&[Vec<RecordedInput>]>,
    opts: &PreviewOptions,
) -> FramePreview {
    let divisions = opts.divisions.max(1);
    let dt = opts.window / divisions as f32;
    let futures = game.ghost_frames(divisions, dt, start_tts, script_inputs);
    let future_frames: Vec<&Frame> = futures.iter().map(|(frame, _)| frame).collect();
    let mut preview = side_overlays(anchor, &future_frames, opts, PreviewSide::Future);
    if opts.backward {
        // The same window, mirrored: `history_frames` reconstructs rather than
        // simulates, and returns nearest-past-first — the same
        // walking-away-from-the-anchor order the ghosts use, so the identical
        // diff produces the backward tracks.
        let past = game.history_frames(divisions, dt);
        let past_frames: Vec<&Frame> = past.iter().map(|(frame, _)| frame).collect();
        preview.merge(side_overlays(anchor, &past_frames, opts, PreviewSide::Past));
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, Camera2D, SpriteLayer, SpriteSampling, TextureDescription};
    use cgmath::vec3;

    /// The FUTURE side's overlays for an explicit frame sequence — the shape
    /// these tests were written against, before the preview grew a second
    /// direction.
    fn preview_from_frames(anchor: &Frame, futures: &[&Frame], opts: &PreviewOptions) -> FramePreview {
        side_overlays(anchor, futures, opts, PreviewSide::Future)
    }

    fn ball_at(x: f32, y: f32) -> Scene3D {
        Scene3D::sphere().transform(Matrix4::from_translation(vec3(x, y, 0.0)))
    }

    #[test]
    fn interactive_preview_is_enabled_by_the_toggle_not_by_pause() {
        assert_eq!(
            interactive_preview(PreviewMode::Both, true, false),
            InteractivePreview {
                trail: true,
                strobe: true,
            }
        );
        assert_eq!(
            interactive_preview(PreviewMode::Trail, true, false),
            InteractivePreview {
                trail: true,
                strobe: false,
            }
        );
        assert_eq!(
            interactive_preview(PreviewMode::Both, false, false),
            InteractivePreview::default()
        );
        assert_eq!(
            interactive_preview(PreviewMode::Both, true, true),
            InteractivePreview::default()
        );
    }

    // A group holding a mover (sphere 0) and a static sphere (sphere 1).
    fn frame(x: f32, y: f32) -> Scene3D {
        Scene3D {
            obj: SceneObject::Group(vec![ball_at(x, y), ball_at(5.0, 0.0)]),
            xform: Matrix4::identity(),
        }
    }

    fn rendered_frame_with_sprite(x: f32, camera_x: f32) -> Frame {
        let mut rendered = Frame::new(Camera::default(), Scene3D::cube());
        rendered.sprite_layers.push(SpriteLayer {
            camera: Camera2D::new(24.0, 13.5).with_center(camera_x, 0.0),
            scene: frame(x, 0.0),
        });
        rendered
    }

    fn rendered_frame_with_colored_sprite(x: f32, color: [f32; 3]) -> Frame {
        let sprite = Scene3D {
            obj: SceneObject::Material(
                MaterialDescription::emissive(color[0], color[1], color[2], 1.0),
                vec![ball_at(x, 0.0)],
            ),
            xform: Matrix4::identity(),
        };
        let mut rendered = Frame::new(Camera::default(), Scene3D::cube());
        rendered.sprite_layers.push(SpriteLayer {
            camera: Camera2D::new(24.0, 13.5),
            scene: sprite,
        });
        rendered
    }

    fn preview_options(trail: bool, strobe: bool) -> PreviewOptions {
        PreviewOptions {
            divisions: 4,
            window: 1.0,
            eps: 0.05,
            max_step: 3.0,
            trail,
            strobe: strobe.then(|| StrobeOptions {
                copies: 2,
                ..Default::default()
            }),
            backward: false,
        }
    }

    #[test]
    fn frame_preview_builds_trails_and_strobes_for_sprite_layers() {
        let frames: Vec<Frame> = (0..=4)
            .map(|i| rendered_frame_with_sprite(i as f32 * 0.5, 0.0))
            .collect();
        let futures: Vec<&Frame> = frames.iter().skip(1).collect();
        let preview = preview_from_frames(&frames[0], &futures, &preview_options(true, true));

        assert!(
            preview.scene.is_empty(),
            "the static 3D scene stays untouched"
        );
        assert_eq!(preview.sprite_layers.len(), 1);
        let layer = &preview.sprite_layers[0];
        let trail = layer.trail.as_ref().expect("the 2D mover gets a trail");
        let strobe = layer
            .strobe
            .as_ref()
            .expect("the 2D mover gets strobe copies");
        match &trail.obj {
            // Five samples, with strobe copies on future samples 2 and 4:
            // anchor + samples 1 and 3 remain as dots.
            SceneObject::Group(dots) => assert_eq!(dots.len(), 3),
            _ => panic!("expected a group of trail dots"),
        }
        match &strobe.obj {
            SceneObject::Group(copies) => assert_eq!(copies.len(), 2),
            _ => panic!("expected a group of strobe copies"),
        }

        let mut displayed = frames[0].clone();
        preview.apply_all(&mut displayed);
        match &displayed.sprite_layers[0].scene.obj {
            SceneObject::Group(children) => assert!(
                matches!(children[0].obj, SceneObject::Group(_)),
                "trail and strobe overlays wrap the original sprite-layer group"
            ),
            _ => panic!("expected the composed sprite layer to remain a group"),
        }
    }

    #[test]
    fn camera_motion_alone_does_not_make_a_sprite_trajectory() {
        let anchor = rendered_frame_with_sprite(0.0, 0.0);
        let future = rendered_frame_with_sprite(0.0, 5.0);
        let preview = preview_from_frames(&anchor, &[&future], &preview_options(true, true));

        assert!(
            preview.is_empty(),
            "scene-space overlays use the anchor Camera2D, so camera motion alone moves nothing"
        );
    }

    #[test]
    fn a_missing_future_sprite_layer_truncates_its_tracks() {
        let anchor = rendered_frame_with_sprite(0.0, 0.0);
        let next = rendered_frame_with_sprite(0.5, 0.0);
        let without_layer = Frame::new(Camera::default(), Scene3D::cube());
        let preview = preview_from_frames(
            &anchor,
            &[&next, &without_layer],
            &preview_options(true, false),
        );
        let trail = preview.sprite_layers[0]
            .trail
            .as_ref()
            .expect("the matching prefix still draws");
        match &trail.obj {
            SceneObject::Group(dots) => assert_eq!(dots.len(), 2),
            _ => panic!("expected a group of prefix dots"),
        }
    }

    #[test]
    fn a_non_tail_sprite_layer_removal_never_cross_wires() {
        let mut anchor = rendered_frame_with_sprite(0.0, 0.0);
        anchor
            .sprite_layers
            .push(rendered_frame_with_sprite(8.0, 0.0).sprite_layers.remove(0));
        let next = rendered_frame_with_sprite(8.5, 0.0);
        let preview = preview_from_frames(&anchor, &[&next], &preview_options(true, true));

        assert!(
            preview.is_empty(),
            "a layer-count change must not match anchor world layer 0 to future HUD layer 0"
        );
    }

    #[test]
    fn sprite_strobes_use_future_materials_and_alpha_fade() {
        let frames = [
            rendered_frame_with_colored_sprite(0.0, [1.0, 0.0, 0.0]),
            rendered_frame_with_colored_sprite(0.5, [0.0, 1.0, 0.0]),
            rendered_frame_with_colored_sprite(1.0, [0.0, 0.0, 1.0]),
        ];
        let preview = preview_from_frames(
            &frames[0],
            &[&frames[1], &frames[2]],
            &preview_options(false, true),
        );
        let copies = match &preview.sprite_layers[0]
            .strobe
            .as_ref()
            .expect("sprite strobe")
            .obj
        {
            SceneObject::Group(copies) => copies,
            _ => panic!("expected strobe-copy group"),
        };
        let colors: Vec<_> = copies
            .iter()
            .map(|copy| match &copy.obj {
                SceneObject::Group(children) => match &children[0].obj {
                    SceneObject::Material(MaterialDescription::Emissive { color, .. }, _) => *color,
                    _ => panic!("expected emissive future material"),
                },
                _ => panic!("expected wrapped strobe copy"),
            })
            .collect();
        assert_eq!(colors.len(), 2);
        // Each copy still carries ITS OWN future frame's material — but tinted
        // toward the side color, which dominates (`STROBE_TINT_WEIGHT`), so the
        // direction is legible before the object is. The residual fraction of
        // the source color is what keeps the green and blue copies distinct.
        let tinted = |c: [f32; 3]| {
            let w = STROBE_TINT_WEIGHT;
            let f = FUTURE_COLOR;
            vec3(
                c[0] + (f[0] - c[0]) * w,
                c[1] + (f[1] - c[1]) * w,
                c[2] + (f[2] - c[2]) * w,
            )
        };
        assert_eq!(colors[0].truncate(), tinted([0.0, 1.0, 0.0]));
        assert_eq!(colors[1].truncate(), tinted([0.0, 0.0, 1.0]));
        assert_ne!(colors[0].truncate(), colors[1].truncate());
        // Alpha still carries the age fade, untouched by the tint.
        assert!((colors[0].w - 0.8).abs() < 1e-4);
        assert!((colors[1].w - 0.2).abs() < 1e-4);
    }

    #[test]
    fn sprite_strobes_preserve_atlas_regions_and_sampling() {
        let material = MaterialDescription::SpriteTexture {
            color: vec4(0.25, 0.5, 0.75, 0.8),
            texture: TextureDescription::FileClamped("hero-atlas.png".to_string()),
            source_pixels: Some([96.0, 0.0, 96.0, 96.0]),
            sampling: SpriteSampling::Nearest,
        };

        match alpha_faded_material(Some(&material), 0.25, None) {
            Some(MaterialDescription::SpriteTexture {
                color,
                texture,
                source_pixels,
                sampling,
            }) => {
                assert_eq!(color, vec4(0.25, 0.5, 0.75, 0.2));
                assert!(matches!(
                    texture,
                    TextureDescription::FileClamped(path) if path == "hero-atlas.png"
                ));
                assert_eq!(source_pixels, Some([96.0, 0.0, 96.0, 96.0]));
                assert_eq!(sampling, SpriteSampling::Nearest);
            }
            _ => panic!("expected a faded sprite texture"),
        }
    }

    #[test]
    fn sprite_trail_radius_tracks_camera_world_scale() {
        let platformer_scale = sprite_trail_radius(&Camera2D::new(24.0, 13.5));
        let pixel_scale = sprite_trail_radius(&Camera2D::new(320.0, 180.0));
        assert!((platformer_scale - TRAIL_RADIUS_3D).abs() < 1e-6);
        assert!((pixel_scale / platformer_scale - 180.0 / 13.5).abs() < 1e-4);
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

    // The marks of a trail, in track order.
    fn marks(trail: &Scene3D) -> Vec<Scene3D> {
        match &trail.obj {
            SceneObject::Group(marks) => marks.clone(),
            _ => panic!("expected a group of trail marks"),
        }
    }

    // A mark's leaf shapes, under its Group -> Material wrapper.
    fn mark_shapes(mark: &Scene3D) -> Vec<Shape> {
        match &mark.obj {
            SceneObject::Group(children) => match &children[0].obj {
                SceneObject::Material(_, leaves) => leaves
                    .iter()
                    .map(|leaf| match &leaf.obj {
                        SceneObject::Geometry(shape) => shape.clone(),
                        other => panic!("expected a geometry leaf, got {other:?}"),
                    })
                    .collect(),
                other => panic!("expected a material wrapper, got {other:?}"),
            },
            other => panic!("expected a placed mark group, got {other:?}"),
        }
    }

    // Where a mark's own +X (the arrowhead's tip direction) points in world space.
    fn mark_heading(mark: &Scene3D) -> Vector3<f32> {
        mark.xform.x.truncate().normalize()
    }

    fn assert_close(actual: Vector3<f32>, expected: Vector3<f32>) {
        assert!(
            (actual - expected).magnitude() < 1e-4,
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn marks_point_along_the_local_direction_of_travel() {
        // A mover running down +X, then one running down +Z: every mark's tip
        // direction follows the track, and each 3D mark is the crossed pair of
        // arrowheads (readable from any camera angle).
        let along_x: Vec<Scene3D> = (0..=3).map(|i| frame(i as f32, 0.0)).collect();
        let refs: Vec<&Scene3D> = along_x.iter().collect();
        let trail = trajectory_trail(&refs, 0.05, 3.0).expect("a trail");
        let along_x_marks = marks(&trail);
        assert_eq!(along_x_marks.len(), 4);
        for mark in &along_x_marks {
            assert_close(mark_heading(mark), vec3(1.0, 0.0, 0.0));
            let shapes = mark_shapes(mark);
            assert_eq!(
                shapes.len(),
                3,
                "a 3D mark is two crossed heads plus the sphere core"
            );
            assert!(matches!(shapes[0], Shape::ConvexPolygon { .. }));
            assert!(matches!(shapes[1], Shape::ConvexPolygon { .. }));
            assert!(
                matches!(shapes[2], Shape::Sphere),
                "the core keeps an edge-on mark visible, got {:?}",
                shapes[2]
            );
        }

        let along_z = |z: f32| Scene3D {
            obj: SceneObject::Group(vec![
                Scene3D::sphere().transform(Matrix4::from_translation(vec3(0.0, 0.0, z)))
            ]),
            xform: Matrix4::identity(),
        };
        let frames = [along_z(0.0), along_z(1.0), along_z(2.0)];
        let refs: Vec<&Scene3D> = frames.iter().collect();
        let trail = trajectory_trail(&refs, 0.05, 3.0).expect("a trail");
        for mark in marks(&trail) {
            assert_close(mark_heading(&mark), vec3(0.0, 0.0, 1.0));
        }
    }

    #[test]
    fn a_near_stationary_sample_falls_back_to_a_dot() {
        // The mover advances once and then rests: the moving samples get
        // arrowheads, the resting ones have no heading and stay dots.
        let frames: Vec<Scene3D> = [0.0, 1.0, 1.0, 1.0]
            .iter()
            .map(|x| frame(*x, 0.0))
            .collect();
        let refs: Vec<&Scene3D> = frames.iter().collect();
        let trail = trajectory_trail(&refs, 0.05, 3.0).expect("a trail");
        let stepped = marks(&trail);
        assert_eq!(stepped.len(), 4);
        assert!(matches!(mark_shapes(&stepped[0])[0], Shape::ConvexPolygon { .. }));
        assert!(matches!(mark_shapes(&stepped[1])[0], Shape::ConvexPolygon { .. }));
        for resting in &stepped[2..] {
            let shapes = mark_shapes(resting);
            assert_eq!(shapes.len(), 1);
            assert!(
                matches!(shapes[0], Shape::Sphere),
                "a heading-less sample keeps the dot form, got {:?}",
                shapes[0]
            );
        }
    }

    #[test]
    fn sprite_marks_stay_flat_in_the_layers_plane() {
        // A 2D mover heading up (+Y): its mark points that way and keeps facing
        // the layer camera (its own +Z stays +Z) — an out-of-plane 3D basis
        // would turn the arrowhead edge-on and invisible.
        let sprite_at = |x: f32, y: f32| {
            let mut rendered = Frame::new(Camera::default(), Scene3D::cube());
            rendered.sprite_layers.push(SpriteLayer {
                camera: Camera2D::new(24.0, 13.5),
                scene: frame(x, y),
            });
            rendered
        };
        let frames: Vec<Frame> = (0..=2).map(|i| sprite_at(0.0, i as f32)).collect();
        let futures: Vec<&Frame> = frames.iter().skip(1).collect();
        let preview = preview_from_frames(&frames[0], &futures, &preview_options(true, false));
        let trail = preview.sprite_layers[0].trail.as_ref().expect("a trail");
        for mark in marks(trail) {
            assert_close(mark_heading(&mark), vec3(0.0, 1.0, 0.0));
            assert_close(mark.xform.z.truncate().normalize(), vec3(0.0, 0.0, 1.0));
            assert_eq!(
                mark_shapes(&mark).len(),
                1,
                "a sprite mark is one flat arrowhead"
            );
        }
    }

    #[test]
    fn a_falling_track_points_straight_down() {
        // The branch the physics preview leans on: gravity pulls movers along
        // -Y, and every mark on the fall must point that way.
        let frames: Vec<Scene3D> = (0..=3).map(|i| frame(0.0, -(i as f32))).collect();
        let refs: Vec<&Scene3D> = frames.iter().collect();
        let trail = trajectory_trail(&refs, 0.05, 3.0).expect("a trail");
        for mark in marks(&trail) {
            assert_close(mark_heading(&mark), vec3(0.0, -1.0, 0.0));
        }
    }

    // Every mark of a constant-velocity track moving `step` per sample, as
    // "is it an arrow?" flags. The mover epsilon is well below the heading
    // threshold here so the track still counts as moving at speeds that do
    // NOT earn a heading — the band this test is about.
    fn mark_forms_at_speed(step: f32) -> Vec<bool> {
        let frames: Vec<Scene3D> = (0..=3).map(|i| frame(i as f32 * step, 0.0)).collect();
        let refs: Vec<&Scene3D> = frames.iter().collect();
        let trail = trajectory_trail(&refs, 0.001, 3.0).expect("a trail");
        marks(&trail)
            .iter()
            .map(|m| matches!(mark_shapes(m)[0], Shape::ConvexPolygon { .. }))
            .collect()
    }

    #[test]
    fn constant_velocity_gives_one_mark_form_across_the_whole_track() {
        // The threshold is per STEP, so a constant-velocity track is all
        // arrows or all dots — never dot ends around arrow interiors. An
        // interior sample spans two steps and the endpoints one, so a
        // raw-segment comparison would flip form exactly in this band: just
        // under the per-step threshold, where the two-step interior still
        // clears it.
        let eps = TRAIL_RADIUS_3D * HEADING_EPS_RADII;
        let slow = mark_forms_at_speed(eps * 0.75);
        assert_eq!(
            slow,
            vec![false; 4],
            "below the per-step threshold every mark is a dot"
        );
        let fast = mark_forms_at_speed(eps * 1.25);
        assert_eq!(
            fast,
            vec![true; 4],
            "above the per-step threshold every mark is an arrow"
        );
    }

    #[test]
    fn the_marks_basis_varies_continuously_with_the_heading() {
        // The transverse basis (and so the crossed heads' roll) must not snap
        // as a heading sweeps past any particular elevation — a helper-axis
        // switch at |dir.y| = 0.9 used to spin the mark ~90 degrees there.
        let basis = |y: f32| {
            let dir = vec3((1.0f32 - y * y).sqrt(), y, 0.0).normalize();
            heading_rotation(dir)
        };
        for &y in &[0.88, 0.89, 0.90, 0.91, 0.92] {
            let a = basis(y);
            let b = basis(y + 0.005);
            for (col_a, col_b) in [(a.y, b.y), (a.z, b.z)] {
                let delta = (col_a.truncate() - col_b.truncate()).magnitude();
                assert!(
                    delta < 0.1,
                    "basis jumped by {delta} between y = {y} and y = {}",
                    y + 0.005
                );
            }
        }
    }

    #[test]
    fn sprite_marks_follow_leftward_motion() {
        // Negative dx must point the mark along -X, not fold onto +X.
        let sprite_at = |x: f32| {
            let mut rendered = Frame::new(Camera::default(), Scene3D::cube());
            rendered.sprite_layers.push(SpriteLayer {
                camera: Camera2D::new(24.0, 13.5),
                scene: frame(x, 0.0),
            });
            rendered
        };
        let frames: Vec<Frame> = (0..=2).map(|i| sprite_at(-(i as f32))).collect();
        let futures: Vec<&Frame> = frames.iter().skip(1).collect();
        let preview = preview_from_frames(&frames[0], &futures, &preview_options(true, false));
        let trail = preview.sprite_layers[0].trail.as_ref().expect("a trail");
        for mark in marks(trail) {
            assert_close(mark_heading(&mark), vec3(-1.0, 0.0, 0.0));
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
    fn mid_list_despawn_truncates_and_never_cross_wires() {
        // Group [mover, staticA, staticB]; in the last frame the mover
        // despawns, shifting the statics down one list slot. With bare-index
        // paths the statics' old paths would resolve to their neighbors and
        // fabricate a phantom trail between two objects that never moved; with
        // sibling counts in the key the changed group stops matching, so the
        // mover keeps its trail up to the despawn and the statics get nothing.
        let f = |x: f32| Scene3D {
            obj: SceneObject::Group(vec![
                ball_at(x, 0.0),
                ball_at(1.0, 0.0),
                ball_at(2.0, 0.0),
            ]),
            xform: Matrix4::identity(),
        };
        let last = Scene3D {
            obj: SceneObject::Group(vec![ball_at(1.0, 0.0), ball_at(2.0, 0.0)]),
            xform: Matrix4::identity(),
        };
        let trail = trajectory_trail(&[&f(0.0), &f(0.5), &last], 0.05, 3.0).expect("a trail");
        match trail.obj {
            // Two dots: the mover's frames before the despawn. Anything more
            // means a static sibling was aliased onto a neighbor's position.
            SceneObject::Group(dots) => assert_eq!(dots.len(), 2, "expected only the mover's pre-despawn dots"),
            _ => panic!("expected a group of dots"),
        }
    }

    #[test]
    fn trail_stops_at_a_teleport() {
        // A node steps smoothly (0 → 0.5 → 1.0) then RESPAWNS to a far position
        // (a platformer-style reset). The trail must cover the smooth run only — 3
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

    // --- strobe ---

    // A mover wrapped in a Lit material under a translated group, so the strobe
    // must fold the full parent chain into each copy AND carry the material.
    fn lit_frame(x: f32) -> Scene3D {
        Scene3D {
            obj: SceneObject::Group(vec![Scene3D {
                obj: SceneObject::Material(
                    MaterialDescription::lit(1.0, 0.5, 0.0, 1.0),
                    vec![ball_at(x, 0.0)],
                ),
                xform: Matrix4::identity(),
            }]),
            xform: Matrix4::from_translation(vec3(2.0, 0.0, 0.0)),
        }
    }

    #[test]
    fn strobe_copies_land_at_future_world_poses_with_faded_material() {
        let frames = [lit_frame(0.0), lit_frame(1.0), lit_frame(2.0)];
        let refs: Vec<&Scene3D> = frames.iter().collect();
        let tracks = mover_tracks(&refs, 0.05, 9.0);
        assert_eq!(tracks.len(), 1);
        let strobe = strobe_overlay(
            &tracks,
            &StrobeOptions {
                copies: 2,
                fade_to: [0.0, 0.0, 0.0],
                fade: (0.8, 0.2),
                tint: None,
            },
        )
        .expect("a strobe");
        let copies = match strobe.obj {
            SceneObject::Group(c) => c,
            _ => panic!(),
        };
        assert_eq!(copies.len(), 2);
        // Copies at the two future poses: world x = 2 (group) + local.
        assert!((copies[0].xform.w.x - 3.0).abs() < 1e-4);
        assert!((copies[1].xform.w.x - 4.0).abs() < 1e-4);
        // Each copy: Group -> Material(faded Lit) -> leaf. The LAST copy fades
        // hardest: k = 0.2 → red channel 1.0 * 0.2.
        let mat = match &copies[1].obj {
            SceneObject::Group(children) => match &children[0].obj {
                SceneObject::Material(m, _) => m.clone(),
                _ => panic!("expected a material wrapper"),
            },
            _ => panic!("expected a group"),
        };
        match mat {
            MaterialDescription::Lit { color, .. } => {
                assert!((color.x - 0.2).abs() < 1e-4, "expected faded red, got {}", color.x);
            }
            _ => panic!("expected a Lit material"),
        }
    }

    #[test]
    fn both_mode_dots_stay_off_the_strobe_cadence() {
        // 4 future samples, 2 copies → copies stand on samples 2 and 4; the
        // trail must drop those and keep the anchor plus samples 1 and 3, so
        // dots fill the gaps between copies instead of hiding under them.
        let frames: Vec<Scene3D> = (0..=4).map(|i| frame(i as f32, 0.0)).collect();
        let refs: Vec<&Scene3D> = frames.iter().collect();
        let tracks = mover_tracks(&refs, 0.05, 9.0);
        let opts = StrobeOptions {
            copies: 2,
            ..Default::default()
        };
        let trail = trail_from_tracks(&tracks, Some(&opts), TRAIL_RADIUS_3D, TrailSpace::Scene3D, PreviewSide::Future).expect("a trail");
        match trail.obj {
            SceneObject::Group(dots) => {
                assert_eq!(dots.len(), 3, "anchor + the two non-copy samples")
            }
            _ => panic!(),
        }
    }

    #[test]
    fn in_place_rotation_strobes_but_leaves_no_trail() {
        // A cube spinning in place: its world POSITION never changes, but its
        // basis vectors do. It must earn strobe copies (which can depict the
        // spin) and no trail (whose dots would pile on one spot).
        let spin = |angle: f32| Scene3D {
            obj: SceneObject::Group(vec![
                Scene3D::cube().transform(Matrix4::from_angle_y(cgmath::Rad(angle)))
            ]),
            xform: Matrix4::identity(),
        };
        let frames = [spin(0.0), spin(0.5), spin(1.0)];
        let refs: Vec<&Scene3D> = frames.iter().collect();
        let tracks = mover_tracks(&refs, 0.05, 3.0);
        assert_eq!(tracks.len(), 1, "a spinner is a mover");
        assert!(!tracks[0].translated);
        assert!(strobe_overlay(&tracks, &StrobeOptions::default()).is_some());
        assert!(
            trail_from_tracks(&tracks, None, TRAIL_RADIUS_3D, TrailSpace::Scene3D, PreviewSide::Future).is_none(),
            "no dots for pure rotation"
        );
    }

    #[test]
    fn strobe_skips_statics_and_material_less_leaves_render_bare() {
        // Two nodes: a bare (no material) mover and a static. Only the mover
        // strobes, and its copies carry no Material wrapper (full fidelity —
        // the model/no-material case).
        let f = |x: f32| frame(x, 0.0);
        let frames = [f(0.0), f(1.0)];
        let refs: Vec<&Scene3D> = frames.iter().collect();
        let tracks = mover_tracks(&refs, 0.05, 9.0);
        assert_eq!(tracks.len(), 1);
        let strobe = strobe_overlay(&tracks, &StrobeOptions::default()).expect("a strobe");
        let copies = match strobe.obj {
            SceneObject::Group(c) => c,
            _ => panic!(),
        };
        assert_eq!(copies.len(), 1, "one future sample → one copy");
        match &copies[0].obj {
            SceneObject::Geometry(_) => {}
            other => panic!("expected a bare geometry copy, got {other:?}"),
        }
    }

    // ---- the presence ramp (the whole overlay's fade in / out) ----

    /// Every material alpha in a scene, depth-first. `overlay` APPENDS, so a
    /// frame's own materials always come first and the overlay's follow — which
    /// is what lets the tests below separate them by index.
    fn material_alphas(scene: &Scene3D, out: &mut Vec<f32>) {
        match &scene.obj {
            SceneObject::Material(material, items) => {
                out.push(material.color_alpha().expect("a color-bearing material"));
                for item in items {
                    material_alphas(item, out);
                }
            }
            SceneObject::Group(items) | SceneObject::Opacity(_, items) => {
                for item in items {
                    material_alphas(item, out);
                }
            }
            SceneObject::Geometry(_) | SceneObject::Model(_) | SceneObject::Terrain(_) => {}
        }
    }

    /// One alpha list PER scene tree — the 3D scene, then each sprite layer.
    /// Kept separate because each tree gets its own overlay appended, so "the
    /// frame's own materials first" only holds within a tree.
    fn frame_material_alphas(frame: &Frame) -> Vec<Vec<f32>> {
        std::iter::once(&frame.scene)
            .chain(frame.sprite_layers.iter().map(|layer| &layer.scene))
            .map(|scene| {
                let mut out = Vec::new();
                material_alphas(scene, &mut out);
                out
            })
            .collect()
    }

    /// A mover carrying its own material, beside a static — in BOTH the 3D
    /// scene and one sprite layer, so one fixture exercises both overlay paths.
    fn materialized_frame(x: f32) -> Frame {
        let scene = |x: f32| Scene3D {
            obj: SceneObject::Group(vec![
                Scene3D {
                    obj: SceneObject::Material(
                        MaterialDescription::color(1.0, 0.0, 0.0, 1.0),
                        vec![ball_at(x, 0.0)],
                    ),
                    xform: Matrix4::identity(),
                },
                ball_at(5.0, 0.0),
            ]),
            xform: Matrix4::identity(),
        };
        let mut rendered = Frame::new(Camera::default(), scene(x));
        rendered.sprite_layers.push(SpriteLayer {
            camera: Camera2D::new(24.0, 13.5),
            scene: scene(x),
        });
        rendered
    }

    fn moving_preview() -> (Frame, FramePreview) {
        let frames: Vec<Frame> = (0..=4).map(|i| materialized_frame(i as f32 * 0.5)).collect();
        let futures: Vec<&Frame> = frames.iter().skip(1).collect();
        let preview = preview_from_frames(&frames[0], &futures, &preview_options(true, true));
        assert!(!preview.is_empty(), "the fixture must produce overlays");
        (frames[0].clone(), preview)
    }

    #[test]
    fn presence_step_is_monotone_clamped_and_completes_within_the_ramp() {
        // Rising: never decreases, never leaves [0, 1], fully there after
        // exactly one ramp.
        let dt = PRESENCE_RAMP_SECONDS / 8.0;
        let mut p = 0.0;
        for _ in 0..8 {
            let next = presence_step(p, 1.0, dt);
            assert!(next >= p, "presence must not go backwards while rising");
            assert!((0.0..=1.0).contains(&next), "presence escaped [0, 1]: {next}");
            p = next;
        }
        assert!((p - 1.0).abs() < 1e-5, "one ramp must complete the fade in: {p}");

        // Falling: the mirror image.
        for _ in 0..8 {
            let next = presence_step(p, 0.0, dt);
            assert!(next <= p, "presence must not go forwards while falling");
            assert!((0.0..=1.0).contains(&next), "presence escaped [0, 1]: {next}");
            p = next;
        }
        assert!(p.abs() < 1e-5, "one ramp must complete the fade out: {p}");
    }

    #[test]
    fn presence_step_is_robust_to_the_frame_delta() {
        // Same wall-clock elapsed, very different frame rates → same presence.
        let mut fast = 0.0;
        for _ in 0..100 {
            fast = presence_step(fast, 1.0, PRESENCE_RAMP_SECONDS / 200.0);
        }
        let slow = presence_step(0.0, 1.0, PRESENCE_RAMP_SECONDS / 2.0);
        assert!((fast - slow).abs() < 1e-4, "{fast} vs {slow}");

        // A hitch longer than the whole ramp lands exactly ON the target, never
        // past it; a non-advancing or non-finite delta is a no-op.
        assert_eq!(presence_step(0.0, 1.0, 10.0), 1.0);
        assert_eq!(presence_step(1.0, 0.0, 10.0), 0.0);
        assert_eq!(presence_step(0.4, 1.0, 0.0), 0.4);
        assert_eq!(presence_step(0.4, 1.0, -1.0), 0.4);
        assert_eq!(presence_step(0.4, 1.0, f32::NAN), 0.4);
        // Out-of-range state is clamped rather than propagated.
        assert_eq!(presence_step(5.0, 1.0, 0.01), 1.0);
        assert_eq!(presence_step(-5.0, 0.0, 0.01), 0.0);
    }

    #[test]
    fn presence_ease_is_pinned_at_the_ends_and_symmetric() {
        assert_eq!(presence_ease(0.0), 0.0);
        assert_eq!(presence_ease(1.0), 1.0);
        assert_eq!(presence_ease(0.5), 0.5);
        assert_eq!(presence_ease(-2.0), 0.0);
        assert_eq!(presence_ease(2.0), 1.0);
        for i in 0..=10 {
            let p = i as f32 / 10.0;
            // Fading out is fading in played backwards.
            assert!(
                (presence_ease(1.0 - p) - (1.0 - presence_ease(p))).abs() < 1e-6,
                "asymmetric at {p}"
            );
            assert!(presence_ease(p) >= presence_ease((p - 0.1).max(0.0)));
        }
    }

    #[test]
    fn presence_phase_inverts_the_ease_so_releasing_a_pin_is_continuous() {
        // Seeding the phase from a pinned alpha must render at exactly that
        // alpha, or clearing the pin visibly jumps.
        for i in 0..=20 {
            let alpha = i as f32 / 20.0;
            let phase = presence_phase(alpha);
            assert!((0.0..=1.0).contains(&phase), "phase escaped [0, 1]: {phase}");
            assert!(
                (presence_ease(phase) - alpha).abs() < 1e-4,
                "pin at {alpha} would resume at {} (phase {phase})",
                presence_ease(phase)
            );
        }
        // Endpoints land on the ends (within float slop from the trig form).
        assert!(presence_phase(0.0).abs() < 1e-6, "{}", presence_phase(0.0));
        assert!((presence_phase(1.0) - 1.0).abs() < 1e-6);
        // Out of range is clamped, not propagated as NaN.
        assert!(presence_phase(-3.0).abs() < 1e-6);
        assert!((presence_phase(3.0) - 1.0).abs() < 1e-6);
        assert!(presence_phase(f32::NAN).is_finite());
    }

    #[test]
    fn nested_overlay_materials_all_scale() {
        // `Material` nodes nest (a strobe copy of a game subtree that sets its
        // own material inside another). Every level must fade, not just the
        // outermost.
        let mut scene = Scene3D {
            obj: SceneObject::Material(
                MaterialDescription::color(1.0, 1.0, 1.0, 1.0),
                vec![Scene3D {
                    obj: SceneObject::Material(
                        MaterialDescription::color(1.0, 0.0, 0.0, 0.5),
                        vec![Scene3D::sphere()],
                    ),
                    xform: Matrix4::identity(),
                }],
            ),
            xform: Matrix4::identity(),
        };
        scale_presence(&mut scene, 0.5);
        let mut alphas = Vec::new();
        material_alphas(&scene, &mut alphas);
        assert_eq!(alphas, vec![0.5, 0.25]);
    }

    #[test]
    fn full_presence_leaves_the_overlay_untouched() {
        let (anchor, preview) = moving_preview();

        let mut explicit = anchor.clone();
        preview.apply_all_with_presence(&mut explicit, 1.0);
        let mut implicit = anchor.clone();
        preview.apply_all(&mut implicit);

        assert_eq!(
            format!("{:?}", explicit.scene),
            format!("{:?}", implicit.scene),
            "presence 1.0 must be identical to the pre-ramp path (goldens/captures)"
        );
    }

    #[test]
    fn presence_scales_every_overlay_alpha_in_3d_and_sprite_layers() {
        let (anchor, preview) = moving_preview();
        let own = frame_material_alphas(&anchor);

        let mut full = anchor.clone();
        preview.apply_all_with_presence(&mut full, 1.0);
        let full = frame_material_alphas(&full);

        let mut half = anchor.clone();
        preview.apply_all_with_presence(&mut half, 0.5);
        let half = frame_material_alphas(&half);

        assert_eq!(own.len(), 2, "the fixture has a 3D scene and one sprite layer");
        assert_eq!(full.len(), own.len());
        assert_eq!(half.len(), own.len());

        for (tree, ((own, full), half)) in own.iter().zip(&full).zip(&half).enumerate() {
            assert_eq!(full.len(), half.len());
            assert!(
                full.len() > own.len(),
                "tree {tree}'s overlay must contribute materials to scale"
            );
            // The game's own materials come first and are never touched...
            assert_eq!(&full[..own.len()], &own[..], "tree {tree}");
            assert_eq!(&half[..own.len()], &own[..], "tree {tree}");
            // ...and every overlay material — 3D marks and copies in tree 0,
            // sprite ones in tree 1 — is exactly halved.
            for (i, (one, halved)) in full[own.len()..].iter().zip(&half[own.len()..]).enumerate() {
                assert!(
                    (halved - one * 0.5).abs() < 1e-6,
                    "tree {tree} overlay material {i}: {one} at presence 1.0 but {halved} at 0.5"
                );
            }
        }
    }

    #[test]
    fn zero_presence_applies_nothing_at_all() {
        let (anchor, preview) = moving_preview();
        let before = format!("{:?}", anchor.scene);
        let sprites_before: Vec<String> = anchor
            .sprite_layers
            .iter()
            .map(|layer| format!("{:?}", layer.scene))
            .collect();

        let mut faded = anchor.clone();
        preview.apply_all_with_presence(&mut faded, 0.0);

        // NOT "transparent": ABSENT. An alpha-0 overlay would still write depth
        // and occlude the scene behind it.
        assert_eq!(format!("{:?}", faded.scene), before);
        let sprites_after: Vec<String> = faded
            .sprite_layers
            .iter()
            .map(|layer| format!("{:?}", layer.scene))
            .collect();
        assert_eq!(sprites_after, sprites_before);

        // Negative / non-finite presence is treated the same way.
        let mut negative = anchor.clone();
        preview.apply_all_with_presence(&mut negative, -1.0);
        assert_eq!(format!("{:?}", negative.scene), before);
        let mut nan = anchor.clone();
        preview.apply_all_with_presence(&mut nan, f32::NAN);
        assert_eq!(format!("{:?}", nan.scene), before);
    }
}
