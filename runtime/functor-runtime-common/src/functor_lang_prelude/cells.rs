//! `Cells.*` — cells and portals as data (see `cells.funi`).

use std::sync::Arc;

use crate::cells::{Cell, Cells, CellsData, Plane, Portal};

use super::*;

/// Validated [`Cells`] as an opaque value.
pub struct FunctorLangCells(pub Arc<Cells>);

impl HostData for FunctorLangCells {
    fn type_name(&self) -> &'static str {
        "Cells"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // Immutable data: safe to keep in a model across reloads.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

crate::host_returnable!(FunctorLangCells);

/// A record's `name` field.
pub(super) fn field<'a>(value: &'a Value, name: &str, what: &str) -> Result<&'a Value, String> {
    match value {
        Value::Record(fields) => fields.iter().find(|(f, _)| f == name).map(|(_, v)| v),
        _ => None,
    }
    .ok_or_else(|| format!("{what} has no `{name}`"))
}

pub(super) fn list<'a>(value: &'a Value, what: &str) -> Result<&'a [Value], String> {
    match value {
        Value::List(items) => Ok(items),
        other => Err(format!("{what} must be a list, got {}", other.kind_name())),
    }
}

pub(super) fn number(value: &Value, what: &str) -> Result<f64, String> {
    match value {
        Value::Number(n) => Ok(*n),
        other => Err(format!(
            "{what} must be a number, got {}",
            other.kind_name()
        )),
    }
}

/// `n` as an id: a whole number ≥ 0 that fits a `u32`.
pub(super) fn whole(n: f64) -> Option<u32> {
    (n >= 0.0 && n.fract() == 0.0 && n <= f64::from(u32::MAX)).then_some(n as u32)
}

/// A cell id: a whole number ≥ 0.
fn cell_id(n: f64, what: &str) -> Result<u32, String> {
    whole(n).ok_or_else(|| format!("{what} must be a cell id (a whole number ≥ 0), got {n}"))
}

fn id(value: &Value, what: &str) -> Result<u32, String> {
    cell_id(number(value, what)?, what)
}

pub(super) fn point(value: &Value, what: &str, span: Span) -> Result<[f32; 3], String> {
    vec3_of(value, what, span)
        .map(|(x, y, z)| [x, y, z])
        .map_err(|e| e.message)
}

fn level(value: &Value, span: Span) -> Result<CellsData, String> {
    let cell = |(i, cell): (usize, &Value)| {
        let what = format!("cell {i}");
        let planes = list(field(cell, "planes", &what)?, &what)?;
        let planes = planes.iter().map(|plane| {
            Ok(Plane {
                n: point(field(plane, "n", &what)?, &what, span)?,
                d: number(field(plane, "d", &what)?, &what)? as f32,
            })
        });
        Ok(Cell {
            planes: planes.collect::<Result<_, String>>()?,
        })
    };
    let portal = |(k, portal): (usize, &Value)| {
        let what = format!("portal {k}");
        let polygon = list(field(portal, "polygon", &what)?, &what)?;
        Ok(Portal {
            from: id(field(portal, "from", &what)?, &format!("{what} `from`"))?,
            to: id(field(portal, "to", &what)?, &format!("{what} `to`"))?,
            polygon: polygon
                .iter()
                .map(|p| point(p, &what, span))
                .collect::<Result<_, _>>()?,
        })
    };
    Ok(CellsData {
        cells: list(field(value, "cells", "the level")?, "cells")?
            .iter()
            .enumerate()
            .map(cell)
            .collect::<Result<_, String>>()?,
        portals: list(field(value, "portals", "the level")?, "portals")?
            .iter()
            .enumerate()
            .map(portal)
            .collect::<Result<_, String>>()?,
    })
}

impl crate::host_registry::FromArg for CellsData {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        level(value, span).map_err(|e| RunError {
            message: format!("Cells.make: {e}"),
            span,
        })
    }
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn1(
        "Cells.make",
        "Cells.make({ cells, portals })",
        |data: CellsData| {
            Cells::new(data)
                .map(|cells| FunctorLangCells(Arc::new(cells)))
                .map_err(|e| format!("Cells.make: {e}"))
        },
    );
    reg.fn2(
        "Cells.locate",
        "Cells.locate(point, cells)",
        |p: FunctorLangVec3, cells: FunctorLangCells| {
            crate::input::option_value(
                cells
                    .0
                    .locate(p.0.into())
                    .map(|cell| Value::Number(f64::from(cell))),
            )
        },
    );
    reg.fn3(
        "Cells.segment",
        "Cells.segment(a, b, cells)",
        |a: FunctorLangVec3, b: FunctorLangVec3, cells: FunctorLangCells| {
            Value::Bool(cells.0.segment(a.0.into(), b.0.into()))
        },
    );
    reg.fn2(
        "Scene.cells",
        "Scene.cells(cells, scene)",
        |cells: FunctorLangCells, scene: FunctorLangScene| {
            FunctorLangScene(Scene3D {
                obj: SceneObject::Cells(crate::CellsNode::Walk(cells.0), vec![scene.0]),
                xform: Matrix4::from_scale(1.0),
            })
        },
    );
    reg.fn2(
        "Scene.inCells",
        "Scene.inCells(ids, scene)",
        |ids: Vec<f64>, scene: FunctorLangScene| {
            let ids = ids
                .into_iter()
                .map(|id| cell_id(id, "Scene.inCells: an id"))
                .collect::<Result<_, _>>()?;
            Ok::<_, String>(FunctorLangScene(Scene3D {
                obj: SceneObject::Cells(crate::CellsNode::In(ids), vec![scene.0]),
                xform: Matrix4::from_scale(1.0),
            }))
        },
    );
}
