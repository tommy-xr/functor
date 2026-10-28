//! `Lightmap.unwrap`, `Lightmap.readMesh` and the `bakes` jobs `functor bake`
//! reads (see `lightmap.funi`).

use std::rc::Rc;

use crate::light::Light;
use crate::lightmap::{read_mesh, unwrap, unwrapped_source, UnwrapOptions};
use crate::model::{ModelData, ModelSource};

use super::bytes::{ByteView, FunctorLangBytes};
use super::*;

const NAME: &str = "Lightmap.unwrap";

fn fail<T>(message: String, span: Span) -> Result<T, RunError> {
    Err(RunError {
        message: format!("{NAME}: {message}"),
        span,
    })
}

/// The `Model.staticData` record, kept whole (its `groups` pass through)
/// beside its validated contents.
struct DataArg {
    fields: Rc<Vec<(String, Value)>>,
    data: ModelData,
}

impl crate::host_registry::FromArg for DataArg {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        let mut source = model::static_source(value, span).or_else(|e| fail(e.message, e.span))?;
        // Both are replaced, so neither needs to be valid.
        source.uvs2.clear();
        source.lightmap = None;
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
fn f32s<const N: usize>(entries: &[[f32; N]]) -> Value {
    bytes(entries.as_flattened().iter().flat_map(|f| f.to_le_bytes()).collect())
}

/// `mesh`'s vertex data and triangles as `Model.staticData` fields.
fn mesh_fields(mesh: &ModelSource) -> [(&'static str, Value); 5] {
    [
        ("positions", f32s(&mesh.positions)),
        ("normals", f32s(&mesh.normals)),
        ("uvs", f32s(&mesh.uvs)),
        ("uvs2", f32s(&mesh.uvs2)),
        (
            "triangles",
            bytes(mesh.triangles.as_flattened().iter().flat_map(|v| v.to_le_bytes()).collect()),
        ),
    ]
}

/// One `Lightmap.bakeJob`: a validated mesh and the lights to bake into it.
#[derive(Debug, Clone)]
pub struct BakeJob {
    pub name: String,
    pub mesh: ModelSource,
    pub lights: Vec<Light>,
}

/// The jobs in a `List<Lightmap.bakeJob>` value (`functor bake`'s `bakes`),
/// or an error naming the job and field. Names must be unique file stems.
pub fn bake_jobs(value: &Value) -> Result<Vec<BakeJob>, String> {
    let span = Span::new(0, 0);
    let Value::List(jobs) = value else {
        return Err(format!("expected a list of Lightmap.bakeJob, got {}", value.kind_name()));
    };
    let mut out: Vec<BakeJob> = Vec::new();
    for (i, job) in jobs.iter().enumerate() {
        let what = format!("bakes[{i}]");
        let Value::Record(fields) = job else {
            return Err(format!("{what}: expected a record, got {}", job.kind_name()));
        };
        let get = |name: &str| {
            fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, v)| v)
                .ok_or_else(|| format!("{what}: missing `{name}`"))
        };
        let name = match get("name")? {
            Value::String(name) => name.to_string(),
            other => return Err(format!("{what}.name: expected a string, got {}", other.kind_name())),
        };
        if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
            return Err(format!(
                "{what}.name: {name:?} must be a file name without a folder (it names `<name>.png`)"
            ));
        }
        if out.iter().any(|job| job.name == name) {
            return Err(format!("{what}.name: {name:?} is used twice"));
        }
        let mut mesh = model::static_source(get("mesh")?, span)
            .map_err(|e| format!("{what}.mesh: {}", e.message))?;
        // The bake unwraps it, so neither needs to be valid.
        mesh.uvs2.clear();
        mesh.lightmap = None;
        let mesh = ModelData::new(mesh)
            .map_err(|e| format!("{what}.mesh: {e}"))?
            .source()
            .clone();
        let Value::List(lights) = get("lights")? else {
            return Err(format!("{what}.lights: expected a list"));
        };
        let lights = lights
            .iter()
            .enumerate()
            .map(|(l, light)| {
                let path = format!("{what}.lights[{l}]");
                <FunctorLangLight as crate::host_registry::FromArg>::from_arg(light, &path, span)
                    .map(|light| light.0)
                    .map_err(|e| e.message)
            })
            .collect::<Result<_, _>>()?;
        out.push(BakeJob { name, mesh, lights });
    }
    Ok(out)
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn2(
        NAME,
        "Lightmap.unwrap(data, { texelsPerMeter, padding, maxSize })",
        |input: DataArg, options: OptionsArg| {
            let s = input.data.source();
            let out = unwrap(&s.positions, &s.triangles, options.0)
                .map_err(|e| format!("{NAME}: {e}"))?;
            // A lightmap baked for the old uvs2 no longer fits.
            let replaced = mesh_fields(&unwrapped_source(s, &out))
                .into_iter()
                .chain([("lightmap", crate::input::option_value(None))]);
            let mut mesh = (*input.fields).clone();
            for (name, value) in replaced {
                let field = mesh.iter_mut().find(|(field, _)| field == name);
                field.expect("static_source checked the field").1 = value;
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
    reg.fn1(
        "Lightmap.readMesh",
        "Lightmap.readMesh(File.bytes(\"baked/room.mesh.bin\"))",
        |data: FunctorLangBytes| {
            let mesh = read_mesh(data.0.as_slice()).map_err(|e| format!("Lightmap.readMesh: {e}"))?;
            let groups = mesh
                .groups
                .iter()
                .map(|g| {
                    crate::input::record([
                        ("first", Value::Number(g.first as f64)),
                        ("count", Value::Number(g.count as f64)),
                        ("texture", crate::input::option_value(None)),
                        ("unlit", Value::Bool(g.unlit)),
                    ])
                })
                .collect();
            Ok::<_, String>(crate::input::record(mesh_fields(&mesh).into_iter().chain([
                ("groups", Value::List(Rc::new(groups))),
                ("lightmap", crate::input::option_value(None)),
            ])))
        },
    );
}
