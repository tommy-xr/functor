//! `Texture.fromPixels`, `Texture.atlas` and `Texture.withLayers` — textures
//! from decoded pixels (see `texture.funi`).

use std::rc::Rc;
use std::sync::Arc;

use crate::texture::{ColorSpace, Layer, LayeredImage, PixelData, PixelEncoding};

use super::bytes::FunctorLangBytes;
use super::*;

fn fail<T>(what: &str, message: String, span: Span) -> Result<T, RunError> {
    Err(RunError {
        message: format!("{what}: {message}"),
        span,
    })
}

fn record<'a>(value: &'a Value, what: &str, span: Span) -> Result<&'a [(String, Value)], RunError> {
    match value {
        Value::Record(fields) => Ok(fields),
        other => fail(
            what,
            format!("expected a record, got {}", other.kind_name()),
            span,
        ),
    }
}

fn field<'a>(
    fields: &'a [(String, Value)],
    name: &str,
    what: &str,
    span: Span,
) -> Result<&'a Value, RunError> {
    match fields.iter().find(|(field, _)| field == name) {
        Some((_, value)) => Ok(value),
        None => fail(what, format!("missing `{name}`"), span),
    }
}

/// A side length field (`width`, `maxSize`): a whole number in range.
fn side(
    fields: &[(String, Value)],
    name: &str,
    record_name: &str,
    what: &str,
    span: Span,
) -> Result<u32, RunError> {
    let n = finite_record_number(fields, name, record_name, what, span)?;
    crate::texture::check_side(name, n as f64).or_else(|e| fail(what, e, span))
}

/// A `Texture.colorSpace` constructor.
fn color_space_of(value: &Value, what: &str, span: Span) -> Result<ColorSpace, RunError> {
    match value {
        Value::Variant { ctor, args } if args.is_empty() => match &**ctor {
            "Texture.Srgb" => Ok(ColorSpace::Srgb),
            "Texture.Linear" => Ok(ColorSpace::Linear),
            other => fail(what, format!("colorSpace: unknown colour space {other}"), span),
        },
        other => fail(
            what,
            format!("colorSpace: expected a constructor, got {}", other.kind_name()),
            span,
        ),
    }
}

struct ColorSpaceArg(ColorSpace);

impl crate::host_registry::FromArg for ColorSpaceArg {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        color_space_of(value, path, span).map(ColorSpaceArg)
    }
}

/// A `Texture.pixels` record, validated and converted, with its encoding.
/// Errors start `what: `.
fn pixels_of(value: &Value, what: &str, span: Span) -> Result<(PixelData, PixelEncoding), RunError> {
    let fields = record(value, what, span)?;
    let ctor = |name: &str| match field(fields, name, what, span)? {
        Value::Variant { ctor, args } if args.is_empty() => Ok(ctor.to_string()),
        other => fail(
            what,
            format!("{name}: expected a constructor, got {}", other.kind_name()),
            span,
        ),
    };
    let encoding = match ctor("format")?.as_str() {
        "Texture.Rgba8" => PixelEncoding::Rgba8,
        "Texture.Rgb8" => PixelEncoding::Rgb8,
        "Texture.Rgb555le" => PixelEncoding::Rgb555le,
        other => return fail(what, format!("format: unknown format {other}"), span),
    };
    let color_space = color_space_of(field(fields, "colorSpace", what, span)?, what, span)?;
    let bytes: FunctorLangBytes = crate::host_registry::FromArg::from_arg(
        field(fields, "pixels", what, span)?,
        &format!("{what}: pixels"),
        span,
    )?;
    PixelData::new(
        side(fields, "width", "pixels", what, span)?,
        side(fields, "height", "pixels", what, span)?,
        encoding,
        color_space,
        bytes.0.as_slice(),
    )
    .map(|pixels| (pixels, encoding))
    .or_else(|e| fail(what, e, span))
}

/// An image's `layers`: each converted from `image`'s size and `encoding`,
/// ids distinct.
fn layers_of(
    value: &Value,
    image: &PixelData,
    encoding: PixelEncoding,
    what: &str,
    span: Span,
) -> Result<Vec<Layer>, RunError> {
    let Value::List(layers) = value else {
        return fail(what, "layers: expected a list".to_string(), span);
    };
    let mut out: Vec<Layer> = Vec::with_capacity(layers.len());
    for (j, layer) in layers.iter().enumerate() {
        let what = format!("{what}: layers[{j}]");
        let fields = record(layer, &what, span)?;
        let id = match field(fields, "id", &what, span)? {
            Value::Number(id) if id.is_finite() => *id,
            Value::Number(id) => return fail(&what, format!("id must be finite, got {id}"), span),
            other => return fail(&what, format!("id: expected a number, got {}", other.kind_name()), span),
        };
        if let Some(k) = out.iter().position(|l| l.id == id) {
            return fail(&what, format!("id {id} repeats layers[{k}]"), span);
        }
        let bytes: FunctorLangBytes = crate::host_registry::FromArg::from_arg(
            field(fields, "pixels", &what, span)?,
            &format!("{what}: pixels"),
            span,
        )?;
        let pixels = PixelData::new(
            image.width(),
            image.height(),
            encoding,
            image.color_space(),
            bytes.0.as_slice(),
        )
        .or_else(|e| fail(&what, e, span))?;
        out.push(Layer { id, rgba8: pixels.rgba8().to_vec() });
    }
    Ok(out)
}

struct PixelsArg(PixelData);

impl crate::host_registry::FromArg for PixelsArg {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        pixels_of(value, path, span).map(|(pixels, _)| PixelsArg(pixels))
    }
}

/// `Texture.atlas`'s record: images and their layers converted, padding and
/// size checked.
struct AtlasArg {
    images: Vec<(PixelData, Vec<Layer>)>,
    padding: u32,
    max_size: u32,
}

impl crate::host_registry::FromArg for AtlasArg {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let fields = record(value, path, span)?;
        let Value::List(images) = field(fields, "images", path, span)? else {
            return fail(path, "images: expected a list".to_string(), span);
        };
        let images = images
            .iter()
            .enumerate()
            .map(|(i, image)| {
                let what = format!("{path}: images[{i}]");
                let image = record(image, &what, span)?;
                let (pixels, encoding) = pixels_of(field(image, "pixels", &what, span)?, &what, span)?;
                let layers = layers_of(field(image, "layers", &what, span)?, &pixels, encoding, &what, span)?;
                Ok((pixels, layers))
            })
            .collect::<Result<_, _>>()?;
        let padding = finite_record_number(fields, "padding", "atlas", path, span)?;
        if padding.fract() != 0.0 || padding < 0.0 {
            return fail(
                path,
                format!("padding must be a whole number from 0, got {padding}"),
                span,
            );
        }
        Ok(AtlasArg {
            images,
            padding: padding as u32,
            max_size: side(fields, "maxSize", "atlas", path, span)?,
        })
    }
}

fn texture_value(data: PixelData) -> Value {
    Value::HostData(Rc::new(FunctorLangTexture(TextureDescription::Data(
        Arc::new(data),
    ))))
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn1(
        "Texture.fromPixels",
        "Texture.fromPixels(pixels)",
        |p: PixelsArg| texture_value(p.0),
    );
    reg.fn2(
        "Texture.fromPng",
        "Texture.fromPng(File.bytes(Path.of(\"baked/room.png\")), Texture.Srgb)",
        |png: FunctorLangBytes, space: ColorSpaceArg| {
            let fail = |e: String| format!("Texture.fromPng: {e}");
            let image = image::load_from_memory_with_format(png.0.as_slice(), image::ImageFormat::Png)
                .map_err(|e| fail(e.to_string()))?
                .to_rgba8();
            let (width, height) = image.dimensions();
            PixelData::new(width, height, PixelEncoding::Rgba8, space.0, image.as_raw())
                .map(texture_value)
                .map_err(fail)
        },
    );
    reg.fn1(
        "Texture.atlas",
        "Texture.atlas({ images, padding, maxSize })",
        |a: AtlasArg| {
            let images: Vec<&PixelData> = a.images.iter().map(|(image, _)| image).collect();
            let (atlas, placed) = crate::texture::atlas(&images, a.padding, a.max_size)
                .map_err(|e| format!("Texture.atlas: {e}"))?;
            let side = atlas.width() as f64;
            // Images with layers keep them, placed where the image landed.
            let layered = a
                .images
                .iter()
                .zip(&placed)
                .filter(|((_, layers), _)| !layers.is_empty())
                .map(|((image, layers), p)| LayeredImage {
                    x: p.x,
                    y: p.y,
                    width: image.width(),
                    height: image.height(),
                    padding: a.padding,
                    layers: layers.clone(),
                })
                .collect();
            let atlas = atlas
                .with_layers(layered)
                .map_err(|e| format!("Texture.atlas: {e}"))?;
            // UVs inset half a texel, so bilinear taps stay inside the image.
            let rects = images
                .iter()
                .zip(placed)
                .map(|(image, p)| {
                    let (x, y) = (p.x as f64, p.y as f64);
                    crate::input::record([
                        ("x", Value::Number(x)),
                        ("y", Value::Number(y)),
                        ("u0", Value::Number((x + 0.5) / side)),
                        ("v0", Value::Number((y + 0.5) / side)),
                        ("u1", Value::Number((x + image.width() as f64 - 0.5) / side)),
                        (
                            "v1",
                            Value::Number((y + image.height() as f64 - 0.5) / side),
                        ),
                    ])
                })
                .collect();
            Ok::<_, String>(crate::input::record([
                ("texture", texture_value(atlas)),
                ("rects", Value::List(Rc::new(rects))),
            ]))
        },
    );
    reg.fn2(
        "Texture.withLayers",
        "Texture.withLayers(intensities, atlas)",
        |levels: IntensitiesArg, texture: FunctorLangTexture| {
            // A layered texture gets new intensities for the same atlas.
            let (TextureDescription::Data(base) | TextureDescription::Layered { base, .. }) =
                texture.0
            else {
                return Err("Texture.withLayers: expected a Texture.atlas or Texture.fromPixels \
texture"
                    .to_string());
            };
            Ok(FunctorLangTexture(TextureDescription::Layered {
                base,
                intensities: levels.0,
            }))
        },
    );
}

/// `Texture.withLayers`'s `Map<float, float>`: intensities finite and from 0.
struct IntensitiesArg(Vec<(f64, f32)>);

impl crate::host_registry::FromArg for IntensitiesArg {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        use functor_lang::value::MapKey;
        let Value::Map(entries) = value else {
            return fail(path, format!("expected a Map<float, float>, got {}", value.kind_name()), span);
        };
        let out = entries
            .iter()
            .map(|(key, value)| match (key, value) {
                (MapKey::Number(id), Value::Number(level)) if (*level as f32).is_finite() && *level >= 0.0 => {
                    Ok((*id, *level as f32))
                }
                (MapKey::Number(id), Value::Number(level)) => fail(
                    path,
                    format!("layer {id}: intensity must be a finite number from 0, got {level}"),
                    span,
                ),
                _ => fail(path, "expected a Map<float, float>".to_string(), span),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(IntensitiesArg(out))
    }
}
