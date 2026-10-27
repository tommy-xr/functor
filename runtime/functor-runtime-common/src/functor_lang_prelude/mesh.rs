//! `Mesh.*` and `Solid.fromMesh` — triangle meshes built from decoded data
//! (see `mesh.funi`), printed by remeshing them closed.

use std::sync::Arc;

use functor_solid::{Mesh, Solid};

use super::bytes::FunctorLangBytes;
use super::solid::{bounds_value, FunctorLangSolid};
use super::*;

/// A [`Mesh`] (Z up, the source's own units) as an opaque value.
pub struct FunctorLangMesh(pub Arc<Mesh>);

impl HostData for FunctorLangMesh {
    fn type_name(&self) -> &'static str {
        "Mesh"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // Immutable data: safe to keep in a model across reloads.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

crate::host_returnable!(FunctorLangMesh);

fn mesh(m: Mesh) -> FunctorLangMesh {
    FunctorLangMesh(Arc::new(m))
}

/// `Solid.fromMesh`'s `{ height, nozzle }` (mm).
struct PrintOptions {
    height: f64,
    nozzle: f64,
}

impl crate::host_registry::FromArg for PrintOptions {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let Value::Record(fields) = value else {
            return Err(RunError {
                message: format!(
                    "{path}: expected {{ height, nozzle }}, got {}",
                    value.kind_name()
                ),
                span,
            });
        };
        let number = |field| finite_record_number(fields, field, "print options", path, span);
        Ok(PrintOptions {
            height: f64::from(number("height")?),
            nozzle: f64::from(number("nozzle")?),
        })
    }
}

/// A vertex index into `count` vertices, or why not.
fn index(i: f64, count: usize, what: &str) -> Result<u32, String> {
    if !(i >= 0.0 && i.fract() == 0.0) {
        Err(format!(
            "{what} has vertex index {i}; indices are whole numbers ≥ 0"
        ))
    } else if i < count as f64 {
        Ok(i as u32)
    } else {
        Err(format!("{what} uses vertex {i}, but there are {count}"))
    }
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn2(
        "Mesh.fromPolygons",
        "Mesh.fromPolygons(positions, polygons)",
        |positions: FunctorLangBytes, polygons: Vec<Vec<f64>>| {
            let raw = positions.0.as_slice();
            if raw.len() % 12 != 0 {
                return Err(format!(
                    "Mesh.fromPolygons: positions are f32 x, y, z triples (12 bytes each), \
got {} bytes",
                    raw.len()
                ));
            }
            let mut out = Mesh::default();
            out.vertices = raw
                .chunks_exact(12)
                .map(|p| {
                    std::array::from_fn(|k| {
                        f64::from(f32::from_le_bytes(p[4 * k..4 * k + 4].try_into().unwrap()))
                    })
                })
                .collect();
            if out.vertices.iter().flatten().any(|x| !x.is_finite()) {
                return Err("Mesh.fromPolygons: a position is not a finite number".to_string());
            }
            let count = out.vertices.len();
            for (p, polygon) in polygons.iter().enumerate() {
                let what = format!("Mesh.fromPolygons: polygon {p}");
                if polygon.len() < 3 {
                    return Err(format!(
                        "{what} has {} vertices; it needs 3 or more",
                        polygon.len()
                    ));
                }
                let ids = polygon
                    .iter()
                    .map(|&i| index(i, count, &what))
                    .collect::<Result<Vec<_>, _>>()?;
                let corners: Vec<[f64; 3]> =
                    ids.iter().map(|&i| out.vertices[i as usize]).collect();
                out.triangles.extend(
                    functor_solid::triangulate_polygon(&corners)
                        .into_iter()
                        .map(|t| t.map(|k| ids[k])),
                );
            }
            Ok(mesh(out))
        },
    );
    reg.fn4(
        "Mesh.transformVertices",
        "Mesh.transformVertices(start, count, matrix, mesh)",
        |start: f64, count: f64, matrix: Vec<f64>, m: FunctorLangMesh| {
            let total = m.0.vertices.len();
            let range = (start >= 0.0
                && count >= 0.0
                && start.fract() == 0.0
                && count.fract() == 0.0
                && start + count <= total as f64)
                .then(|| start as usize..(start + count) as usize)
                .ok_or_else(|| {
                    format!(
                        "Mesh.transformVertices: vertices {start} .. {start} + {count} \
aren't within the mesh's {total}"
                    )
                })?;
            let [a, b, c, tx, d, e, f, ty, g, h, i, tz]: [f64; 12] =
                matrix.try_into().map_err(|m: Vec<f64>| {
                    format!(
                        "Mesh.transformVertices: the matrix is 12 numbers (3 rows of x, y, z, \
translation), got {}",
                        m.len()
                    )
                })?;
            let mut out = (*m.0).clone();
            for v in &mut out.vertices[range] {
                let [x, y, z] = *v;
                *v = [
                    a * x + b * y + c * z + tx,
                    d * x + e * y + f * z + ty,
                    g * x + h * y + i * z + tz,
                ];
            }
            Ok::<_, String>(mesh(out))
        },
    );
    reg.fn1(
        "Mesh.combine",
        "Mesh.combine(meshes)",
        |meshes: Vec<FunctorLangMesh>| {
            let mut out = Mesh::default();
            for m in &meshes {
                out.extend(&m.0);
            }
            mesh(out)
        },
    );
    reg.fn1(
        "Mesh.triangleCount",
        "Mesh.triangleCount(mesh)",
        |m: FunctorLangMesh| Value::Number(m.0.triangles.len() as f64),
    );
    reg.fn1("Mesh.bounds", "Mesh.bounds(mesh)", |m: FunctorLangMesh| {
        m.0.bounds()
            .map(bounds_value)
            .ok_or_else(|| "Mesh.bounds: the mesh is empty".to_string())
    });
    reg.fn2(
        "Solid.fromMesh",
        "Solid.fromMesh({ height, nozzle }, mesh)",
        |PrintOptions { height, nozzle }: PrintOptions, m: FunctorLangMesh| {
            if !(height > 0.0 && nozzle > 0.0) {
                return Err("Solid.fromMesh: height and nozzle must be positive".to_string());
            }
            if m.0.triangles.is_empty() {
                return Err("Solid.fromMesh: the mesh is empty".to_string());
            }
            Ok(FunctorLangSolid(Arc::new(Solid::printed(
                m.0, height, nozzle,
            ))))
        },
    );
}
