//! `Bytes.*`, `File.bytes` and `Binary.*` — binary data for decoders written
//! in Functor Lang (see `bytes.funi`). Readers return the language's `Result`
//! (not an evaluation error) so a decoder can report or recover from short or
//! malformed data; `Binary.require` turns an error into a located one.

use std::rc::Rc;
use std::sync::Arc;

use super::*;

/// An immutable byte range; slices share the underlying storage.
#[derive(Clone)]
pub struct ByteView {
    data: Arc<[u8]>,
    start: usize,
    len: usize,
}

impl ByteView {
    pub fn new(data: Vec<u8>) -> ByteView {
        let len = data.len();
        ByteView {
            data: data.into(),
            start: 0,
            len,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data[self.start..self.start + self.len]
    }
}

/// [`ByteView`] as an opaque Functor Lang value.
pub struct FunctorLangBytes(pub ByteView);

impl HostData for FunctorLangBytes {
    fn type_name(&self) -> &'static str {
        "Bytes"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // Immutable data: safe to keep in a model across reloads.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

crate::host_returnable!(FunctorLangBytes);

/// The bytes of a file in the project (the entry file's folder). `api` and
/// `example` shape the errors: `Typeface.file("…"): use a path inside the
/// project, like "fonts/MyFont.ttf"`.
pub(super) fn read_project_file(api: &str, path: &str, example: &str) -> Result<Vec<u8>, String> {
    use crate::storage::ProjectReadError;
    let outside = || format!("{api}({path:?}): use a path inside the project, like {example:?}");
    // The same portable rule as project assets; the platform's reader keeps
    // symlinks from reaching outside too.
    crate::debug_protocol::validate_project_asset_path(path).map_err(|_| outside())?;
    let reader = crate::storage::project_reader().ok_or_else(|| {
        format!(
            "{api}({path:?}): project files load on desktop, in `functor export` and on Quest \
(`functor run vr`), not on the web yet"
        )
    })?;
    reader.read(path).map_err(|e| match e {
        ProjectReadError::Outside => outside(),
        ProjectReadError::Unreadable(e) => format!("{api}: can't read {path} in the project ({e})"),
    })
}

/// Whether `path` names a file in the project (false for paths outside it,
/// and on hosts that can't read project files).
fn project_file_exists(path: &str) -> bool {
    crate::debug_protocol::validate_project_asset_path(path).is_ok()
        && crate::storage::project_reader().is_some_and(|reader| reader.exists(path))
}

/// A `Binary.error` record at `offset` with an empty path.
fn error(offset: f64, message: String) -> Value {
    crate::input::record([
        ("offset", Value::Number(offset)),
        ("path", Value::List(Rc::new(Vec::new()))),
        ("message", Value::String(message.into())),
    ])
}

fn ok(value: Value) -> Value {
    Value::Variant {
        ctor: "Result.Ok".into(),
        args: Rc::new(vec![value]),
    }
}

fn err(offset: f64, message: String) -> Value {
    Value::Variant {
        ctor: "Result.Error".into(),
        args: Rc::new(vec![error(offset, message)]),
    }
}

/// An offset or length argument: any number, so a bad one (NaN, huge) becomes
/// a recoverable `Result.Error` in [`take`] rather than an evaluation error.
struct Count(f64);

impl crate::host_registry::FromArg for Count {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            Value::Number(n) => Ok(Count(*n)),
            other => Err(RunError {
                message: format!("expected a number, got {}", other.kind_name()),
                span,
            }),
        }
    }
}

/// `count` bytes at `offset`, or the `Result.Error` explaining why not.
fn take(bytes: &ByteView, offset: f64, count: f64) -> Result<&[u8], Value> {
    let whole = |x: f64| x >= 0.0 && x.fract() == 0.0 && x <= 9_007_199_254_740_992.0;
    if !whole(offset) || !whole(count) {
        return Err(err(
            offset,
            format!("offset {offset} and length {count} must be whole numbers ≥ 0"),
        ));
    }
    let (at, n) = (offset as usize, count as usize);
    let data = bytes.as_slice();
    if at > data.len() {
        return Err(err(offset, format!("past the end ({} bytes)", data.len())));
    }
    let left = data.len() - at;
    if left < n {
        return Err(err(offset, format!("needs {n} bytes, {left} left")));
    }
    Ok(&data[at..at + n])
}

/// The text of a `Binary.error`: `header.count[2] at byte 12: needs 4 bytes, 1 left`.
fn describe(error: &Value) -> Option<String> {
    let Value::Record(fields) = error else {
        return None;
    };
    let field = |name: &str| fields.iter().find(|(k, _)| k == name).map(|(_, v)| v);
    let (Some(Value::Number(offset)), Some(Value::List(path)), Some(Value::String(message))) =
        (field("offset"), field("path"), field("message"))
    else {
        return None;
    };
    let mut at = String::new();
    for segment in path.iter() {
        match segment {
            Value::Variant { ctor, args } if &**ctor == "Binary.Field" => {
                if let Some(Value::String(name)) = args.first() {
                    if !at.is_empty() {
                        at.push('.');
                    }
                    at.push_str(name);
                }
            }
            Value::Variant { ctor, args } if &**ctor == "Binary.Index" => {
                if let Some(Value::Number(i)) = args.first() {
                    at.push_str(&format!("[{i}]"));
                }
            }
            _ => {}
        }
    }
    let at = if at.is_empty() { at } else { at + " " };
    Some(format!("{at}at byte {offset}: {message}"))
}

/// Register a fixed-width number reader: `Bytes.u32le(offset, bytes)`.
macro_rules! reader {
    ($reg:ident, $name:literal, $ty:ty, $from:ident) => {
        $reg.fn2(
            concat!("Bytes.", $name),
            concat!("Bytes.", $name, "(offset, bytes)"),
            |Count(offset): Count, bytes: FunctorLangBytes| {
                const N: usize = std::mem::size_of::<$ty>();
                match take(&bytes.0, offset, N as f64) {
                    Ok(raw) => ok(Value::Number(
                        <$ty>::$from(raw.try_into().expect("N bytes")) as f64,
                    )),
                    Err(e) => e,
                }
            },
        );
    };
}

/// Register a bulk number writer: `Bytes.fromU16le([1.0, 2.0])`. Integers
/// must be whole and in range — encoding is the program's own data, so a bad
/// value is an error at the call, not a recoverable `Result`.
macro_rules! writer {
    ($reg:ident, $name:literal, $ty:ty, $to:ident, $int:literal) => {
        $reg.fn1(
            concat!("Bytes.", $name),
            concat!("Bytes.", $name, "(numbers)"),
            |numbers: Vec<Count>| {
                let mut out = Vec::with_capacity(numbers.len() * std::mem::size_of::<$ty>());
                for (i, Count(n)) in numbers.iter().enumerate() {
                    let fits = if $int {
                        n.fract() == 0.0 && *n >= <$ty>::MIN as f64 && *n <= <$ty>::MAX as f64
                    } else {
                        // Rounding may not overflow a finite number to infinity.
                        !n.is_finite() || ((*n as $ty) as f64).is_finite()
                    };
                    if !fits {
                        return Err(format!(
                            concat!(
                                "Bytes.",
                                $name,
                                ": number {} ({}) doesn't fit a ",
                                stringify!($ty)
                            ),
                            i, n
                        ));
                    }
                    out.extend((*n as $ty).$to());
                }
                Ok(FunctorLangBytes(ByteView::new(out)))
            },
        );
    };
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn1("File.bytes", "File.bytes(path)", |path: String| {
        read_project_file("File.bytes", &path, "data/model.bin")
            .map(|data| FunctorLangBytes(ByteView::new(data)))
    });
    reg.fn1("File.exists", "File.exists(path)", |path: String| {
        Value::Bool(project_file_exists(&path))
    });
    reg.fn1(
        "Bytes.length",
        "Bytes.length(bytes)",
        |bytes: FunctorLangBytes| Value::Number(bytes.0.len as f64),
    );
    reg.fn3(
        "Bytes.slice",
        "Bytes.slice(start, length, bytes)",
        |Count(start): Count, Count(length): Count, bytes: FunctorLangBytes| match take(
            &bytes.0, start, length,
        ) {
            Ok(_) => ok(Value::HostData(Rc::new(FunctorLangBytes(ByteView {
                data: bytes.0.data.clone(),
                start: bytes.0.start + start as usize,
                len: length as usize,
            })))),
            Err(e) => e,
        },
    );
    reg.fn2(
        "Bytes.equals",
        "Bytes.equals(a, b)",
        |a: FunctorLangBytes, b: FunctorLangBytes| Value::Bool(a.0.as_slice() == b.0.as_slice()),
    );
    reg.fn3(
        "Bytes.latin1",
        "Bytes.latin1(offset, length, bytes)",
        |Count(offset): Count, Count(length): Count, bytes: FunctorLangBytes| match take(
            &bytes.0, offset, length,
        ) {
            Ok(raw) => ok(Value::String(
                raw.iter().map(|&b| b as char).collect::<String>().into(),
            )),
            Err(e) => e,
        },
    );
    reg.fn3(
        "Bytes.utf8",
        "Bytes.utf8(offset, length, bytes)",
        |Count(offset): Count, Count(length): Count, bytes: FunctorLangBytes| match take(
            &bytes.0, offset, length,
        ) {
            Ok(raw) => match std::str::from_utf8(raw) {
                Ok(s) => ok(Value::String(s.into())),
                Err(e) => err(offset + e.valid_up_to() as f64, "invalid UTF-8".to_string()),
            },
            Err(e) => e,
        },
    );
    reg.fn1(
        "Bytes.concat",
        "Bytes.concat(parts)",
        |parts: Vec<FunctorLangBytes>| {
            let slices: Vec<&[u8]> = parts.iter().map(|p| p.0.as_slice()).collect();
            FunctorLangBytes(ByteView::new(slices.concat()))
        },
    );
    reg.fn1("Bytes.zeros", "Bytes.zeros(count)", |Count(n): Count| {
        if n >= 0.0 && n.fract() == 0.0 && n <= 1_000_000_000.0 {
            Ok(FunctorLangBytes(ByteView::new(vec![0; n as usize])))
        } else {
            Err(format!(
                "Bytes.zeros: {n} isn't a whole number of bytes ≥ 0"
            ))
        }
    });
    reg.fn1(
        "Binary.invalid",
        "Binary.invalid(message)",
        |message: String| Err::<Value, String>(message),
    );
    reg.fn1("Bytes.fromUtf8", "Bytes.fromUtf8(text)", |text: String| {
        FunctorLangBytes(ByteView::new(text.into_bytes()))
    });
    reg.fn1(
        "Bytes.fromLatin1",
        "Bytes.fromLatin1(text)",
        |text: String| {
            text.chars()
                .map(|c| u8::try_from(u32::from(c)).map_err(|_| c))
                .collect::<Result<Vec<u8>, char>>()
                .map(|raw| FunctorLangBytes(ByteView::new(raw)))
                .map_err(|c| format!("Bytes.fromLatin1: {c:?} isn't a Latin-1 character"))
        },
    );
    writer!(reg, "fromU8", u8, to_le_bytes, true);
    writer!(reg, "fromI8", i8, to_le_bytes, true);
    writer!(reg, "fromU16le", u16, to_le_bytes, true);
    writer!(reg, "fromU16be", u16, to_be_bytes, true);
    writer!(reg, "fromI16le", i16, to_le_bytes, true);
    writer!(reg, "fromI16be", i16, to_be_bytes, true);
    writer!(reg, "fromU32le", u32, to_le_bytes, true);
    writer!(reg, "fromU32be", u32, to_be_bytes, true);
    writer!(reg, "fromI32le", i32, to_le_bytes, true);
    writer!(reg, "fromI32be", i32, to_be_bytes, true);
    writer!(reg, "fromF32le", f32, to_le_bytes, false);
    writer!(reg, "fromF32be", f32, to_be_bytes, false);
    writer!(reg, "fromF64le", f64, to_le_bytes, false);
    writer!(reg, "fromF64be", f64, to_be_bytes, false);
    reader!(reg, "u8", u8, from_le_bytes);
    reader!(reg, "i8", i8, from_le_bytes);
    reader!(reg, "u16le", u16, from_le_bytes);
    reader!(reg, "u16be", u16, from_be_bytes);
    reader!(reg, "i16le", i16, from_le_bytes);
    reader!(reg, "i16be", i16, from_be_bytes);
    reader!(reg, "u32le", u32, from_le_bytes);
    reader!(reg, "u32be", u32, from_be_bytes);
    reader!(reg, "i32le", i32, from_le_bytes);
    reader!(reg, "i32be", i32, from_be_bytes);
    reader!(reg, "f32le", f32, from_le_bytes);
    reader!(reg, "f32be", f32, from_be_bytes);
    reader!(reg, "f64le", f64, from_le_bytes);
    reader!(reg, "f64be", f64, from_be_bytes);
    reg.fn1(
        "Binary.describe",
        "Binary.describe(error)",
        |error: Value| {
            describe(&error)
                .map(|s| Value::String(s.into()))
                .ok_or_else(|| "Binary.describe: expected a Binary.error".to_string())
        },
    );
    reg.fn1(
        "Binary.require",
        "Binary.require(result)",
        |result: Value| match &result {
            Value::Variant { ctor, args } if &**ctor == "Result.Ok" && args.len() == 1 => {
                Ok(args[0].clone())
            }
            Value::Variant { ctor, args } if &**ctor == "Result.Error" && args.len() == 1 => {
                Err(describe(&args[0])
                    .unwrap_or_else(|| "Binary.require: the read failed".to_string()))
            }
            _ => Err("Binary.require: expected a Result".to_string()),
        },
    );
}
