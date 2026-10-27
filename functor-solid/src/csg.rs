//! 3D booleans via `manifold-rust` (a pure-Rust port of the Manifold library):
//! exact, closed output that is identical across native and wasm. Kept behind
//! this one module so the engine can be swapped.

use cgmath::{Matrix4, SquareMatrix};
use manifold_rust::csg_tree::CsgNode;
use manifold_rust::manifold::Manifold;
use manifold_rust::types::{Error, MeshGL64, OpType};

use crate::{BoolOp, Mesh, Solid};

/// Inputs are snapped to this grid (2⁻¹⁶ mm ≈ 15 nm, exact in binary): features
/// placed a hair apart (e.g. with f32 angles) usually become exactly coincident,
/// which Manifold resolves cleanly, and output matches across platforms whose
/// trig differs in the last bit. Not a guarantee — results are still checked.
const SNAP: f64 = 1.0 / 65536.0;

/// Evaluate a boolean solid, in its own frame, as one Manifold tree: nested
/// booleans and groups become tree nodes, so only real inputs are snapped and
/// Manifold batches the whole expression.
pub(crate) fn evaluate(solid: &Solid) -> Result<Mesh, String> {
    let tree = node(solid, Matrix4::identity())?;
    let mesh = from_manifold(&Manifold::from_impl(tree.evaluate()))?;
    if mesh.shared_positions() > 0 {
        return Err(
            "the boolean left features closer than 32-bit coordinates can \
separate — nudge the nearly-touching pieces apart or into each other"
                .to_string(),
        );
    }
    Ok(mesh)
}

fn node(solid: &Solid, xform: Matrix4<f64>) -> Result<CsgNode, String> {
    let children = |children: &[std::sync::Arc<Solid>]| {
        children
            .iter()
            .map(|c| node(c, xform))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(match solid {
        Solid::Boolean {
            op, children: c, ..
        } => {
            let op = match op {
                BoolOp::Union => OpType::Add,
                BoolOp::Subtract => OpType::Subtract,
                BoolOp::Intersect => OpType::Intersect,
            };
            CsgNode::op_n(op, children(c)?)
        }
        // Inside a boolean, a group is the union of its pieces.
        Solid::Group(c) => CsgNode::op_n(OpType::Add, children(c)?),
        Solid::Transform { matrix, child } => node(child, xform * matrix)?,
        primitive => {
            let mut mesh = Mesh::default();
            primitive.append_to(&mut mesh, xform)?;
            CsgNode::leaf(to_manifold(&mesh)?.into_impl())
        }
    })
}

fn to_manifold(mesh: &Mesh) -> Result<Manifold, String> {
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

fn from_manifold(manifold: &Manifold) -> Result<Mesh, String> {
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
