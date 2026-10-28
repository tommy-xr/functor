//! `Model.*` and `Scene.modelOf` — skinned models built from decoded data
//! (see `model.funi`), drawn and animated like glTF models.

use std::rc::Rc;
use std::sync::Arc;

use crate::model::{ClipSource, JointSource, SkinnedModelData, SkinnedModelSource, TrackSource};

use super::bytes::{ByteView, FunctorLangBytes};
use super::*;

/// A [`SkinnedModelData`] as an opaque value.
pub struct FunctorLangModel(pub Arc<SkinnedModelData>);

impl HostData for FunctorLangModel {
    fn type_name(&self) -> &'static str {
        "Model"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // Immutable data: safe to keep in a model across reloads.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

crate::host_returnable!(FunctorLangModel);

/// `Model.skinned`'s data record, unpacked (validation is
/// [`SkinnedModelData::new`]'s).
struct SourceArg(SkinnedModelSource);

/// Errors here name the field: `Model.skinned: skeleton[1].parent: …`.
fn fail<T>(message: String, span: Span) -> Result<T, RunError> {
    Err(RunError {
        message: format!("Model.skinned: {message}"),
        span,
    })
}

fn field<'a>(
    fields: &'a [(String, Value)],
    name: &str,
    what: &str,
    span: Span,
) -> Result<&'a Value, RunError> {
    match fields.iter().find(|(field, _)| field == name) {
        Some((_, value)) => Ok(value),
        None => fail(format!("{what} is missing `{name}`"), span),
    }
}

fn record<'a>(value: &'a Value, what: &str, span: Span) -> Result<&'a [(String, Value)], RunError> {
    match value {
        Value::Record(fields) => Ok(fields),
        other => fail(
            format!("{what}: expected a record, got {}", other.kind_name()),
            span,
        ),
    }
}

fn list<'a>(value: &'a Value, what: &str, span: Span) -> Result<&'a [Value], RunError> {
    match value {
        Value::List(items) => Ok(items),
        other => fail(
            format!("{what}: expected a list, got {}", other.kind_name()),
            span,
        ),
    }
}

fn string(value: &Value, what: &str, span: Span) -> Result<String, RunError> {
    match value {
        Value::String(s) => Ok(s.to_string()),
        other => fail(
            format!("{what}: expected a string, got {}", other.kind_name()),
            span,
        ),
    }
}

fn number(value: &Value, what: &str, span: Span) -> Result<f64, RunError> {
    match value {
        Value::Number(n) if n.is_finite() => Ok(*n),
        Value::Number(n) => fail(format!("{what}: expected a finite number, got {n}"), span),
        other => fail(
            format!(
                "{what}: expected a finite number, got {}",
                other.kind_name()
            ),
            span,
        ),
    }
}

/// A whole number in `0..=max`.
fn index(value: &Value, max: f64, what: &str, span: Span) -> Result<u32, RunError> {
    let n = number(value, what, span)?;
    if n >= 0.0 && n.fract() == 0.0 && n <= max {
        Ok(n as u32)
    } else {
        fail(
            format!("{what}: expected a whole number from 0, got {n}"),
            span,
        )
    }
}

/// Little-endian `N`-component entries of `T` (`size` bytes each).
fn packed<T: Copy, const N: usize>(
    value: &Value,
    what: &str,
    span: Span,
    size: usize,
    read: impl Fn(&[u8]) -> T,
) -> Result<Vec<[T; N]>, RunError> {
    let bytes = <FunctorLangBytes as crate::host_registry::FromArg>::from_arg(
        value,
        &format!("Model.skinned: {what}"),
        span,
    )?;
    let raw = bytes.0.as_slice();
    let stride = size * N;
    if raw.len() % stride != 0 {
        return fail(
            format!(
                "{what}: {} bytes is not a whole number of {stride}-byte entries",
                raw.len()
            ),
            span,
        );
    }
    Ok(raw
        .chunks_exact(stride)
        .map(|entry| std::array::from_fn(|k| read(&entry[size * k..size * (k + 1)])))
        .collect())
}

fn f32s<const N: usize>(value: &Value, what: &str, span: Span) -> Result<Vec<[f32; N]>, RunError> {
    packed(value, what, span, 4, |b| {
        f32::from_le_bytes(b.try_into().unwrap())
    })
}

/// `Option.Some(x)` as `Some(some(x))`, `Option.None` as `None`.
fn option<T>(
    value: &Value,
    what: &str,
    inner: &str,
    span: Span,
    some: impl FnOnce(&Value) -> Result<T, RunError>,
) -> Result<Option<T>, RunError> {
    match value {
        Value::Variant { ctor, args } if ctor.as_ref() == "Option.Some" && args.len() == 1 => {
            some(&args[0]).map(Some)
        }
        Value::Variant { ctor, .. } if ctor.as_ref() == "Option.None" => Ok(None),
        other => fail(
            format!(
                "{what}: expected Option.Some({inner}) or Option.None, got {}",
                other.kind_name()
            ),
            span,
        ),
    }
}

fn joint(value: &Value, what: &str, span: Span) -> Result<JointSource, RunError> {
    let fields = record(value, what, span)?;
    let get = |name: &str| field(fields, name, what, span);
    let parent = option(get("parent")?, &format!("{what}.parent"), "index", span, |v| {
        index(v, u32::MAX as f64, &format!("{what}.parent"), span)
    })?;
    let (x, y, z) = vec3_of(
        get("translation")?,
        &format!("Model.skinned: {what}.translation"),
        span,
    )?;
    let rotation = record(get("rotation")?, &format!("{what}.rotation"), span)?;
    let path = format!("Model.skinned: {what}");
    let component = |name| finite_record_number(rotation, name, "rotation", &path, span);
    Ok(JointSource {
        name: string(get("name")?, &format!("{what}.name"), span)?,
        parent,
        translation: [x, y, z],
        rotation: [
            component("x")?,
            component("y")?,
            component("z")?,
            component("w")?,
        ],
    })
}

fn clip(value: &Value, what: &str, span: Span) -> Result<ClipSource, RunError> {
    let fields = record(value, what, span)?;
    let get = |name: &str| field(fields, name, what, span);
    let tracks = list(get("tracks")?, &format!("{what}.tracks"), span)?
        .iter()
        .enumerate()
        .map(|(t, track)| {
            let what = format!("{what}.tracks[{t}]");
            let fields = record(track, &what, span)?;
            let get = |name: &str| field(fields, name, &what, span);
            Ok(TrackSource {
                joint: string(get("joint")?, &format!("{what}.joint"), span)?,
                rotations: f32s(get("rotations")?, &format!("{what}.rotations"), span)?,
                translations: f32s(get("translations")?, &format!("{what}.translations"), span)?,
            })
        })
        .collect::<Result<_, RunError>>()?;
    Ok(ClipSource {
        name: string(get("name")?, &format!("{what}.name"), span)?,
        fps: number(get("fps")?, &format!("{what}.fps"), span)? as f32,
        frames: index(
            get("frames")?,
            u32::MAX as f64,
            &format!("{what}.frames"),
            span,
        )?,
        tracks,
    })
}

impl crate::host_registry::FromArg for SourceArg {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        let fields = record(value, "data", span)?;
        let get = |name: &str| field(fields, name, "data", span);
        let skeleton = list(get("skeleton")?, "skeleton", span)?
            .iter()
            .enumerate()
            .map(|(i, j)| joint(j, &format!("skeleton[{i}]"), span))
            .collect::<Result<_, _>>()?;
        let clips = list(get("clips")?, "clips", span)?
            .iter()
            .enumerate()
            .map(|(c, value)| clip(value, &format!("clips[{c}]"), span))
            .collect::<Result<_, _>>()?;
        let texture = option(get("texture")?, "texture", "texture", span, |v| {
            texture_of(v, "Model.skinned: texture", span)
        })?;
        Ok(SourceArg(SkinnedModelSource {
            positions: f32s(get("positions")?, "positions", span)?,
            normals: f32s(get("normals")?, "normals", span)?,
            uvs: f32s(get("uvs")?, "uvs", span)?,
            joints: packed(get("joints")?, "joints", span, 1, |b| b[0])?,
            weights: f32s(get("weights")?, "weights", span)?,
            triangles: packed(get("triangles")?, "triangles", span, 4, |b| {
                u32::from_le_bytes(b.try_into().unwrap())
            })?,
            skeleton,
            clips,
            texture,
        }))
    }
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn1("Model.skinned", "Model.skinned(data)", |data: SourceArg| {
        SkinnedModelData::new(data.0)
            .map(|model| FunctorLangModel(Arc::new(model)))
            .map_err(|e| format!("Model.skinned: {e}"))
    });
    reg.fn2(
        "Model.pose",
        "Model.pose(anim, model)",
        |anim: FunctorLangAnim, model: FunctorLangModel| {
            let data = &model.0;
            let mut missing = None;
            let palette = crate::anim::rig_skinning_transforms(
                data.skeleton(),
                data.animations(),
                &anim.0,
                &mut |warning| {
                    use crate::anim::AnimWarning;
                    missing.get_or_insert_with(|| match warning {
                        AnimWarning::MissingClip(name) => format!("no clip named {name:?}"),
                        AnimWarning::MissingJoint(name) => format!("no joint named {name:?}"),
                        AnimWarning::InvalidChain { root, middle, end } => format!(
                            "{root:?} -> {middle:?} -> {end:?} is not a direct two-bone chain"
                        ),
                        AnimWarning::NonUniformRootScale { root } => {
                            format!("reach root {root:?} has non-uniform scale")
                        }
                    });
                },
            );
            if let Some(missing) = missing {
                return Err(format!("Model.pose: {missing}"));
            }
            let bytes = data
                .skin_positions(&palette)
                .iter()
                .flatten()
                .flat_map(|x| x.to_le_bytes())
                .collect();
            Ok(FunctorLangBytes(ByteView::new(bytes)))
        },
    );
    reg.fn1(
        "Model.inspect",
        "Model.inspect(model)",
        |model: FunctorLangModel| {
            let s = model.0.source();
            let strings = |names: Vec<Value>| Value::List(Rc::new(names));
            crate::input::record([
                ("vertices", Value::Number(s.positions.len() as f64)),
                ("triangles", Value::Number(s.triangles.len() as f64)),
                (
                    "joints",
                    strings(
                        s.skeleton
                            .iter()
                            .map(|j| Value::String(j.name.as_str().into()))
                            .collect(),
                    ),
                ),
                (
                    "clips",
                    Value::List(Rc::new(
                        model
                            .0
                            .animations()
                            .iter()
                            .map(|a| {
                                crate::input::record([
                                    ("name", Value::String(a.name.as_str().into())),
                                    ("duration", Value::Number(a.duration as f64)),
                                ])
                            })
                            .collect(),
                    )),
                ),
            ])
        },
    );
    reg.fn1(
        "Scene.modelOf",
        "Scene.modelOf(model)",
        |model: FunctorLangModel| {
            FunctorLangScene(Scene3D::model(ModelDescription {
                handle: ModelHandle::Data(model.0),
                overrides: Vec::new(),
                animation: None,
                while_pending: Vec::new(),
            }))
        },
    );
}
