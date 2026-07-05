//! Rigid-body physics — the Phase 1a "world spine" of `docs/physics.md`
//! (Phase 1b adds the `Simulatable`/`Timeline` rewind seam on top).
//!
//! Physics is *described, not commanded*: the game declares the bodies that
//! should exist as a [`PhysicsScene`] (the physics analogue of `Scene3D` /
//! `AudioScene`), and the shell reconciles that declaration against a live
//! Rapier [`World`] each frame. The Rapier world is a cache/accelerator living
//! runtime-side — it is never stored in the game model; the model holds plain
//! data and the world is reconstructible from a snapshot or a replay.
//!
//! Determinism contract (see the Determinism section of the doc): **local,
//! single-binary determinism** — same build, same declared-scene history, same
//! fixed steps → byte-identical snapshots. Rapier provides this with default
//! features; what this module must uphold is a fixed timestep (never variable
//! dt) and a fully deterministic insert/remove history (Rapier arena handles
//! depend on it), which is why reconciliation orders despawns by tag and keeps
//! spawns in declaration order.
//!
//! No F# surface yet — that lands in Phase 2 (`physicsScape`). Everything here
//! is exercised headlessly by the determinism goldens (`cargo test`, no GPU).

mod driver;
mod registry;
mod scene;
mod timeline;
mod world;

#[cfg(test)]
mod goldens;

// `driver::SteppedPhysics` is the production drive: the recorded wrapper the
// MLE shells call instead of `World::step_frame` directly (Phase 6).
pub use driver::*;
pub use registry::*;
pub use scene::*;
pub use timeline::*;
pub use world::*;
