//! `Texture.fromPixels` — textures from decoded pixels (see `texture.funi`).

use std::sync::Arc;

use crate::texture::{ColorSpace, PixelData, PixelEncoding};

use super::bytes::FunctorLangBytes;
use super::*;

/// `Texture.fromPixels`'s record, unpacked (validation is [`PixelData::new`]'s).
struct PixelsArg {
    width: f64,
    height: f64,
    encoding: PixelEncoding,
    color_space: ColorSpace,
    pixels: FunctorLangBytes,
}

const PATH: &str = "Texture.fromPixels";

fn fail<T>(message: String, span: Span) -> Result<T, RunError> {
    Err(RunError {
        message: format!("{PATH}: {message}"),
        span,
    })
}

/// A whole number, or an error naming `name` (range is checked later).
fn side(value: f64, name: &str) -> Result<u32, String> {
    if value.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&value) {
        Ok(value as u32)
    } else {
        Err(format!("{name} must be a whole number, got {value}"))
    }
}

impl crate::host_registry::FromArg for PixelsArg {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        let Value::Record(fields) = value else {
            return fail(format!("expected a record, got {}", value.kind_name()), span);
        };
        let get = |name: &str| match fields.iter().find(|(field, _)| field == name) {
            Some((_, value)) => Ok(value),
            None => fail(format!("missing `{name}`"), span),
        };
        let number = |name: &str| match get(name)? {
            Value::Number(n) => Ok(*n),
            other => fail(
                format!("{name}: expected a number, got {}", other.kind_name()),
                span,
            ),
        };
        let ctor = |name: &str| match get(name)? {
            Value::Variant { ctor, args } if args.is_empty() => Ok(ctor.to_string()),
            other => fail(
                format!("{name}: expected a constructor, got {}", other.kind_name()),
                span,
            ),
        };
        let encoding = match ctor("format")?.as_str() {
            "Texture.Rgba8" => PixelEncoding::Rgba8,
            "Texture.Rgb8" => PixelEncoding::Rgb8,
            "Texture.Rgb555le" => PixelEncoding::Rgb555le,
            other => return fail(format!("format: unknown format {other}"), span),
        };
        let color_space = match ctor("colorSpace")?.as_str() {
            "Texture.Srgb" => ColorSpace::Srgb,
            "Texture.Linear" => ColorSpace::Linear,
            other => return fail(format!("colorSpace: unknown colour space {other}"), span),
        };
        Ok(PixelsArg {
            width: number("width")?,
            height: number("height")?,
            encoding,
            color_space,
            pixels: crate::host_registry::FromArg::from_arg(
                get("pixels")?,
                &format!("{PATH}: pixels"),
                span,
            )?,
        })
    }
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn1(PATH, "Texture.fromPixels(pixels)", |p: PixelsArg| {
        let build = || {
            let (w, h) = (side(p.width, "width")?, side(p.height, "height")?);
            PixelData::new(w, h, p.encoding, p.color_space, p.pixels.0.as_slice())
        };
        build()
            .map(|data| FunctorLangTexture(TextureDescription::Data(Arc::new(data))))
            .map_err(|e| format!("{PATH}: {e}"))
    });
}
