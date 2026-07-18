//! Build-time asset verification (Track B.3 of the asset revamp).
//!
//! `functor build` is the strict gate, so it PROVES the typed asset surface:
//! every literal-string `Asset.model/texture/sound(…)` argument in the
//! project (the generated manifest is just code, so it's covered too) is
//! checked —
//!
//! - a relative path must exist in the project dir (**error**: today a typo'd
//!   or missing file silently renders the invisible fallback at draw);
//! - a URL verifies against the remote-asset disk cache first (no network),
//!   then a HEAD request: provably absent (404/410) is an **error**, an
//!   unverifiable URL (offline, 403, HEAD-rejecting CDN) is a **warning** —
//!   an offline build must stay usable.
//!
//! Dynamic (non-literal) constructor args can't be checked — that's the
//! catalog pattern's data boundary, deliberately unlinted.
//!
//! Separately, bare-string asset args to the CONSUMERS (`Scene.model("x.glb")`,
//! `Effect.play`, `AudioSource.*` — the pre-manifest form the flag day will
//! retire) get a deprecation **warning** pointing at the manifest — but only
//! in projects that HAVE a generated `assets.fun`: a project that hasn't
//! opted into the typed world isn't nagged on every build.
//!
//! The walk is over the lowered IR (`Call { callee: External(path), args }`),
//! not raw tokens, so `Asset.model("x.glb")` (sanctioned constructor) and
//! `Scene.model("x.glb")` (deprecated coercion) are distinguished precisely,
//! and every finding carries the call site's span.

use std::collections::HashMap;
use std::path::Path;

use functor_lang::ir::{Expr, ExprKind, Module};
use functor_lang::Span;

/// One finding, anchored to the offending argument's span.
pub struct Finding {
    pub span: Span,
    pub message: String,
}

/// The pass's result: `errors` must fail the build; `warnings` must not.
#[derive(Default)]
pub struct AssetFindings {
    pub errors: Vec<Finding>,
    pub warnings: Vec<Finding>,
}

/// What a URL probe learned. Injectable so tests never touch the network.
pub enum UrlVerdict {
    /// Cache hit, or the server acknowledged the resource.
    Ok,
    /// Provably absent (HTTP 404/410) — carries the evidence for the message.
    Missing(String),
    /// Could not be verified (an auth wall, a HEAD-rejecting CDN) — carries
    /// why. Only this URL is affected; others are still probed.
    Unverifiable(String),
    /// The NETWORK is unreachable (transport error, not an HTTP status).
    /// Same warning severity as [`Self::Unverifiable`], but it also stops
    /// further probing: offline, every remaining probe would burn its
    /// timeout to learn the same thing.
    Offline(String),
}

/// Verify every literal asset locator in `module`. `probe_url` is called at
/// most once per distinct URL (results are memoized here); the production
/// prober is [`probe_url_live`].
pub fn verify_assets(
    module: &Module,
    project_dir: &Path,
    has_generated_manifest: bool,
    probe_url: &mut dyn FnMut(&str) -> UrlVerdict,
) -> AssetFindings {
    let mut findings = AssetFindings::default();
    // url -> (is_error, finding text or None for a clean verdict)
    let mut probed: HashMap<String, (bool, Option<String>)> = HashMap::new();
    let mut network_down = false;
    for def in &module.defs {
        walk(&def.value, &mut |expr| {
            let ExprKind::Call { callee, args } = &expr.kind else {
                return;
            };
            let ExprKind::External(path) = &callee.kind else {
                return;
            };
            let path = path.join(".");
            // The sanctioned constructors: verify the locator itself.
            if matches!(path.as_str(), "Asset.model" | "Asset.texture" | "Asset.sound") {
                let Some((locator, span)) = string_arg(args, 0) else {
                    return; // dynamic construction — the data boundary, unlinted
                };
                if locator.starts_with("http://") || locator.starts_with("https://") {
                    let (is_error, text) = probed
                        .entry(locator.to_string())
                        .or_insert_with(|| {
                            let verdict = if network_down {
                                UrlVerdict::Offline("network unavailable".to_string())
                            } else {
                                probe_url(locator)
                            };
                            match verdict {
                                UrlVerdict::Ok => (false, None),
                                UrlVerdict::Missing(why) => (
                                    true,
                                    Some(format!(
                                        "remote asset not found: {locator} ({why}) — it \
would load as the invisible fallback"
                                    )),
                                ),
                                UrlVerdict::Unverifiable(why) => (
                                    false,
                                    Some(format!(
                                        "cannot verify remote asset {locator} ({why}) — \
skipping the existence check"
                                    )),
                                ),
                                UrlVerdict::Offline(why) => {
                                    network_down = true;
                                    (
                                        false,
                                        Some(format!(
                                            "cannot verify remote asset {locator} ({why}) — \
skipping the existence check"
                                        )),
                                    )
                                }
                            }
                        })
                        .clone();
                    if let Some(text) = text {
                        let finding = Finding {
                            span,
                            message: text,
                        };
                        if is_error {
                            findings.errors.push(finding);
                        } else {
                            findings.warnings.push(finding);
                        }
                    }
                } else if !project_dir.join(locator).is_file() {
                    findings.errors.push(Finding {
                        span,
                        message: format!(
                            "asset file not found: \"{locator}\" — it would load as the \
invisible fallback. Fetched sample assets need `npm run fetch:assets`; if the file was \
renamed or removed, rerun `functor import`"
                        ),
                    });
                }
                return;
            }
            // The deprecated coercions: a bare path string where an Asset
            // value belongs. Only nag projects that have a manifest.
            if !has_generated_manifest {
                return;
            }
            let deprecated = match path.as_str() {
                "Scene.model" | "Effect.play" | "Effect.playAt" | "Effect.playThen" => {
                    string_arg(args, 0)
                }
                "AudioSource.ambient" | "AudioSource.at" => string_arg(args, 1),
                _ => None,
            };
            if let Some((s, span)) = deprecated {
                findings.warnings.push(Finding {
                    span,
                    message: format!(
                        "bare asset path \"{s}\" — reference the generated manifest \
(Assets.*) or construct an Asset.* value; the string form will become an error at the \
flag day"
                    ),
                });
            }
        });
    }
    findings
}

/// The `i`th argument when it is a string literal, with its span.
fn string_arg(args: &[Expr], i: usize) -> Option<(&str, Span)> {
    match args.get(i) {
        Some(Expr {
            kind: ExprKind::String(s),
            span,
            ..
        }) => Some((s.as_str(), *span)),
        _ => None,
    }
}

/// Depth-first walk over every sub-expression, `f` applied pre-order.
fn walk(expr: &Expr, f: &mut impl FnMut(&Expr)) {
    f(expr);
    match &expr.kind {
        ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Bool(_)
        | ExprKind::Local { .. }
        | ExprKind::Global(_)
        | ExprKind::External(_)
        | ExprKind::LocalMut { .. }
        | ExprKind::Ctor { .. } => {}
        ExprKind::Record(fields) => {
            for field in fields {
                walk(&field.value, f);
            }
        }
        ExprKind::RecordUpdate { base, fields } => {
            walk(base, f);
            for field in fields {
                walk(&field.value, f);
            }
        }
        ExprKind::List(items) | ExprKind::Tuple(items) => {
            for item in items {
                walk(item, f);
            }
        }
        ExprKind::ListCons { items, tail } => {
            for item in items {
                walk(item, f);
            }
            walk(tail, f);
        }
        ExprKind::Let { value, body, .. } => {
            walk(value, f);
            walk(body, f);
        }
        ExprKind::Assign { value, rest, .. } => {
            walk(value, f);
            walk(rest, f);
        }
        ExprKind::FieldAccess { object, .. } => walk(object, f),
        ExprKind::Lambda { body, .. } => walk(body, f),
        ExprKind::Call { callee, args } => {
            walk(callee, f);
            for arg in args {
                walk(arg, f);
            }
        }
        ExprKind::Binary { lhs, rhs, .. } | ExprKind::Logical { lhs, rhs, .. } => {
            walk(lhs, f);
            walk(rhs, f);
        }
        ExprKind::Neg(inner) | ExprKind::Not(inner) => walk(inner, f),
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            walk(cond, f);
            walk(then_branch, f);
            walk(else_branch, f);
        }
        ExprKind::Match { scrutinee, arms } => {
            walk(scrutinee, f);
            for arm in arms {
                walk(&arm.body, f);
            }
        }
    }
}

/// The production URL prober: the remote-asset disk cache first (no
/// network), then a HEAD request on a dedicated thread (the CLI runs inside
/// tokio, where reqwest's blocking client can't be constructed). Only a
/// definite 404/410 is `Missing` — auth walls, HEAD-rejecting CDNs, and
/// network failures are `Unverifiable` (a warning, not a broken offline
/// build).
pub fn probe_url_live(url: &str) -> UrlVerdict {
    if functor_runtime_common::io::remote_cache_hit(url) {
        return UrlVerdict::Ok;
    }
    let owned = url.to_string();
    let status = std::thread::spawn(move || -> Result<u16, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(client
            .head(&owned)
            .send()
            .map_err(|e| e.to_string())?
            .status()
            .as_u16())
    })
    .join()
    .map_err(|_| "probe thread panicked".to_string())
    .and_then(|r| r);
    match status {
        Ok(404) | Ok(410) => UrlVerdict::Missing(format!("HTTP {}", status.unwrap())),
        // 2xx only: reqwest follows redirects, so a TERMINAL 3xx (no
        // Location, 300 Multiple Choices) proves nothing about the asset.
        Ok(code) if (200..300).contains(&code) => UrlVerdict::Ok,
        Ok(code) => UrlVerdict::Unverifiable(format!("HTTP {code} from a HEAD request")),
        // A transport error (no HTTP status at all) means the NETWORK is the
        // problem — stop probing further urls this build.
        Err(e) => UrlVerdict::Offline(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(src: &str) -> functor_lang::project::Project {
        functor_lang::project::load_single_source("game", src)
            .unwrap_or_else(|e| panic!("load: {}", e.render()))
    }

    fn run(
        src: &str,
        dir: &Path,
        has_manifest: bool,
        probe: &mut dyn FnMut(&str) -> UrlVerdict,
    ) -> AssetFindings {
        verify_assets(&project(src).module, dir, has_manifest, probe)
    }

    fn no_probe(url: &str) -> UrlVerdict {
        panic!("unexpected probe of {url}")
    }

    #[test]
    fn missing_local_asset_is_an_error_and_present_is_not() {
        let dir = std::env::temp_dir().join(format!("asset-verify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("here.glb"), b"glTF").unwrap();

        let f = run(
            "let a = Asset.model(\"here.glb\")\nlet b = Asset.texture(\"gone.png\")",
            &dir,
            false,
            &mut no_probe,
        );
        assert_eq!(f.errors.len(), 1, "only the missing file errors");
        assert!(f.errors[0].message.contains("\"gone.png\""));
        assert!(f.errors[0].message.contains("fetch:assets"));
        assert!(f.warnings.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn url_verdicts_map_to_error_warning_ok_and_probe_once() {
        let dir = std::env::temp_dir();
        let mut probes: Vec<String> = Vec::new();
        let f = run(
            "let a = Asset.model(\"https://cdn/x.glb\")\n\
             let b = Asset.model(\"https://cdn/x.glb\")\n\
             let c = Asset.sound(\"https://cdn/gone.ogg\")\n\
             let d = Asset.texture(\"https://cdn/dark.png\")",
            &dir,
            false,
            &mut |url| {
                probes.push(url.to_string());
                match url {
                    "https://cdn/x.glb" => UrlVerdict::Ok,
                    "https://cdn/gone.ogg" => UrlVerdict::Missing("HTTP 404".to_string()),
                    _ => UrlVerdict::Unverifiable("offline".to_string()),
                }
            },
        );
        assert_eq!(probes.len(), 3, "each distinct url probed once: {probes:?}");
        assert_eq!(f.errors.len(), 1);
        assert!(f.errors[0].message.contains("gone.ogg"));
        assert!(f.errors[0].message.contains("HTTP 404"));
        assert_eq!(f.warnings.len(), 1);
        assert!(f.warnings[0].message.contains("cannot verify"));
    }

    #[test]
    fn offline_short_circuits_further_probes() {
        // The first transport-level failure stops probing: offline, N urls
        // must not burn N timeouts. Each still gets its own warning.
        let mut probes = 0;
        let f = run(
            "let a = Asset.model(\"https://cdn/a.glb\")\n\
             let b = Asset.model(\"https://cdn/b.glb\")\n\
             let c = Asset.model(\"https://cdn/c.glb\")",
            &std::env::temp_dir(),
            false,
            &mut |_| {
                probes += 1;
                UrlVerdict::Offline("connection refused".to_string())
            },
        );
        assert_eq!(probes, 1, "one real probe, then short-circuit");
        assert_eq!(f.warnings.len(), 3, "every url still gets its warning");
        assert!(f.errors.is_empty());
    }

    #[test]
    fn dynamic_locators_are_not_checked() {
        let f = run(
            "let make = (p) => Asset.model(p)\nlet a = make(\"anything.glb\")",
            &std::env::temp_dir(),
            false,
            &mut no_probe,
        );
        assert!(f.errors.is_empty());
        assert!(f.warnings.is_empty());
    }

    #[test]
    fn bare_string_consumers_warn_only_with_a_manifest() {
        let src = "let s = Scene.model(\"shark.glb\")\n\
                   let e = Effect.play(\"boom.ogg\")\n\
                   let v = AudioSource.ambient(\"bed\", \"wind.ogg\")\n\
                   let ok = Scene.model(Asset.model(\"https://cdn/x.glb\"))";
        let mut ok_probe = |_: &str| UrlVerdict::Ok;
        let without = run(src, &std::env::temp_dir(), false, &mut ok_probe);
        assert!(
            without.warnings.is_empty(),
            "no manifest -> no nagging: {:?}",
            without.warnings.iter().map(|w| &w.message).collect::<Vec<_>>()
        );
        let with = run(src, &std::env::temp_dir(), true, &mut ok_probe);
        assert_eq!(with.warnings.len(), 3, "one per bare-string consumer arg");
        assert!(with.warnings.iter().all(|w| w.message.contains("flag day")));
        // The AudioSource KEY ("bed") is identity, not an asset — the warning
        // must cite the SOUND arg.
        assert!(with.warnings[2].message.contains("wind.ogg"));
    }
}
