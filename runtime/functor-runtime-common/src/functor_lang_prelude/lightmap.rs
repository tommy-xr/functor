//! `Lightmap.unwrap` (see `lightmap.funi`).

use std::rc::Rc;

use crate::lightmap::{unwrap, UnwrapOptions};
use crate::model::ModelData;

use super::bytes::{ByteView, FunctorLangBytes};
use super::*;

const NAME: &str = "Lightmap.unwrap";

fn fail<T>(message: String, span: Span) -> Result<T, RunError> {
    Err(RunError {
        message: format!("{NAME}: {message}"),
        span,
    })
}

/// The `Model.staticData` record, kept whole (its `groups` and `lightmap`
/// pass through) beside its validated contents.
struct DataArg {
    fields: Rc<Vec<(String, Value)>>,
    data: ModelData,
}

impl crate::host_registry::FromArg for DataArg {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        let source = model::static_source(value, span).or_else(|e| fail(e.message, e.span))?;
        let data = ModelData::new(source).or_else(|e| fail(e, span))?;
        let Value::Record(fields) = value else {
            unreachable!("static_source checked the record");
        };
        Ok(DataArg {
            fields: fields.clone(),
            data,
        })
    }
}

struct OptionsArg(UnwrapOptions);

impl crate::host_registry::FromArg for OptionsArg {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        let Value::Record(fields) = value else {
            return fail(
                format!("options: expected a record, got {}", value.kind_name()),
                span,
            );
        };
        let number = |name| finite_record_number(fields, name, "options", NAME, span);
        let padding = number("padding")?;
        if padding.fract() != 0.0 || padding < 0.0 {
            return fail(
                format!("padding must be a whole number from 0, got {padding}"),
                span,
            );
        }
        let max_size = crate::texture::check_side("maxSize", number("maxSize")? as f64)
            .or_else(|e| fail(e, span))?;
        Ok(OptionsArg(UnwrapOptions {
            texels_per_meter: number("texelsPerMeter")? as f64,
            padding: padding as u32,
            max_size,
        }))
    }
}

fn bytes(data: Vec<u8>) -> Value {
    Value::HostData(Rc::new(FunctorLangBytes(ByteView::new(data))))
}

/// Little-endian f32 components of each entry.
fn f32s<const N: usize>(entries: impl Iterator<Item = [f32; N]>) -> Value {
    bytes(entries.flatten().flat_map(f32::to_le_bytes).collect())
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn2(
        NAME,
        "Lightmap.unwrap(data, { texelsPerMeter, padding, maxSize })",
        |input: DataArg, options: OptionsArg| {
            let s = input.data.source();
            let out = unwrap(&s.positions, &s.triangles, options.0)
                .map_err(|e| format!("{NAME}: {e}"))?;
            let copy = |values: &[[f32; 3]]| f32s(out.source.iter().map(|&v| values[v as usize]));
            let uvs = if s.uvs.is_empty() {
                f32s::<2>(std::iter::empty())
            } else {
                f32s(out.source.iter().map(|&v| s.uvs[v as usize]))
            };
            let replaced = [
                ("positions", copy(&s.positions)),
                ("normals", copy(&s.normals)),
                ("uvs", uvs),
                ("uvs2", f32s(out.uvs2.iter().copied())),
                (
                    "triangles",
                    bytes(out.triangles.iter().flatten().flat_map(|v| v.to_le_bytes()).collect()),
                ),
            ];
            let mut mesh = (*input.fields).clone();
            for (name, value) in replaced {
                if let Some(field) = mesh.iter_mut().find(|(field, _)| field == name) {
                    field.1 = value;
                }
            }
            let side = Value::Number(out.size as f64);
            Ok::<_, String>(crate::input::record([
                ("mesh", Value::Record(Rc::new(mesh))),
                (
                    "source",
                    bytes(out.source.iter().flat_map(|v| v.to_le_bytes()).collect()),
                ),
                ("width", side.clone()),
                ("height", side),
            ]))
        },
    );
}
