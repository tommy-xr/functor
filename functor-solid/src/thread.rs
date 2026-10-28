//! Helical ISO threads as one closed solid: the thread surface is a helical
//! band (profile corners × angular steps) whose top edge is welded by index to
//! the next turn's bottom edge, closed at each end by a helicoid ramp and a
//! flat wall at θ = 0. The over-long body is trimmed to length by a boolean.

use std::f64::consts::TAU;

use crate::Mesh;

/// The ISO 68-1 basic profile for one pitch (60° flanks; flats P/8 at the
/// major diameter, P/4 at the minor), grown by `offset` mm along its normals
/// (shrunk when negative). `(u, r)` corners: u (axial) rising within one
/// pitch from the root start, r the radius. A flat the offset closes becomes
/// a sharp crest or root.
pub(crate) fn profile(diameter: f64, pitch: f64, offset: f64) -> Result<Vec<[f64; 2]>, String> {
    let (p, big) = (pitch, diameter / 2.0);
    let small = big - 5.0 * 3f64.sqrt() / 16.0 * p; // 5H/8 deep, H = √3/2·P
                                                    // The four sides of one period: root, rising flank, crest, falling flank.
    let corners = [
        [0.0, small],
        [p / 4.0, small],
        [9.0 * p / 16.0, big],
        [11.0 * p / 16.0, big],
        [p, small],
    ];
    // Each side as a point + direction, moved `offset` along its outward
    // (left, larger-r) normal.
    let side = |i: usize| {
        let ([u0, r0], [u1, r1]) = (corners[i], corners[i + 1]);
        let len = (u1 - u0).hypot(r1 - r0);
        let d = [(u1 - u0) / len, (r1 - r0) / len];
        ([u0 - d[1] * offset, r0 + d[0] * offset], d)
    };
    let (root, rise, crest, fall) = (side(0), side(1), side(2), side(3));
    let prev_fall = ([fall.0[0] - p, fall.0[1]], fall.1);
    let mut out = vec![
        meet(prev_fall, root),
        meet(root, rise),
        meet(rise, crest),
        meet(crest, fall),
    ];
    if out[3][0] - out[2][0] <= 1e-9 * p {
        out.splice(2..4, [meet(rise, fall)]); // sharp crest
    } else if out[1][0] - out[0][0] <= 1e-9 * p {
        out.splice(0..2, [meet(prev_fall, rise)]); // sharp root
    }
    let low = out.iter().map(|c| c[1]).fold(f64::INFINITY, f64::min);
    let high = out.iter().map(|c| c[1]).fold(0.0, f64::max);
    let rising = out.windows(2).all(|w| w[1][0] > w[0][0]) && out[out.len() - 1][0] - out[0][0] < p;
    if !(rising && high - low > 0.01 * p && low > 0.0) {
        return Err(format!(
            "an offset of {offset} mm leaves no thread on a {diameter} mm × {pitch} mm thread"
        ));
    }
    Ok(out)
}

/// Where two lines (point, direction) cross.
fn meet((p1, d1): ([f64; 2], [f64; 2]), (p2, d2): ([f64; 2], [f64; 2])) -> [f64; 2] {
    let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
    let t = cross([p2[0] - p1[0], p2[1] - p1[1]], d2) / cross(d1, d2);
    [p1[0] + t * d1[0], p1[1] + t * d1[1]]
}

/// The closed helical body: `profile` swept `turns` times around Z with
/// `segments` steps per turn, starting at z = `from`. Its ends are ramps,
/// so only z ∈ [from + P, from + (turns − 1)·P] has the full profile.
pub(crate) fn helix(
    profile: &[[f64; 2]],
    pitch: f64,
    from: f64,
    turns: usize,
    segments: usize,
) -> Mesh {
    let (k_len, n) = (profile.len(), segments);
    let steps = turns * n;
    let mut mesh = Mesh::default();
    // Band vertex (s, k): corner k at angular step s. θ from s % n keeps the
    // θ = 0 walls exactly in the y = 0 plane.
    let at = |s: usize, k: usize| -> [f64; 3] {
        let theta = TAU * (s % n) as f64 / n as f64;
        let [u, r] = profile[k];
        [
            r * theta.cos(),
            r * theta.sin(),
            from + u + pitch * s as f64 / n as f64,
        ]
    };
    for s in 0..=steps {
        for k in 0..k_len {
            mesh.vertices.push(at(s, k));
        }
    }
    let id = |s: usize, k: usize| (s * k_len + k) as u32;
    // Corner k_len of step s is corner 0 one turn on: the weld between turns.
    let wrap = |s: usize, k: usize| if k == k_len { id(s + n, 0) } else { id(s, k) };
    for s in 0..steps {
        for k in 0..k_len {
            if k == k_len - 1 && s + 1 + n > steps {
                continue; // the last turn's top edge stays open for the cap
            }
            let (a, b, c, d) = (id(s, k), id(s + 1, k), wrap(s + 1, k + 1), wrap(s, k + 1));
            mesh.triangles.extend([[a, b, c], [a, c, d]]);
        }
    }
    // Bottom cap: the first turn's lowest edge (corner 0) over steps 0..=n,
    // and the profile at step 0 up to corner 0 of step n.
    let chain: Vec<u32> = (0..k_len).map(|k| id(0, k)).chain([id(n, 0)]).collect();
    cap(
        &mut mesh,
        (0..=n).map(|s| id(s, 0)).collect(),
        &chain,
        false,
    );
    // Top cap: the last turn's highest edge (last corner), and the profile
    // from that corner one turn back up through the final step.
    let last = k_len - 1;
    let chain: Vec<u32> = [id(steps - n, last)]
        .into_iter()
        .chain((0..k_len).map(|k| id(steps, k)))
        .collect();
    cap(
        &mut mesh,
        (steps - n..=steps).map(|s| id(s, last)).collect(),
        &chain,
        true,
    );
    mesh
}

/// Close one end: a helicoid ramp from the `edge` helix (one turn, rising) in
/// to the Z axis, and a flat wall at θ = 0 between the axis and `chain` (the
/// profile, rising in z). Axis vertices sit at every edge and chain height so
/// ramp and wall share them; `top` flips the facing.
fn cap(mesh: &mut Mesh, edge: Vec<u32>, chain: &[u32], top: bool) {
    let z = |mesh: &Mesh, i: u32| mesh.vertices[i as usize][2];
    // Heights closer than Manifold's input grid (15 nm) would collapse.
    const MERGE: f64 = 1e-4;
    let mut heights: Vec<f64> = edge.iter().chain(chain).map(|&i| z(mesh, i)).collect();
    heights.sort_by(f64::total_cmp);
    heights.dedup_by(|b, a| *b - *a <= MERGE);
    let base = mesh.vertices.len() as u32;
    mesh.vertices.extend(heights.iter().map(|&h| [0.0, 0.0, h]));
    let axis = |h: f64| base + heights.partition_point(|&x| x < h - MERGE) as u32;
    let tri = |mesh: &mut Mesh, [a, b, c]: [u32; 3]| {
        mesh.triangles.push(if top { [a, c, b] } else { [a, b, c] })
    };
    // Ramp: per step, the quad between the helix edge and the axis. Fan it
    // from the edge's far end — except on the last step, whose far end is
    // back at θ = 0: a fan from there would lie in the wall's plane, folding
    // onto it, so that step fans from its near end.
    let steps = edge.len() - 1;
    for (i, w) in edge.windows(2).enumerate() {
        let (lo, hi) = (axis(z(mesh, w[0])), axis(z(mesh, w[1])));
        if i + 1 < steps {
            tri(mesh, [lo, w[1], w[0]]);
            for a in lo..hi {
                tri(mesh, [a, a + 1, w[1]]);
            }
        } else {
            for a in lo..hi {
                tri(mesh, [a, a + 1, w[0]]);
            }
            tri(mesh, [hi, w[1], w[0]]);
        }
    }
    // Wall: per chain side, a fan from its lower corner across the axis
    // heights it spans (a convex trapezoid), then the side itself.
    for w in chain.windows(2) {
        let (lo, hi) = (axis(z(mesh, w[0])), axis(z(mesh, w[1])));
        for a in lo..hi {
            tri(mesh, [w[0], a + 1, a]);
        }
        tri(mesh, [w[0], w[1], hi]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_profile_matches_iso_68_1() {
        let p = 1.25;
        let c = profile(8.0, p, 0.0).unwrap();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        assert_eq!(c.len(), 4);
        // Root flat P/4 at d − 1.0825·P, crest flat P/8 at d.
        assert!(close(c[1][0] - c[0][0], p / 4.0) && close(c[3][0] - c[2][0], p / 8.0));
        assert!(close(c[0][1] * 2.0, 8.0 - 5.0 * 3f64.sqrt() / 8.0 * p) && close(c[2][1], 4.0));
    }

    #[test]
    fn offsets_move_every_side_and_close_flats_into_points() {
        let p = 1.25;
        let h = 3f64.sqrt() / 2.0 * p;
        // A small shrink: crest and root both move in by the offset.
        let c = profile(8.0, p, -0.05).unwrap();
        assert!(
            (c[2][1] - (4.0 - 0.05)).abs() < 1e-12
                && (c[0][1] - (4.0 - 5.0 * h / 8.0 - 0.05)).abs() < 1e-12
        );
        // Past H/8 the crest flat is gone: a point 2·offset below the
        // fundamental triangle's apex (flanks are 30° off radial).
        let c = profile(8.0, p, -0.15).unwrap();
        assert_eq!(c.len(), 3);
        assert!((c[2][1] - (4.0 + h / 8.0 - 0.3)).abs() < 1e-12);
        // Past H/4 growing, the root flat is gone.
        let c = profile(8.0, p, 0.3).unwrap();
        assert_eq!(c.len(), 3);
        assert!((c[0][1] - (4.0 - 5.0 * h / 8.0 - h / 4.0 + 0.6)).abs() < 1e-12);
        assert!(profile(8.0, p, -1.0).is_err());
    }

    #[test]
    fn helix_body_is_closed() {
        // Includes a crest so narrow its corners fall within one angular step.
        for (d, p, offset, n) in [
            (8.0, 1.25, 0.0, 48),
            (8.0, 1.25, -0.15, 48),
            (8.0, 1.25, 0.3, 48),
            (2.3, 1.0, -0.6, 24),
        ] {
            let c = profile(d, p, offset).unwrap();
            let mesh = helix(&c, p, -1.5 * p, 6, n);
            assert_eq!(mesh.non_manifold_edges(), 0, "offset {offset}");
            assert_eq!(mesh.shared_positions(), 0, "offset {offset}");
            assert!(mesh.volume() > 0.0);
            // Each end's wall faces one way: a ramp folded back onto the
            // wall's plane would add a triangle facing the other.
            let mid = mesh.bounds().unwrap().max[2] / 2.0;
            for upper in [false, true] {
                let mut facing = mesh.triangles.iter().filter_map(|t| {
                    let v = t.map(|i| mesh.vertices[i as usize]);
                    let flat = v.iter().all(|p| p[1] == 0.0) && (v[0][2] > mid) == upper;
                    let n = (v[1][2] - v[0][2]) * (v[2][0] - v[0][0])
                        - (v[1][0] - v[0][0]) * (v[2][2] - v[0][2]);
                    flat.then_some(n > 0.0)
                });
                let first = facing.next().expect("a wall");
                assert!(facing.all(|f| f == first), "offset {offset}, upper {upper}");
            }
        }
    }

    use crate::{BoolOp, Solid};
    use std::sync::Arc;

    /// The mesh's section by the half-plane y = 0, x > 0, as (x, z) segments.
    fn section(mesh: &Mesh) -> Vec<[[f64; 2]; 2]> {
        let mut out = Vec::new();
        for t in &mesh.triangles {
            let v = t.map(|i| mesh.vertices[i as usize]);
            let mut pts: Vec<[f64; 2]> = Vec::new();
            for (a, b) in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])] {
                if a[1] == 0.0 {
                    pts.push([a[0], a[2]]);
                }
                if (a[1] < 0.0 && b[1] > 0.0) || (a[1] > 0.0 && b[1] < 0.0) {
                    let t = a[1] / (a[1] - b[1]);
                    pts.push([a[0] + t * (b[0] - a[0]), a[2] + t * (b[2] - a[2])]);
                }
            }
            pts.dedup();
            if pts.len() == 2 && pts[0] != pts[1] && pts[0][0] > 0.0 && pts[1][0] > 0.0 {
                out.push([pts[0], pts[1]]);
            }
        }
        out
    }

    fn point_to_segment(p: [f64; 2], [a, b]: [[f64; 2]; 2]) -> f64 {
        let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
        let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1])).clamp(0.0, 1.0);
        (ap[0] - t * ab[0]).hypot(ap[1] - t * ab[1])
    }

    /// Closest approach of two non-crossing segment sets.
    fn closest(a: &[[[f64; 2]; 2]], b: &[[[f64; 2]; 2]]) -> f64 {
        let one_way = |a: &[[[f64; 2]; 2]], b: &[[[f64; 2]; 2]]| {
            a.iter()
                .flatten()
                .flat_map(|&p| b.iter().map(move |&s| point_to_segment(p, s)))
                .fold(f64::INFINITY, f64::min)
        };
        one_way(a, b).min(one_way(b, a))
    }

    fn thread(d: f64, p: f64, length: f64, offset: f64) -> Arc<Solid> {
        Arc::new(Solid::thread(d, p, length, offset).unwrap())
    }

    #[test]
    fn thread_section_shows_pitch_and_diameters() {
        let (d, p, length) = (8.0, 1.25, 10.0);
        let mesh = thread(d, p, length, 0.0).mesh().unwrap();
        assert_eq!(mesh.non_manifold_edges(), 0);
        assert_eq!(mesh.shared_positions(), 0);
        let b = mesh.bounds().unwrap();
        assert!(
            b.min[2].abs() < 1e-4 && (b.max[2] - length).abs() < 1e-4,
            "{b:?}"
        );
        let cut = section(&mesh);
        let xs = cut
            .iter()
            .flatten()
            .filter(|q| q[1] > p && q[1] < length - p);
        let (lo, hi) = xs.fold((f64::INFINITY, 0.0f64), |(lo, hi), q| {
            (lo.min(q[0]), hi.max(q[0]))
        });
        assert!((2.0 * hi - d).abs() < 1e-4, "major {}", 2.0 * hi);
        assert!(
            (2.0 * lo - (d - 1.0825 * p)).abs() < 1e-3,
            "minor {}",
            2.0 * lo
        );
        // Crest flats start one pitch apart.
        let mut crests: Vec<f64> = cut
            .iter()
            .filter(|s| (s[0][0] - hi).abs() < 1e-4 && (s[1][0] - hi).abs() < 1e-4)
            .map(|s| s[0][1].min(s[1][1]))
            .collect();
        crests.sort_by(f64::total_cmp);
        crests.dedup_by(|b, a| (*b - *a).abs() < 1e-6); // two triangles share each
        assert!(crests.len() >= 6, "{crests:?}");
        for w in crests.windows(2) {
            assert!((w[1] - w[0] - p).abs() < 1e-4, "{crests:?}");
        }
    }

    /// A nominal bolt and a nut's thread grown by the gap: the two surfaces
    /// stay exactly `gap` apart, yet still overlap radially, so the bolt
    /// can't slide straight out.
    #[test]
    fn rod_and_tap_keep_the_gap_and_engage() {
        let (d, p, gap) = (8.0, 1.25, 0.3);
        let rod = thread(d, p, 12.0, 0.0);
        let tap = thread(d, p, 6.0, gap);
        let (rod_mesh, tap_mesh) = (rod.mesh().unwrap(), tap.mesh().unwrap());
        assert_eq!(tap_mesh.non_manifold_edges(), 0);
        // Away from the end faces (where the two flat ends meet).
        let flanks = |m: &Mesh| -> Vec<_> {
            section(m)
                .into_iter()
                .filter(|s| s.iter().all(|q| q[1] > p && q[1] < 6.0 - p))
                .collect()
        };
        let (rod_cut, tap_cut) = (flanks(&rod_mesh), flanks(&tap_mesh));
        let between = closest(&rod_cut, &tap_cut);
        assert!((between - gap).abs() < 1e-3, "{between}");
        // Radial engagement: the rod's crest less the tap's inner point (its
        // P/4 flat closed: 2·gap over the 60° triangle's bottom) — what
        // `Thread.engagement` reports.
        let h = 3f64.sqrt() / 2.0 * p;
        let x = |cut: &[[[f64; 2]; 2]]| cut.iter().flatten().map(|q| q[0]).collect::<Vec<_>>();
        let rod_out = x(&rod_cut).into_iter().fold(0.0, f64::max);
        let tap_in = x(&tap_cut).into_iter().fold(f64::INFINITY, f64::min);
        let expected = 4.0 - (4.0 - 5.0 * h / 8.0 - h / 4.0 + 2.0 * gap);
        assert!(
            (rod_out - tap_in - expected).abs() < 1e-4,
            "{} vs {expected}",
            rod_out - tap_in
        );
        // In phase (z shift a whole pitch) they don't touch; half a pitch off
        // they collide.
        let block = Arc::new(
            Arc::new(Solid::Box {
                size: [14.0, 14.0, 4.0],
            })
            .translate([0.0, 0.0, 3.0]),
        );
        let overlap = |rod: &Arc<Solid>, tap: &Arc<Solid>, dz: f64| {
            let nut = Arc::new(Solid::boolean(
                BoolOp::Subtract,
                vec![block.clone(), tap.clone()],
            ));
            let moved = Arc::new(nut.translate([0.0, 0.0, dz]));
            Solid::boolean(BoolOp::Intersect, vec![rod.clone(), moved])
                .mesh()
                .unwrap()
                .volume()
        };
        assert!(overlap(&rod, &tap, 2.0 * p).abs() < 1e-6);
        assert!(overlap(&rod, &tap, 2.0 * p + p / 2.0) > 1.0);
        // Rod and tap share angular steps (faceted for the nominal size), so
        // even a 1 µm gap holds between the steps, not just at θ = 0.
        let (d, p) = (4.1534506223, 0.7);
        let (rod, tap) = (thread(d, p, 8.0, 0.0), thread(d, p, 6.0, 0.001));
        assert!(
            overlap(&rod, &tap, 2.0 * p).abs() < 1e-9,
            "{}",
            overlap(&rod, &tap, 2.0 * p)
        );
    }

    #[test]
    fn thread_errors_teach() {
        let err = Solid::thread(3.0, 0.5, 10.0, -1.0).unwrap_err();
        assert!(err.contains("leaves no thread"), "{err}");
        assert!(Solid::thread(8.0, 0.001, 100.0, 0.0)
            .unwrap_err()
            .contains("too many turns"));
    }
}
