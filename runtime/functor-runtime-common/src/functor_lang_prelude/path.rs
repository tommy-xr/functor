//! `Path.*` — normalized project-relative paths, the only argument `File.*`
//! takes (see `path.funi`). [`resolve`] is the one rule: `Path.of`,
//! `Path.join` and `Typeface.file` all go through it, and the readers trust
//! its output.

use super::*;

/// A [`resolve`]d path: `/`-separated segments, none empty, `.` or `..`.
/// `""` is the project root.
#[derive(Clone)]
pub struct FunctorLangPath(pub Rc<str>);

impl HostData for FunctorLangPath {
    fn type_name(&self) -> &'static str {
        "Path"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // Immutable text: safe to keep in a model across reloads.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

crate::host_returnable!(FunctorLangPath);

/// A bare string where a path belongs teaches the wrapper, quoting it.
impl crate::host_registry::FromArg for FunctorLangPath {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        match value {
            Value::HostData(data) => {
                if let Some(p) = data.as_any().downcast_ref::<FunctorLangPath>() {
                    return Ok(p.clone());
                }
            }
            Value::String(s) => {
                return Err(RunError {
                    message: format!("{path}: expected a Path.t, got a string — wrap it: Path.of({s:?})"),
                    span,
                })
            }
            _ => {}
        }
        Err(RunError {
            message: format!("{path}: expected a Path.t, got {}", value.kind_name()),
            span,
        })
    }
}

/// `relative` resolved inside the normalized `base` (`""` = the project
/// root): `.` and empty segments drop out, `..` pops a segment but never past
/// the root. `Err` is the reason, for the caller to prefix with its call.
pub(super) fn resolve(base: &str, relative: &str) -> Result<String, &'static str> {
    if relative.is_empty() {
        return Err("a path can't be empty (\".\" is the project root)");
    }
    if relative.contains('\\') {
        return Err("use `/` between folders, not `\\`");
    }
    if relative.contains('\0') {
        return Err("a path can't hold a NUL byte");
    }
    if relative.contains("://") {
        return Err("a URL isn't a project file");
    }
    // `/x`, or a Windows drive (`C:/x`).
    let drive = relative.as_bytes().get(1) == Some(&b':')
        && relative.as_bytes()[0].is_ascii_alphabetic()
        && matches!(relative.as_bytes().get(2), None | Some(b'/'));
    if relative.starts_with('/') || drive {
        return Err("an absolute path — use one relative to the entry file's folder");
    }
    let mut segments: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for segment in relative.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err("`..` climbs out of the project — use a path inside the project");
                }
            }
            name => segments.push(name),
        }
    }
    let resolved = segments.join("/");
    if resolved.len() > crate::debug_protocol::MAX_ASSET_PATH_BYTES {
        return Err("the path is too long");
    }
    Ok(resolved)
}

/// The last segment; `""` for the root.
fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or("")
}

/// The extension of [`name`] without the dot; `""` for none, a dotfile, or a
/// trailing dot.
fn ext(path: &str) -> &str {
    match name(path).rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext,
        _ => "",
    }
}

/// The folder holding `path`; the root is its own parent.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn path_value(path: &str) -> FunctorLangPath {
    FunctorLangPath(path.into())
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn1("Path.of", "Path.of(text)", |text: String| {
        resolve("", &text)
            .map(|p| path_value(&p))
            .map_err(|reason| format!("Path.of({text:?}): {reason}"))
    });
    reg.fn2(
        "Path.join",
        "Path.join(relative, path)",
        |relative: String, path: FunctorLangPath| {
            resolve(&path.0, &relative)
                .map(|p| path_value(&p))
                .map_err(|reason| format!("Path.join({relative:?}): {reason}"))
        },
    );
    reg.fn1("Path.name", "Path.name(path)", |path: FunctorLangPath| {
        Value::String(name(&path.0).into())
    });
    reg.fn1("Path.ext", "Path.ext(path)", |path: FunctorLangPath| {
        Value::String(ext(&path.0).into())
    });
    reg.fn1("Path.parent", "Path.parent(path)", |path: FunctorLangPath| {
        path_value(parent(&path.0))
    });
    reg.fn1("Path.toString", "Path.toString(path)", |path: FunctorLangPath| {
        Value::String(if path.0.is_empty() { ".".into() } else { path.0.clone() })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_normalizes_and_rejects() {
        for (input, want) in [
            ("data/x.bin", "data/x.bin"),
            ("./data//x.bin", "data/x.bin"),
            ("data/", "data"),
            ("data/sub/../x.bin", "data/x.bin"),
            (".", ""),
            ("a/..", ""),
            ("C:x", "C:x"),
        ] {
            assert_eq!(resolve("", input).as_deref(), Ok(want), "{input}");
        }
        for (input, reason) in [
            ("", "empty"),
            ("/etc/passwd", "absolute"),
            ("C:/x", "absolute"),
            ("c:", "absolute"),
            ("data\\x.bin", "`/`"),
            ("../x.bin", "inside the project"),
            ("a/../../x", "inside the project"),
            ("https://x.com/a.bin", "URL"),
            ("a\0b", "NUL"),
        ] {
            let err = resolve("", input).unwrap_err();
            assert!(err.contains(reason), "{input:?}: {err}");
        }
        let long = "a/".repeat(crate::debug_protocol::MAX_ASSET_PATH_BYTES);
        assert!(resolve("", &long).unwrap_err().contains("too long"));
    }

    #[test]
    fn resolve_joins_against_a_base() {
        assert_eq!(resolve("ss2/mesh", "a.bin").as_deref(), Ok("ss2/mesh/a.bin"));
        assert_eq!(resolve("ss2/mesh", "sub/a.bin/").as_deref(), Ok("ss2/mesh/sub/a.bin"));
        assert_eq!(resolve("ss2/mesh", "../tex/a.png").as_deref(), Ok("ss2/tex/a.png"));
        assert_eq!(resolve("", "a.bin").as_deref(), Ok("a.bin"));
        assert!(resolve("ss2", "../../x").is_err());
        assert!(resolve("ss2", "/x").is_err());
        assert!(resolve("ss2", "").is_err());
    }

    #[test]
    fn name_ext_parent() {
        for (path, n, e, p) in [
            ("data/x.bin", "x.bin", "bin", "data"),
            ("a.tar.gz", "a.tar.gz", "gz", ""),
            ("data/.gitignore", ".gitignore", "", "data"),
            ("data/README", "README", "", "data"),
            ("data/file.", "file.", "", "data"),
            ("data/.config.json", ".config.json", "json", "data"),
            ("a/b/c", "c", "", "a/b"),
            ("", "", "", ""),
        ] {
            assert_eq!((name(path), ext(path), parent(path)), (n, e, p), "{path}");
        }
    }
}
