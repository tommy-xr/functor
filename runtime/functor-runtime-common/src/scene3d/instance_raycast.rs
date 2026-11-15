//! `Instance.raycast`: a ray against every instance's box, pure and GL-free.
//! Picking among thousands of instanced boxes without physics bodies.

use cgmath::{InnerSpace, Quaternion, Vector3};

use super::InstanceData;

/// Where a ray first meets one of `Instance.raycast`'s boxes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxHit {
    /// The box's position in the instance list.
    pub index: usize,
    /// Along the ray, in the instances' units (0 when the ray starts inside).
    pub distance: f64,
    pub point: [f64; 3],
    /// The entered face's outward unit normal; the reversed ray direction
    /// when the ray starts inside.
    pub normal: [f64; 3],
}

/// The nearest box a ray meets: each instance's unit cube
/// (`[-0.5, 0.5]³` under its scale, rotation, then position — what an
/// instanced `Scene.cube()` draws). Brute force: one slab test per box.
///
/// `None` for a zero or non-finite direction, or no hit. A ray that starts
/// inside or on a box hits it at distance 0. Equal distances go to the lower
/// index; a ray through an edge takes the face of the box's lowest local
/// axis. A box with non-finite data is never hit.
pub fn raycast_instances(
    origin: [f64; 3],
    direction: [f64; 3],
    instances: &[InstanceData],
) -> Option<BoxHit> {
    let length = Vector3::from(direction).magnitude();
    if !(length > 0.0 && length.is_finite()) {
        return None;
    }
    let (o, d) = (Vector3::from(origin), Vector3::from(direction) / length);
    let mut best: Option<BoxHit> = None;
    for (index, instance) in instances.iter().enumerate() {
        let Some((distance, normal)) = slab(o, d, instance) else {
            continue;
        };
        if best.is_some_and(|b| b.distance <= distance) {
            continue;
        }
        let point = (o + d * distance).into();
        let normal = normal.into();
        best = Some(BoxHit { index, distance, point, normal });
    }
    best
}

/// One oriented box against a unit ray: the entry distance and face normal.
fn slab(o: Vector3<f64>, d: Vector3<f64>, instance: &InstanceData) -> Option<(f64, Vector3<f64>)> {
    let p = o - Vector3::from(instance.position).cast::<f64>()?;
    let half = Vector3::from(instance.scale).cast::<f64>()?.map(|s| s.abs() * 0.5);
    let finite = |v: Vector3<f64>| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
    if !finite(p) || !finite(half) || !instance.rotation.iter().all(|c| c.is_finite()) {
        return None;
    }
    // Most boxes miss: reject by the bounding sphere before any rotation.
    // The ray's closest approach to the centre is `p + d·along` (not
    // `|p|² − along²`, which cancels far from the origin).
    let radius2 = half.magnitude2();
    let along = -p.dot(d);
    if (p + d * along).magnitude2() > radius2 || (along < 0.0 && along * along > radius2) {
        return None;
    }
    // Builders keep rotations unit length.
    let [x, y, z, w] = instance.rotation.map(f64::from);
    let q = Quaternion::new(w, x, y, z);
    let (mut near, mut far) = (f64::NEG_INFINITY, f64::INFINITY);
    let mut normal = Vector3::new(0.0, 0.0, 0.0);
    for (axis, unit) in [Vector3::unit_x(), Vector3::unit_y(), Vector3::unit_z()]
        .into_iter()
        .enumerate()
    {
        let u = q * unit;
        let (e, f) = (u.dot(p), u.dot(d));
        if f.abs() < 1e-12 {
            // Parallel to this slab: inside it or never.
            if e.abs() > half[axis] {
                return None;
            }
            continue;
        }
        let (t1, t2) = ((-half[axis] - e) / f, (half[axis] - e) / f);
        let (enter, exit) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
        if enter > near {
            near = enter;
            normal = u * -f.signum();
        }
        far = far.min(exit);
        if near > far {
            return None;
        }
    }
    // With a unit ray some axis is never parallel, so `near` is bounded.
    if !near.is_finite() || far < 0.0 {
        return None;
    }
    if near <= 0.0 {
        return Some((0.0, -d));
    }
    Some((near, normal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{Deg, InnerSpace, Quaternion, Rotation3, SquareMatrix, Vector4};

    fn boxed(position: [f32; 3], scale: [f32; 3]) -> InstanceData {
        InstanceData::at(position).scaled_xyz(scale[0], scale[1], scale[2])
    }

    fn hit(o: [f64; 3], d: [f64; 3], boxes: &[InstanceData]) -> Option<BoxHit> {
        raycast_instances(o, d, boxes)
    }

    #[test]
    fn the_nearest_face_and_its_normal() {
        let boxes = [boxed([0.0, 0.0, 5.0], [2.0, 2.0, 2.0]), boxed([0.0, 0.0, 10.0], [1.0; 3])];
        let h = hit([0.0, 0.0, 0.0], [0.0, 0.0, 2.0], &boxes).unwrap();
        assert_eq!(h.index, 0);
        assert!((h.distance - 4.0).abs() < 1e-9);
        assert_eq!(h.normal, [0.0, 0.0, -1.0]);
        assert!((h.point[2] - 4.0).abs() < 1e-9);
        // Behind the origin, or beside the ray: nothing.
        assert!(hit([0.0, 0.0, 20.0], [0.0, 0.0, 1.0], &boxes).is_none());
        assert!(hit([3.0, 0.0, 0.0], [0.0, 0.0, 1.0], &boxes).is_none());
    }

    #[test]
    fn edge_cases_are_defined() {
        let unit = [boxed([0.0; 3], [1.0; 3])];
        // Starting inside: distance 0 at the origin, normal against the ray.
        let inside = hit([0.1, 0.0, 0.0], [1.0, 0.0, 0.0], &unit).unwrap();
        assert_eq!((inside.distance, inside.point), (0.0, [0.1, 0.0, 0.0]));
        assert_eq!(inside.normal, [-1.0, 0.0, 0.0]);
        // A zero direction hits nothing.
        assert!(hit([0.0, 0.0, -5.0], [0.0; 3], &unit).is_none());
        // Parallel to a slab: inside it hits, outside misses.
        assert!(hit([0.0, 0.4, -5.0], [0.0, 0.0, 1.0], &unit).is_some());
        assert!(hit([0.0, 0.6, -5.0], [0.0, 0.0, 1.0], &unit).is_none());
        // A flat (zero-height) box is hit when the ray crosses it.
        let flat = [boxed([0.0; 3], [1.0, 0.0, 1.0])];
        let down = hit([0.2, 3.0, 0.1], [0.0, -1.0, 0.0], &flat).unwrap();
        assert!((down.distance - 3.0).abs() < 1e-9);
        assert_eq!(down.normal, [0.0, 1.0, 0.0]);
        // Equal distances: the lower index.
        let twins = [boxed([0.0, 0.0, 5.0], [1.0; 3]), boxed([0.0, 0.0, 5.0], [1.0; 3])];
        assert_eq!(hit([0.0; 3], [0.0, 0.0, 1.0], &twins).unwrap().index, 0);
        // Through an edge: the lowest axis's face (X before Z).
        let corner = hit([-5.0, 0.0, -5.0], [1.0, 0.0, 1.0], &unit).unwrap();
        assert_eq!(corner.normal, [-1.0, 0.0, 0.0]);
        // On a face, pointing in: inside, at distance 0.
        let on = hit([0.0, 0.0, -0.5], [0.0, 0.0, 1.0], &unit).unwrap();
        assert_eq!((on.distance, on.normal), (0.0, [0.0, 0.0, -1.0]));
        // Non-finite data is never hit, nor a non-finite ray.
        let nan = [boxed([f32::NAN, 0.0, 0.0], [1.0; 3]), boxed([0.0, 0.0, 5.0], [1.0; 3])];
        assert_eq!(hit([0.0; 3], [0.0, 0.0, 1.0], &nan).unwrap().index, 1);
        assert!(hit([f64::NAN, 0.0, 0.0], [0.0, 0.0, 1.0], &unit).is_none());
        assert!(hit([0.0, 0.0, -5.0], [0.0, 0.0, f64::INFINITY], &unit).is_none());
        // A non-finite scale on an axis the ray is parallel to.
        for s in [f32::NAN, f32::INFINITY] {
            assert!(hit([0.0, 0.0, -5.0], [0.0, 0.0, 1.0], &[boxed([0.0; 3], [s, 1.0, 1.0])]).is_none());
        }
        // Far from the origin, a ray through the centre still hits.
        let far = hit([1e8, 1e8, 0.0], [-1.0, -1.0, 0.0], &unit).unwrap();
        assert!((far.distance - (2.0f64.sqrt() * 1e8 - 0.5 * 2.0f64.sqrt())).abs() < 1e-3, "{far:?}");
    }

    #[test]
    fn rotated_boxes_are_oriented() {
        // A 2×0.2×0.2 bar turned 90° about Y lies along Z.
        let bar = [InstanceData::at([0.0; 3])
            .scaled_xyz(2.0, 0.2, 0.2)
            .rotated(Quaternion::from_angle_y(Deg(90.0)))];
        assert!(hit([0.0, 0.0, -5.0], [0.0, 0.0, 1.0], &bar).is_some());
        assert!(hit([0.8, 0.0, -5.0], [0.0, 0.0, 1.0], &bar).is_none());
        let side = hit([-5.0, 0.0, 0.8], [1.0, 0.0, 0.0], &bar).unwrap();
        assert!((side.distance - 4.9).abs() < 1e-6, "{side:?}");
        assert!((side.normal[0] + 1.0).abs() < 1e-6);
    }

    /// A float in `[lo, hi)` from the crate's seeded generator.
    fn range(rng: &mut crate::net::Rng, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * f64::from(rng.next_f32())
    }

    /// The reference: the ray in each box's local space (through the inverse
    /// instance matrix) against the unit cube, the nearest by world distance.
    fn reference(o: [f64; 3], d: [f64; 3], boxes: &[InstanceData]) -> Option<(usize, f64)> {
        let mut best: Option<(usize, f64)> = None;
        let unit = cgmath::vec3(d[0], d[1], d[2]).normalize();
        for (index, instance) in boxes.iter().enumerate() {
            let inv = instance.matrix().cast::<f64>().unwrap().invert().unwrap();
            let lo = inv * Vector4::new(o[0], o[1], o[2], 1.0);
            let ld = inv * unit.extend(0.0);
            let (mut near, mut far) = (f64::NEG_INFINITY, f64::INFINITY);
            for k in 0..3 {
                let (t1, t2) = ((-0.5 - lo[k]) / ld[k], (0.5 - lo[k]) / ld[k]);
                near = near.max(t1.min(t2));
                far = far.min(t1.max(t2));
            }
            if near > far || far < 0.0 {
                continue;
            }
            // `ld` is a world-unit direction mapped locally, so local t is
            // world distance.
            let t = near.max(0.0);
            if best.is_none_or(|(_, b)| t < b) {
                best = Some((index, t));
            }
        }
        best
    }

    #[test]
    fn matches_a_local_space_reference() {
        let mut rng = crate::net::Rng::new(7);
        let mut hits = 0;
        for _ in 0..500 {
            let boxes: Vec<InstanceData> = (0..20)
                .map(|_| {
                    let at = [0; 3].map(|_| range(&mut rng, -10.0, 10.0) as f32);
                    let size = [0; 3].map(|_| range(&mut rng, 0.2, 3.0) as f32);
                    boxed(at, size)
                        .rotated(Quaternion::from_angle_y(Deg(range(&mut rng, 0.0, 360.0) as f32)))
                        .rotated(Quaternion::from_angle_x(Deg(range(&mut rng, 0.0, 360.0) as f32)))
                })
                .collect();
            let o = [0; 3].map(|_| range(&mut rng, -15.0, 15.0));
            // Aim near a random box, so most rays hit something.
            let aim = boxes[rng.range_u32(0, boxes.len() as u32 - 1) as usize].position;
            let d: [f64; 3] =
                std::array::from_fn(|k| f64::from(aim[k]) + range(&mut rng, -1.5, 1.5) - o[k]);
            let got = hit(o, d, &boxes);
            match (got, reference(o, d, &boxes)) {
                (Some(h), Some((wi, wd))) => {
                    hits += 1;
                    assert!((h.distance - wd).abs() < 1e-4, "{h:?} vs {wi} {wd}");
                    // The same box, unless another ties it at that distance.
                    let tied = reference(o, d, &[boxes[h.index].clone()]).map(|(_, t)| t);
                    assert!(h.index == wi || tied.is_some_and(|t| (t - wd).abs() < 1e-4));
                    // A unit normal facing the ray (or reversing it from inside).
                    let n = cgmath::Vector3::from(h.normal);
                    assert!((n.magnitude() - 1.0).abs() < 1e-6); // rotations are stored as f32
                    assert!(n.dot(cgmath::Vector3::from(d)) < 0.0, "{h:?}");
                }
                (None, None) => {}
                (got, want) => panic!("{got:?} vs {want:?} for {o:?} {d:?}"),
            }
        }
        assert!(hits > 100, "too few hits to mean anything: {hits}");
    }
}
