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
use functor_runtime_common::manifest::{self, ManifestInput, ModelEntry};

/// The generated module's filename (also skipped when scanning).
const ASSETS_FILE: &str = "assets.fun";

/// The scanned asset files of a project directory, per kind, each sorted by
/// name so scan order (and any identifier disambiguation) is deterministic
/// across filesystems. Non-recursive by design: subdirectories (e.g. the
/// golden-image folders) are not assets.
struct ScannedAssets {
    models: Vec<PathBuf>,
    textures: Vec<PathBuf>,
    sounds: Vec<PathBuf>,
}

impl ScannedAssets {
    fn is_empty(&self) -> bool {
        self.models.is_empty() && self.textures.is_empty() && self.sounds.is_empty()
    }

    fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.models
            .iter()
            .chain(self.textures.iter())
            .chain(self.sounds.iter())
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

#[derive(Clone, Copy, PartialEq)]
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
    };
    for entry in fs::read_dir(dir)? {
        // One unreadable entry shouldn't sink the scan — skip it and keep
        // going (opening the directory itself failing still propagates).
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_file() {
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
    Ok(scanned)
}

/// Scan `dir`, inspect its models, and write (or remove) `assets.fun`.
pub fn execute(dir: &Path) -> io::Result<()> {
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
        return Ok(());
    }

    let mut input = ManifestInput::default();
    for path in &scanned.models {
        let Some(file) = file_name(path) else { continue };
        let bytes = fs::read(path)?;
        let clips: Vec<(String, f32)> = match inspect_model(bytes, None, None) {
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
        if manifest::shadows_clip_type(&clips) {
            emit(Event::Warning {
                message: format!(
                    "{file}: its clip names are exactly `name`/`duration`, which collide \
with the generated Clip record's fields — importing without clip constants"
                ),
            });
        } else if !clips.is_empty() {
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
        input.models.push(ModelEntry { file, clips });
    }
    input.textures = scanned.textures.iter().filter_map(|p| file_name(p)).collect();
    input.sounds = scanned.sounds.iter().filter_map(|p| file_name(p)).collect();

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
    Ok(())
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
        execute(dir)?;
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
