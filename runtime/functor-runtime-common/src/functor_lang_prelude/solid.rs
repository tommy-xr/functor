//! `Solid.*` — printable parts in millimetres (see `solid.funi`).
//!
//! A solid crosses into Functor Lang as an opaque, immutable description tree
//! ([`functor_solid::Solid`]); meshing happens only when it is measured or
//! exported, so building one in a hot-reloading file stays cheap.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Weak};

use cgmath::Deg;
use functor_solid::{Sketch, Solid};

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

/// A [`Sketch`] (2D profile, mm) as an opaque Functor Lang value.
pub struct FunctorLangSketch(pub Arc<Sketch>);

impl HostData for FunctorLangSketch {
    fn type_name(&self) -> &'static str {
        "Sketch"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // Plain immutable geometry: safe to keep in a model across reloads.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

crate::host_returnable!(FunctorLangSolid, FunctorLangSketch);

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

/// Render meshes kept at most; the least recently used is evicted beyond it.
/// Bounds solids kept alive elsewhere (e.g. rebuilt each tick and retained in
/// time-travel history), like the instance-list memo's cap.
const RENDER_MESH_CAP: usize = 16;

struct RenderMeshEntry {
    // Weak: the memo never keeps a solid alive, yet pins its address.
    solid: Weak<Solid>,
    mesh: Arc<MeshData>,
    last_used: u64,
}

#[derive(Default)]
struct RenderMeshMemo {
    entries: HashMap<usize, RenderMeshEntry>,
    clock: u64,
}

thread_local! {
    /// Solid → render mesh, keyed by the solid's `Arc` address. `draw`
    /// converts the same top-level `let part` every frame; this meshes it once
    /// and hands the renderer the same `Arc` each time, so the GPU mesh is
    /// reused too. Pure memoization: it cannot change what is drawn.
    static RENDER_MESHES: RefCell<RenderMeshMemo> = RefCell::new(RenderMeshMemo::default());
}

fn render_mesh(solid: &Arc<Solid>) -> Arc<MeshData> {
    let key = Arc::as_ptr(solid) as usize;
    RENDER_MESHES.with(|memo| {
        let mut memo = memo.borrow_mut();
        memo.clock += 1;
        let clock = memo.clock;
        if let Some(entry) = memo.entries.get_mut(&key) {
            entry.last_used = clock;
            return entry.mesh.clone();
        }
        // Dead solids can never be looked up again — drop them first, then
        // the least recently used if still full.
        memo.entries
            .retain(|_, entry| entry.solid.strong_count() > 0);
        if memo.entries.len() >= RENDER_MESH_CAP {
            if let Some(oldest) = memo
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key)
            {
                memo.entries.remove(&oldest);
            }
        }
        let mesh = solid.mesh();
        let mesh = Arc::new(MeshData {
            positions: mesh.vertices.iter().map(|v| v.map(|x| x as f32)).collect(),
            triangles: mesh.triangles,
        });
        memo.entries.insert(
            key,
            RenderMeshEntry {
                solid: Arc::downgrade(solid),
                mesh: mesh.clone(),
                last_used: clock,
            },
        );
        mesh
    })
}

fn sketch(sketch: Sketch) -> FunctorLangSketch {
    FunctorLangSketch(Arc::new(sketch))
}

fn solid(solid: Solid) -> FunctorLangSolid {
    FunctorLangSolid(Arc::new(solid))
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    register_sketch(reg);
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
        "Solid.extrude",
        "Solid.extrude(height, sketch) — a non-empty sketch",
        |h: f64, s: FunctorLangSketch| {
            let height = positive(h, "Solid.extrude height")?;
            if s.0.is_empty() {
                return Err(
                    "Solid.extrude: the sketch is empty — an offset or subtract \
removed all of it"
                        .to_string(),
                );
            }
            Ok(solid(Solid::Extrude {
                sketch: s.0,
                height,
            }))
        },
    );

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
            FunctorLangScene(Scene3D {
                obj: SceneObject::Geometry(Shape::Mesh {
                    mesh: render_mesh(&s.0),
                }),
                xform: Matrix4::from_scale(0.001) * Matrix4::from_angle_x(Deg(-90.0)),
            })
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

fn register_sketch(reg: &mut crate::host_registry::Registry) {
    reg.fn2(
        "Sketch.rect",
        "Sketch.rect(width, height)",
        |w: f64, h: f64| {
            let w = positive(w, "Sketch.rect width")?;
            let h = positive(h, "Sketch.rect height")?;
            Ok(sketch(Sketch::rect(w, h)))
        },
    );
    reg.fn3(
        "Sketch.roundedRect",
        "Sketch.roundedRect(width, height, radius) — radius under half the shorter side",
        |w: f64, h: f64, r: f64| {
            let w = positive(w, "Sketch.roundedRect width")?;
            let h = positive(h, "Sketch.roundedRect height")?;
            let r = positive(r, "Sketch.roundedRect radius")?;
            if 2.0 * r >= w.min(h) {
                return Err(format!(
                    "Sketch.roundedRect radius {r} must be under half the shorter side ({})",
                    w.min(h) / 2.0
                ));
            }
            Ok(sketch(Sketch::rect(w - 2.0 * r, h - 2.0 * r).offset(r)))
        },
    );
    reg.fn1("Sketch.circle", "Sketch.circle(radius)", |r: f64| {
        Ok(sketch(Sketch::circle(positive(r, "Sketch.circle radius")?)))
    });
    reg.fn1(
        "Sketch.polygon",
        "Sketch.polygon([{ x, y }, …]) — at least 3 points",
        |points: Vec<super::sprite::FunctorLangPoint2>| {
            if points.len() < 3 {
                return Err("Sketch.polygon needs at least 3 points".to_string());
            }
            let points = points.iter().map(|p| p.0.map(f64::from)).collect();
            Ok(sketch(Sketch::polygon(points)))
        },
    );
    reg.fn1(
        "Sketch.union",
        "Sketch.union([sketch, …])",
        |items: Vec<FunctorLangSketch>| {
            let items: Vec<Sketch> = items.iter().map(|s| (*s.0).clone()).collect();
            sketch(Sketch::union(&items))
        },
    );
    reg.fn2(
        "Sketch.subtract",
        "Sketch.subtract(cut, sketch)",
        |cut: FunctorLangSketch, s: FunctorLangSketch| sketch(s.0.subtract(&cut.0)),
    );
    reg.fn2(
        "Sketch.intersect",
        "Sketch.intersect(other, sketch)",
        |other: FunctorLangSketch, s: FunctorLangSketch| sketch(s.0.intersect(&other.0)),
    );
    reg.fn2(
        "Sketch.offset",
        "Sketch.offset(distance, sketch)",
        |d: f64, s: FunctorLangSketch| sketch(s.0.offset(d)),
    );
    reg.fn3(
        "Sketch.translate",
        "Sketch.translate(x, y, sketch)",
        |x: f64, y: f64, s: FunctorLangSketch| sketch(s.0.map_points(|[px, py]| [px + x, py + y])),
    );
    reg.fn2(
        "Sketch.rotate",
        "Sketch.rotate(angle, sketch)",
        |a: FunctorLangAngle, s: FunctorLangSketch| {
            let (sin, cos) = (a.0.radians() as f64).sin_cos();
            sketch(s.0.map_points(|[x, y]| [x * cos - y * sin, x * sin + y * cos]))
        },
    );
    reg.fn2(
        "Sketch.scale",
        "Sketch.scale(factor, sketch)",
        |k: f64, s: FunctorLangSketch| {
            let k = positive(k, "Sketch.scale factor")?;
            Ok(sketch(s.0.map_points(|[x, y]| [x * k, y * k])))
        },
    );
    reg.fn1(
        "Sketch.area",
        "Sketch.area(sketch)",
        |s: FunctorLangSketch| Value::Number(s.0.area()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memo_len() -> usize {
        RENDER_MESHES.with(|memo| memo.borrow().entries.len())
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

    #[test]
    fn render_mesh_memo_is_capped_for_live_solids() {
        let live: Vec<_> = (0..RENDER_MESH_CAP + 4)
            .map(|i| {
                Arc::new(Solid::Box {
                    size: [1.0 + i as f64; 3],
                })
            })
            .collect();
        for solid in &live {
            render_mesh(solid);
        }
        assert_eq!(memo_len(), RENDER_MESH_CAP);
    }
}
