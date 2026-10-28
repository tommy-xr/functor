//! Cells and portals: space split into convex cells, joined by convex portal
//! polygons. Pure, immutable data, so the sim queries (`locate`, `segment`)
//! are deterministic and replay-safe.

use cgmath::{InnerSpace, Matrix4, Vector3, Vector4};
use serde::{Deserialize, Serialize};

use crate::inspect::Aabb;

/// Boundary tolerance in scene units (1 mm): validation, `locate` ties and
/// portal crossings.
pub const TOLERANCE: f32 = 1e-3;

/// A cell's inward plane: `n·p + d ≥ 0` inside.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plane {
    pub n: [f32; 3],
    pub d: f32,
}

impl Plane {
    pub fn distance(&self, p: Vector3<f32>) -> f32 {
        Vector3::from(self.n).dot(p) + self.d
    }
}

/// A convex cell: the points inside all its planes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub planes: Vec<Plane>,
}

/// A convex polygon on a face of `from`, leading to `to`. One-way: the walk
/// from `to` back to `from` needs its own portal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Portal {
    pub from: u32,
    pub to: u32,
    pub polygon: Vec<[f32; 3]>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CellsData {
    pub cells: Vec<Cell>,
    pub portals: Vec<Portal>,
}

/// Validated [`CellsData`] (unit plane normals) plus lookup tables.
/// Serializes as its data; deserializing validates again.
#[derive(Debug)]
pub struct Cells {
    data: CellsData,
    // Per cell: its box (within the tolerance) and its portals' indices.
    bounds: Vec<Aabb>,
    outgoing: Vec<Vec<u32>>,
    // Per portal: the `from` face it lies on, and its unit normal wound
    // like its corners.
    faces: Vec<(Plane, Vector3<f32>)>,
}

impl PartialEq for Cells {
    fn eq(&self, other: &Cells) -> bool {
        self.data == other.data
    }
}

impl Serialize for Cells {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.data.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Cells {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Cells, D::Error> {
        Cells::new(CellsData::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

fn v(p: [f32; 3]) -> Vector3<f32> {
    Vector3::from(p)
}

impl Cells {
    /// Validate: every cell bounded and non-empty; every portal flat, convex,
    /// with area, on a face of `from`, and between two existing cells.
    pub fn new(mut data: CellsData) -> Result<Cells, String> {
        let count = data.cells.len();
        let mut bounds = Vec::with_capacity(count);
        for (i, cell) in data.cells.iter_mut().enumerate() {
            for plane in &mut cell.planes {
                let n = v(plane.n);
                let len = n.magnitude();
                if !(len > 1e-6 && len.is_finite() && plane.d.is_finite()) {
                    return Err(format!(
                        "cell {i} has a plane with a zero or non-finite normal"
                    ));
                }
                *plane = Plane {
                    n: (n / len).into(),
                    d: plane.d / len,
                };
            }
            bounds.push(cell_bounds(&cell.planes).map_err(|e| format!("cell {i} {e}"))?);
        }

        let mut outgoing = vec![Vec::new(); count];
        let mut faces = Vec::with_capacity(data.portals.len());
        for (k, portal) in data.portals.iter().enumerate() {
            let what = format!("portal {k} ({} → {})", portal.from, portal.to);
            let (from, to) = (portal.from as usize, portal.to as usize);
            if from >= count || to >= count {
                return Err(format!(
                    "{what} names a cell that doesn't exist (there are {count})"
                ));
            }
            if from == to {
                return Err(format!("{what} leads back into its own cell"));
            }
            let polygon: Vec<Vector3<f32>> = portal.polygon.iter().map(|&p| v(p)).collect();
            if polygon.len() < 3 {
                return Err(format!(
                    "{what} has {} corners; it needs 3 or more",
                    polygon.len()
                ));
            }
            if polygon
                .iter()
                .any(|p| !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()))
            {
                return Err(format!("{what} has a corner that isn't a finite point"));
            }
            // Newell's normal: twice the area, pointing the way the corners wind.
            let newell = (0..polygon.len()).fold(Vector3::new(0.0, 0.0, 0.0), |acc, i| {
                acc + polygon[i].cross(polygon[(i + 1) % polygon.len()])
            });
            if newell.magnitude() < 1e-6 {
                return Err(format!("{what} has no area"));
            }
            let normal = newell.normalize();
            let center = polygon.iter().sum::<Vector3<f32>>() / polygon.len() as f32;
            if polygon
                .iter()
                .any(|p| normal.dot(p - center).abs() > TOLERANCE)
            {
                return Err(format!("{what} isn't flat"));
            }
            if !polygon
                .iter()
                .all(|&p| inside_edges(&portal.polygon, normal, p))
            {
                return Err(format!("{what} isn't convex"));
            }
            // On a face of both cells: corners on one of its planes, inside the rest.
            let face_of = |cell: usize| {
                let planes = &data.cells[cell].planes;
                let inside = polygon
                    .iter()
                    .all(|&p| planes.iter().all(|plane| plane.distance(p) >= -TOLERANCE));
                planes
                    .iter()
                    .find(|plane| {
                        polygon
                            .iter()
                            .all(|&p| plane.distance(p).abs() <= TOLERANCE)
                    })
                    .filter(|_| inside)
            };
            let Some(face) = face_of(from) else {
                return Err(format!("{what} isn't on a face of cell {from}"));
            };
            if face_of(to).is_none() {
                return Err(format!("{what} isn't on a face of cell {to}"));
            }
            faces.push((*face, normal));
            outgoing[from].push(k as u32);
        }
        Ok(Cells {
            data,
            bounds,
            outgoing,
            faces,
        })
    }

    pub fn data(&self) -> &CellsData {
        &self.data
    }

    pub fn len(&self) -> usize {
        self.data.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.cells.is_empty()
    }

    fn contains(&self, cell: usize, p: Vector3<f32>) -> bool {
        let b = &self.bounds[cell];
        (b.min.x <= p.x && p.x <= b.max.x)
            && (b.min.y <= p.y && p.y <= b.max.y)
            && (b.min.z <= p.z && p.z <= b.max.z)
            && self.data.cells[cell]
                .planes
                .iter()
                .all(|plane| plane.distance(p) >= -TOLERANCE)
    }

    /// The cell containing `p` (within the tolerance); the lowest id on a
    /// shared boundary.
    pub fn locate(&self, p: Vector3<f32>) -> Option<u32> {
        (0..self.len())
            .find(|&cell| self.contains(cell, p))
            .map(|cell| cell as u32)
    }

    /// Line of sight from `a` to `b`: walk the segment cell to cell, leaving
    /// each only through a portal. False when either end is outside every
    /// cell or the segment meets a wall.
    pub fn segment(&self, a: Vector3<f32>, b: Vector3<f32>) -> bool {
        // On a shared face `a` is in several cells; any may start the walk.
        (0..self.len()).any(|cell| self.contains(cell, a) && self.walk(cell, a, b))
    }

    fn walk(&self, mut cell: usize, a: Vector3<f32>, b: Vector3<f32>) -> bool {
        let dir = b - a;
        let mut t = 0.0f32;
        // A segment crosses each convex cell once, so this bounds the walk.
        for _ in 0..self.len() {
            let planes = &self.data.cells[cell].planes;
            if planes.iter().all(|plane| plane.distance(b) >= -TOLERANCE) {
                return true;
            }
            // Where the segment leaves this cell.
            let exit = planes
                .iter()
                .filter_map(|plane| {
                    let s = v(plane.n).dot(dir);
                    (s < 0.0).then(|| -plane.distance(a) / s)
                })
                .fold(f32::INFINITY, f32::min)
                .max(t);
            let x = a + dir * exit;
            // A portal on a face the segment leaves through, containing `x`.
            let through = self.outgoing[cell].iter().find(|&&k| {
                let (face, normal) = &self.faces[k as usize];
                v(face.n).dot(dir) < 0.0
                    && face.distance(x).abs() <= TOLERANCE
                    && inside_edges(&self.data.portals[k as usize].polygon, *normal, x)
            });
            let Some(&k) = through else {
                return false;
            };
            cell = self.data.portals[k as usize].to as usize;
            t = exit;
        }
        false
    }
}

/// A screen rectangle in NDC: `[min x, min y, max x, max y]`.
type Rect = [f32; 4];

impl Cells {
    /// The cells an eye at `eye` sees through portals, flooding from its
    /// cell and narrowing a screen rectangle at each portal. `clip` is the
    /// eye's `projection * view * world`, `eye` in this cells' space. `None`
    /// (draw every cell) when the eye is outside every cell, or the walk runs
    /// long.
    pub fn visible(&self, eye: Vector3<f32>, clip: &Matrix4<f32>) -> Option<Vec<bool>> {
        let start = self.locate(eye)? as usize;
        let mut reached: Vec<Option<Rect>> = vec![None; self.len()];
        let mut stack = vec![(start, [-1.0, -1.0, 1.0, 1.0])];
        // Each visit reaches a new cell or grows its rectangle; cap the
        // pathological case.
        let mut budget = 16 * (self.len() + self.data.portals.len());
        while let Some((cell, rect)) = stack.pop() {
            // A cell reached again is re-walked with the union of its
            // rectangles, unless the new one adds nothing.
            let rect = match reached[cell] {
                Some(seen) if contains(seen, rect) => continue,
                Some(seen) => union(seen, rect),
                None => rect,
            };
            reached[cell] = Some(rect);
            budget = budget.checked_sub(1)?;
            for &k in &self.outgoing[cell] {
                let k = k as usize;
                let distance = self.faces[k].0.distance(eye);
                // Leaving a cell through a face needs the eye on its inner side.
                // On the portal's plane its projection degenerates, so it
                // passes the whole rectangle.
                let through = if distance < -TOLERANCE {
                    None
                } else if distance <= TOLERANCE {
                    Some(rect)
                } else {
                    project(clip, &self.data.portals[k].polygon).and_then(|p| intersect(rect, p))
                };
                if let Some(through) = through {
                    stack.push((self.data.portals[k].to as usize, through));
                }
            }
        }
        Some(reached.iter().map(Option::is_some).collect())
    }
}

/// A polygon's screen rectangle under `clip`, clipped to the part in front
/// of the eye (`w > 0`) before the divide. `None` when it is all behind.
fn project(clip: &Matrix4<f32>, polygon: &[[f32; 3]]) -> Option<Rect> {
    const W_MIN: f32 = 1e-5;
    let corners = polygon.iter().map(|&p| clip * v(p).extend(1.0));
    let mut rect = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    let mut add = |c: Vector4<f32>| {
        let (x, y) = (c.x / c.w, c.y / c.w);
        rect = [
            rect[0].min(x),
            rect[1].min(y),
            rect[2].max(x),
            rect[3].max(y),
        ];
    };
    let ring: Vec<Vector4<f32>> = corners.collect();
    for (i, &a) in ring.iter().enumerate() {
        let b = ring[(i + 1) % ring.len()];
        if a.w >= W_MIN {
            add(a);
        }
        // An edge crossing w = W_MIN adds its crossing point.
        if (a.w >= W_MIN) != (b.w >= W_MIN) {
            add(a + (b - a) * ((W_MIN - a.w) / (b.w - a.w)));
        }
    }
    (rect[0] <= rect[2]).then_some(rect)
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && inner[2] <= outer[2] && inner[3] <= outer[3]
}

fn union(a: Rect, b: Rect) -> Rect {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// The overlap of two rectangles, if it has area.
fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let r = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (r[0] < r[2] && r[1] < r[3]).then_some(r)
}

/// Whether `p` is inside every edge of the convex `polygon` (wound about
/// `normal`), within the tolerance. Ignores distance from its plane.
fn inside_edges(polygon: &[[f32; 3]], normal: Vector3<f32>, p: Vector3<f32>) -> bool {
    (0..polygon.len()).all(|i| {
        let a = v(polygon[i]);
        let edge = v(polygon[(i + 1) % polygon.len()]) - a;
        edge.cross(p - a).dot(normal) >= -TOLERANCE * edge.magnitude()
    })
}

/// The box of a cell grown by the tolerance (every point `locate` accepts),
/// from the corners where its tolerance-offset planes meet. A far box closes
/// an open cell, so a corner on it means unbounded.
fn cell_bounds(planes: &[Plane]) -> Result<Aabb, &'static str> {
    const FAR: f64 = 1e7;
    let mut all: Vec<([f64; 3], f64)> = planes
        .iter()
        .map(|p| (p.n.map(f64::from), f64::from(p.d) + f64::from(TOLERANCE)))
        .collect();
    for axis in 0..3 {
        for sign in [1.0, -1.0] {
            let mut n = [0.0; 3];
            n[axis] = sign;
            all.push((n, FAR));
        }
    }
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let mut bounds = Aabb::empty();
    for i in 0..all.len() {
        for j in i + 1..all.len() {
            for k in j + 1..all.len() {
                let ((ni, di), (nj, dj), (nk, dk)) = (all[i], all[j], all[k]);
                let (jk, ki, ij) = (cross(nj, nk), cross(nk, ni), cross(ni, nj));
                let det = dot(ni, jk);
                if det.abs() < 1e-9 {
                    continue;
                }
                let p: [f64; 3] =
                    std::array::from_fn(|c| -(di * jk[c] + dj * ki[c] + dk * ij[c]) / det);
                if !all.iter().all(|&(n, d)| dot(n, p) + d >= -1e-6) {
                    continue;
                }
                if p.iter().any(|c| c.abs() > FAR / 2.0) {
                    return Err("is unbounded: its planes don't close it");
                }
                bounds.extend(Vector3::new(p[0] as f32, p[1] as f32, p[2] as f32));
            }
        }
    }
    if bounds.is_empty() {
        return Err("is empty: its planes enclose no point");
    }
    Ok(bounds)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An axis-aligned box cell.
    pub fn room(min: [f32; 3], max: [f32; 3]) -> Cell {
        let plane = |n: [f32; 3], d: f32| Plane { n, d };
        Cell {
            planes: vec![
                plane([1.0, 0.0, 0.0], -min[0]),
                plane([-1.0, 0.0, 0.0], max[0]),
                plane([0.0, 1.0, 0.0], -min[1]),
                plane([0.0, -1.0, 0.0], max[1]),
                plane([0.0, 0.0, 1.0], -min[2]),
                plane([0.0, 0.0, -1.0], max[2]),
            ],
        }
    }

    /// Both directions of a doorway: `polygon` lies on the shared face.
    pub fn door(a: u32, b: u32, polygon: Vec<[f32; 3]>) -> [Portal; 2] {
        let mut back = polygon.clone();
        back.reverse();
        [
            Portal {
                from: a,
                to: b,
                polygon,
            },
            Portal {
                from: b,
                to: a,
                polygon: back,
            },
        ]
    }

    /// A 1 × 2 doorway in the wall x = `x`, centred on z = 0.
    pub fn x_door(x: f32) -> Vec<[f32; 3]> {
        vec![[x, 0.0, -0.5], [x, 0.0, 0.5], [x, 2.0, 0.5], [x, 2.0, -0.5]]
    }

    /// Three 4 × 3 × 4 rooms along X (0..4, 4..8, 8..12), a doorway in
    /// each shared wall.
    pub fn three_rooms() -> Cells {
        let rooms = (0..3)
            .map(|i| {
                let x = 4.0 * i as f32;
                room([x, 0.0, -2.0], [x + 4.0, 3.0, 2.0])
            })
            .collect();
        let mut portals = Vec::new();
        portals.extend(door(0, 1, x_door(4.0)));
        portals.extend(door(1, 2, x_door(8.0)));
        Cells::new(CellsData {
            cells: rooms,
            portals,
        })
        .unwrap()
    }

    /// An L: a corridor along X (cell 0, x 0..4, z 0..2), a corner (cell 1,
    /// x 4..6, z 0..2), and a leg along Z (cell 2, x 4..6, z 2..6).
    pub fn l_corridor() -> Cells {
        let cells = vec![
            room([0.0, 0.0, 0.0], [4.0, 3.0, 2.0]),
            room([4.0, 0.0, 0.0], [6.0, 3.0, 2.0]),
            room([4.0, 0.0, 2.0], [6.0, 3.0, 6.0]),
        ];
        let mut portals = Vec::new();
        portals.extend(door(
            0,
            1,
            vec![
                [4.0, 0.0, 0.0],
                [4.0, 0.0, 2.0],
                [4.0, 3.0, 2.0],
                [4.0, 3.0, 0.0],
            ],
        ));
        portals.extend(door(
            1,
            2,
            vec![
                [4.0, 0.0, 2.0],
                [6.0, 0.0, 2.0],
                [6.0, 3.0, 2.0],
                [4.0, 3.0, 2.0],
            ],
        ));
        Cells::new(CellsData { cells, portals }).unwrap()
    }

    fn p(x: f32, y: f32, z: f32) -> Vector3<f32> {
        Vector3::new(x, y, z)
    }

    #[test]
    fn locate_finds_the_room_and_ties_go_to_the_lowest_id() {
        let cells = three_rooms();
        assert_eq!(cells.locate(p(1.0, 1.0, 0.0)), Some(0));
        assert_eq!(cells.locate(p(6.0, 1.0, 1.0)), Some(1));
        assert_eq!(cells.locate(p(11.0, 2.0, -1.0)), Some(2));
        // On the wall between rooms 1 and 2, and within the tolerance of it.
        assert_eq!(cells.locate(p(8.0, 1.0, 0.0)), Some(1));
        assert_eq!(cells.locate(p(8.0 - 0.0005, 1.0, 0.0)), Some(1));
        assert_eq!(cells.locate(p(8.0005, 1.0, 0.0)), Some(1));
        assert_eq!(cells.locate(p(8.01, 1.0, 0.0)), Some(2));
        // Outside every room.
        assert_eq!(cells.locate(p(1.0, 5.0, 0.0)), None);
        assert_eq!(cells.locate(p(-0.01, 1.0, 0.0)), None);
        assert_eq!(cells.locate(p(f32::NAN, 1.0, 0.0)), None);
    }

    #[test]
    fn segment_passes_doorways_and_stops_at_walls() {
        let cells = three_rooms();
        // Straight down the doorways, room 0 to room 2.
        assert!(cells.segment(p(1.0, 1.0, 0.0), p(11.0, 1.0, 0.0)));
        assert!(cells.segment(p(11.0, 1.0, 0.0), p(1.0, 1.0, 0.0)));
        // Within one room.
        assert!(cells.segment(p(1.0, 1.0, -1.5), p(3.0, 2.5, 1.5)));
        // Through the wall beside the doorway.
        assert!(!cells.segment(p(1.0, 1.0, 1.5), p(6.0, 1.0, 1.5)));
        // Over the doorway's lintel (it is 2 high, the room 3).
        assert!(!cells.segment(p(1.0, 2.5, 0.0), p(6.0, 2.5, 0.0)));
        // An end outside every cell.
        assert!(!cells.segment(p(1.0, 1.0, 0.0), p(1.0, 5.0, 0.0)));
        assert!(!cells.segment(p(-1.0, 1.0, 0.0), p(1.0, 1.0, 0.0)));
    }

    #[test]
    fn segment_around_an_l_corner_is_blocked_and_down_each_leg_is_clear() {
        let cells = l_corridor();
        // Corridor start to the far end of the leg: cuts the inside corner.
        assert!(!cells.segment(p(0.5, 1.0, 1.0), p(5.0, 1.0, 5.5)));
        // From the corner both legs are clear.
        assert!(cells.segment(p(5.0, 1.0, 1.0), p(0.5, 1.0, 1.0)));
        assert!(cells.segment(p(5.0, 1.0, 1.0), p(5.0, 1.0, 5.5)));
        // Diagonally across the corner cell into the leg, via both portals.
        assert!(cells.segment(p(3.5, 1.0, 1.0), p(4.8, 1.0, 2.5)));
        // Through the corner where both portals meet: leaves by the first
        // portal, not back by its return.
        assert!(cells.segment(p(3.0, 1.0, 1.0), p(5.0, 1.0, 3.0)));
    }

    #[test]
    fn locate_accepts_the_tolerance_at_acute_corners() {
        // A prism |x| + |y| ≤ 1, 0 ≤ z ≤ 1: 1.2 mm past the x = 1 corner is
        // within 1 mm of both slanted planes.
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let plane = |n: [f32; 3], d: f32| Plane { n, d };
        let prism = Cell {
            planes: vec![
                plane([-s, -s, 0.0], s),
                plane([-s, s, 0.0], s),
                plane([s, -s, 0.0], s),
                plane([s, s, 0.0], s),
                plane([0.0, 0.0, 1.0], 0.0),
                plane([0.0, 0.0, -1.0], 1.0),
            ],
        };
        let cells = Cells::new(CellsData {
            cells: vec![prism],
            portals: vec![],
        })
        .unwrap();
        assert_eq!(cells.locate(p(1.0012, 0.0, 0.5)), Some(0));
        assert_eq!(cells.locate(p(1.002, 0.0, 0.5)), None);
    }

    #[test]
    fn far_from_the_origin_is_still_bounded() {
        let far = room([60_000.0, 0.0, 0.0], [60_004.0, 3.0, 4.0]);
        assert!(Cells::new(CellsData {
            cells: vec![far],
            portals: vec![]
        })
        .is_ok());
    }

    #[test]
    fn segment_through_a_corner_where_four_rooms_meet() {
        // A 2 × 2 grid of 4 × 4 rooms, every shared face fully open.
        let at =
            |i: f32, j: f32| room([4.0 * i, 0.0, 4.0 * j], [4.0 * i + 4.0, 3.0, 4.0 * j + 4.0]);
        let x_face = |z: f32| {
            vec![
                [4.0, 0.0, z],
                [4.0, 0.0, z + 4.0],
                [4.0, 3.0, z + 4.0],
                [4.0, 3.0, z],
            ]
        };
        let z_face = |x: f32| {
            vec![
                [x, 0.0, 4.0],
                [x + 4.0, 0.0, 4.0],
                [x + 4.0, 3.0, 4.0],
                [x, 3.0, 4.0],
            ]
        };
        let mut portals = Vec::new();
        portals.extend(door(0, 1, x_face(0.0)));
        portals.extend(door(2, 3, x_face(4.0)));
        portals.extend(door(0, 2, z_face(0.0)));
        portals.extend(door(1, 3, z_face(4.0)));
        let cells = Cells::new(CellsData {
            cells: vec![at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0)],
            portals,
        })
        .unwrap();
        for off in [0.0, 0.0005] {
            assert!(
                cells.segment(p(2.0, 1.0, 2.0 + off), p(6.0, 1.0, 6.0)),
                "{off}"
            );
            assert!(
                cells.segment(p(6.0, 1.0, 6.0), p(2.0, 1.0, 2.0 + off)),
                "{off}"
            );
        }
    }

    #[test]
    fn segment_from_a_shared_wall_starts_in_either_room() {
        // 0.5 mm into room 1, on the wall room 0 wins `locate` ties for.
        let cells = three_rooms();
        assert_eq!(cells.locate(p(4.0005, 1.0, 1.5)), Some(0));
        assert!(cells.segment(p(4.0005, 1.0, 1.5), p(7.0, 1.0, 1.5)));
    }

    #[test]
    fn a_one_way_portal_only_lets_sight_through_one_way() {
        let mut data = three_rooms().data().clone();
        // Drop room 1 → room 0, keeping room 0 → room 1.
        data.portals
            .retain(|portal| !(portal.from == 1 && portal.to == 0));
        let cells = Cells::new(data).unwrap();
        assert!(cells.segment(p(1.0, 1.0, 0.0), p(6.0, 1.0, 0.0)));
        assert!(!cells.segment(p(6.0, 1.0, 0.0), p(1.0, 1.0, 0.0)));
    }

    /// The cells seen from `eye` looking at `target` (90° square view).
    fn seen(cells: &Cells, eye: [f32; 3], target: [f32; 3]) -> Option<Vec<bool>> {
        use cgmath::{perspective, Deg, Point3};
        let clip = perspective(Deg(90.0), 1.0, 0.1, 100.0)
            * Matrix4::look_at_rh(Point3::from(eye), Point3::from(target), Vector3::unit_y());
        cells.visible(v(eye), &clip)
    }

    #[test]
    fn the_walk_sees_through_aligned_doorways_only() {
        let cells = three_rooms();
        let all = Some(vec![true, true, true]);
        assert_eq!(seen(&cells, [1.0, 1.5, 0.0], [12.0, 1.5, 0.0]), all);
        // Beside the doorway the next one is out of line: room 2 is hidden.
        assert_eq!(
            seen(&cells, [1.0, 1.5, 1.5], [12.0, 1.5, 1.5]),
            Some(vec![true, true, false])
        );
        // Facing away, or at a side wall with doorways off to the sides.
        assert_eq!(
            seen(&cells, [2.0, 1.5, 0.0], [-10.0, 1.5, 0.0]),
            Some(vec![true, false, false])
        );
        assert_eq!(
            seen(&cells, [6.0, 1.5, 0.0], [6.0, 1.5, 10.0]),
            Some(vec![false, true, false])
        );
        // Standing in a doorway (on its plane) sees through it.
        assert_eq!(seen(&cells, [4.0, 1.0, 0.0], [12.0, 1.0, 0.0]), all);
        // Outside every cell: draw everything.
        assert_eq!(seen(&cells, [1.0, 5.0, 0.0], [12.0, 5.0, 0.0]), None);
    }

    #[test]
    fn an_l_corner_hides_the_far_leg_until_the_corner() {
        // Corridor 0 (x 0..4, z 0..2) → corner 1 (x 4..6) through a door at
        // z 0..1; corner → leg 2 (z 2..6) through a door at x 5..6.
        let cells = Cells::new(CellsData {
            cells: vec![
                room([0.0, 0.0, 0.0], [4.0, 3.0, 2.0]),
                room([4.0, 0.0, 0.0], [6.0, 3.0, 2.0]),
                room([4.0, 0.0, 2.0], [6.0, 3.0, 6.0]),
            ],
            portals: [
                door(
                    0,
                    1,
                    vec![
                        [4.0, 0.0, 0.0],
                        [4.0, 0.0, 1.0],
                        [4.0, 2.0, 1.0],
                        [4.0, 2.0, 0.0],
                    ],
                ),
                door(
                    1,
                    2,
                    vec![
                        [5.0, 0.0, 2.0],
                        [6.0, 0.0, 2.0],
                        [6.0, 2.0, 2.0],
                        [5.0, 2.0, 2.0],
                    ],
                ),
            ]
            .concat(),
        })
        .unwrap();
        assert_eq!(
            seen(&cells, [0.5, 1.0, 0.5], [6.0, 1.0, 0.5]),
            Some(vec![true, true, false])
        );
        assert_eq!(
            seen(&cells, [4.5, 1.0, 0.5], [5.5, 1.0, 6.0]),
            Some(vec![false, true, true])
        );
    }

    #[test]
    fn a_cell_reached_again_through_a_wider_view_is_walked_again() {
        // Two doorways from room 0 into room 1, and room 2 beyond only lined
        // up with the second. The walk takes the first doorway first (last
        // listed), reaching room 1 with a view that misses room 2's door.
        let x_door = |x: f32, z: f32| {
            vec![
                [x, 0.0, z - 0.3],
                [x, 0.0, z + 0.3],
                [x, 2.0, z + 0.3],
                [x, 2.0, z - 0.3],
            ]
        };
        let rooms = (0..3)
            .map(|i| {
                room(
                    [4.0 * i as f32, 0.0, -2.0],
                    [4.0 * i as f32 + 4.0, 3.0, 2.0],
                )
            })
            .collect();
        let cells = Cells::new(CellsData {
            cells: rooms,
            portals: [
                door(0, 1, x_door(4.0, 1.5)),
                door(0, 1, x_door(4.0, -1.5)),
                door(1, 2, x_door(8.0, 1.5)),
            ]
            .concat(),
        })
        .unwrap();
        assert_eq!(
            seen(&cells, [1.0, 1.0, 1.5], [12.0, 1.0, 1.5]),
            Some(vec![true, true, true])
        );
    }

    #[test]
    fn each_eye_walks_from_its_own_cell() {
        // Eyes 1.5 m apart, both facing +X: only the one in line sees room 2.
        let cells = three_rooms();
        assert_eq!(
            seen(&cells, [1.0, 1.5, 0.0], [12.0, 1.5, 0.0]).unwrap()[2],
            true
        );
        assert_eq!(
            seen(&cells, [1.0, 1.5, 1.5], [12.0, 1.5, 1.5]).unwrap()[2],
            false
        );
        // Eyes straddling a doorway: each starts in its own room.
        assert_eq!(
            seen(&cells, [3.97, 1.0, 0.0], [3.97, 1.0, 10.0]),
            Some(vec![true, true, false])
        );
        assert_eq!(
            seen(&cells, [4.03, 1.0, 0.0], [4.03, 1.0, 10.0]),
            Some(vec![true, true, false])
        );
        assert_eq!(
            seen(&cells, [3.0, 1.0, 1.5], [3.0, 1.0, 10.0]).unwrap()[1],
            false
        );
    }

    #[test]
    fn make_rejects_bad_cells_and_portals() {
        let err = |data: CellsData| Cells::new(data).unwrap_err();
        let unit = || room([0.0; 3], [1.0; 3]);
        let mut open = unit();
        open.planes.pop();
        assert_eq!(
            err(CellsData {
                cells: vec![open],
                portals: vec![]
            }),
            "cell 0 is unbounded: its planes don't close it"
        );
        let mut empty = unit();
        empty.planes[0].d = -2.0; // x ≥ 2 and x ≤ 1
        assert_eq!(
            err(CellsData {
                cells: vec![empty],
                portals: vec![]
            }),
            "cell 0 is empty: its planes enclose no point"
        );
        let mut zero = unit();
        zero.planes[0].n = [0.0; 3];
        assert!(err(CellsData {
            cells: vec![zero],
            portals: vec![]
        })
        .contains("zero or non-finite normal"));

        let two = || vec![unit(), room([1.0, 0.0, 0.0], [2.0, 1.0, 1.0])];
        let face = vec![
            [1.0, 0.2, 0.2],
            [1.0, 0.2, 0.8],
            [1.0, 0.8, 0.8],
            [1.0, 0.8, 0.2],
        ];
        let portal = |from, to, polygon: Vec<[f32; 3]>| CellsData {
            cells: two(),
            portals: vec![Portal { from, to, polygon }],
        };
        assert!(Cells::new(portal(0, 1, face.clone())).is_ok());
        assert_eq!(
            err(portal(0, 2, face.clone())),
            "portal 0 (0 → 2) names a cell that doesn't exist (there are 2)"
        );
        assert_eq!(
            err(portal(0, 0, face.clone())),
            "portal 0 (0 → 0) leads back into its own cell"
        );
        assert!(err(portal(0, 1, face[..2].to_vec())).contains("it needs 3 or more"));
        // Off the face: the plane x = 0.5 cuts through cell 0.
        let inside: Vec<[f32; 3]> = face.iter().map(|&[_, y, z]| [0.5, y, z]).collect();
        assert_eq!(
            err(portal(0, 1, inside)),
            "portal 0 (0 → 1) isn't on a face of cell 0"
        );
        // On cell 0's face, but cell 1 starts 1 m further on.
        let gap = CellsData {
            cells: vec![unit(), room([2.0, 0.0, 0.0], [3.0, 1.0, 1.0])],
            portals: vec![Portal {
                from: 0,
                to: 1,
                polygon: face.clone(),
            }],
        };
        assert_eq!(err(gap), "portal 0 (0 → 1) isn't on a face of cell 1");
        // On the face's plane but past the cell's edge.
        let past: Vec<[f32; 3]> = face.iter().map(|&[x, y, z]| [x, y + 0.5, z]).collect();
        assert!(err(portal(0, 1, past)).contains("isn't on a face"));
        let mut bent = face.clone();
        bent[2][0] = 1.1;
        assert_eq!(err(portal(0, 1, bent)), "portal 0 (0 → 1) isn't flat");
        let dart = vec![
            [1.0, 0.2, 0.2],
            [1.0, 0.4, 0.5],
            [1.0, 0.2, 0.8],
            [1.0, 0.8, 0.5],
        ];
        assert_eq!(err(portal(0, 1, dart)), "portal 0 (0 → 1) isn't convex");
        let line = vec![[1.0, 0.2, 0.2], [1.0, 0.5, 0.5], [1.0, 0.8, 0.8]];
        assert_eq!(err(portal(0, 1, line)), "portal 0 (0 → 1) has no area");
    }

    #[test]
    fn make_normalizes_planes_and_round_trips_through_serde() {
        let mut cell = room([0.0; 3], [1.0; 3]);
        cell.planes[0] = Plane {
            n: [2.0, 0.0, 0.0],
            d: -1.0,
        };
        let cells = Cells::new(CellsData {
            cells: vec![cell],
            portals: vec![],
        })
        .unwrap();
        // 2x - 1 ≥ 0 is x ≥ 0.5.
        assert_eq!(
            cells.data().cells[0].planes[0],
            Plane {
                n: [1.0, 0.0, 0.0],
                d: -0.5
            }
        );
        assert_eq!(cells.locate(p(0.25, 0.5, 0.5)), None);
        let json = serde_json::to_string(&cells).unwrap();
        let back: Cells = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cells);
        // Deserializing validates.
        let bad = json.replace("-0.5", "-5.0");
        assert!(serde_json::from_str::<Cells>(&bad).is_err());
    }
}
