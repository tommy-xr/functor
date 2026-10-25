//! 3D booleans via `manifold-rust` (a pure-Rust port of the Manifold library):
//! exact, closed output that is identical across native and wasm. Kept behind
//! this one module so the engine can be swapped.

use manifold_rust::csg_tree::CsgNode;
use manifold_rust::manifold::Manifold;
use manifold_rust::types::{Error, MeshGL64, OpType};

use crate::{BoolOp, Mesh};

/// Combine closed meshes: union of all, the first minus the rest, or the
/// intersection of all.
pub(crate) fn boolean(op: BoolOp, meshes: &[Mesh]) -> Result<Mesh, String> {
    let leaves = meshes
        .iter()
        .map(|m| to_manifold(m).map(|m| CsgNode::leaf(m.into_impl())))
        .collect::<Result<Vec<_>, _>>()?;
    let op = match op {
        BoolOp::Union => OpType::Add,
        BoolOp::Subtract => OpType::Subtract,
        BoolOp::Intersect => OpType::Intersect,
    };
    // One n-ary node: Manifold batches it (far faster than folding pairwise).
    from_manifold(&Manifold::from_impl(CsgNode::op_n(op, leaves).evaluate()))
}

/// Inputs are snapped to this grid (2⁻¹⁶ mm ≈ 15 nm, exact in binary): features
/// that nearly coincide (e.g. placed with f32 angles) coincide exactly, which
/// Manifold resolves cleanly instead of leaving nanometre slivers, and output
/// matches across platforms whose trig differs in the last bit.
const SNAP: f64 = 1.0 / 65536.0;

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
