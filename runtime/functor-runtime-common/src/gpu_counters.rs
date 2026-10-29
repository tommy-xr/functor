//! Process-wide GPU-resource counters — a leak tripwire.
//!
//! There are no `Drop` impls for GL resources in this crate (the context isn't
//! available at drop time); frees happen through explicit `delete(gl)` methods.
//! A leak — a resource created every frame and never freed — is therefore
//! invisible to ordinary instrumentation. These counters make it a visibly
//! climbing number: every `create_*` site bumps a live count, every explicit
//! delete decrements it, so a leak class shows up as a monotonically rising
//! `live_buffers`/`live_vaos`/`live_textures` in the frame-stats stream.
//!
//! It is a global (a single `static` of atomics), not a struct threaded through
//! the create sites, because those sites are scattered across trait impls
//! (`RenderableAsset::hydrate`, `Geometry::draw`, `RenderTargetBuffers`, …) with
//! no shared owner to thread through, and the native runtime is a single process
//! with a single GL context. Bumping a `Relaxed` atomic is the entire cost at
//! each site — no signature churn, matching "don't refactor create sites beyond
//! adding the increment".
//!
//! Native-only reads it (the desktop `FrameStats` reporter drains it each stats
//! window); the shared create sites still bump it on wasm, harmlessly unread.

use std::sync::atomic::{AtomicU64, Ordering};

/// The counters. `live_*` persist across frames (current count of alive GL
/// objects); the rest are per-window accumulators the reporter drains with
/// [`take_window`](GpuCounters::take_window).
pub struct GpuCounters {
    live_vaos: AtomicU64,
    live_buffers: AtomicU64,
    live_textures: AtomicU64,
    bytes_uploaded: AtomicU64,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    instance_uploads: AtomicU64,
    instance_upload_bytes: AtomicU64,
    // The current rendered frame's instance uploads, zeroed by the instanced
    // renderer when a new frame begins — so read between frames (where the
    // shells serve `GET /state`) they are the last rendered frame's.
    frame_instance_uploads: AtomicU64,
    frame_instance_upload_bytes: AtomicU64,
    // The current rendered frame's terrain draws, zeroed by the terrain
    // renderer when a new frame begins (same read-between-frames contract).
    // One word (patches << 32 | vertices) so a reader on another thread
    // never pairs one draw's patches with another's vertices.
    frame_terrain: AtomicU64,
    // The current rendered frame's skinned-model work, zeroed when a new
    // frame begins (same contract). One word (draws << 32 | pose evals).
    frame_skinned: AtomicU64,
    // The current rendered frame's `ShaderProgram` uniform uploads (same contract).
    frame_uniforms: AtomicU64,
    // The current rendered frame's frustum tests (same contract). One word
    // (tested << 32 | culled).
    frame_cull: AtomicU64,
}

static COUNTERS: GpuCounters = GpuCounters::new();

/// The process-wide counters.
pub fn gpu_counters() -> &'static GpuCounters {
    &COUNTERS
}

/// A snapshot of the live counts — instantaneous, so reported as the latest.
#[derive(Debug, Clone, Copy)]
pub struct GpuLive {
    pub vaos: u64,
    pub buffers: u64,
    pub textures: u64,
}

/// The per-window accumulators, as drained by [`GpuCounters::take_window`].
#[derive(Debug, Clone, Copy)]
pub struct GpuWindow {
    pub bytes_uploaded: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    /// `Scene.instanced` instance-buffer uploads (count, bytes) — zero in the
    /// steady state when instance lists are retained.
    pub instance_uploads: u64,
    pub instance_upload_bytes: u64,
}

/// One frame's terrain draws, summed over every draw of the frame (a stereo
/// frame draws the shared patch set once per eye, so it counts twice).
/// `vertices` is patches × the patch mesh's vertex count (grid + skirt).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TerrainDraws {
    pub patches: u64,
    pub vertices: u64,
}

/// One frame's skinned-model work, summed over every pass (shadow and each
/// eye): mesh draw calls (an instanced draw counts once), and pose
/// evaluations (one per skinned model per pass).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkinnedWork {
    pub draws: u64,
    pub pose_evals: u64,
}

/// One frame's frustum culling, summed over every pass (shadow, render
/// targets, each eye): draws tested against the pass's frustum, and those
/// skipped. A hardware-instanced node counts once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CullCounts {
    pub tested: u64,
    pub culled: u64,
}

/// One frame's `Scene.instanced` instance-buffer uploads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstanceUploads {
    pub count: u64,
    pub bytes: u64,
}

impl GpuCounters {
    const fn new() -> Self {
        GpuCounters {
            live_vaos: AtomicU64::new(0),
            live_buffers: AtomicU64::new(0),
            live_textures: AtomicU64::new(0),
            bytes_uploaded: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            instance_uploads: AtomicU64::new(0),
            instance_upload_bytes: AtomicU64::new(0),
            frame_instance_uploads: AtomicU64::new(0),
            frame_instance_upload_bytes: AtomicU64::new(0),
            frame_terrain: AtomicU64::new(0),
            frame_skinned: AtomicU64::new(0),
            frame_uniforms: AtomicU64::new(0),
            frame_cull: AtomicU64::new(0),
        }
    }

    pub fn vao_created(&self) {
        self.live_vaos.fetch_add(1, Ordering::Relaxed);
    }
    pub fn vao_deleted(&self) {
        self.live_vaos.fetch_sub(1, Ordering::Relaxed);
    }
    pub fn buffer_created(&self) {
        self.live_buffers.fetch_add(1, Ordering::Relaxed);
    }
    pub fn buffer_deleted(&self) {
        self.live_buffers.fetch_sub(1, Ordering::Relaxed);
    }
    pub fn texture_created(&self) {
        self.live_textures.fetch_add(1, Ordering::Relaxed);
    }
    pub fn texture_deleted(&self) {
        self.live_textures.fetch_sub(1, Ordering::Relaxed);
    }

    /// Record `bytes` uploaded to the GPU this frame (a `buffer_data` /
    /// `buffer_sub_data` / `tex_image` payload).
    pub fn uploaded(&self, bytes: usize) {
        self.bytes_uploaded.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Record one instance-buffer upload of `bytes` (also counted in the
    /// general upload bytes).
    pub fn instance_uploaded(&self, bytes: usize) {
        self.uploaded(bytes);
        self.instance_uploads.fetch_add(1, Ordering::Relaxed);
        self.instance_upload_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
        self.frame_instance_uploads.fetch_add(1, Ordering::Relaxed);
        self.frame_instance_upload_bytes
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Zero the per-frame instance uploads (a new rendered frame began).
    pub fn begin_instance_frame(&self) {
        self.frame_instance_uploads.store(0, Ordering::Relaxed);
        self.frame_instance_upload_bytes.store(0, Ordering::Relaxed);
    }

    /// The instance uploads of the most recently rendered frame.
    pub fn frame_instance_uploads(&self) -> InstanceUploads {
        InstanceUploads {
            count: self.frame_instance_uploads.load(Ordering::Relaxed),
            bytes: self.frame_instance_upload_bytes.load(Ordering::Relaxed),
        }
    }

    /// Record one terrain draw of `patches` instances, `vertices` in total.
    /// Both stay far below 2^32 per frame (8192 instances × a few thousand
    /// vertices × a few draws).
    pub fn terrain_drawn(&self, patches: u64, vertices: u64) {
        self.frame_terrain
            .fetch_add(patches << 32 | vertices, Ordering::Relaxed);
    }

    /// Zero the per-frame terrain draws (a new rendered frame began).
    pub fn begin_terrain_frame(&self) {
        self.frame_terrain.store(0, Ordering::Relaxed);
    }

    /// The terrain draws of the most recently rendered frame.
    pub fn frame_terrain(&self) -> TerrainDraws {
        let packed = self.frame_terrain.load(Ordering::Relaxed);
        TerrainDraws {
            patches: packed >> 32,
            vertices: packed & u64::from(u32::MAX),
        }
    }

    /// Record one skinned mesh draw.
    pub fn skinned_drawn(&self) {
        self.frame_skinned.fetch_add(1 << 32, Ordering::Relaxed);
    }

    /// Record one skinned-model pose evaluation.
    pub fn pose_evaluated(&self) {
        self.frame_skinned.fetch_add(1, Ordering::Relaxed);
    }

    /// Zero the per-frame skinned work (a new rendered frame began).
    pub fn begin_skinned_frame(&self) {
        self.frame_skinned.store(0, Ordering::Relaxed);
    }

    /// Record one `ShaderProgram` uniform upload.
    pub fn uniform_uploaded(&self) {
        self.frame_uniforms.fetch_add(1, Ordering::Relaxed);
    }

    /// Zero the per-frame uniform uploads (a new rendered frame began).
    pub fn begin_uniform_frame(&self) {
        self.frame_uniforms.store(0, Ordering::Relaxed);
    }

    /// The `ShaderProgram` uniform uploads of the most recently rendered frame.
    pub fn frame_uniform_uploads(&self) -> u64 {
        self.frame_uniforms.load(Ordering::Relaxed)
    }

    /// The skinned work of the most recently rendered frame.
    pub fn frame_skinned(&self) -> SkinnedWork {
        let packed = self.frame_skinned.load(Ordering::Relaxed);
        SkinnedWork {
            draws: packed >> 32,
            pose_evals: packed & u64::from(u32::MAX),
        }
    }

    /// Record one frustum test.
    pub fn cull_tested(&self, culled: bool) {
        self.frame_cull
            .fetch_add(1 << 32 | u64::from(culled), Ordering::Relaxed);
    }

    /// Zero the per-frame frustum tests (a new rendered frame began).
    pub fn begin_cull_frame(&self) {
        self.frame_cull.store(0, Ordering::Relaxed);
    }

    /// The frustum tests of the most recently rendered frame.
    pub fn frame_cull(&self) -> CullCounts {
        let packed = self.frame_cull.load(Ordering::Relaxed);
        CullCounts {
            tested: packed >> 32,
            culled: packed & u64::from(u32::MAX),
        }
    }

    pub fn cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }
    pub fn cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// The current live counts.
    pub fn live(&self) -> GpuLive {
        GpuLive {
            vaos: self.live_vaos.load(Ordering::Relaxed),
            buffers: self.live_buffers.load(Ordering::Relaxed),
            textures: self.live_textures.load(Ordering::Relaxed),
        }
    }

    /// Read and zero the per-window accumulators. Called once per stats window,
    /// so the returned totals cover exactly the frames since the last drain.
    pub fn take_window(&self) -> GpuWindow {
        GpuWindow {
            bytes_uploaded: self.bytes_uploaded.swap(0, Ordering::Relaxed),
            cache_hits: self.cache_hits.swap(0, Ordering::Relaxed),
            cache_misses: self.cache_misses.swap(0, Ordering::Relaxed),
            instance_uploads: self.instance_uploads.swap(0, Ordering::Relaxed),
            instance_upload_bytes: self.instance_upload_bytes.swap(0, Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_draws_sum_per_frame_and_reset() {
        let counters = GpuCounters::new();
        counters.terrain_drawn(66, 66 * 4481);
        counters.terrain_drawn(66, 66 * 4481);
        let both_eyes = TerrainDraws {
            patches: 132,
            vertices: 132 * 4481,
        };
        assert_eq!(counters.frame_terrain(), both_eyes);
        counters.begin_terrain_frame();
        assert_eq!(counters.frame_terrain(), TerrainDraws::default());
    }

    #[test]
    fn skinned_work_sums_per_frame_and_resets() {
        let counters = GpuCounters::new();
        for _ in 0..3 {
            counters.pose_evaluated();
            counters.skinned_drawn();
            counters.skinned_drawn();
        }
        let expected = SkinnedWork {
            draws: 6,
            pose_evals: 3,
        };
        assert_eq!(counters.frame_skinned(), expected);
        counters.begin_skinned_frame();
        assert_eq!(counters.frame_skinned(), SkinnedWork::default());
    }
}
