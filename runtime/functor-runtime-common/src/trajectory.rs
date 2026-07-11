//! Scene-diff preview (docs/time-travel.md T6): given a game's live frame plus
//! its forward-simulated future frames, find the scene nodes whose WORLD
//! transform changes across the sequence ("movers") and render their future two
//! ways:
//!
//! - a **trail** of dots tracing each mover's path (the clean-lines view), and
//! - a **scene-space strobe**: real-geometry copies of each mover at its future
//!   poses, color-faded by age (the chronophotography view). Unlike the
//!   screen-space `--ghost` compositor — which averages N whole frames, pinning
//!   every ghost copy at 1/N opacity — copies here are ordinary geometry on the
//!   normal render path: full intensity, no division cap, and the camera stays
//!   live. (The compositor remains the right tool for non-geometry motion such
//!   as animated lighting, which no geometry copy can represent.)
//!
//! The point is that this needs NO game cooperation: the runtime derives
//! everything purely from what `draw` already renders. It diffs the rendered
//! *scene* (which carries concrete world transforms), not the opaque model — so
//! "which numbers are positions" is unambiguous and "what moved" falls out of
//! comparing world transforms across the forward-sim.
//!
//! Pure and testable — no GPU, no interpreter needed (see the unit tests). The
//! one host-facing entry point is [`scene_preview`]: one forward-sim, both
//! consumers.

use std::collections::BTreeMap;

use cgmath::{vec4, InnerSpace, Matrix4, SquareMatrix, Vector3, Vector4};

use crate::protocol::GameProducer;
use crate::{MaterialDescription, RecordedInput, Scene3D, SceneObject};

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

/// A LEAF node of the anchor scene: its world transform, the innermost
/// enclosing material (what the renderer would shade it with), and the leaf
/// object itself.
struct AnchorLeaf {
    world: Matrix4<f32>,
    material: Option<MaterialDescription>,
    leaf: SceneObject,
}

/// Walk `scene`, accumulating world transforms, and record each leaf's world
/// matrix keyed by its path. A `BTreeMap` so iteration (and thus the emitted
/// scene order) is deterministic.
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
        SceneObject::Geometry(_) | SceneObject::Model(_) => {
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

/// The anchor-scene walk: like [`collect_transforms`] but also records each
/// leaf's object and innermost enclosing material, which the strobe needs to
/// build faithful copies. Run once per preview (on the anchor only).
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
        SceneObject::Geometry(_) | SceneObject::Model(_) => {
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
/// depicts both, but a dotted trail of a spinner would just pile dots on one
/// spot, so the trail consumer skips them.
pub struct MoverTrack {
    pub leaf: SceneObject,
    pub material: Option<MaterialDescription>,
    pub worlds: Vec<Matrix4<f32>>,
    pub translated: bool,
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
    let futures: Vec<_> = scenes[1..].iter().map(|s| transforms_by_path(s)).collect();
    let eps2 = eps * eps;
    let mut tracks = Vec::new();
    for (path, a) in &anchor {
        let mut worlds = vec![a.world];
        for m in &futures {
            match m.get(path) {
                Some(w) => worlds.push(*w),
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
            leaf: a.leaf.clone(),
            material: a.material.clone(),
            worlds,
            translated,
        });
    }
    tracks
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

/// The 1-based future-sample index the strobe's `c`-th copy stands on, for a
/// track with `n_future` future samples and `count` copies: evenly spread,
/// always including the window's end. Shared by the strobe (to place copies)
/// and the trail (to stay OFF the strobe's cadence).
fn strobe_idx(c: usize, count: usize, n_future: usize) -> usize {
    (((c + 1) as f32 * n_future as f32 / count as f32).round() as usize).clamp(1, n_future)
}

fn trail_from_tracks(tracks: &[MoverTrack], strobe: Option<&StrobeOptions>) -> Option<Scene3D> {
    let mut dots = Vec::new();
    for track in tracks {
        // A pure in-place spinner has a track (for the strobe) but no path to
        // dot — its dots would all land on one spot.
        if !track.translated {
            continue;
        }
        // Off-cadence with the strobe: skip the samples where a copy stands,
        // so dots fill the gaps BETWEEN copies instead of hiding under them.
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
            dots.push(trail_dot(world_pos(w)));
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

/// Build a dotted-trail scene from a scene sequence. Returns `None` when
/// nothing moved. (The trail consumer of [`mover_tracks`].)
pub fn trajectory_trail(scenes: &[&Scene3D], eps: f32, max_step: f32) -> Option<Scene3D> {
    trail_from_tracks(&mover_tracks(scenes, eps, max_step), None)
}

/// Scene-space strobe options.
pub struct StrobeOptions {
    /// Ghost copies per mover across the window (evenly sampled from its track).
    pub copies: usize,
    /// The color ghosts fade toward with age — pick the scene's background so
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
        None => None,
    }
}

/// One strobe copy: the mover's leaf at a future world pose, shaded by its
/// (age-faded) material. Transforms go on a Group / the leaf itself — never on
/// a Material node, which the renderer ignores (see [`trail_dot`]).
fn strobe_copy(track: &MoverTrack, world: Matrix4<f32>, fade_to: [f32; 3], k: f32) -> Scene3D {
    let leaf = Scene3D {
        obj: track.leaf.clone(),
        xform: Matrix4::identity(),
    };
    match faded_material(track.material.as_ref(), fade_to, k) {
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
            let age = if n_future <= 1 {
                0.0
            } else {
                (idx - 1) as f32 / (n_future - 1) as f32
            };
            let k = opts.fade.0 + (opts.fade.1 - opts.fade.0) * age;
            copies.push(strobe_copy(track, track.worlds[idx], opts.fade_to, k));
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

/// The interactive future-preview mode — what the scrubber's selector (and the
/// `--trajectory`/`--strobe`/`--ghost` launch flags, which seed it) ask the
/// shell to overlay. One selector covers both preview families: the scene-diff
/// overlays (trail / strobe / both — geometry on the normal render path) and
/// `Ghost`, the screen-space compositor strobe (the only mode that can depict
/// non-geometry motion such as animated lighting). Shared by both shells so
/// the wire encoding and cycle order match.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PreviewMode {
    #[default]
    Off,
    Trail,
    Strobe,
    Both,
    Ghost,
}

impl PreviewMode {
    pub fn wants_trail(self) -> bool {
        matches!(self, PreviewMode::Trail | PreviewMode::Both)
    }
    pub fn wants_strobe(self) -> bool {
        matches!(self, PreviewMode::Strobe | PreviewMode::Both)
    }
    /// The screen-space compositor strobe (docs/time-travel.md T6d).
    pub fn wants_ghost(self) -> bool {
        matches!(self, PreviewMode::Ghost)
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
            PreviewMode::Ghost => "ghost",
        }
    }
    /// Stable wire encoding for the wasm scrubber bridge (the DOM `<select>`
    /// values); anything unknown is `Off`.
    pub fn from_index(i: u32) -> PreviewMode {
        match i {
            1 => PreviewMode::Trail,
            2 => PreviewMode::Strobe,
            3 => PreviewMode::Both,
            4 => PreviewMode::Ghost,
            _ => PreviewMode::Off,
        }
    }
}

/// What [`scene_preview`] should compute.
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
    /// Emit the dotted trail?
    pub trail: bool,
    /// Emit the scene-space strobe?
    pub strobe: Option<StrobeOptions>,
}

/// A computed preview: overlays for the normal render path. Cheap to clone
/// relative to recomputing (hosts cache it on the anchor).
#[derive(Clone)]
pub struct ScenePreview {
    pub trail: Option<crate::Scene3D>,
    pub strobe: Option<crate::Scene3D>,
}

/// The SHARED composition step both shells call (desktop `run.rs`; web
/// `lib.rs`): run ONE forward-sim via the producer's `ghost_frames`, diff the
/// scenes into mover tracks, and build whichever overlays `opts` asks for.
/// `script_inputs` follows `ghost_frames`' contract (docs/time-travel.md F2) —
/// the caller builds the slice, since only the shell knows its script and
/// anchor convention.
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
    scenes.extend(futures.iter().map(|(f, _)| &f.scene));
    let tracks = mover_tracks(&scenes, opts.eps, opts.max_step);
    ScenePreview {
        trail: if opts.trail {
            // When the strobe draws too, the trail stays off its cadence.
            trail_from_tracks(&tracks, opts.strobe.as_ref())
        } else {
            None
        },
        strobe: opts
            .strobe
            .as_ref()
            .and_then(|s| strobe_overlay(&tracks, s)),
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
        let trail = trail_from_tracks(&tracks, Some(&opts)).expect("a trail");
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
            trail_from_tracks(&tracks, None).is_none(),
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
