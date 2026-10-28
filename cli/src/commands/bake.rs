//! `functor bake`: lightmaps for the project's `bakes` (see `lightmap.funi`).
//! Each job writes `<name>.png` (light ÷ `scale`, sRGB-encoded RGBA8; alpha
//! marks texels the bake reached) and `<name>.mesh.bin` (the unwrapped mesh).

use std::io::Error;
use std::path::Path;
use std::time::Instant;

use functor_runtime_common::functor_lang_prelude::lightmap::bake_jobs;
use functor_runtime_common::functor_lang_prelude::FunctorHost;
use functor_runtime_common::lightmap::{
    bake_mesh, write_mesh, BakeOptions, UnwrapOptions, BAKE_VERSION,
};

use crate::output::{emit, Event};

/// Texels kept around each chart, filled by dilation.
const PADDING: u32 = 2;

pub struct Settings {
    pub texels_per_meter: f64,
    pub ao_samples: u32,
    pub ao_distance: f32,
    pub bounces: u32,
    pub gi_samples: u32,
    pub scale: f32,
    pub seed: u64,
    pub max_size: u32,
}

pub fn execute(
    project: &functor_lang::project::Project,
    output: &Path,
    settings: &Settings,
) -> Result<(), Error> {
    // The game may load the previous bake; it reads as not there yet.
    functor_runtime_common::functor_lang_prelude::bytes::hide_files_under(output);
    let session =
        functor_lang::Session::load(&project.module, &mut FunctorHost).map_err(|failure| {
            Error::other(
                project
                    .sources
                    .render(failure.error.span.start, &failure.error.message),
            )
        })?;
    let value = session.global("bakes").ok_or_else(|| {
        Error::other("no top-level `let bakes = …` (a List<Lightmap.bakeJob>) to bake")
    })?;
    if !(settings.scale.is_finite() && settings.scale > 0.0) {
        return Err(Error::other(format!(
            "--scale must be a finite number above 0, got {}",
            settings.scale
        )));
    }
    let jobs = bake_jobs(&value).map_err(|e| Error::other(format!("`bakes`: {e}")))?;
    if jobs.is_empty() {
        return Err(Error::other("`bakes` is empty: nothing to bake"));
    }
    let options = BakeOptions {
        unwrap: UnwrapOptions {
            texels_per_meter: settings.texels_per_meter,
            padding: PADDING,
            max_size: settings.max_size,
        },
        ao_samples: settings.ao_samples,
        ao_distance: settings.ao_distance,
        bounces: settings.bounces,
        gi_samples: settings.gi_samples,
        seed: settings.seed,
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
    };
    // Recorded in the PNG, so a lightmap says how it was made.
    let made = format!(
        "texelsPerMeter={} aoSamples={} aoDistance={} bounces={} giSamples={} scale={} seed={} maxSize={}",
        settings.texels_per_meter,
        settings.ao_samples,
        settings.ao_distance,
        settings.bounces,
        settings.gi_samples,
        settings.scale,
        settings.seed,
        settings.max_size
    );
    // Bake and encode everything before writing anything, so an error
    // leaves the old files.
    let mut files = Vec::new();
    let mut reports = Vec::new();
    for job in &jobs {
        let start = Instant::now();
        let (mesh, lightmap) = bake_mesh(&job.mesh, &job.lights, &job.albedo, &options)
            .map_err(|e| Error::other(format!("`{}`: {e}", job.name)))?;
        let png = output.join(format!("{}.png", job.name));
        let bin = output.join(format!("{}.mesh.bin", job.name));
        let filled = lightmap.filled.iter().filter(|&&f| f).count();
        // The brightest channel, so clipping (over `scale`) shows.
        let peak = lightmap
            .light
            .iter()
            .flatten()
            .fold(0.0f32, |a, &b| a.max(b));
        reports.push(format!(
            "baked `{}`: {} triangles, {} lights, {s}×{s} lightmap ({filled} texels, peak {peak:.2}{}), in {:.2} s → {}, {}",
            job.name,
            mesh.triangles.len(),
            job.lights.len(),
            if peak > settings.scale { ", clipped: raise --scale" } else { "" },
            start.elapsed().as_secs_f64(),
            png.display(),
            bin.display(),
            s = lightmap.size,
        ));
        files.push((
            png,
            encode_png(lightmap.size, &lightmap.rgba8(settings.scale), &made)?,
        ));
        files.push((bin, write_mesh(&mesh)));
    }
    // Write beside the targets, then rename each into place, so a failed
    // write never leaves a lightmap paired with another bake's mesh.
    std::fs::create_dir_all(output)?;
    let mut staged = Vec::new();
    for (path, bytes) in &files {
        let extension = path.extension().unwrap().to_string_lossy();
        let temp = path.with_extension(format!("{extension}.tmp"));
        if let Err(e) = std::fs::write(&temp, bytes) {
            staged.iter().chain([&(temp, path)]).for_each(|(t, _)| {
                std::fs::remove_file(t).ok();
            });
            return Err(e);
        }
        staged.push((temp, path));
    }
    for (temp, path) in staged {
        std::fs::rename(temp, path)?;
    }
    for message in reports {
        emit(Event::Info { message });
    }
    Ok(())
}

/// An RGBA8 PNG with the baker's version and settings as text chunks.
fn encode_png(size: u32, rgba: &[u8], made: &str) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let text = [
        ("Software", "functor bake".to_string()),
        ("functor-bake-version", BAKE_VERSION.to_string()),
        ("functor-bake-settings", made.to_string()),
        ("functor-bake-color-space", "srgb".to_string()),
    ];
    for (key, value) in text {
        encoder
            .add_text_chunk(key.to_string(), value)
            .map_err(Error::other)?;
    }
    let mut writer = encoder.write_header().map_err(Error::other)?;
    writer.write_image_data(rgba).map_err(Error::other)?;
    writer.finish().map_err(Error::other)?;
    Ok(out)
}
