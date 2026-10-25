//! `Solid.*` — printable parts in millimetres (see `solid.funi`).
//!
//! A solid crosses into Functor Lang as an opaque, immutable description tree
//! ([`functor_solid::Solid`]); meshing happens only when it is measured or
//! exported, so building one in a hot-reloading file stays cheap.

use std::sync::Arc;

use functor_solid::Solid;

use super::*;

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

impl crate::host_registry::FromArg for FunctorLangSolid {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        if let Value::HostData(data) = value {
            if let Some(solid) = data.as_any().downcast_ref::<FunctorLangSolid>() {
                return Ok(FunctorLangSolid(solid.0.clone()));
            }
        }
        Err(RunError {
            message: format!("{path}: expected a Solid, got {}", value.kind_name()),
            span,
        })
    }
}

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

fn positive(path: &str, what: &str, value: f64) -> Result<f64, String> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(format!(
            "{path}: {what} must be a positive number of millimetres, got {value}"
        ))
    }
}

fn solid(solid: Solid) -> FunctorLangSolid {
    FunctorLangSolid(Arc::new(solid))
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn3(
        "Solid.box",
        "Solid.box(width, depth, height)",
        |w: f64, d: f64, h: f64| {
            let p = "Solid.box";
            let size = [
                positive(p, "width", w)?,
                positive(p, "depth", d)?,
                positive(p, "height", h)?,
            ];
            Ok(solid(Solid::Box { size }))
        },
    );
    reg.fn2(
        "Solid.cylinder",
        "Solid.cylinder(radius, height)",
        |r: f64, h: f64| {
            let p = "Solid.cylinder";
            let (radius, height) = (positive(p, "radius", r)?, positive(p, "height", h)?);
            Ok(solid(Solid::Cylinder { radius, height }))
        },
    );
    reg.fn1("Solid.sphere", "Solid.sphere(radius)", |r: f64| {
        Ok(solid(Solid::Sphere {
            radius: positive("Solid.sphere", "radius", r)?,
        }))
    });

    reg.fn2(
        "Solid.translate",
        "Solid.translate(Vec3.make(x, y, z), solid)",
        |v: FunctorLangVec3, s: FunctorLangSolid| {
            let (x, y, z) = v.0;
            solid(s.0.translate([x as f64, y as f64, z as f64]))
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
            let k = positive("Solid.scale", "factor", k)?;
            Ok(solid(s.0.scale(k)))
        },
    );
    reg.fn1(
        "Solid.group",
        "Solid.group([solid, …])",
        |items: Vec<FunctorLangSolid>| {
            solid(Solid::Group(items.into_iter().map(|s| s.0).collect()))
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
            let b = s.0.mesh().bounds().unwrap_or(functor_solid::Bounds {
                min: [0.0; 3],
                max: [0.0; 3],
            });
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
