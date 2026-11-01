//! `Solid.*` — printable parts in millimetres (see `solid.funi`).
//!
//! A solid crosses into Functor Lang as an opaque, immutable description tree
//! ([`functor_solid::Solid`]); meshing happens only when it is measured or
//! exported, so building one in a hot-reloading file stays cheap.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Weak};

use cgmath::Deg;
use functor_solid::{EdgeShape, Font, Sketch, Solid};

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

/// Values derived from `Arc`s, keyed by the `Arc`'s address and held weakly: a
/// dead key's entry is dropped, and the least recently used is evicted beyond
/// [`RENDER_MESH_CAP`]. The memo never keeps a key alive, yet pins its address.
pub(super) struct WeakMemo<K, V> {
    entries: HashMap<usize, (Weak<K>, V, u64)>,
    clock: u64,
}

impl<K, V> Default for WeakMemo<K, V> {
    fn default() -> Self {
        WeakMemo {
            entries: HashMap::new(),
            clock: 0,
        }
    }
}

impl<K, V: Clone> WeakMemo<K, V> {
    pub(super) fn get_or_try_insert<E>(
        &mut self,
        key: &Arc<K>,
        make: impl FnOnce() -> Result<V, E>,
    ) -> Result<V, E> {
        let address = Arc::as_ptr(key) as usize;
        self.clock += 1;
        let clock = self.clock;
        if let Some((_, value, last_used)) = self.entries.get_mut(&address) {
            *last_used = clock;
            return Ok(value.clone());
        }
        // Dead keys can never be looked up again — drop them first, then the
        // least recently used if still full.
        self.entries
            .retain(|_, (weak, _, _)| weak.strong_count() > 0);
        if self.entries.len() >= RENDER_MESH_CAP {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, _, last_used))| *last_used)
                .map(|(address, _)| *address)
            {
                self.entries.remove(&oldest);
            }
        }
        let value = make()?;
        self.entries
            .insert(address, (Arc::downgrade(key), value.clone(), clock));
        Ok(value)
    }

    pub(super) fn get_or_insert(&mut self, key: &Arc<K>, make: impl FnOnce() -> V) -> V {
        match self.get_or_try_insert(key, || Ok::<_, std::convert::Infallible>(make())) {
            Ok(value) => value,
            Err(never) => match never {},
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// One solid's mesh: f64 for measuring, f32 for the renderer.
#[derive(Clone)]
struct Meshed {
    mesh: Arc<functor_solid::Mesh>,
    render: Arc<MeshData>,
}

/// A mesh as the renderer draws it.
pub(super) fn render_data(mesh: &functor_solid::Mesh) -> Arc<MeshData> {
    Arc::new(MeshData {
        positions: mesh.vertices.iter().map(|v| v.map(|x| x as f32)).collect(),
        triangles: mesh.triangles.clone(),
    })
}

thread_local! {
    /// Solid → mesh. `draw` converts the same top-level `let part` every
    /// frame, and booleans are costly to re-evaluate: this meshes it once
    /// (for `toScene`, `volume`, `bounds`, `check`) and hands the renderer the same
    /// `Arc` each time, so the GPU mesh is reused too. Pure memoization: it
    /// cannot change any result.
    static RENDER_MESHES: RefCell<WeakMemo<Solid, Meshed>> = RefCell::new(WeakMemo::default());
}

fn meshed(solid: &Arc<Solid>) -> Result<Meshed, String> {
    RENDER_MESHES.with(|memo| {
        memo.borrow_mut().get_or_try_insert(solid, || {
            let mesh = solid.mesh()?;
            Ok(Meshed {
                render: render_data(&mesh),
                mesh: Arc::new(mesh),
            })
        })
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

fn vec3_value([x, y, z]: [f64; 3]) -> Value {
    Value::HostData(Rc::new(FunctorLangVec3((x as f32, y as f32, z as f32))))
}

/// A `{ min, max }` bounds record of `Vec3`s.
pub(super) fn bounds_value(b: functor_solid::Bounds) -> Value {
    Value::Record(Rc::new(vec![
        ("min".to_string(), vec3_value(b.min)),
        ("max".to_string(), vec3_value(b.max)),
    ]))
}

/// A record's field by name.
fn field<'v>(value: &'v Value, name: &str) -> Option<&'v Value> {
    match value {
        Value::Record(fields) => fields.iter().find(|(f, _)| f == name).map(|(_, v)| v),
        _ => None,
    }
}

/// `Solid.split`'s `{ normal, offset }` plane.
struct Plane {
    normal: [f64; 3],
    offset: f64,
}

impl crate::host_registry::FromArg for Plane {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let (Some(normal), Some(offset)) = (field(value, "normal"), field(value, "offset")) else {
            return Err(RunError {
                message: format!(
                    "{path}: expected a plane {{ normal: Vec3, offset: float }}, got {}",
                    value.kind_name()
                ),
                span,
            });
        };
        let (x, y, z) = vec3_of(normal, &format!("{path} plane normal"), span)?;
        Ok(Plane {
            normal: [x, y, z].map(f64::from),
            offset: f64::from_arg(offset, path, span)?,
        })
    }
}

/// A sketch that extrudes to a closed solid, or why not.
fn extrudable(s: &Sketch, op: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err(format!("{op}: the sketch has no area"));
    }
    if let Some([x, y]) = s.touching_point() {
        return Err(format!(
            "{op}: the sketch's regions meet at a single point ({x:.3}, {y:.3}), \
which won't print as one solid — overlap them or move them apart"
        ));
    }
    Ok(())
}

/// `Solid.extrudeWith`'s `{ top, bottom }` edge finishes.
struct Edges {
    top: EdgeShape,
    bottom: EdgeShape,
}

impl crate::host_registry::FromArg for Edges {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let field = |name: &str| match value {
            Value::Record(fields) => fields.iter().find(|(f, _)| f == name).map(|(_, v)| v),
            _ => None,
        };
        let shape = |v: &Value| match v {
            Value::Variant { ctor, args } => match (ctor.as_ref(), args.as_slice()) {
                ("Solid.Sharp", []) => Some(EdgeShape::Sharp),
                ("Solid.Bevel", [Value::Number(s)]) => Some(EdgeShape::Bevel(*s)),
                ("Solid.Round", [Value::Number(s)]) => Some(EdgeShape::Round(*s)),
                _ => None,
            },
            _ => None,
        };
        match (
            field("top").and_then(shape),
            field("bottom").and_then(shape),
        ) {
            (Some(top), Some(bottom)) => Ok(Edges { top, bottom }),
            _ => Err(RunError {
                message: format!(
                    "{path}: expected {{ top, bottom }}, each Solid.Sharp, Solid.Bevel(mm) or \
Solid.Round(mm), got {value}"
                ),
                span,
            }),
        }
    }
}

/// `Solid.frame`'s `{ origin, x, y, z }`: an origin and right-handed unit axes.
struct Frame {
    origin: [f64; 3],
    axes: [[f64; 3]; 3],
}

impl crate::host_registry::FromArg for Frame {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let error = |message: String| RunError { message, span };
        let mut vecs = [[0.0; 3]; 4];
        for (v, name) in vecs.iter_mut().zip(["origin", "x", "y", "z"]) {
            let Some(field) = field(value, name) else {
                return Err(error(format!(
                    "{path}: expected a frame {{ origin, x, y, z }} from Solid.frame, got {}",
                    value.kind_name()
                )));
            };
            let (x, y, z) = vec3_of(field, &format!("{path} frame {name}"), span)?;
            *v = [x, y, z].map(f64::from);
        }
        let [origin, x, y, z] = vecs;
        if !origin.iter().all(|c| c.is_finite()) {
            return Err(error(format!(
                "{path}: a frame's origin must be finite, got {origin:?}"
            )));
        }
        // Rebuilt in f64 from x and z (Vec3s are f32); y and the rest must
        // already agree, so a hand-written record can't skew or scale.
        let axes = functor_solid::basis(x, z).map_err(|e| error(format!("{path}: {e}")))?;
        let agrees = [x, y, z]
            .iter()
            .zip(&axes)
            .all(|(given, unit)| given.iter().zip(unit).all(|(g, u)| (g - u).abs() < 1e-4));
        if !agrees {
            return Err(error(format!(
                "{path}: a frame's axes must be unit length, perpendicular and right-handed — \
build it with Solid.frame(origin, xAxis, zAxis)"
            )));
        }
        Ok(Frame { origin, axes })
    }
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
            extrudable(&s.0, "Solid.extrude")?;
            Ok(solid(Solid::Extrude {
                sketch: s.0,
                height,
                top: EdgeShape::Sharp,
                bottom: EdgeShape::Sharp,
            }))
        },
    );
    reg.fn3(
        "Solid.extrudeWith",
        "Solid.extrudeWith({ top, bottom }, height, sketch)",
        |e: Edges, h: f64, s: FunctorLangSketch| {
            let height = positive(h, "Solid.extrudeWith height")?;
            extrudable(&s.0, "Solid.extrudeWith")?;
            Solid::extrude_with(s.0, height, e.top, e.bottom)
                .map(solid)
                .map_err(|e| format!("Solid.extrudeWith: {e}"))
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
    reg.fn2(
        "Solid.split",
        "Solid.split({ normal, offset }, solid)",
        |p: Plane, s: FunctorLangSolid| {
            let (above, below) =
                Solid::split(s.0, p.normal, p.offset).map_err(|e| format!("Solid.split: {e}"))?;
            let half = |h| Value::HostData(Rc::new(solid(h)));
            Ok::<_, String>(Value::Tuple(Rc::new(vec![half(above), half(below)])))
        },
    );
    reg.fn2(
        "Solid.trim",
        "Solid.trim({ normal, offset }, solid)",
        |p: Plane, s: FunctorLangSolid| {
            Solid::trim(s.0, p.normal, p.offset)
                .map(solid)
                .map_err(|e| format!("Solid.trim: {e}"))
        },
    );
    reg.fn2(
        "Solid.mirror",
        "Solid.mirror({ normal, offset }, solid)",
        |p: Plane, s: FunctorLangSolid| {
            s.0.mirror(p.normal, p.offset)
                .map(solid)
                .map_err(|e| format!("Solid.mirror: {e}"))
        },
    );
    reg.fn3(
        "Solid.frame",
        "Solid.frame(origin, xAxis, zAxis)",
        |o: FunctorLangVec3, x: FunctorLangVec3, z: FunctorLangVec3| {
            let f64s = |v: FunctorLangVec3| [v.0 .0, v.0 .1, v.0 .2].map(f64::from);
            let [x, y, z] =
                functor_solid::basis(f64s(x), f64s(z)).map_err(|e| format!("Solid.frame: {e}"))?;
            Ok::<_, String>(crate::input::record([
                ("origin", Value::HostData(Rc::new(o))),
                ("x", vec3_value(x)),
                ("y", vec3_value(y)),
                ("z", vec3_value(z)),
            ]))
        },
    );
    reg.fn2(
        "Solid.place",
        "Solid.place(frame, solid)",
        |f: Frame, s: FunctorLangSolid| solid(s.0.place(f.origin, f.axes)),
    );
    for (path, sig, cut) in [
        (
            "Solid.roundEdge",
            "Solid.roundEdge(radius, faceA, faceB, solid)",
            functor_solid::EdgeCut::Round as fn(f64) -> functor_solid::EdgeCut,
        ),
        (
            "Solid.bevelEdge",
            "Solid.bevelEdge(distance, faceA, faceB, solid)",
            functor_solid::EdgeCut::Bevel,
        ),
    ] {
        reg.fn4(
            path,
            sig,
            move |size: f64, a: Plane, b: Plane, s: FunctorLangSolid| {
                Solid::edge(s.0, (a.normal, a.offset), (b.normal, b.offset), cut(size))
                    .map(solid)
                    .map_err(|e| format!("{path}: {e}"))
            },
        );
    }
    reg.fn1(
        "Solid.hull",
        "Solid.hull([solid, …]) — at least one solid",
        |items: Vec<FunctorLangSolid>| {
            if items.is_empty() {
                return Err("Solid.hull needs at least one solid".to_string());
            }
            Ok(solid(Solid::hull(items.into_iter().map(|s| s.0).collect())))
        },
    );
    reg.fn2(
        "Solid.offset",
        "Solid.offset(distance, solid) — negative shrinks",
        |d: f64, s: FunctorLangSolid| {
            if !d.is_finite() {
                return Err(format!("Solid.offset distance must be finite, got {d}"));
            }
            // A computed fit of 0 is a no-op, not a mistake.
            Ok(if d == 0.0 { s } else { solid(s.0.offset(d)) })
        },
    );
    reg.fn2(
        "Solid.round",
        "Solid.round(radius, solid)",
        |r: f64, s: FunctorLangSolid| {
            if !(r.is_finite() && r > 0.0) {
                return Err(format!("Solid.round radius must be positive, got {r}"));
            }
            Ok(solid(s.0.rounded(r)))
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
    // Export's own check, so `expect` catches what `functor export` refuses.
    reg.fn1(
        "Solid.check",
        "Solid.check(solid)",
        |s: FunctorLangSolid| {
            let problems = match meshed(&s.0) {
                Ok(m) => m.mesh.print_problems("the part"),
                Err(e) => vec![e],
            };
            crate::input::record([
                ("ok", Value::Bool(problems.is_empty())),
                (
                    "problems",
                    Value::List(Rc::new(
                        problems
                            .into_iter()
                            .map(|p| Value::String(p.into()))
                            .collect(),
                    )),
                ),
            ])
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
        "Sketch.bounds",
        "Sketch.bounds(sketch)",
        |s: FunctorLangSketch| {
            let [lo, hi] = s.0.bounds().ok_or("Sketch.bounds: the sketch is empty")?;
            let point = |[x, y]: [f64; 2]| {
                crate::input::record([("x", Value::Number(x)), ("y", Value::Number(y))])
            };
            Ok::<_, String>(crate::input::record([
                ("min", point(lo)),
                ("max", point(hi)),
            ]))
        },
    );
    reg.fn1(
        "Sketch.hull",
        "Sketch.hull([sketch, …]) — at least one sketch",
        |items: Vec<FunctorLangSketch>| {
            if items.is_empty() {
                return Err("Sketch.hull needs at least one sketch".to_string());
            }
            let items: Vec<Sketch> = items.iter().map(|s| (*s.0).clone()).collect();
            Ok(sketch(Sketch::hull(&items)))
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
        RENDER_MESHES.with(|memo| memo.borrow().len())
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
    let bytes = super::path::resolve("", path)
        .map_err(|reason| format!("Typeface.file({path:?}): {reason}"))
        .and_then(|resolved| super::bytes::read_project_file("Typeface.file", &resolved))
        .map_err(
            |e| match cfg!(any(target_arch = "wasm32", target_os = "android")) {
                true => format!("{e} — use Typeface.sans() or Typeface.script()"),
                false => e,
            },
        )?;
    Font::from_bytes(path, bytes.to_vec()).map_err(|e| format!("Typeface.file: {e}"))
}
