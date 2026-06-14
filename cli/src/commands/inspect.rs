use std::fs;
use std::io;
use std::path::Path;

use functor_runtime_common::inspect::{inspect_model, Aabb, ModelReport};

/// Inspect a glTF/glb model on the CPU and print a text report. No GL context
/// is created, so this is safe to run headless (CI / scripts / LLMs).
pub async fn execute_model(path: &str, time: Option<f32>) -> io::Result<()> {
    let model_path = Path::new(path);
    if !model_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Model not found: {}", path),
        ));
    }

    let bytes = fs::read(model_path)?;

    let report = inspect_model(bytes, time)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    print_report(path, &report);
    Ok(())
}

fn format_aabb(aabb: &Aabb) -> String {
    if aabb.is_empty() {
        return "<empty>".to_string();
    }
    format!(
        "min ({:.4}, {:.4}, {:.4})  max ({:.4}, {:.4}, {:.4})",
        aabb.min.x, aabb.min.y, aabb.min.z, aabb.max.x, aabb.max.y, aabb.max.z
    )
}

fn print_report(path: &str, report: &ModelReport) {
    println!("Model: {}", path);
    println!();

    println!("Meshes: {}", report.mesh_count);
    println!("Primitives: {}", report.primitives.len());
    for (i, p) in report.primitives.iter().enumerate() {
        println!(
            "  [{}] {} - vertices: {}, indices: {}, joints: {}, weights: {}, skinned: {}",
            i,
            p.mesh_name,
            p.vertex_count,
            p.index_count,
            p.joint_count,
            p.weight_count,
            p.has_skinning,
        );
    }
    println!();

    if report.has_skeleton {
        println!("Skeleton: present ({} joints)", report.joint_count);
    } else {
        println!("Skeleton: none");
    }
    println!();

    if report.animations.is_empty() {
        println!("Animations: none");
    } else {
        println!("Animations: {}", report.animations.len());
        for a in &report.animations {
            println!("  - {} ({:.4}s)", a.name, a.duration);
        }
    }
    println!();

    println!("Static AABB: {}", format_aabb(&report.static_aabb));

    match &report.skinned_aabb {
        Some(s) => {
            println!(
                "Skinned AABB @ t={:.4} (anim '{}', sampled {:.4}s): {}",
                s.requested_time,
                s.animation_name,
                s.sampled_time,
                format_aabb(&s.aabb),
            );
        }
        None => {
            println!("Skinned AABB: not computed (no --time, or model is not animated/skinned)");
        }
    }
}
