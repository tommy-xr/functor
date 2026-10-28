//! Cells and portals: space split into convex cells, joined by convex portal
//! polygons. Pure, immutable data, so the sim queries (`locate`, `segment`)
//! are deterministic and replay-safe.

use cgmath::{InnerSpace, Vector3};
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
    // Per cell: its box, grown by the tolerance, and its portals' indices.
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
            if !polygon.iter().all(|&p| inside_edges(&polygon, normal, p)) {
                return Err(format!("{what} isn't convex"));
            }
            let planes = &data.cells[from].planes;
            let on = |plane: &Plane| {
                polygon
                    .iter()
                    .all(|&p| plane.distance(p).abs() <= TOLERANCE)
            };
            let inside = polygon
                .iter()
                .all(|&p| planes.iter().all(|plane| plane.distance(p) >= -TOLERANCE));
            let face = planes.iter().find(|plane| on(plane)).filter(|_| inside);
            let Some(face) = face else {
                return Err(format!("{what} isn't on a face of cell {from}"));
            };
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
        let Some(mut cell) = self.locate(a).map(|c| c as usize) else {
            return false;
        };
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
            let through = self.outgoing[cell].iter().find(|&&k| {
                let (face, normal) = &self.faces[k as usize];
                face.distance(x).abs() <= TOLERANCE
                    && inside_edges(&self.polygon(k as usize), *normal, x)
            });
            let Some(&k) = through else {
                return false;
            };
            cell = self.data.portals[k as usize].to as usize;
            t = exit;
        }
        false
    }

    fn polygon(&self, portal: usize) -> Vec<Vector3<f32>> {
        self.data.portals[portal]
            .polygon
            .iter()
            .map(|&p| v(p))
            .collect()
    }
}

/// Whether `p` is inside every edge of the convex `polygon` (wound about
/// `normal`), within the tolerance. Ignores distance from its plane.
fn inside_edges(polygon: &[Vector3<f32>], normal: Vector3<f32>, p: Vector3<f32>) -> bool {
    (0..polygon.len()).all(|i| {
        let a = polygon[i];
        let edge = polygon[(i + 1) % polygon.len()] - a;
        edge.cross(p - a).dot(normal) >= -TOLERANCE * edge.magnitude()
    })
}

/// A cell's box (grown by the tolerance), from the corners where its planes
/// meet. A far box closes an open cell, so a corner on it means unbounded.
fn cell_bounds(planes: &[Plane]) -> Result<Aabb, &'static str> {
    const FAR: f64 = 1e5;
    let mut all: Vec<([f64; 3], f64)> = planes
        .iter()
        .map(|p| (p.n.map(f64::from), f64::from(p.d)))
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
    let tol = f64::from(TOLERANCE);
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
                if !all.iter().all(|&(n, d)| dot(n, p) + d >= -tol) {
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
    let grow = Vector3::new(TOLERANCE, TOLERANCE, TOLERANCE);
    Ok(Aabb {
        min: bounds.min - grow,
        max: bounds.max + grow,
    })
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
