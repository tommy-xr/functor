//! Per-frame stage timings and spike records: `GET /frames`, `GET /spikes`,
//! and `/state.frame_stats`.
//!
//! Sim-side stages are timed by [`span`] guards on the producer's thread and
//! accumulate in a thread-local until [`take_sim`] drains them into that
//! frame's `SimSnapshot`. The shell adds its render-side times and counters
//! and pushes one [`FrameRecord`] per frame into the fixed-size
//! [`FrameTelemetry`] ring. Recording never allocates; reports allocate when
//! requested. On Android each span is also an ATrace section, so Perfetto
//! shows the same stages. On wasm, where `Instant` panics, spans are no-ops.

use std::cell::Cell;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Frames the ring keeps (5 s at 120 Hz).
pub const RING_FRAMES: usize = 600;
/// Spike records kept.
pub const SPIKES: usize = 8;
/// Frames kept on each side of a spike.
pub const SPIKE_CONTEXT: usize = 4;
/// A frame over this many display periods is a spike.
pub const SPIKE_PERIODS: f64 = 1.5;

/// A sim-side stage timed by [`span`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimStage {
    /// Parked and job deliveries, `sampledInput`, and subscription `update`s.
    Update,
    Tick,
    /// The whole physics phase: hook, reconcile, steps, queries, events.
    Physics,
    /// Declaring the hook's scene into the world (inside `Physics`).
    PhysicsReconcile,
    /// Fixed steps and their timeline keyframes (inside `Physics`).
    PhysicsStep,
    /// The game's `draw`.
    Draw,
}

/// Sim-side costs and physics counters since the last [`take_sim`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimStages {
    pub update_ns: u64,
    pub tick_ns: u64,
    pub physics_ns: u64,
    pub physics_reconcile_ns: u64,
    pub physics_step_ns: u64,
    pub draw_ns: u64,
    pub bodies_created: u32,
    pub bodies_removed: u32,
    /// The world's size after its last step.
    pub bodies: u32,
    pub colliders: u32,
}

impl SimStages {
    const ZERO: Self = Self {
        update_ns: 0,
        tick_ns: 0,
        physics_ns: 0,
        physics_reconcile_ns: 0,
        physics_step_ns: 0,
        draw_ns: 0,
        bodies_created: 0,
        bodies_removed: 0,
        bodies: 0,
        colliders: 0,
    };

    /// Fold in an older snapshot's stages (a skipped pipelined snapshot).
    pub fn absorb_older(&mut self, older: SimStages) {
        self.update_ns += older.update_ns;
        self.tick_ns += older.tick_ns;
        self.physics_ns += older.physics_ns;
        self.physics_reconcile_ns += older.physics_reconcile_ns;
        self.physics_step_ns += older.physics_step_ns;
        self.draw_ns += older.draw_ns;
        self.bodies_created = self.bodies_created.saturating_add(older.bodies_created);
        self.bodies_removed = self.bodies_removed.saturating_add(older.bodies_removed);
    }
}

thread_local! {
    static SIM: Cell<SimStages> = const { Cell::new(SimStages::ZERO) };
}

fn with_sim(f: impl FnOnce(&mut SimStages)) {
    SIM.with(|cell| {
        let mut stages = cell.get();
        f(&mut stages);
        cell.set(stages);
    });
}

/// Drain this thread's sim stages (once per snapshot).
pub fn take_sim() -> SimStages {
    SIM.with(Cell::take)
}

/// Count a reconcile's spawned and despawned bodies.
pub fn note_reconcile(created: usize, removed: usize) {
    with_sim(|s| {
        // Saturating: a thread no snapshot drains (the web shell) keeps adding.
        s.bodies_created = s.bodies_created.saturating_add(created as u32);
        s.bodies_removed = s.bodies_removed.saturating_add(removed as u32);
    });
}

/// Record the world's size after a step.
pub fn note_world_size(bodies: usize, colliders: usize) {
    with_sim(|s| {
        s.bodies = bodies as u32;
        s.colliders = colliders as u32;
    });
}

/// Times its stage until dropped.
#[must_use]
pub struct Span {
    #[cfg(not(target_arch = "wasm32"))]
    stage: SimStage,
    #[cfg(not(target_arch = "wasm32"))]
    started: std::time::Instant,
}

/// Time `stage` until the returned guard drops.
#[cfg(not(target_arch = "wasm32"))]
pub fn span(stage: SimStage) -> Span {
    #[cfg(target_os = "android")]
    atrace::begin(stage);
    Span {
        stage,
        started: std::time::Instant::now(),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn span(_stage: SimStage) -> Span {
    Span {}
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Span {
    fn drop(&mut self) {
        let ns = self.started.elapsed().as_nanos() as u64;
        with_sim(|s| {
            *match self.stage {
                SimStage::Update => &mut s.update_ns,
                SimStage::Tick => &mut s.tick_ns,
                SimStage::Physics => &mut s.physics_ns,
                SimStage::PhysicsReconcile => &mut s.physics_reconcile_ns,
                SimStage::PhysicsStep => &mut s.physics_step_ns,
                SimStage::Draw => &mut s.draw_ns,
            } += ns;
        });
        #[cfg(target_os = "android")]
        atrace::end();
    }
}

/// Drops the sim stages recorded while it lives, so work that is not the
/// live frame (the trajectory preview's dry run) is not counted as its
/// stages. That time still shows in the frame's `sim_us`.
pub struct Suspended(SimStages);

impl Suspended {
    pub fn new() -> Self {
        Self(take_sim())
    }
}

impl Default for Suspended {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Suspended {
    fn drop(&mut self) {
        SIM.with(|cell| cell.set(self.0));
    }
}

/// One frame as the shell saw it. Times are µs; counts are this frame's (the
/// sim side's since the previous snapshot).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FrameRecord {
    /// The shell's sim step count after this frame's steps.
    pub frame: u64,
    /// `/state.rendered_frames` after this frame (0 under `--headless`).
    pub rendered_frame: u64,
    /// Fixed sim steps this frame ran.
    pub steps: u32,
    /// Wall time since the previous frame (headless: this iteration's work).
    pub frame_us: u32,
    /// All producer work in this frame's snapshot; the stages below are parts
    /// of it.
    pub sim_us: u32,
    pub update_us: u32,
    pub tick_us: u32,
    /// The whole physics phase, `physics_reconcile_us` and `physics_step_us`
    /// included.
    pub physics_us: u32,
    pub physics_reconcile_us: u32,
    pub physics_step_us: u32,
    pub draw_us: u32,
    /// The render thread blocked on the sim thread.
    pub sim_wait_us: u32,
    /// Shell asset driving: preloads and terrain heightmap decodes.
    pub assets_us: u32,
    /// Shell rendering. Quest's includes swapchain waits and frame submission.
    pub render_us: u32,
    /// Desktop: the buffer swap. Quest: `xrWaitFrame`.
    pub present_us: u32,
    /// `Scene.instanced` instance-buffer uploads.
    pub instance_uploads: u32,
    /// GPU bytes uploaded: buffers and textures, instance data included.
    pub upload_bytes: u64,
    /// Frustum tests and the draws they skipped.
    pub cull_tested: u32,
    pub cull_culled: u32,
    pub bodies_created: u32,
    pub bodies_removed: u32,
    /// The physics world's size after its last step.
    pub bodies: u32,
    pub colliders: u32,
    /// Quest: display periods since the previous frame's predicted display
    /// time (1 = on time, 2 or more = missed). 0 where unknown.
    pub display_periods: u32,
}

impl FrameRecord {
    /// Copy in the sim side's stages and counters.
    pub fn set_sim(&mut self, sim_ns: u64, stages: &SimStages) {
        self.sim_us = us(sim_ns);
        self.update_us = us(stages.update_ns);
        self.tick_us = us(stages.tick_ns);
        self.physics_us = us(stages.physics_ns);
        self.physics_reconcile_us = us(stages.physics_reconcile_ns);
        self.physics_step_us = us(stages.physics_step_ns);
        self.draw_us = us(stages.draw_ns);
        self.bodies_created = stages.bodies_created;
        self.bodies_removed = stages.bodies_removed;
        self.bodies = stages.bodies;
        self.colliders = stages.colliders;
    }
}

/// Nanoseconds to whole µs, saturating.
pub fn us(ns: u64) -> u32 {
    (ns / 1000).min(u64::from(u32::MAX)) as u32
}

type Stage = (&'static str, fn(&FrameRecord) -> u32);

/// The stages `frame_stats` summarizes, in report order.
const SUMMARIZED: [Stage; 12] = [
    ("frame", |r| r.frame_us),
    ("sim", |r| r.sim_us),
    ("update", |r| r.update_us),
    ("tick", |r| r.tick_us),
    ("physics", |r| r.physics_us),
    ("physics.reconcile", |r| r.physics_reconcile_us),
    ("physics.step", |r| r.physics_step_us),
    ("draw", |r| r.draw_us),
    ("sim.wait", |r| r.sim_wait_us),
    ("assets", |r| r.assets_us),
    ("render", |r| r.render_us),
    ("present", |r| r.present_us),
];

/// Disjoint stages, for naming the one that dominated a spike. The `other`s
/// are the parent's time outside its timed parts: the physics hook, a hot
/// reload, or untimed shell work (inline-sim time; pipelined, the sim's own
/// time is not subtracted, so `shell.other` reads low).
const LEAVES: [Stage; 12] = [
    ("update", |r| r.update_us),
    ("tick", |r| r.tick_us),
    ("physics.reconcile", |r| r.physics_reconcile_us),
    ("physics.step", |r| r.physics_step_us),
    ("physics.other", |r| {
        r.physics_us
            .saturating_sub(r.physics_reconcile_us + r.physics_step_us)
    }),
    ("draw", |r| r.draw_us),
    ("sim.other", |r| {
        r.sim_us
            .saturating_sub(r.update_us + r.tick_us + r.physics_us + r.draw_us)
    }),
    ("sim.wait", |r| r.sim_wait_us),
    ("assets", |r| r.assets_us),
    ("render", |r| r.render_us),
    ("present", |r| r.present_us),
    ("shell.other", |r| {
        r.frame_us.saturating_sub(
            r.sim_us + r.sim_wait_us + r.assets_us + r.render_us + r.present_us,
        )
    }),
];

/// The leaf stage that took longest.
fn dominant(record: &FrameRecord) -> (&'static str, u32) {
    LEAVES
        .iter()
        .map(|(name, field)| (*name, field(record)))
        .fold(("none", 0), |best, next| if next.1 > best.1 { next } else { best })
}

/// One stage's distribution over the ring, µs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StagePercentiles {
    pub p50_us: u32,
    pub p99_us: u32,
    pub max_us: u32,
}

/// `/state.frame_stats`: the ring's stage distributions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FrameStats {
    /// Frames in the ring.
    pub frames: u32,
    /// Spikes since launch.
    pub spikes: u64,
    pub spike_threshold_us: u32,
    /// `frame`, `sim`, `update`, `tick`, `physics`, `physics.reconcile`,
    /// `physics.step`, `draw`, `sim.wait`, `assets`, `render`, `present`.
    pub stages: BTreeMap<String, StagePercentiles>,
}

/// A frame over the spike threshold, with its neighbours.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SpikeRecord {
    pub frame: FrameRecord,
    /// The disjoint stage that took longest, e.g. `physics.reconcile`.
    pub stage: String,
    pub stage_us: u32,
    /// Up to [`SPIKE_CONTEXT`] frames either side, oldest first, the spike
    /// included. Frames after it fill in as they render.
    pub frames: Vec<FrameRecord>,
}

/// `GET /frames`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FramesReport {
    pub stats: FrameStats,
    /// The newest frames, oldest first.
    pub frames: Vec<FrameRecord>,
}

/// `GET /spikes`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SpikesReport {
    pub threshold_us: u32,
    /// Spikes since launch; `spikes` keeps the newest [`SPIKES`].
    pub total: u64,
    /// Oldest first.
    pub spikes: Vec<SpikeRecord>,
}

/// What [`FrameTelemetry::push`] reports for a spike, for the shell's one
/// log line: `spike frame 812: 172.3 ms (physics.reconcile 160.2 ms)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpikeNote {
    pub frame: u64,
    pub frame_us: u32,
    pub stage: &'static str,
    pub stage_us: u32,
}

impl std::fmt::Display for SpikeNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ms = |us: u32| f64::from(us) / 1000.0;
        write!(
            f,
            "spike frame {}: {:.1} ms ({} {:.1} ms)",
            self.frame,
            ms(self.frame_us),
            self.stage,
            ms(self.stage_us)
        )
    }
}

const WINDOW: usize = 2 * SPIKE_CONTEXT + 1;

#[derive(Clone, Copy, Default)]
struct Spike {
    frames: [FrameRecord; WINDOW],
    len: usize,
    /// Index of the spike in `frames`.
    at: usize,
    stage: &'static str,
    stage_us: u32,
}

/// A shell's frame ring and spike records.
pub struct FrameTelemetry {
    /// `gpu_counters`' upload total at the last [`Self::record_gpu`].
    uploads_seen: u64,
    ring: Box<[FrameRecord]>,
    /// Where the next record goes.
    next: usize,
    len: usize,
    threshold_us: u32,
    /// Set by `FUNCTOR_SPIKE_MS`; the display period no longer moves it.
    threshold_pinned: bool,
    spikes: [Spike; SPIKES],
    spikes_total: u64,
}

impl Default for FrameTelemetry {
    /// A 60 Hz display until [`Self::set_display_hz`] says otherwise.
    fn default() -> Self {
        Self::new(60.0)
    }
}

impl FrameTelemetry {
    /// A ring whose spike threshold is [`SPIKE_PERIODS`] display periods of
    /// `display_hz`, or `FUNCTOR_SPIKE_MS` when set.
    pub fn new(display_hz: f32) -> Self {
        let pinned = std::env::var("FUNCTOR_SPIKE_MS")
            .ok()
            .and_then(|ms| ms.parse::<f64>().ok())
            .filter(|ms| *ms > 0.0);
        let mut telemetry = Self {
            uploads_seen: crate::gpu_counters::gpu_counters().bytes_uploaded_total(),
            ring: vec![FrameRecord::default(); RING_FRAMES].into_boxed_slice(),
            next: 0,
            len: 0,
            threshold_us: 0,
            threshold_pinned: pinned.is_some(),
            spikes: [Spike::default(); SPIKES],
            spikes_total: 0,
        };
        match pinned {
            Some(ms) => telemetry.threshold_us = (ms * 1000.0) as u32,
            None => telemetry.set_display_hz(display_hz),
        }
        telemetry
    }

    /// Follow a display rate change (unless `FUNCTOR_SPIKE_MS` pins it).
    pub fn set_display_hz(&mut self, hz: f32) {
        if !self.threshold_pinned && hz > 0.0 {
            self.threshold_us = (SPIKE_PERIODS * 1e6 / f64::from(hz)) as u32;
        }
    }

    /// Copy the last rendered frame's GPU counters into `record`.
    pub fn record_gpu(&mut self, record: &mut FrameRecord) {
        let counters = crate::gpu_counters::gpu_counters();
        let uploaded = counters.bytes_uploaded_total();
        record.upload_bytes = uploaded - self.uploads_seen;
        self.uploads_seen = uploaded;
        record.instance_uploads = counters.frame_instance_uploads().count as u32;
        let cull = counters.frame_cull();
        record.cull_tested = cull.tested as u32;
        record.cull_culled = cull.culled as u32;
    }

    /// Record one frame. Returns a note when it is a spike, for the shell to
    /// log.
    pub fn push(&mut self, record: FrameRecord) -> Option<SpikeNote> {
        for spike in self.spikes.iter_mut().take(self.spikes_total.min(SPIKES as u64) as usize) {
            if spike.len < WINDOW && spike.len - spike.at <= SPIKE_CONTEXT {
                spike.frames[spike.len] = record;
                spike.len += 1;
            }
        }
        let note = (record.frame_us > self.threshold_us).then(|| {
            let (stage, stage_us) = dominant(&record);
            let mut spike = Spike {
                stage,
                stage_us,
                ..Spike::default()
            };
            for before in self.newest(SPIKE_CONTEXT).rev() {
                spike.frames[spike.len] = before;
                spike.len += 1;
            }
            spike.at = spike.len;
            spike.frames[spike.len] = record;
            spike.len += 1;
            self.spikes[(self.spikes_total % SPIKES as u64) as usize] = spike;
            self.spikes_total += 1;
            SpikeNote {
                frame: record.frame,
                frame_us: record.frame_us,
                stage,
                stage_us,
            }
        });
        self.ring[self.next] = record;
        self.next = (self.next + 1) % RING_FRAMES;
        self.len = (self.len + 1).min(RING_FRAMES);
        note
    }

    /// The newest `n` records, newest first.
    fn newest(&self, n: usize) -> impl DoubleEndedIterator<Item = FrameRecord> + '_ {
        (1..=n.min(self.len)).map(|back| self.ring[(self.next + RING_FRAMES - back) % RING_FRAMES])
    }

    /// The ring's stage distributions; `None` before the first frame.
    pub fn stats(&self) -> Option<FrameStats> {
        (self.len > 0).then(|| {
            let mut values = [0u32; RING_FRAMES];
            let stages = SUMMARIZED
                .iter()
                .map(|(name, field)| {
                    let values = &mut values[..self.len];
                    for (value, record) in values.iter_mut().zip(self.newest(self.len)) {
                        *value = field(&record);
                    }
                    values.sort_unstable();
                    // Nearest rank.
                    let rank = |p: usize| values[(self.len * p).div_ceil(100).max(1) - 1];
                    let stats = StagePercentiles {
                        p50_us: rank(50),
                        p99_us: rank(99),
                        max_us: values[self.len - 1],
                    };
                    (name.to_string(), stats)
                })
                .collect();
            FrameStats {
                frames: self.len as u32,
                spikes: self.spikes_total,
                spike_threshold_us: self.threshold_us,
                stages,
            }
        })
    }

    /// The newest `last` frames (all when `None`), with the ring's stats.
    pub fn frames(&self, last: Option<usize>) -> FramesReport {
        let mut frames: Vec<FrameRecord> = self.newest(last.unwrap_or(RING_FRAMES)).collect();
        frames.reverse();
        FramesReport {
            stats: self.stats().unwrap_or_default(),
            frames,
        }
    }

    /// The kept spike records, oldest first.
    pub fn spikes(&self) -> SpikesReport {
        let kept = self.spikes_total.min(SPIKES as u64) as usize;
        let oldest = (self.spikes_total as usize).saturating_sub(kept);
        let spikes = (oldest..self.spikes_total as usize)
            .map(|n| {
                let spike = &self.spikes[n % SPIKES];
                SpikeRecord {
                    frame: spike.frames[spike.at],
                    stage: spike.stage.to_string(),
                    stage_us: spike.stage_us,
                    frames: spike.frames[..spike.len].to_vec(),
                }
            })
            .collect();
        SpikesReport {
            threshold_us: self.threshold_us,
            total: self.spikes_total,
            spikes,
        }
    }
}

/// ATrace sections, shown by Perfetto/systrace under the `app` category.
#[cfg(target_os = "android")]
mod atrace {
    use std::ffi::{c_char, CStr};

    use super::SimStage;

    #[link(name = "android")]
    extern "C" {
        fn ATrace_beginSection(name: *const c_char);
        fn ATrace_endSection();
    }

    fn name(stage: SimStage) -> &'static CStr {
        match stage {
            SimStage::Update => c"functor.update",
            SimStage::Tick => c"functor.tick",
            SimStage::Physics => c"functor.physics",
            SimStage::PhysicsReconcile => c"functor.physics.reconcile",
            SimStage::PhysicsStep => c"functor.physics.step",
            SimStage::Draw => c"functor.draw",
        }
    }

    pub fn begin(stage: SimStage) {
        // SAFETY: a NUL-terminated static string; sections nest per thread
        // and `Span` ends each on the thread that began it.
        unsafe { ATrace_beginSection(name(stage).as_ptr()) }
    }

    pub fn end() {
        // SAFETY: paired with `begin` by `Span`.
        unsafe { ATrace_endSection() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(frame: u64, frame_us: u32) -> FrameRecord {
        FrameRecord {
            frame,
            frame_us,
            ..FrameRecord::default()
        }
    }

    fn telemetry() -> FrameTelemetry {
        let mut t = FrameTelemetry::new(60.0);
        t.threshold_pinned = false;
        t.set_display_hz(100.0); // 15 ms threshold
        t
    }

    #[test]
    fn ring_keeps_the_newest_frames_oldest_first() {
        let mut t = telemetry();
        for frame in 0..(RING_FRAMES as u64 + 5) {
            t.push(record(frame, 10_000));
        }
        let report = t.frames(Some(3));
        let frames: Vec<u64> = report.frames.iter().map(|r| r.frame).collect();
        let last = RING_FRAMES as u64 + 4;
        assert_eq!(frames, [last - 2, last - 1, last]);
        assert_eq!(t.frames(None).frames.len(), RING_FRAMES);
        assert_eq!(report.stats.frames, RING_FRAMES as u32);
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let mut t = telemetry();
        for us in 1..=100 {
            t.push(record(us.into(), us));
        }
        let frame = t.stats().unwrap().stages["frame"];
        assert_eq!(frame, StagePercentiles { p50_us: 50, p99_us: 99, max_us: 100 });
        assert!(FrameTelemetry::new(60.0).stats().is_none());
    }

    #[test]
    fn a_spike_names_its_dominant_stage_and_keeps_its_neighbours() {
        let mut t = telemetry();
        for frame in 0..10 {
            assert_eq!(t.push(record(frame, 10_000)), None);
        }
        let spike = FrameRecord {
            physics_us: 160_000,
            physics_reconcile_us: 150_000,
            physics_step_us: 5_000,
            tick_us: 2_000,
            sim_us: 170_000,
            ..record(10, 172_300)
        };
        let note = t.push(spike).expect("over the threshold");
        assert_eq!(note.stage, "physics.reconcile");
        assert_eq!(
            note.to_string(),
            "spike frame 10: 172.3 ms (physics.reconcile 150.0 ms)"
        );
        for frame in 11..20 {
            t.push(record(frame, 10_000));
        }
        let report = t.spikes();
        assert_eq!(report.total, 1);
        let kept = &report.spikes[0];
        assert_eq!(kept.frame, spike);
        let window: Vec<u64> = kept.frames.iter().map(|r| r.frame).collect();
        assert_eq!(window, (6..=14).collect::<Vec<_>>());
    }

    #[test]
    fn spike_ring_keeps_the_newest() {
        let mut t = telemetry();
        for frame in 0..(SPIKES as u64 + 3) {
            t.push(record(frame, 50_000));
        }
        let report = t.spikes();
        assert_eq!(report.total, SPIKES as u64 + 3);
        let frames: Vec<u64> = report.spikes.iter().map(|s| s.frame.frame).collect();
        assert_eq!(frames, (3..SPIKES as u64 + 3).collect::<Vec<_>>());
    }

    #[test]
    fn spans_accumulate_per_stage_and_suspended_work_is_dropped() {
        take_sim();
        drop(span(SimStage::Tick));
        {
            let _dry_run = Suspended::new();
            drop(span(SimStage::Draw));
            note_reconcile(3, 1);
        }
        note_world_size(7, 9);
        let stages = take_sim();
        assert_eq!(stages.draw_ns, 0);
        assert_eq!((stages.bodies_created, stages.bodies, stages.colliders), (0, 7, 9));
        assert_eq!(take_sim(), SimStages::default());
    }
}
