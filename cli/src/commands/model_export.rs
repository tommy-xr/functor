//! `functor export --model x.glb`: turn a game model into a printable solid —
//! scale it into the print frame, remesh it closed (thickening anything
//! thinner than the nozzle), simplify, and write STL or 3MF. Needs no project.

use std::io::Error;
use std::path::Path;
use std::time::Instant;

use functor_solid::{model, print};

use super::functor_lang_project::{check_printable, export_format, size_label, write_parts};

use crate::output::{emit, Event};

pub struct ModelExport<'a> {
    pub model: &'a Path,
    pub output: &'a Path,
    pub height: f64,
    pub nozzle: f64,
    pub voxel: Option<f64>,
    pub pose: Option<&'a str>,
    pub simplify: Option<f64>,
}

pub fn execute(args: &ModelExport) -> Result<(), Error> {
    let started = Instant::now();
    let three_mf = export_format(args.output)?;
    let positive = |x: f64| x > 0.0 && x.is_finite();
    if !(positive(args.height) && positive(args.nozzle) && args.voxel.is_none_or(positive)) {
        return Err(Error::other(
            "--height, --nozzle and --voxel must be positive numbers",
        ));
    }
    let bytes = std::fs::read(args.model)
        .map_err(|e| Error::other(format!("cannot read {}: {e}", args.model.display())))?;
    let pose = args.pose.map(parse_pose).transpose()?;
    let failed = |e: String| Error::other(format!("{}: {e}", args.model.display()));
    let soup = model::load_gltf(&bytes, pose).map_err(failed)?;

    let printed = print::printable(
        &soup,
        &print::PrintOptions {
            height: args.height,
            nozzle: args.nozzle,
            voxel: args.voxel,
            simplify: args.simplify,
            max_cells: functor_solid::remesh::MAX_CELLS,
        },
    )
    .map_err(failed)?;
    let mesh = printed.mesh;

    let name = args
        .model
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model");
    check_printable(name, &mesh).map_err(|e| match args.simplify {
        Some(_) => Error::other(format!("{e} — try a smaller --simplify")),
        None => e,
    })?;
    write_parts(args.output, three_mf, name, &[(name, &mesh)])?;
    emit(Event::Info {
        message: format!(
            "exported {} to {}: {} triangles (from {}), {}, {:.2} mm voxels, \
{:.2} mm shell, {:.1} s",
            args.model.display(),
            args.output.display(),
            mesh.triangles.len(),
            printed.remeshed,
            size_label(&mesh),
            printed.voxel,
            printed.shell,
            started.elapsed().as_secs_f64()
        ),
    });
    Ok(())
}

/// `[ANIMATION@]SECONDS`: a moment of an animation (by name or index; the
/// first when omitted).
fn parse_pose(spec: &str) -> Result<model::Pose<'_>, Error> {
    let (animation, seconds) = match spec.rsplit_once('@') {
        Some((name, t)) => (Some(name), t),
        None => (None, spec),
    };
    match seconds.parse::<f32>() {
        Ok(seconds) if seconds.is_finite() => Ok(model::Pose { animation, seconds }),
        _ => Err(Error::other(format!(
            "--pose {spec:?}: expected [ANIMATION@]SECONDS"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        [
            env!("CARGO_MANIFEST_DIR"),
            "tests",
            "fixtures",
            "open-box.glb",
        ]
        .iter()
        .collect()
    }

    /// An open-topped box (open edges, like many game meshes) exports as a
    /// closed, printable STL — `check_printable` runs before anything is written.
    #[test]
    fn an_open_model_exports_closed() {
        let out = std::env::temp_dir().join(format!("functor-model-{}.stl", std::process::id()));
        execute(&ModelExport {
            model: &fixture(),
            output: &out,
            height: 20.0,
            nozzle: 0.4,
            voxel: Some(0.4),
            pose: None,
            simplify: None,
        })
        .unwrap();
        let bytes = std::fs::read(&out).unwrap();
        // Exactly the asked height, on the bed (z of every STL vertex).
        let z = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let zs: Vec<f32> = (84..bytes.len())
            .step_by(50)
            .flat_map(|t| (0..3).map(move |v| t + 12 + 12 * v + 8))
            .map(z)
            .collect();
        let (lo, hi) = zs
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &z| (lo.min(z), hi.max(z)));
        assert!(lo.abs() < 1e-4 && (hi - 20.0).abs() < 1e-4, "{lo}..{hi}");
        std::fs::remove_file(&out).ok();
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        assert!(count > 12, "{count} triangles");
        assert_eq!(bytes.len(), 84 + 50 * count);
    }

    #[test]
    fn bad_poses_and_sizes_are_refused() {
        let out =
            std::env::temp_dir().join(format!("functor-model-bad-{}.stl", std::process::id()));
        let export = |pose: Option<&str>, height: f64| {
            execute(&ModelExport {
                model: &fixture(),
                output: &out,
                height,
                nozzle: 0.4,
                voxel: Some(0.4),
                pose,
                simplify: None,
            })
            .unwrap_err()
            .to_string()
        };
        assert!(export(Some("Run@soon"), 20.0).contains("[ANIMATION@]SECONDS"));
        assert!(export(Some("nan"), 20.0).contains("[ANIMATION@]SECONDS"));
        assert!(export(Some("1.0"), 20.0).contains("no animations"));
        assert!(export(None, -1.0).contains("must be positive"));
        assert!(export(None, f64::INFINITY).contains("must be positive"));
        assert!(!out.exists());
    }
}
