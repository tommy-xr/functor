//! `functor export --model x.glb`: turn a game model into a printable solid —
//! scale it into the print frame, remesh it closed (thickening anything
//! thinner than the nozzle), simplify, and write STL or 3MF. Needs no project.

use std::io::{Error, Write};
use std::path::Path;
use std::time::Instant;

use functor_solid::{model, remesh, Mesh};

use crate::output::{emit, Event};

/// A simplify tolerance (mm) that merges only coplanar triangles.
const LOSSLESS: f64 = 1e-6;

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
    let three_mf = super::functor_lang_project::export_format(args.output)?;
    if !(args.height > 0.0 && args.nozzle > 0.0 && args.voxel.is_none_or(|v| v > 0.0)) {
        return Err(Error::other(
            "--height, --nozzle and --voxel must be positive",
        ));
    }
    let bytes = std::fs::read(args.model)
        .map_err(|e| Error::other(format!("cannot read {}: {e}", args.model.display())))?;
    let pose = args.pose.map(parse_pose).transpose()?;
    let mut soup = model::load_gltf(&bytes, pose)
        .map_err(|e| Error::other(format!("{}: {e}", args.model.display())))?;
    model::to_print_frame(&mut soup, args.height);

    // Thicken every surface to the nozzle width so cards and thin features
    // print; sample at half the nozzle unless that grid would be too large.
    let offset = args.nozzle / 2.0;
    let voxel = match args.voxel {
        Some(v) => v,
        None => remesh::fitting_voxel(&soup, args.nozzle / 2.0, offset),
    };
    let dense = remesh::printable(&soup, voxel, offset).map_err(Error::other)?;
    // Lossless by default: merge only coplanar triangles (flat faces drop a
    // lot; nothing moves). Any real tolerance can self-intersect thin regions,
    // which slicers tolerate but strict CAD checks reject — so it's opt-in.
    let mesh =
        functor_solid::simplify(&dense, args.simplify.unwrap_or(LOSSLESS)).map_err(Error::other)?;

    let name = args
        .model
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model");
    super::functor_lang_project::check_printable(name, &mesh).map_err(|e| match args.simplify {
        Some(_) => Error::other(format!("{e} — try a smaller --simplify")),
        None => e,
    })?;
    write(args.output, three_mf, name, &mesh)?;
    let size = mesh
        .bounds()
        .map(|b| {
            format!(
                "{:.1} x {:.1} x {:.1} mm",
                b.max[0] - b.min[0],
                b.max[1] - b.min[1],
                b.max[2] - b.min[2]
            )
        })
        .unwrap_or_default();
    emit(Event::Info {
        message: format!(
            "exported {} to {}: {} triangles (from {}), {size}, {voxel:.2} mm voxels, {:.1} s",
            args.model.display(),
            args.output.display(),
            mesh.triangles.len(),
            dense.triangles.len(),
            started.elapsed().as_secs_f64()
        ),
    });
    Ok(())
}

/// `[ANIMATION@]SECONDS`: a moment of a named (or the first) animation.
fn parse_pose(spec: &str) -> Result<model::Pose<'_>, Error> {
    let (animation, seconds) = match spec.rsplit_once('@') {
        Some((name, t)) => (Some(name), t),
        None => (None, spec),
    };
    let seconds = seconds
        .parse::<f32>()
        .map_err(|_| Error::other(format!("--pose {spec:?}: expected [ANIMATION@]SECONDS")))?;
    Ok(model::Pose { animation, seconds })
}

fn write(output: &Path, three_mf: bool, name: &str, mesh: &Mesh) -> Result<(), Error> {
    let mut file = std::io::BufWriter::new(std::fs::File::create(output)?);
    if three_mf {
        functor_solid::threemf::write(name, &[(name, mesh)], &mut file)?;
    } else {
        functor_solid::stl::write_binary(mesh, &mut file)?;
    }
    file.flush()
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
        assert!(export(Some("1.0"), 20.0).contains("no animations"));
        assert!(export(None, -1.0).contains("must be positive"));
        assert!(!out.exists());
    }
}
