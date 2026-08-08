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
        }
    }
}
