//! `Solid.*` — printable parts in millimetres (see `solid.funi`).
//!
//! A solid crosses into Functor Lang as an opaque, immutable description tree
//! ([`functor_solid::Solid`]); meshing happens only when it is measured or
//! exported, so building one in a hot-reloading file stays cheap.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Weak};

use cgmath::Deg;
use functor_solid::{Font, Sketch, Solid};

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

/// A [`Font`] (built in or from a file) as an opaque Functor Lang value.
pub struct FunctorLangFont(pub Font);

impl HostData for FunctorLangFont {
    fn type_name(&self) -> &'static str {
        "Typeface"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

crate::host_returnable!(FunctorLangSolid, FunctorLangSketch, FunctorLangFont);

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
    meshed: Meshed,
    last_used: u64,
}

/// One solid's mesh: f64 for measuring, f32 for the renderer.
#[derive(Clone)]
struct Meshed {
    mesh: Arc<functor_solid::Mesh>,
    render: Arc<MeshData>,
}

#[derive(Default)]
struct RenderMeshMemo {
    entries: HashMap<usize, RenderMeshEntry>,
    clock: u64,
}

thread_local! {
    /// Solid → mesh, keyed by the solid's `Arc` address. `draw` converts the
    /// same top-level `let part` every frame, and booleans are costly to
    /// re-evaluate: this meshes it once (for `toScene`, `volume`, `bounds`)
    /// and hands the renderer the same `Arc` each time, so the GPU mesh is
    /// reused too. Pure memoization: it cannot change any result.
    static RENDER_MESHES: RefCell<RenderMeshMemo> = RefCell::new(RenderMeshMemo::default());
}

fn meshed(solid: &Arc<Solid>) -> Result<Meshed, String> {
    let key = Arc::as_ptr(solid) as usize;
    RENDER_MESHES.with(|memo| {
        let mut memo = memo.borrow_mut();
        memo.clock += 1;
        let clock = memo.clock;
        if let Some(entry) = memo.entries.get_mut(&key) {
            entry.last_used = clock;
            return Ok(entry.meshed.clone());
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
        let mesh = solid.mesh()?;
        let render = Arc::new(MeshData {
            positions: mesh.vertices.iter().map(|v| v.map(|x| x as f32)).collect(),
            triangles: mesh.triangles.clone(),
        });
        let meshed = Meshed {
            mesh: Arc::new(mesh),
            render,
        };
        memo.entries.insert(
            key,
            RenderMeshEntry {
                solid: Arc::downgrade(solid),
                meshed: meshed.clone(),
                last_used: clock,
            },
        );
        Ok(meshed)
    })
}

fn render_mesh(solid: &Arc<Solid>) -> Result<Arc<MeshData>, String> {
    meshed(solid).map(|m| m.render)
}

fn sketch(sketch: Sketch) -> FunctorLangSketch {
    FunctorLangSketch(Arc::new(sketch))
}

fn solid(solid: Solid) -> FunctorLangSolid {
    FunctorLangSolid(Arc::new(solid))
}

/// A `{ min, max }` bounds record of `Vec3`s.
pub(super) fn bounds_value(b: functor_solid::Bounds) -> Value {
    let vec3 = |[x, y, z]: [f64; 3]| {
        Value::HostData(Rc::new(FunctorLangVec3((x as f32, y as f32, z as f32))))
    };
    Value::Record(Rc::new(vec![
        ("min".to_string(), vec3(b.min)),
        ("max".to_string(), vec3(b.max)),
    ]))
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
    reg.fn3(
        "Solid.cone",
        "Solid.cone(bottomRadius, topRadius, height) — one radius may be 0",
        |b: f64, t: f64, h: f64| {
            let height = positive(h, "Solid.cone height")?;
            let ok = |r: f64| r.is_finite() && r >= 0.0;
            if !(ok(b) && ok(t) && (b > 0.0 || t > 0.0)) {
                return Err(format!(
                    "Solid.cone radii must be finite and not negative, one of them positive; \
got {b} and {t}"
                ));
            }
            Ok(solid(Solid::Cone {
                bottom: b,
                top: t,
                height,
            }))
        },
    );
    reg.fn4(
        "Solid.thread",
        "Solid.thread(diameter, pitch, length, offset)",
        |d: f64, p: f64, l: f64, offset: f64| {
            let diameter = positive(d, "Solid.thread diameter")?;
            let pitch = positive(p, "Solid.thread pitch")?;
            let length = positive(l, "Solid.thread length")?;
            if !offset.is_finite() {
                return Err(format!("Solid.thread offset must be finite, got {offset}"));
            }
            Solid::thread(diameter, pitch, length, offset)
                .map(solid)
                .map_err(|e| format!("Solid.thread: {e}"))
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
                return Err("Solid.extrude: the sketch has no area".to_string());
            }
            if let Some([x, y]) = s.0.touching_point() {
                return Err(format!(
                    "Solid.extrude: the sketch's regions meet at a single point ({x:.3}, {y:.3}), \
which won't print as one solid — overlap them or move them apart"
                ));
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
    let boolean = |op: functor_solid::BoolOp, children: Vec<FunctorLangSolid>| {
        solid(Solid::boolean(
            op,
            children.into_iter().map(|s| s.0).collect(),
        ))
    };
    reg.fn1(
        "Solid.union",
        "Solid.union([solid, …]) — at least one solid",
        move |items: Vec<FunctorLangSolid>| {
            if items.is_empty() {
                return Err("Solid.union needs at least one solid".to_string());
            }
            Ok(boolean(functor_solid::BoolOp::Union, items))
        },
    );
    reg.fn2(
        "Solid.subtract",
        "Solid.subtract(cut, solid)",
        move |cut: FunctorLangSolid, s: FunctorLangSolid| {
            boolean(functor_solid::BoolOp::Subtract, vec![s, cut])
        },
    );
    reg.fn2(
        "Solid.intersect",
        "Solid.intersect(other, solid)",
        move |other: FunctorLangSolid, s: FunctorLangSolid| {
            boolean(functor_solid::BoolOp::Intersect, vec![s, other])
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
            Ok::<_, String>(FunctorLangScene(Scene3D {
                obj: SceneObject::Geometry(Shape::Mesh {
                    mesh: render_mesh(&s.0).map_err(|e| format!("Solid.toScene: {e}"))?,
                }),
                xform: Matrix4::from_scale(0.001) * Matrix4::from_angle_x(Deg(-90.0)),
            }))
        },
    );

    reg.fn1(
        "Solid.volume",
        "Solid.volume(solid)",
        |s: FunctorLangSolid| {
            let m = meshed(&s.0).map_err(|e| format!("Solid.volume: {e}"))?;
            Ok(Value::Number(m.mesh.volume()))
        },
    );
    reg.fn1(
        "Solid.bounds",
        "Solid.bounds(solid)",
        |s: FunctorLangSolid| {
            let m = meshed(&s.0).map_err(|e| format!("Solid.bounds: {e}"))?;
            let b = m
                .mesh
                .bounds()
                .ok_or("Solid.bounds: the solid is empty (e.g. an intersection of solids that don't overlap)")?;
            Ok::<_, String>(bounds_value(b))
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
    reg.fn0("Typeface.script", "Typeface.script()", || {
        FunctorLangFont(Font::Script)
    });
    reg.fn0("Typeface.sans", "Typeface.sans()", || {
        FunctorLangFont(Font::Sans)
    });
    reg.fn1(
        "Typeface.file",
        "Typeface.file(path) — a .ttf or .otf file in the project",
        |path: String| typeface_file(&path).map(FunctorLangFont),
    );
    reg.fn3(
        "Sketch.text",
        "Sketch.text(typeface, size, text)",
        |font: FunctorLangFont, size: f64, s: String| {
            let size = positive(size, "Sketch.text size")?;
            functor_solid::text(&font.0, size, &s)
                .map(sketch)
                .map_err(|e| format!("Sketch.text: {e}"))
        },
    );
    reg.fn3(
        "Sketch.textWidth",
        "Sketch.textWidth(typeface, size, text)",
        |font: FunctorLangFont, size: f64, s: String| {
            let size = positive(size, "Sketch.textWidth size")?;
            functor_solid::text_width(&font.0, size, &s)
                .map(Value::Number)
                .map_err(|e| format!("Sketch.textWidth: {e}"))
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
        assert!(Arc::ptr_eq(
            &render_mesh(&a).unwrap(),
            &render_mesh(&a).unwrap()
        ));
        assert_eq!(memo_len(), 1);
        drop(a);
        // The next miss prunes the dead solid's entry.
        let b = Arc::new(Solid::Sphere { radius: 1.0 });
        render_mesh(&b).unwrap();
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
            render_mesh(solid).unwrap();
        }
        assert_eq!(memo_len(), RENDER_MESH_CAP);
    }
}

/// A font file in the project (the entry file's folder), read on each call.
fn typeface_file(path: &str) -> Result<Font, String> {
    let bytes = super::bytes::read_project_file("Typeface.file", path, "fonts/MyFont.ttf")
        .map_err(|e| match cfg!(any(target_arch = "wasm32", target_os = "android")) {
            true => format!("{e} — use Typeface.sans() or Typeface.script()"),
            false => e,
        })?;
    Font::from_bytes(path, bytes).map_err(|e| format!("Typeface.file: {e}"))
}
