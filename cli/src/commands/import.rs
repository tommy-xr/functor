//! `functor import` — typed asset-name codegen.
//!
//! Scans the project directory for glTF models (`*.glb` / `*.gltf`), inspects
//! each headlessly (see [`functor_runtime_common::inspect`] — no GL context),
//! and writes one generated sibling module, `assets.fun`, holding a record of
//! typed clip constants per model:
//!
//! ```functor
//! let xbot = {
//!   walk: { name: "walk", duration: 0.9667 },
//!   ...
//! }
//! ```
//!
//! `file = module`, so games reference it as `Assets.xbot.walk.name` — a typo
//! is a check-time error instead of a silently-bind-posed `Anim.clip("wlak")`.
//! The file is meant to be CHECKED IN (it typechecks without the models, which
//! are fetched, not committed) and regenerated after the models change.

use std::fs;
use std::io;
use std::path::Path;

use crate::output::{emit, Event};
use functor_runtime_common::inspect::inspect_model;

/// The generated module's filename (also skipped when scanning).
const ASSETS_FILE: &str = "assets.fun";

/// One scanned model's contribution to the generated module: the file stem
/// and its `(clip name, duration seconds)` pairs.
pub type ModelClips = (String, Vec<(String, f32)>);

/// Scan `dir` for models, inspect them, and write `assets.fun`.
pub fn execute(dir: &Path) -> io::Result<()> {
    // Collect model files, sorted by name so the scan order (and any
    // disambiguation) is deterministic across filesystems.
    let mut model_files: Vec<std::path::PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some(ext) if ext.eq_ignore_ascii_case("glb") || ext.eq_ignore_ascii_case("gltf")
                )
        })
        .collect();
    model_files.sort();

    if model_files.is_empty() {
        emit(Event::Info {
            message: format!(
                "no models (*.glb / *.gltf) in {} — nothing to generate \
(sample assets are fetched, not committed: `npm run fetch:assets`)",
                dir.display()
            ),
        });
        return Ok(());
    }

    let mut models: Vec<ModelClips> = Vec::new();
    for path in &model_files {
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let bytes = fs::read(path)?;
        match inspect_model(bytes, None, None) {
            Ok(report) => {
                if report.animations.is_empty() {
                    emit(Event::Info {
                        message: format!("skipped {} (no animation clips)", display_name(path)),
                    });
                    continue;
                }
                let clips: Vec<(String, f32)> = report
                    .animations
                    .iter()
                    .map(|a| (a.name.clone(), a.duration))
                    .collect();
                if shadows_clip_type(&clips) {
                    emit(Event::Warning {
                        message: format!(
                            "skipped {}: its clip names are exactly `name`/`duration`, \
which collide with the generated Clip record's fields",
                            display_name(path)
                        ),
                    });
                    continue;
                }
                emit(Event::Info {
                    message: format!(
                        "{}: {} clip(s)",
                        display_name(path),
                        report.animations.len()
                    ),
                });
                models.push((stem, clips));
            }
            // A model the inspector can't read (corrupt, or a .gltf with
            // external buffers) shouldn't sink the whole import — warn + skip.
            Err(e) => emit(Event::Warning {
                message: format!("skipped {}: {}", display_name(path), e),
            }),
        }
    }

    match generate_assets_source(&models) {
        Some(source) => {
            let out = dir.join(ASSETS_FILE);
            fs::write(&out, source)?;
            emit(Event::Info {
                message: format!(
                    "wrote {} ({} model(s)) — reference clips as Assets.<model>.<clip>.name",
                    out.display(),
                    models.len()
                ),
            });
        }
        None => emit(Event::Info {
            message: format!(
                "no animation clips in {} model(s) — {} not written",
                model_files.len(),
                ASSETS_FILE
            ),
        }),
    }
    Ok(())
}

/// The model file's name relative to nothing fancy — just the basename, for
/// human-facing messages.
fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("<model>")
        .to_string()
}

/// The sanitized clip fields (`(field ident, clip name, duration)`) of one
/// model, sorted by original clip name with collisions disambiguated in that
/// order.
fn clip_fields(clips: &[(String, f32)]) -> Vec<(String, String, f32)> {
    let mut clips: Vec<&(String, f32)> = clips.iter().collect();
    clips.sort_by(|a, b| a.0.cmp(&b.0));
    let mut idents = UniqueIdents::new();
    clips
        .into_iter()
        .map(|(name, duration)| (idents.claim(name), name.clone(), *duration))
        .collect()
}

/// True when a model's sanitized clip fields are exactly `{name, duration}` —
/// the same field set as the `Clip` record itself. Record literals resolve
/// nominally by field NAMES, so such a model's record would collide with
/// `Clip` and fail the typecheck; the caller should skip it (and say so).
pub fn shadows_clip_type(clips: &[(String, f32)]) -> bool {
    let mut fields: Vec<String> = clip_fields(clips).into_iter().map(|(f, _, _)| f).collect();
    fields.sort();
    fields == ["duration", "name"]
}

/// The pure generator core: model `(stem, clips)` reports in, `assets.fun`
/// source out. Returns `None` when no model has any clips (nothing to write).
///
/// Emits DECLARED record types (`type Clip`, one `type <Model>Clips` per
/// distinct clip-field set) so a wrong clip name is a check-time error —
/// anonymous records typecheck gradually and would let typos through. Models
/// with the same clip fields share one type: two same-shaped declarations
/// would make the record literals ambiguous (a load-time error).
///
/// Deterministic by construction: models sort by stem, clips by name, and
/// sanitized-identifier collisions disambiguate in that sorted order — no
/// hash-map iteration order can leak into the generated text.
pub fn generate_assets_source(models: &[ModelClips]) -> Option<String> {
    let mut models: Vec<&ModelClips> = models
        .iter()
        .filter(|(_, clips)| !clips.is_empty() && !shadows_clip_type(clips))
        .collect();
    if models.is_empty() {
        return None;
    }
    models.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    out.push_str("// GENERATED by functor import — do not edit; rerun after changing models\n");
    out.push_str("// Typed clip constants: Assets.<model>.<clip>.name / .duration\n");
    out.push_str("\ntype Clip = { name: string, duration: float }\n");

    // One declared type per distinct field set, named after the first
    // (sorted) model that has it.
    let mut type_by_fields: std::collections::HashMap<Vec<String>, String> =
        std::collections::HashMap::new();
    let mut model_names = UniqueIdents::new();
    for (stem, clips) in models {
        let model_ident = model_names.claim(stem);
        let fields = clip_fields(clips);

        let field_names: Vec<String> = fields.iter().map(|(f, _, _)| f.clone()).collect();
        if !type_by_fields.contains_key(&field_names) {
            let type_name = format!("{}Clips", capitalize(&model_ident));
            out.push_str(&format!("\ntype {} = {{\n", type_name));
            for (field, _, _) in &fields {
                out.push_str(&format!("  {}: Clip,\n", field));
            }
            out.push_str("}\n");
            type_by_fields.insert(field_names, type_name);
        }

        out.push_str(&format!("\nlet {} = {{\n", model_ident));
        for (field, name, duration) in &fields {
            out.push_str(&format!(
                "  {}: {{ name: \"{}\", duration: {} }},\n",
                field,
                escape_string(name),
                format_float(*duration),
            ));
        }
        out.push_str("}\n");
    }
    Some(out)
}

/// Uppercase the first ASCII letter (model idents start lowercase after
/// sanitizing, so this is collision-free across distinct idents).
fn capitalize(s: &str) -> String {
    let mut out = s.to_string();
    if let Some(first) = out.chars().next() {
        if first.is_ascii_lowercase() {
            out.replace_range(..1, &first.to_ascii_uppercase().to_string());
        }
    }
    out
}

/// Words the lexer reserves — a sanitized name landing on one gets a `_`
/// suffix so the generated file still parses.
const KEYWORDS: &[&str] = &["let", "type", "true", "false", "mut", "with", "in", "match"];

/// Turn an arbitrary model stem / clip name into a Functor Lang identifier:
/// non-identifier chars (`-`, `.`, spaces, `:`, …) become `_`, the first
/// letter is lowercased (record fields and bindings read lowercase), a
/// leading digit / empty result gets a `_` prefix, and keywords get a `_`
/// suffix.
fn sanitize_ident(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if let Some(first) = s.chars().next() {
        if first.is_ascii_uppercase() {
            s.replace_range(..1, &first.to_ascii_lowercase().to_string());
        }
    }
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    if KEYWORDS.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

/// Sanitizes names and disambiguates collisions deterministically: the first
/// claimant keeps the sanitized name, later ones get `_2`, `_3`, … (skipping
/// suffixes that are themselves already taken).
struct UniqueIdents {
    used: std::collections::HashSet<String>,
}

impl UniqueIdents {
    fn new() -> Self {
        UniqueIdents {
            used: std::collections::HashSet::new(),
        }
    }

    fn claim(&mut self, raw: &str) -> String {
        let base = sanitize_ident(raw);
        let mut candidate = base.clone();
        let mut n = 2;
        while !self.used.insert(candidate.clone()) {
            candidate = format!("{}_{}", base, n);
            n += 1;
        }
        candidate
    }
}

/// Escape a clip name for a Functor Lang string literal (the language's
/// escapes: `\"` `\\` `\n` `\t`).
fn escape_string(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            '\t' => vec!['\\', 't'],
            c => vec![c],
        })
        .collect()
}

/// Format a duration as a Functor Lang float literal: fixed 4 decimals (the
/// inspector's display precision), trailing zeros trimmed but at least one
/// decimal kept — `2.24`, `0.9667`, `0.0`. Never scientific notation (the
/// lexer has no exponent form).
fn format_float(f: f32) -> String {
    let s = format!("{:.4}", f);
    let trimmed = s.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{}0", trimmed)
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_stems_and_clip_names() {
        assert_eq!(sanitize_ident("Xbot"), "xbot");
        assert_eq!(sanitize_ident("vr_glove_model"), "vr_glove_model");
        assert_eq!(sanitize_ident("sad_pose"), "sad_pose");
        assert_eq!(sanitize_ident("headShake"), "headShake");
        assert_eq!(sanitize_ident("my-model.v2"), "my_model_v2");
        assert_eq!(sanitize_ident("mixamo:rig clip"), "mixamo_rig_clip");
        assert_eq!(sanitize_ident("2fast"), "_2fast");
        assert_eq!(sanitize_ident(""), "_");
        assert_eq!(sanitize_ident("match"), "match_");
    }

    #[test]
    fn collisions_disambiguate_deterministically() {
        let models = vec![(
            "bot".to_string(),
            vec![
                ("walk".to_string(), 1.0),
                ("Walk".to_string(), 2.0),
                ("wa-lk".to_string(), 3.0),
            ],
        )];
        let src = generate_assets_source(&models).unwrap();
        // Clips sort by ORIGINAL name (Walk < wa-lk < walk), then collide on
        // the sanitized "walk"/"wa_lk": first claimant keeps the name.
        assert!(src.contains("  walk: { name: \"Walk\", duration: 2.0 },"));
        assert!(src.contains("  wa_lk: { name: \"wa-lk\", duration: 3.0 },"));
        assert!(src.contains("  walk_2: { name: \"walk\", duration: 1.0 },"));
    }

    #[test]
    fn zero_clip_models_are_skipped() {
        let models = vec![
            ("vr_glove_model".to_string(), vec![]),
            ("Xbot".to_string(), vec![("idle".to_string(), 2.24)]),
        ];
        let src = generate_assets_source(&models).unwrap();
        assert!(!src.contains("vr_glove_model"));
        assert!(src.contains("let xbot = {"));

        // All models clipless -> nothing to write at all.
        assert_eq!(
            generate_assets_source(&[("vr_glove_model".to_string(), vec![])]),
            None
        );
        assert_eq!(generate_assets_source(&[]), None);
    }

    #[test]
    fn output_is_sorted_and_stable() {
        // Feed models and clips in scrambled order; the output must sort both
        // (by stem / clip name), independent of input or hash order.
        let models = vec![
            (
                "zeta".to_string(),
                vec![("run".to_string(), 0.5), ("idle".to_string(), 1.5)],
            ),
            ("Alpha".to_string(), vec![("walk".to_string(), 0.9667)]),
        ];
        let src = generate_assets_source(&models).unwrap();
        let expected = "\
// GENERATED by functor import — do not edit; rerun after changing models
// Typed clip constants: Assets.<model>.<clip>.name / .duration

type Clip = { name: string, duration: float }

type AlphaClips = {
  walk: Clip,
}

let alpha = {
  walk: { name: \"walk\", duration: 0.9667 },
}

type ZetaClips = {
  idle: Clip,
  run: Clip,
}

let zeta = {
  idle: { name: \"idle\", duration: 1.5 },
  run: { name: \"run\", duration: 0.5 },
}
";
        assert_eq!(src, expected);
        // And byte-identical on a second run.
        assert_eq!(generate_assets_source(&models).unwrap(), expected);
    }

    #[test]
    fn same_shaped_models_share_one_declared_type() {
        // Two models with identical clip fields must NOT declare two
        // same-shaped types (the checker would find the record literals
        // ambiguous) — the second reuses the first's.
        let models = vec![
            ("xbot".to_string(), vec![("idle".to_string(), 1.0)]),
            ("ybot".to_string(), vec![("idle".to_string(), 2.0)]),
        ];
        let src = generate_assets_source(&models).unwrap();
        assert_eq!(src.matches("type XbotClips").count(), 1);
        assert!(!src.contains("type YbotClips"));
        assert!(src.contains("let ybot = {"));
    }

    #[test]
    fn clip_shadowing_models_are_excluded() {
        // A model whose clips are literally named "name" and "duration" has
        // the same field set as `Clip` itself — nominal literal resolution
        // would collide, so it is excluded (the command warns).
        let shadowing = vec![
            ("name".to_string(), 1.0),
            ("duration".to_string(), 2.0),
        ];
        assert!(shadows_clip_type(&shadowing));
        assert!(!shadows_clip_type(&[("idle".to_string(), 1.0)]));
        assert_eq!(
            generate_assets_source(&[("weird".to_string(), shadowing)]),
            None
        );
    }

    #[test]
    fn floats_are_plain_literals() {
        assert_eq!(format_float(2.24), "2.24");
        assert_eq!(format_float(0.9667), "0.9667");
        assert_eq!(format_float(2.0), "2.0");
        assert_eq!(format_float(0.0), "0.0");
        // Sub-precision durations round to the fixed grid, never sci-notation.
        assert_eq!(format_float(0.00001), "0.0");
    }

    #[test]
    fn escapes_string_literals() {
        assert_eq!(escape_string("wa\"lk"), "wa\\\"lk");
        assert_eq!(escape_string("a\\b"), "a\\\\b");
        assert_eq!(escape_string("a\nb\tc"), "a\\nb\\tc");
    }
}
