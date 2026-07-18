//! `functor import` — typed asset-manifest generation (Track B.2).
//!
//! Scans the project directory for assets — models (`*.glb` / `*.gltf`),
//! textures (`*.png` / `*.jpg` / `*.jpeg` / `*.hdr`), sounds (`*.wav` /
//! `*.ogg` / `*.mp3`) — inspects models headlessly for animation clips (see
//! [`functor_runtime_common::inspect`] — no GL context), and writes one
//! generated sibling module, `assets.fun`, of branded asset constants:
//!
//! ```functor
//! let xbot = Asset.model("Xbot.glb")
//! let xbotClips = { walk: { name: "walk", duration: 0.9667 }, ... }
//! ```
//!
//! `file = module`, so games write `Scene.model(Assets.xbot)` and
//! `Anim.clip(Assets.xbotClips.walk.name, tts)` — a typo is a check-time
//! error instead of a silent fallback. The file is meant to be CHECKED IN (it
//! typechecks without the binary assets, which are fetched, not committed);
//! `run`/`build` call [`ensure_fresh`] to regenerate it automatically when
//! the project's assets change.
//!
//! The pure generator (layout, identifier sanitization, determinism) lives in
//! [`functor_runtime_common::manifest`] — shared so future tooling (the wasm
//! build the browser IDE will use) emits byte-identical text. This module
//! owns scanning, model inspection, warnings, and file IO.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::output::{emit, Event};
use functor_runtime_common::inspect::inspect_model;
use functor_runtime_common::manifest::{self, AssetEntry, ManifestInput, ModelEntry};

/// The generated module's filename (also skipped when scanning).
const ASSETS_FILE: &str = "assets.fun";

/// Sidecar declarations: `<name>.asset.json` declares (or, later, configures)
/// asset `<name>` — today the schema is `{ "kind"?, "url"? }`, the remote
/// (CDN) locator seam. See the asset-handling design's §2g.
const SIDECAR_SUFFIX: &str = ".asset.json";

/// The scanned asset files of a project directory, per kind, each sorted by
/// name so scan order (and any identifier disambiguation) is deterministic
/// across filesystems. Non-recursive by design: subdirectories (e.g. the
/// golden-image folders) are not assets.
struct ScannedAssets {
    models: Vec<PathBuf>,
    textures: Vec<PathBuf>,
    sounds: Vec<PathBuf>,
    /// `<name>.asset.json` sidecar files — declarations, not assets, but they
    /// join the inventory and mtime checks (editing one must re-import).
    sidecars: Vec<PathBuf>,
}

impl ScannedAssets {
    fn is_empty(&self) -> bool {
        self.models.is_empty()
            && self.textures.is_empty()
            && self.sounds.is_empty()
            && self.sidecars.is_empty()
    }

    fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.models
            .iter()
            .chain(self.textures.iter())
            .chain(self.sounds.iter())
            .chain(self.sidecars.iter())
    }

    /// The stems of every LOCAL asset file — for detecting a sidecar whose
    /// `url` conflicts with a same-named local file.
    fn local_stems(&self) -> Vec<String> {
        self.models
            .iter()
            .chain(self.textures.iter())
            .chain(self.sounds.iter())
            .filter_map(|p| file_name(p))
            .map(|f| stem(&f).to_string())
            .collect()
    }

    /// Every scanned file name, sorted — the shape of the manifest's
    /// `// files:` inventory line.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.paths().filter_map(|p| file_name(p)).collect();
        names.sort_unstable();
        names
    }
}

/// The asset kind an extension maps to, if any (case-insensitive).
fn kind_of_extension(ext: &str) -> Option<Kind> {
    let ext = ext.to_ascii_lowercase();
    match ext.as_str() {
        "glb" | "gltf" => Some(Kind::Model),
        "png" | "jpg" | "jpeg" | "hdr" => Some(Kind::Texture),
        "wav" | "ogg" | "mp3" => Some(Kind::Sound),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Model,
    Texture,
    Sound,
}

fn scan(dir: &Path) -> io::Result<ScannedAssets> {
    let mut scanned = ScannedAssets {
        models: Vec::new(),
        textures: Vec::new(),
        sounds: Vec::new(),
        sidecars: Vec::new(),
    };
    for entry in fs::read_dir(dir)? {
        // One unreadable entry shouldn't sink the scan — skip it and keep
        // going (opening the directory itself failing still propagates).
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if file_name(&path).is_some_and(|f| f.ends_with(SIDECAR_SUFFIX) && f != SIDECAR_SUFFIX) {
            scanned.sidecars.push(path);
            continue;
        }
        let Some(kind) = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(kind_of_extension)
        else {
            continue;
        };
        match kind {
            Kind::Model => scanned.models.push(path),
            Kind::Texture => scanned.textures.push(path),
            Kind::Sound => scanned.sounds.push(path),
        }
    }
    scanned.models.sort();
    scanned.textures.sort();
    scanned.sounds.sort();
    scanned.sidecars.sort();
    Ok(scanned)
}

/// A parsed sidecar declaration. `kind` may be omitted when the url's
/// extension infers it.
#[derive(Debug)]
struct SidecarDecl {
    kind: Option<Kind>,
    url: Option<String>,
}

/// Parse a sidecar's JSON. Errors are load-stoppers for THIS sidecar (bad
/// JSON, a wrong-typed field, an unknown `kind`); unknown KEYS come back as
/// warnings — silently ignoring a typo'd `"ur"` would defeat the point of
/// the typed track.
fn parse_sidecar(source: &str) -> Result<(SidecarDecl, Vec<String>), String> {
    let json: serde_json::Value =
        serde_json::from_str(source).map_err(|e| format!("invalid JSON: {e}"))?;
    let obj = json
        .as_object()
        .ok_or_else(|| "expected a JSON object".to_string())?;
    let mut warnings = Vec::new();
    let mut decl = SidecarDecl {
        kind: None,
        url: None,
    };
    for (key, value) in obj {
        match key.as_str() {
            "kind" => {
                let kind = value
                    .as_str()
                    .ok_or_else(|| "\"kind\" must be a string".to_string())?;
                decl.kind = Some(match kind {
                    "model" => Kind::Model,
                    "texture" => Kind::Texture,
                    "sound" => Kind::Sound,
                    other => {
                        return Err(format!(
                            "unknown \"kind\": \"{other}\" — expected \"model\", \"texture\", \
or \"sound\""
                        ))
                    }
                });
            }
            "url" => {
                let url = value
                    .as_str()
                    .ok_or_else(|| "\"url\" must be a string".to_string())?;
                // The runtime treats exactly http(s) as remote (io::is_remote_path);
                // anything else would land in the manifest as a nonexistent
                // local path.
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return Err(format!(
                        "\"url\" must be an http:// or https:// locator, got \"{url}\""
                    ));
                }
                decl.url = Some(url.to_string());
            }
            other => warnings.push(format!(
                "unknown key \"{other}\" (known: \"kind\", \"url\") — ignored"
            )),
        }
    }
    // An explicit kind CONTRADICTING a recognized url extension would brand
    // an asset the runtime can't load as declared ("kind": "model" with a
    // .png url) — reject rather than trust either side.
    if let (Some(kind), Some(url)) = (decl.kind, decl.url.as_deref()) {
        if let Some(inferred) = kind_of_url(url) {
            if inferred != kind {
                return Err(format!(
                    "\"kind\" says {:?} but the url's extension says {:?} — remove the \
\"kind\" or fix the url",
                    kind, inferred
                ));
            }
        }
    }
    Ok((decl, warnings))
}

/// Infer an asset kind from a URL's path extension (query/fragment stripped),
/// for sidecars that omit `kind`.
fn kind_of_url(url: &str) -> Option<Kind> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let ext = path.rsplit('/').next()?.rsplit_once('.')?.1;
    kind_of_extension(ext)
}

/// The file name without its final extension (`"Xbot.glb"` → `"Xbot"`).
fn stem(file: &str) -> &str {
    file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file)
}

/// A blocking HTTP GET run on a dedicated thread — the CLI executes inside
/// tokio's async context, where constructing reqwest's blocking client
/// panics. Non-2xx is an error (the desktop fetcher's rule); the timeout is
/// the caller's — explicit `import` affords the desktop's 300s for a big
/// model, while auto-reimport keeps it short so an unreachable host can't
/// stall every `develop` launch (the fast-inner-loop rule).
fn download_blocking(url: &str, timeout: std::time::Duration) -> Result<Vec<u8>, String> {
    let url = url.to_string();
    std::thread::spawn(move || -> Result<Vec<u8>, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client.get(&url).send().map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status().as_u16()));
        }
        resp.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string())
    })
    .join()
    .map_err(|_| "download thread panicked".to_string())?
}

/// Scan `dir`, inspect its models (local and sidecar-declared remote), and
/// write (or remove) `assets.fun`.
pub fn execute(dir: &Path) -> io::Result<()> {
    // Explicit import: a remote failure degrades that asset (warn, no clips)
    // rather than blocking the whole command — the user sees the warning.
    let _ = execute_inner(dir, false)?;
    Ok(())
}

/// The import body. With `strict_remote`, a FAILED remote fetch aborts the
/// whole regeneration (returning `false`, nothing written) — the auto-reimport
/// path must never strip a remote model's clip constants just because the
/// machine is offline; the existing manifest stays. Inspect failures on
/// successfully fetched bytes degrade to no-clips in both modes.
fn execute_inner(dir: &Path, strict_remote: bool) -> io::Result<bool> {
    let scanned = scan(dir)?;
    if scanned.is_empty() {
        emit(Event::Info {
            message: format!(
                "no assets (models/textures/sounds) in {} — nothing to generate \
(sample assets are fetched, not committed: `npm run fetch:assets`)",
                dir.display()
            ),
        });
        remove_stale(dir)?;
        return Ok(true);
    }

    let mut input = ManifestInput::default();
    input.files = scanned.names();
    for path in &scanned.models {
        let Some(file) = file_name(path) else { continue };
        let bytes = fs::read(path)?;
        let clips = match inspect_model(bytes, None, None) {
            Ok(report) => report
                .animations
                .iter()
                .map(|a| (a.name.clone(), a.duration))
                .collect(),
            // A model the inspector can't read (corrupt, or a .gltf with
            // external buffers) still gets its asset constant — the reference
            // is real even when the clips are unknowable. Warn + no clips.
            Err(e) => {
                emit(Event::Warning {
                    message: format!("{file}: cannot inspect for clips ({e}) — importing without clip constants"),
                });
                Vec::new()
            }
        };
        let clips = vetted_clips(&file, clips);
        input.models.push(ModelEntry {
            name: stem(&file).to_string(),
            locator: file,
            clips,
        });
    }
    input.textures = local_entries(&scanned.textures);
    input.sounds = local_entries(&scanned.sounds);

    // Sidecar declarations: today's schema is remote (CDN) locators; a
    // sidecar next to a same-named local file is its (future) config seat.
    let local_stems = scanned.local_stems();
    for path in &scanned.sidecars {
        let Some(file) = file_name(path) else { continue };
        let name = file
            .strip_suffix(SIDECAR_SUFFIX)
            .expect("scan collected by suffix")
            .to_string();
        let source = fs::read_to_string(path)?;
        let (decl, warnings) = match parse_sidecar(&source) {
            Ok(parsed) => parsed,
            Err(e) => return sidecar_schema_error(strict_remote, &file, &e),
        };
        for warning in warnings {
            emit(Event::Warning {
                message: format!("{file}: {warning}"),
            });
        }
        let Some(url) = decl.url else {
            if !local_stems.contains(&name) {
                return sidecar_schema_error(
                    strict_remote,
                    &file,
                    &format!(
                        "no \"url\" and no local asset named \"{name}\" — the sidecar \
declares nothing"
                    ),
                );
            }
            // A url-less sidecar beside its local file: the (future)
            // per-asset config seat — nothing to declare today.
            continue;
        };
        if local_stems.contains(&name) {
            // Safe to continue in both modes: the local file emits the same
            // constant name, so no `Assets.*` reference is lost.
            emit(Event::Warning {
                message: format!(
                    "{file}: a local asset named \"{name}\" exists, so it wins over the \
\"url\" — remove one side to make the declaration unambiguous"
                ),
            });
            continue;
        }
        let Some(kind) = decl.kind.or_else(|| kind_of_url(&url)) else {
            return sidecar_schema_error(
                strict_remote,
                &file,
                &format!(
                    "cannot infer the asset kind from \"{url}\" — add \
\"kind\": \"model\" | \"texture\" | \"sound\""
                ),
            );
        };
        match kind {
            Kind::Model => {
                // Auto-reimport keeps the download short: an unreachable
                // host must not stall every `develop` launch. Explicit
                // import affords the desktop fetcher's 300s for big models.
                let timeout = std::time::Duration::from_secs(if strict_remote {
                    30
                } else {
                    300
                });
                // The validator guards the disk cache for extensionless
                // urls (verify_magic can only reject HTML there): a body
                // inspect_model can't read is an error, never cached.
                let clips = match functor_runtime_common::io::fetch_cached_blocking(
                    &url,
                    |u| download_blocking(u, timeout),
                    |bytes| {
                        inspect_model(bytes.to_vec(), None, None)
                            .map(|_| ())
                            .map_err(|e| format!("not a readable model: {e}"))
                    },
                ) {
                    Ok(bytes) => match inspect_model(bytes, None, None) {
                        Ok(report) => report
                            .animations
                            .iter()
                            .map(|a| (a.name.clone(), a.duration))
                            .collect(),
                        Err(e) => {
                            emit(Event::Warning {
                                message: format!(
                                    "{file}: cannot inspect {url} for clips ({e}) — \
importing without clip constants"
                                ),
                            });
                            Vec::new()
                        }
                    },
                    Err(e) if strict_remote => {
                        emit(Event::Warning {
                            message: format!(
                                "cannot fetch {url} ({e}) — keeping the existing \
assets.fun (rerun `functor import` to regenerate)"
                            ),
                        });
                        return Ok(false);
                    }
                    Err(e) => {
                        emit(Event::Warning {
                            message: format!(
                                "{file}: cannot fetch {url} ({e}) — importing without \
clip constants"
                            ),
                        });
                        Vec::new()
                    }
                };
                let clips = vetted_clips(&file, clips);
                input.models.push(ModelEntry {
                    name,
                    locator: url,
                    clips,
                });
            }
            Kind::Texture => input.textures.push(AssetEntry { name, locator: url }),
            Kind::Sound => input.sounds.push(AssetEntry { name, locator: url }),
        }
    }

    match manifest::generate(&input) {
        Some(source) => {
            let out = dir.join(ASSETS_FILE);
            // Write-then-rename so an interrupted run (or disk-full) can't
            // leave a truncated manifest where a valid checked-in one was.
            let tmp = dir.join(format!("{ASSETS_FILE}.tmp"));
            fs::write(&tmp, source)?;
            fs::rename(&tmp, &out)?;
            emit(Event::Info {
                message: format!(
                    "wrote {} ({} asset(s)) — reference them as Assets.<name> \
(clips: Assets.<name>Clips.<clip>)",
                    out.display(),
                    input.models.len() + input.textures.len() + input.sounds.len(),
                ),
            });
        }
        None => remove_stale(dir)?,
    }
    Ok(true)
}

/// A sidecar problem the manifest can't absorb (bad JSON, an unknown kind, a
/// declaration that names nothing): explicit `functor import` FAILS the
/// command — never write a silently-reduced manifest that strips a constant
/// games reference — and auto-reimport keeps the existing manifest.
fn sidecar_schema_error(strict: bool, file: &str, msg: &str) -> io::Result<bool> {
    if strict {
        emit(Event::Warning {
            message: format!(
                "{file}: {msg} — keeping the existing assets.fun (fix the sidecar, then \
rerun `functor import`)"
            ),
        });
        Ok(false)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{file}: {msg}"),
        ))
    }
}

/// Warn about (and strip) clip sets the generator can't emit: duplicates keep
/// only the first, and a `{name, duration}` field set shadows the `Clip` type
/// itself. Shared by the local and sidecar model paths.
fn vetted_clips(file: &str, clips: Vec<(String, f32)>) -> Vec<(String, f32)> {
    if manifest::shadows_clip_type(&clips) {
        emit(Event::Warning {
            message: format!(
                "{file}: its clip names are exactly `name`/`duration`, which collide \
with the generated Clip record's fields — importing without clip constants"
            ),
        });
        return Vec::new();
    }
    if !clips.is_empty() {
        let dups = manifest::duplicate_clip_names(&clips);
        if !dups.is_empty() {
            emit(Event::Warning {
                message: format!(
                    "{file}: duplicate clip name(s) {} — Anim.clip plays the first \
match, so only the first of each is generated",
                    dups.join(", ")
                ),
            });
        }
        emit(Event::Info {
            message: format!("{file}: {} clip(s)", clips.len()),
        });
    }
    clips
}

/// Manifest entries for local (on-disk) texture/sound files.
fn local_entries(paths: &[PathBuf]) -> Vec<AssetEntry> {
    paths
        .iter()
        .filter_map(|p| file_name(p))
        .map(|file| AssetEntry {
            name: stem(&file).to_string(),
            locator: file,
        })
        .collect()
}

/// Regenerate a stale GENERATED manifest before `run`/`build`: when an asset
/// file is newer than `assets.fun`, or a scanned file is missing from the
/// manifest's `// files:` inventory (an ADDED asset — a plain mtime check can
/// miss one copied with an old timestamp). Projects without a generated
/// manifest are untouched — running `functor import` once is the opt-in; a
/// hand-written `assets.fun` is never overwritten.
///
/// Deliberately NEVER triggered by missing files or a missing/unparseable
/// inventory (the pre-B.2 format): sample models are gitignored and fetched,
/// so on an unfetched clone (or CI) the listed assets are legitimately
/// absent — regenerating there would DELETE their constants and break every
/// `Assets.*` reference. Removing an asset's constant, and migrating a
/// legacy manifest, stay an explicit `functor import`. Failures here must
/// not block the build, so the caller treats errors as warnings.
pub fn ensure_fresh(dir: &Path) -> io::Result<()> {
    let out = dir.join(ASSETS_FILE);
    let Ok(existing) = fs::read_to_string(&out) else {
        return Ok(());
    };
    if !manifest::is_generated(&existing) {
        return Ok(());
    }
    let scanned = scan(dir)?;
    let manifest_mtime = fs::metadata(&out)?.modified()?;
    // An unreadable mtime counts as newer: regenerating is cheap and safe.
    let any_newer = scanned.paths().any(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .map(|t| t > manifest_mtime)
            .unwrap_or(true)
    });
    if is_stale(
        manifest::listed_files(&existing).as_deref(),
        &scanned.names(),
        any_newer,
    ) {
        emit(Event::Info {
            message: "assets changed — regenerating assets.fun (functor import)".to_string(),
        });
        // strict_remote: an offline machine must not strip a remote model's
        // clip constants — a failed fetch keeps the existing manifest.
        let _ = execute_inner(dir, true)?;
    }
    Ok(())
}

/// The auto-reimport decision (pure for testability): regenerate for
/// additions and newer files; never when nothing is on disk (unfetched
/// assets), never for missing-only differences, and never for a generated
/// manifest WITHOUT a parseable `// files:` inventory (the pre-B.2 format,
/// or a corrupted line) — without the inventory we can't tell which absent
/// assets the manifest still speaks for, and regenerating on a partially
/// fetched clone would delete their constants. Explicit `functor import`
/// migrates legacy manifests.
fn is_stale(listed: Option<&[String]>, scanned: &[String], any_newer: bool) -> bool {
    if scanned.is_empty() {
        return false;
    }
    match listed {
        None => false,
        Some(listed) => {
            // A listed file missing from disk (unfetched assets) disables
            // auto-reimport entirely — regenerating would lose its constant.
            if listed.iter().any(|n| !scanned.contains(n)) {
                return false;
            }
            any_newer || scanned.iter().any(|n| !listed.contains(n))
        }
    }
}

/// Remove a previously GENERATED `assets.fun` so stale constants for
/// since-removed assets can't linger (a hand-written file without the marker
/// is left alone).
fn remove_stale(dir: &Path) -> io::Result<()> {
    let out = dir.join(ASSETS_FILE);
    let previously_generated = fs::read_to_string(&out)
        .map(|s| manifest::is_generated(&s))
        .unwrap_or(false);
    if previously_generated {
        fs::remove_file(&out)?;
        emit(Event::Info {
            message: format!("no assets to import — removed stale {}", out.display()),
        });
    }
    Ok(())
}

/// The file's basename as a String (`None` for non-UTF-8 names, which are
/// skipped — a manifest constant needs a printable path).
fn file_name(path: &Path) -> Option<String> {
    path.file_name().and_then(|s| s.to_str()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_map_to_kinds_case_insensitively() {
        assert!(matches!(kind_of_extension("GLB"), Some(Kind::Model)));
        assert!(matches!(kind_of_extension("gltf"), Some(Kind::Model)));
        assert!(matches!(kind_of_extension("PNG"), Some(Kind::Texture)));
        assert!(matches!(kind_of_extension("jpeg"), Some(Kind::Texture)));
        assert!(matches!(kind_of_extension("hdr"), Some(Kind::Texture)));
        assert!(matches!(kind_of_extension("ogg"), Some(Kind::Sound)));
        assert!(matches!(kind_of_extension("WAV"), Some(Kind::Sound)));
        // Buffer files and unrelated extensions are not standalone assets.
        assert_eq!(kind_of_extension("bin").is_some(), false);
        assert_eq!(kind_of_extension("fun").is_some(), false);
        assert_eq!(kind_of_extension("json").is_some(), false);
    }

    #[test]
    fn sidecars_parse_with_teaching_errors_and_unknown_key_warnings() {
        // Happy path: url + inferred kind.
        let (decl, warnings) =
            parse_sidecar(r#"{ "url": "https://cdn.example.com/shark.glb" }"#).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(decl.url.as_deref(), Some("https://cdn.example.com/shark.glb"));
        assert!(decl.kind.is_none());

        // Explicit kind.
        let (decl, _) = parse_sidecar(r#"{ "kind": "sound", "url": "https://x/api/boom" }"#)
            .unwrap();
        assert!(matches!(decl.kind, Some(Kind::Sound)));

        // Unknown keys warn (a typo'd "ur" must not be silent).
        let (decl, warnings) =
            parse_sidecar(r#"{ "ur": "https://x/a.glb", "scale": 0.5 }"#).unwrap();
        assert!(decl.url.is_none());
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("\"scale\"") || warnings[1].contains("\"scale\""));

        // Wrong-typed and unknown-vocabulary fields are errors, not silence.
        assert!(parse_sidecar(r#"{ "url": 42 }"#).is_err());
        assert!(parse_sidecar(r#"{ "kind": "modle" }"#)
            .unwrap_err()
            .contains("expected \"model\""));
        assert!(parse_sidecar("[]").is_err());
        assert!(parse_sidecar("not json").is_err());

        // Only http(s) locators: anything else would land in the manifest
        // as a nonexistent local path (io::is_remote_path is http/https).
        assert!(parse_sidecar(r#"{ "url": "file:///tmp/a.glb" }"#)
            .unwrap_err()
            .contains("http"));
        assert!(parse_sidecar(r#"{ "url": "shark.glb" }"#).is_err());
        assert!(parse_sidecar(r#"{ "url": "" }"#).is_err());

        // An explicit kind contradicting a recognized url extension is an
        // error; agreeing or unrecognized-extension cases pass.
        assert!(parse_sidecar(r#"{ "kind": "model", "url": "https://x/wood.png" }"#)
            .unwrap_err()
            .contains("extension"));
        assert!(parse_sidecar(r#"{ "kind": "texture", "url": "https://x/wood.png" }"#).is_ok());
        assert!(parse_sidecar(r#"{ "kind": "model", "url": "https://x/api/asset/9" }"#).is_ok());
    }

    #[test]
    fn url_kinds_infer_from_the_path_extension() {
        assert!(matches!(
            kind_of_url("https://cdn.x.com/meshes/shark.glb"),
            Some(Kind::Model)
        ));
        assert!(matches!(
            kind_of_url("https://cdn.x.com/a/wood.PNG?v=2#frag"),
            Some(Kind::Texture)
        ));
        assert!(matches!(
            kind_of_url("https://cdn.x.com/boom.ogg"),
            Some(Kind::Sound)
        ));
        // No extension / unknown extension -> None (sidecar must say "kind").
        assert!(kind_of_url("https://cdn.x.com/api/asset/123").is_none());
        assert!(kind_of_url("https://cdn.x.com/file.zip").is_none());
    }

    #[test]
    fn staleness_regenerates_for_additions_and_newer_files_only() {
        let listed = vec!["Xbot.glb".to_string(), "wood.png".to_string()];
        let same = listed.clone();
        // Unchanged set, nothing newer -> fresh.
        assert!(!is_stale(Some(&listed), &same, false));
        // A newer file -> stale.
        assert!(is_stale(Some(&listed), &same, true));
        // An added file -> stale even with old mtimes (cp -p).
        let added = vec![
            "Xbot.glb".to_string(),
            "new.ogg".to_string(),
            "wood.png".to_string(),
        ];
        assert!(is_stale(Some(&listed), &added, false));
        // A missing listed file (unfetched clone / CI / partial fetch):
        // NEVER regenerate, even with newer files or additions alongside —
        // that would delete the absent assets' constants and break Assets.*
        // references. Renames therefore need an explicit `functor import`.
        let missing = vec!["wood.png".to_string()];
        assert!(!is_stale(Some(&listed), &missing, false));
        assert!(!is_stale(Some(&listed), &missing, true));
        let missing_plus_added = vec!["new.ogg".to_string(), "wood.png".to_string()];
        assert!(!is_stale(Some(&listed), &missing_plus_added, false));
        assert!(!is_stale(Some(&listed), &[], false));
        assert!(!is_stale(Some(&listed), &[], true));
        // Pre-B.2 generated format / corrupted inventory: no inventory means
        // no way to know which ABSENT assets the manifest speaks for, so
        // auto-reimport keeps its hands off — explicit `functor import`
        // migrates legacy manifests.
        assert!(!is_stale(None, &same, false));
        assert!(!is_stale(None, &same, true));
        assert!(!is_stale(None, &[], true));
    }
}
