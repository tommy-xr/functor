//! `Solid.*` — printable parts in millimetres (see `solid.funi`).
//!
//! A solid crosses into Functor Lang as an opaque, immutable description tree
//! ([`functor_solid::Solid`]); meshing happens only when it is measured or
//! exported, so building one in a hot-reloading file stays cheap.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Weak};

use cgmath::Deg;
use functor_solid::Solid;

use super::*;
use crate::scene3d::MeshData;

/// A [`Solid`] as an opaque Functor Lang value. `Arc` so transforms share
/// subtrees instead of copying them.
pub struct FunctorLangSolid(pub Arc<Solid>);

impl HostData for FunctorLangSolid {
    fn type_name(&self) -> &'static str {
        "Solid"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // Plain immutable geometry: safe to keep in a model across reloads.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

crate::host_returnable!(FunctorLangSolid);

/// The solid bound to `name` in an evaluated session, or a teaching error.
pub fn solid_global(session: &functor_lang::Session, name: &str) -> Result<Arc<Solid>, String> {
    let value = session
        .global(name)
        .ok_or_else(|| format!("no top-level `let {name} = …` to export"))?;
    match &value {
        Value::HostData(data) => data.as_any().downcast_ref::<FunctorLangSolid>(),
        _ => None,
    }
    .map(|solid| solid.0.clone())
    .ok_or_else(|| {
        format!(
            "`{name}` must be a Solid (e.g. `Solid.box(…)`), got {}",
            value.kind_name()
        )
    })
}

thread_local! {
    /// Solid → render mesh, keyed by the solid's `Arc` address (the `Weak`
    /// pins the address). `draw` converts the same top-level `let part` every
    /// frame; this meshes it once and hands the renderer the same `Arc` each
    /// time, so the GPU mesh is reused too. Dead solids are pruned on a miss.
    static RENDER_MESHES: RefCell<HashMap<usize, (Weak<Solid>, Arc<MeshData>)>> =
        RefCell::new(HashMap::new());
}

fn render_mesh(solid: &Arc<Solid>) -> Arc<MeshData> {
    let key = Arc::as_ptr(solid) as usize;
    RENDER_MESHES.with(|memo| {
        let mut memo = memo.borrow_mut();
        if let Some((_, mesh)) = memo.get(&key) {
            return mesh.clone();
        }
        memo.retain(|_, (source, _)| source.strong_count() > 0);
        let mesh = solid.mesh();
        let mesh = Arc::new(MeshData {
            positions: mesh.vertices.iter().map(|v| v.map(|x| x as f32)).collect(),
            triangles: mesh.triangles,
        });
        memo.insert(key, (Arc::downgrade(solid), mesh.clone()));
        mesh
    })
}

fn solid(solid: Solid) -> FunctorLangSolid {
    FunctorLangSolid(Arc::new(solid))
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn3(
        "Solid.box",
        "Solid.box(width, depth, height)",
        |w: f64, d: f64, h: f64| {
            let size = [
                positive(w, "Solid.box width")?,
                positive(d, "Solid.box depth")?,
                positive(h, "Solid.box height")?,
            ];
            Ok(solid(Solid::Box { size }))
        },
    );
    reg.fn2(
        "Solid.cylinder",
        "Solid.cylinder(radius, height)",
        |r: f64, h: f64| {
            let radius = positive(r, "Solid.cylinder radius")?;
            let height = positive(h, "Solid.cylinder height")?;
            Ok(solid(Solid::Cylinder { radius, height }))
        },
    );
    reg.fn1("Solid.sphere", "Solid.sphere(radius)", |r: f64| {
        let radius = positive(r, "Solid.sphere radius")?;
        Ok(solid(Solid::Sphere { radius }))
    });

    reg.fn2(
        "Solid.translate",
        "Solid.translate(Vec3.make(x, y, z), solid)",
        |v: FunctorLangVec3, s: FunctorLangSolid| {
            let (x, y, z) = v.0;
            let offset = [x as f64, y as f64, z as f64];
            if !offset.iter().all(|c| c.is_finite()) {
                return Err(format!(
                    "Solid.translate offset must be finite, got {offset:?}"
                ));
            }
            Ok(solid(s.0.translate(offset)))
        },
    );
    for (path, sig, axis) in [
        ("Solid.rotateX", "Solid.rotateX(angle, solid)", 0),
        ("Solid.rotateY", "Solid.rotateY(angle, solid)", 1),
        ("Solid.rotateZ", "Solid.rotateZ(angle, solid)", 2),
    ] {
        reg.fn2(
            path,
            sig,
            move |a: FunctorLangAngle, s: FunctorLangSolid| {
                solid(s.0.rotate(axis, a.0.radians() as f64))
            },
        );
    }
    reg.fn2(
        "Solid.scale",
        "Solid.scale(factor, solid)",
        |k: f64, s: FunctorLangSolid| {
            let k = positive(k, "Solid.scale factor")?;
            Ok(solid(s.0.scale(k)))
        },
    );
    reg.fn1(
        "Solid.group",
        "Solid.group([solid, …]) — at least one solid",
        |items: Vec<FunctorLangSolid>| {
            if items.is_empty() {
                return Err("Solid.group needs at least one solid".to_string());
            }
            Ok(solid(Solid::Group(
                items.into_iter().map(|s| s.0).collect(),
            )))
        },
    );

    // Millimetres, Z up → scene metres, Y up: scale by 1/1000 and turn +Z to +Y.
    reg.fn1(
        "Solid.toScene",
        "Solid.toScene(solid)",
        |s: FunctorLangSolid| {
            let leaf = Scene3D {
                obj: SceneObject::Geometry(Shape::Mesh {
                    mesh: render_mesh(&s.0),
                }),
                xform: Matrix4::from_scale(1.0),
            };
            let xform = Matrix4::from_scale(0.001) * Matrix4::from_angle_x(Deg(-90.0));
            FunctorLangScene(group(vec![leaf], xform))
        },
    );

    reg.fn1(
        "Solid.volume",
        "Solid.volume(solid)",
        |s: FunctorLangSolid| Value::Number(s.0.mesh().volume()),
    );
    reg.fn1(
        "Solid.bounds",
        "Solid.bounds(solid)",
        |s: FunctorLangSolid| {
            // Every constructor yields geometry (groups are never empty).
            let b = s.0.mesh().bounds().expect("a Solid always has vertices");
            let vec3 = |[x, y, z]: [f64; 3]| {
                Value::HostData(Rc::new(FunctorLangVec3((x as f32, y as f32, z as f32))))
            };
            Value::Record(Rc::new(vec![
                ("min".to_string(), vec3(b.min)),
                ("max".to_string(), vec3(b.max)),
            ]))
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memo_len() -> usize {
        RENDER_MESHES.with(|memo| memo.borrow().len())
    }

    #[test]
    fn render_mesh_is_cached_per_solid_and_pruned_when_it_dies() {
        let a = Arc::new(Solid::Box { size: [1.0; 3] });
        assert!(Arc::ptr_eq(&render_mesh(&a), &render_mesh(&a)));
        assert_eq!(memo_len(), 1);
        drop(a);
        // The next miss prunes the dead solid's entry.
        let b = Arc::new(Solid::Sphere { radius: 1.0 });
        render_mesh(&b);
        assert_eq!(memo_len(), 1);
    }
}
