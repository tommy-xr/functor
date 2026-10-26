mod asset_cache;
mod asset_handle;
mod asset_loader;
mod asset_pipeline;

mod renderable_asset;

pub mod pipelines;
pub mod preload;

pub use asset_cache::*;
pub use asset_handle::*;
pub use asset_loader::*;
pub use asset_pipeline::*;

pub use renderable_asset::*;

/// File extensions copied/synchronized with a project because the runtimes
/// may resolve them dynamically. `bin` covers external glTF buffers.
pub const PROJECT_ASSET_EXTENSIONS: &[&str] = &[
    "glb", "gltf", "bin", "wav", "ogg", "mp3", "png", "jpg", "jpeg", "hdr",
];

/// Whether a filesystem path is a project asset the runtime may load.
pub fn is_project_asset_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            PROJECT_ASSET_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

/// Whether an asset can be consumed from one independently uploaded blob.
/// `.gltf` projects may reference sibling buffers/images, but the current
/// model pipeline only decodes self-contained `.glb`; keep those multi-file
/// models out of live push until URI resolution is cache-aware.
pub fn is_live_project_asset_file(path: &std::path::Path) -> bool {
    is_project_asset_file(path)
        && !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("gltf") || extension.eq_ignore_ascii_case("bin")
            })
}
