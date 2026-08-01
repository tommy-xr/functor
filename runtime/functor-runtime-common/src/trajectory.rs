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
        SceneObject::Group(children) | SceneObject::Material(_, children) => {
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
        SceneObject::Group(children) => {
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
        SceneObject::Group(children) => {
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

/// The emissive cyan wrapper every trail mark shares. The renderer applies a
/// node's `xform` on `Group`/`Geometry` but NOT on `Material`, so placement
/// goes on the enclosing Group and the mark's own shape on the leaves.
fn trail_mark(marks: Vec<Scene3D>, place: Matrix4<f32>) -> Scene3D {
    let material = Scene3D {
        obj: SceneObject::Material(MaterialDescription::emissive(0.25, 0.85, 1.0, 1.0), marks),
        xform: Matrix4::identity(),
    };
    Scene3D {
        obj: SceneObject::Group(vec![material]),
        xform: place,
    }
}

/// A single dim emissive marker at a world position — the direction-free
/// fallback for a sample with no meaningful heading.
fn trail_dot(p: Vector3<f32>, radius: f32) -> Scene3D {
    let sphere = Scene3D::sphere().transform(Matrix4::from_scale(radius));
    trail_mark(vec![sphere], Matrix4::from_translation(p))
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
) -> Option<Scene3D> {
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
            let p = world_pos(w);
            marks.push(
                heading_at(&track.worlds, i, radius)
                    .and_then(|dir| trail_arrow(p, dir, radius, space))
                    .unwrap_or_else(|| trail_dot(p, radius)),
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
    )
}

/// Scene-space strobe options.
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
}

impl Default for StrobeOptions {
    fn default() -> Self {
        StrobeOptions {
            copies: 8,
            // The runtime's clear color (run.rs / web lib.rs) — overridden by
            // hosts whose scene has its own backdrop.
            fade_to: [0.1, 0.2, 0.3],
            fade: (0.8, 0.2),
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
) -> Option<MaterialDescription> {
    let lerp = |c: Vector4<f32>| {
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
) -> Option<MaterialDescription> {
    let fade = |mut color: Vector4<f32>| {
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
fn strobe_copy(
    leaf: &SceneObject,
    material: Option<&MaterialDescription>,
    world: Matrix4<f32>,
    fade_to: [f32; 3],
    k: f32,
    fade: StrobeFade,
) -> Scene3D {
    let leaf = Scene3D {
        obj: leaf.clone(),
        xform: Matrix4::identity(),
    };
    let material = match fade {
        StrobeFade::Color => faded_material(material, fade_to, k),
        StrobeFade::Alpha => alpha_faded_material(material, k),
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

    fn apply(&self, scene: &mut Scene3D) {
        if let Some(trail) = &self.trail {
            overlay(scene, trail.clone());
        }
        if let Some(strobe) = &self.strobe {
            overlay(scene, strobe.clone());
        }
    }
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
    pub fn is_empty(&self) -> bool {
        self.scene.is_empty() && self.sprite_layers.iter().all(SceneOverlays::is_empty)
    }

    /// Add every requested scene-space overlay to a normal display frame.
    pub fn apply_all(&self, frame: &mut Frame) {
        self.scene.apply(&mut frame.scene);
        if frame.sprite_layers.len() != self.sprite_layers.len() {
            return;
        }
        for (layer, overlays) in frame.sprite_layers.iter_mut().zip(&self.sprite_layers) {
            overlays.apply(&mut layer.scene);
        }
    }
}

fn scene_overlays(
    scenes: &[&Scene3D],
    opts: &PreviewOptions,
    trail_radius: f32,
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
            )
        } else {
            None
        },
        strobe: opts
            .strobe
            .as_ref()
            .and_then(|strobe| strobe_overlay(&tracks, strobe)),
    }
}

fn sprite_scene_overlays(
    scenes: &[&Scene3D],
    opts: &PreviewOptions,
    trail_radius: f32,
) -> SceneOverlays {
    let trail = if opts.trail {
        let tracks = mover_tracks(scenes, opts.eps, opts.max_step);
        trail_from_tracks(
            &tracks,
            opts.strobe.as_ref(),
            trail_radius,
            TrailSpace::Sprite2D,
        )
    } else {
        None
    };
    let strobe = opts.strobe.as_ref().and_then(|strobe| {
        let sampled = sampled_mover_tracks(scenes, opts.eps, opts.max_step);
        sampled_strobe_overlay(&sampled, strobe, StrobeFade::Alpha)
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
    scene_overlays(&scenes, opts, TRAIL_RADIUS_3D)
}

fn preview_from_frames(anchor: &Frame, futures: &[&Frame], opts: &PreviewOptions) -> FramePreview {
    let mut scenes = Vec::with_capacity(futures.len() + 1);
    scenes.push(&anchor.scene);
    scenes.extend(futures.iter().map(|frame| &frame.scene));

    let matching_futures: Vec<_> = futures
        .iter()
        .take_while(|frame| frame.sprite_layers.len() == anchor.sprite_layers.len())
        .copied()
        .collect();
    let sprite_layers = anchor
        .sprite_layers
        .iter()
        .enumerate()
        .map(|(index, anchor_layer)| {
            let mut scenes = Vec::with_capacity(futures.len() + 1);
            scenes.push(&anchor_layer.scene);
            for future in &matching_futures {
                scenes.push(&future.sprite_layers[index].scene);
            }
            sprite_scene_overlays(
                &scenes,
                opts,
                sprite_trail_radius(&anchor_layer.camera),
            )
        })
        .collect();

    FramePreview {
        scene: scene_overlays(&scenes, opts, TRAIL_RADIUS_3D),
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
    preview_from_frames(anchor, &future_frames, opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, Camera2D, SpriteLayer, SpriteSampling, TextureDescription};
    use cgmath::vec3;

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
        assert_eq!(colors[0].truncate(), vec3(0.0, 1.0, 0.0));
        assert_eq!(colors[1].truncate(), vec3(0.0, 0.0, 1.0));
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

        match alpha_faded_material(Some(&material), 0.25) {
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
        let mario_scale = sprite_trail_radius(&Camera2D::new(24.0, 13.5));
        let pixel_scale = sprite_trail_radius(&Camera2D::new(320.0, 180.0));
        assert!((mario_scale - TRAIL_RADIUS_3D).abs() < 1e-6);
        assert!((pixel_scale / mario_scale - 180.0 / 13.5).abs() < 1e-4);
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
        let trail = trail_from_tracks(&tracks, Some(&opts), TRAIL_RADIUS_3D, TrailSpace::Scene3D).expect("a trail");
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
            trail_from_tracks(&tracks, None, TRAIL_RADIUS_3D, TrailSpace::Scene3D).is_none(),
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
}
