//! 3D booleans via `manifold-rust` (a pure-Rust port of the Manifold library):
//! exact, closed output that is identical across native and wasm. Kept behind
//! this one module so the engine can be swapped.

use std::collections::HashMap;
use std::ops::ControlFlow;

use cgmath::{Matrix4, SquareMatrix};
use manifold_rust::csg_tree::CsgNode;
use manifold_rust::linalg::Vec3;
use manifold_rust::manifold::Manifold;
use manifold_rust::types::{Error, MeshGL64, OpType};

use crate::{plane, BoolOp, Mesh, Solid};

/// Inputs are snapped to this grid (2⁻¹⁶ mm ≈ 15 nm, exact in binary): features
/// placed a hair apart (e.g. with f32 angles) usually become exactly coincident,
/// which Manifold resolves cleanly, and output matches across platforms whose
/// trig differs in the last bit. Not a guarantee — results are still checked.
pub(crate) const SNAP: f64 = 1.0 / 65536.0;

/// Evaluate a boolean solid, in its own frame, as one Manifold tree: nested
/// booleans and groups become tree nodes, so only real inputs are snapped and
/// Manifold batches the whole expression.
pub(crate) fn evaluate(solid: &Solid) -> Result<Mesh, String> {
    let tree = node(solid, Matrix4::identity())?;
    let mesh = from_manifold(&Manifold::from_impl(tree.evaluate()))?;
    let clashes = mesh.shared_points();
    if clashes.is_empty() {
        return Ok(mesh);
    }
    Err(blame(solid, &clashes)?)
}

/// The error for `clashes` (merged f32 points) in `solid`'s result: naming
/// the boolean that made them when one can be found.
fn blame(solid: &Solid, clashes: &Clashes) -> Result<String, String> {
    Ok(match locate(solid, Matrix4::identity(), clashes)? {
        ControlFlow::Break(culprit) => culprit,
        ControlFlow::Continue(_) => near_miss("the boolean", "", clashes, APART),
    })
}

/// Cut `child`, in its own frame, along the plane `normal·p = offset`
/// (`normal` unit length): [the side the normal points to, the other]. A
/// plane that misses or only touches the solid is an error, not an empty half.
pub(crate) fn split(child: &Solid, normal: [f64; 3], offset: f64) -> Result<[Mesh; 2], String> {
    let whole = Manifold::from_impl(node(child, Matrix4::identity())?.evaluate());
    let mesh = from_manifold(&whole)?;
    if mesh.triangles.is_empty() {
        return Err("there is nothing to split: the solid is empty".to_string());
    }
    // Where the solid is along the normal: a plane at either end (missing it,
    // or flush with a face) would leave one side empty or a zero-volume skin.
    let (lo, hi) = mesh
        .vertices
        .iter()
        .map(|v| v[0] * normal[0] + v[1] * normal[1] + v[2] * normal[2])
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), d| {
            (lo.min(d), hi.max(d))
        });
    let (above, below) = whole.split_by_plane(Vec3::new(normal[0], normal[1], normal[2]), offset);
    if !(lo < offset && offset < hi) || above.is_empty() || below.is_empty() {
        return Err(format!(
            "the split plane misses or only touches the solid: along the unit normal it lies \
at {offset:.3} mm, the solid spans {lo:.3} to {hi:.3} mm"
        ));
    }
    // Clashes the solid already had are a boolean's inside it, not the plane's.
    let inherited = mesh.shared_points();
    let half = |m: &Manifold| {
        let mesh = from_manifold(m)?;
        let clashes = mesh.shared_points();
        if clashes.is_empty() {
            Ok(mesh)
        } else if clashes.keys().any(|k| inherited.contains_key(k)) {
            Err(blame(child, &clashes)?)
        } else {
            Err(near_miss(
                "the split",
                "",
                &clashes,
                "move the plane at least 0.01 mm",
            ))
        }
    };
    Ok([half(&above)?, half(&below)?])
}

/// Re-evaluate `solid` bottom-up, one boolean at a time, to find the deepest
/// boolean whose own result already has one of `clashes` (the whole result's
/// merged f32 points), described as an error. Runs only once a boolean fails.
fn locate(
    solid: &Solid,
    xform: Matrix4<f64>,
    clashes: &Clashes,
) -> Result<ControlFlow<String, Manifold>, String> {
    let (op, children, name) = match solid {
        Solid::Boolean { op, children, .. } => (*op, children, op_name(*op)),
        Solid::Group(children) => (BoolOp::Union, children, "Solid.group"),
        Solid::Transform { matrix, child } => return locate(child, xform * matrix, clashes),
        leaf => {
            let tree = node(leaf, xform)?;
            return Ok(ControlFlow::Continue(Manifold::from_impl(tree.evaluate())));
        }
    };
    let mut operands = Vec::new();
    for child in children {
        match locate(child, xform, clashes)? {
            ControlFlow::Continue(m) => operands.push(m),
            culprit => return Ok(culprit),
        }
    }
    let leaves = operands.iter().map(|m| CsgNode::leaf(m.as_impl().clone()));
    let result = Manifold::from_impl(CsgNode::op_n(op_type(op), leaves.collect()).evaluate());
    let here: Clashes = from_manifold(&result)?
        .shared_points()
        .into_iter()
        .filter(|(key, _)| clashes.contains_key(key))
        .collect();
    if here.is_empty() {
        return Ok(ControlFlow::Continue(result));
    }
    // An operand is involved if its surface passes within ~0.002 mm of a
    // merged vertex: a tiny cube there touches it. Every merged vertex lies
    // on the operand it came from, however coarse f32 is at that position.
    const SIZE: f64 = 1e-3;
    let probes: Vec<_> = here
        .values()
        .flatten()
        .map(|p| Manifold::cube(Vec3::splat(SIZE), true).translate(Vec3::new(p[0], p[1], p[2])))
        .collect();
    let n = operands.len();
    let involved: Vec<usize> = (0..n)
        .filter(|&i| probes.iter().any(|p| operands[i].min_gap(p, SIZE) < SIZE))
        .collect();
    // Binary subtract/intersect operands by role, as `s |> Solid.subtract(cut)`
    // reads; list items (union, group) by index, as `List.nth` counts.
    let roles = n == 2 && op != BoolOp::Union;
    let role = |i: usize| match (op, i) {
        (_, 0) => "the solid",
        (BoolOp::Subtract, _) => "`cut`",
        _ => "`other`",
    };
    let who = if roles {
        and(involved.iter().map(|&i| role(i).to_string()))
    } else {
        let items = if involved.len() == 1 { "item" } else { "items" };
        let indices = and(involved.iter().map(|i| i.to_string()));
        format!("{items} {indices} (counting from 0)")
    };
    let what = if roles {
        format!("a {name}")
    } else {
        format!("a {name} of {n} solids")
    };
    let between = match involved.len() {
        0 => String::new(),
        1 => format!(" inside {who}"),
        _ => format!(" where {who} nearly meet"),
    };
    Ok(ControlFlow::Break(near_miss(&what, &between, &here, APART)))
}

/// Merged f32 positions (as bits), each with the vertices that round to it.
type Clashes = HashMap<[u32; 3], Vec<[f64; 3]>>;

const APART: &str = "move them at least 0.01 mm apart, or overlap them by at least 0.01 mm";

/// The error for `what` leaving `clashes`, e.g. "a Solid.subtract left
/// features … at x 1.00, y 0.00 to 2.00, z 5.00 mm (as built, …) — {fix}".
/// Positions are in the evaluated solid's own frame.
fn near_miss(what: &str, between: &str, clashes: &Clashes, fix: &str) -> String {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in clashes.values().flatten() {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let count = clashes.len();
    let axis = |k: usize| {
        let (a, b) = (format!("{:.2}", lo[k]), format!("{:.2}", hi[k]));
        if a == b {
            a
        } else {
            format!("{a} to {b}")
        }
    };
    let at = if count == 1 {
        "at"
    } else {
        &format!("at {count} points within")
    };
    format!(
        "{what} left features closer than 32-bit coordinates can separate{between}, \
{at} x {}, y {}, z {} mm (as built, before moving the result) — {fix}",
        axis(0),
        axis(1),
        axis(2)
    )
}

/// `["a", "b", "c"]` → "a, b and c".
fn and(items: impl Iterator<Item = String>) -> String {
    let items: Vec<_> = items.collect();
    match items.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => items.concat(),
    }
}

fn op_type(op: BoolOp) -> OpType {
    match op {
        BoolOp::Union => OpType::Add,
        BoolOp::Subtract => OpType::Subtract,
        BoolOp::Intersect => OpType::Intersect,
    }
}

fn op_name(op: BoolOp) -> &'static str {
    match op {
        BoolOp::Union => "Solid.union",
        BoolOp::Subtract => "Solid.subtract",
        BoolOp::Intersect => "Solid.intersect",
    }
}

/// Reduce a closed mesh's triangle count, moving no surface more than
/// `tolerance` mm and keeping it closed (Manifold's topology-safe collapse).
pub fn simplify(mesh: &Mesh, tolerance: f64) -> Result<Mesh, String> {
    from_manifold(&to_manifold(mesh)?.simplify(tolerance))
}

pub(crate) fn node(solid: &Solid, xform: Matrix4<f64>) -> Result<CsgNode, String> {
    let children = |children: &[std::sync::Arc<Solid>]| {
        children
            .iter()
            .map(|c| node(c, xform))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(match solid {
        Solid::Boolean {
            op, children: c, ..
        } => CsgNode::op_n(op_type(*op), children(c)?),
        // Inside a boolean, a group is the union of its pieces.
        Solid::Group(c) => CsgNode::op_n(OpType::Add, children(c)?),
        Solid::Transform { matrix, child } => node(child, xform * matrix)?,
        Solid::Trim { child, plane, .. } => {
            plane::trim(node(child, xform)?, plane::moved(*plane, xform))?
        }
        Solid::Edge {
            child,
            faces: [a, b],
            cut,
            ..
        } => {
            let phase = plane::facet_phase(*a, *b, *cut);
            let scale = xform.determinant().cbrt().abs();
            let (a, b) = (plane::moved(*a, xform), plane::moved(*b, xform));
            plane::edge(node(child, xform)?, a, b, *cut, scale, phase)?
        }
        primitive => {
            let mut mesh = Mesh::default();
            primitive.append_to(&mut mesh, xform)?;
            CsgNode::leaf(to_manifold(&mesh)?.into_impl())
        }
    })
}

pub(crate) fn to_manifold(mesh: &Mesh) -> Result<Manifold, String> {
    let gl = MeshGL64 {
        num_prop: 3,
        vert_properties: mesh
            .vertices
            .iter()
            .flatten()
            .map(|x| (x / SNAP).round() * SNAP)
            .collect(),
        tri_verts: mesh.triangles.iter().flatten().map(|&i| i as u64).collect(),
        ..MeshGL64::default()
    };
    let manifold = Manifold::from_mesh_gl64(&gl);
    match manifold.status() {
        Error::NoError => Ok(manifold),
        e => Err(format!("a boolean input is not a closed solid ({e:?})")),
    }
}

pub(crate) fn from_manifold(manifold: &Manifold) -> Result<Mesh, String> {
    if manifold.status() != Error::NoError {
        return Err(format!("the boolean failed ({:?})", manifold.status()));
    }
    let gl = manifold.get_mesh_gl64(-1);
    let stride = gl.num_prop as usize;
    Ok(Mesh {
        vertices: gl
            .vert_properties
            .chunks(stride)
            .map(|c| [c[0], c[1], c[2]])
            .collect(),
        triangles: gl
            .tri_verts
            .chunks(3)
            .map(|c| [c[0] as u32, c[1] as u32, c[2] as u32])
            .collect(),
    })
}
