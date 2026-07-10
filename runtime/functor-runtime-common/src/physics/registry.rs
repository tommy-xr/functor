//! `WorldId`-keyed registry of live physics worlds (docs/physics.md,
//! "Singleton now, explicit worlds later — for free").
//!
//! The `physicsScape`-driven world is [`DEFAULT_WORLD`] (id 0), created lazily
//! on first touch. Every engine call is world-parameterized from day one; the
//! future F# surface just defaults the id, and explicit `PhysicsWorld.t` values
//! later map to [`create_world`] with zero engine refactor.
//!
//! Like the HTTP tagger registry (`net::registry`), this is a thread-local:
//! the game loop is single-threaded, and living outside the game model means
//! it survives hot reloads of the game dylib (the world is shell state, like
//! the renderer and audio voices).

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use super::{World, DEFAULT_GRAVITY};

pub type WorldId = u32;

/// The singleton `physicsScape`-driven world.
pub const DEFAULT_WORLD: WorldId = 0;

thread_local! {
    static WORLDS: RefCell<BTreeMap<WorldId, World>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_ID: Cell<WorldId> = const { Cell::new(1) };
    static ACTIVE_WORLD: Cell<WorldId> = const { Cell::new(DEFAULT_WORLD) };
}

/// The world game-code physics currently resolves against: readbacks
/// (`Physics.position` / `Physics.transformed` / `Physics.raycast`) and queued
/// commands (`Physics.applyImpulse`, …) in
/// the Functor Lang prelude target this world, not [`DEFAULT_WORLD`] directly. Live
/// frames leave it at the default; the dry-run forward-step
/// (docs/time-travel.md T6b) scopes it to a throwaway world via
/// [`ActiveWorldScope`] so ghost frames read and command the projected world.
pub fn active_world() -> WorldId {
    ACTIVE_WORLD.with(|c| c.get())
}

/// RAII scope routing [`active_world`] to `id` until dropped. Nests: the guard
/// restores whatever was active when it was entered (drop order must mirror
/// entry order, which Rust scoping gives for free).
pub struct ActiveWorldScope {
    prev: WorldId,
}

impl ActiveWorldScope {
    pub fn enter(id: WorldId) -> ActiveWorldScope {
        let prev = ACTIVE_WORLD.with(|c| c.replace(id));
        ActiveWorldScope { prev }
    }
}

impl Drop for ActiveWorldScope {
    fn drop(&mut self) {
        ACTIVE_WORLD.with(|c| c.set(self.prev));
    }
}

/// Create an explicit world with its own gravity, returning its id.
pub fn create_world(gravity: [f32; 3]) -> WorldId {
    let id = NEXT_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    });
    WORLDS.with(|w| w.borrow_mut().insert(id, World::new(gravity)));
    id
}

/// Drop a world and all its bodies. Removing [`DEFAULT_WORLD`] resets it — the
/// next touch recreates it empty.
pub fn remove_world(id: WorldId) {
    WORLDS.with(|w| w.borrow_mut().remove(&id));
}

/// Run `f` against the world `id`. [`DEFAULT_WORLD`] is created on first touch
/// (standard gravity — the per-frame reconcile overwrites it anyway); an
/// unknown explicit id yields `None`. Do not call `with_world` from inside `f`
/// (the registry is borrowed for the duration).
pub fn with_world<R>(id: WorldId, f: impl FnOnce(&mut World) -> R) -> Option<R> {
    WORLDS.with(|w| {
        let mut worlds = w.borrow_mut();
        if id == DEFAULT_WORLD {
            worlds
                .entry(DEFAULT_WORLD)
                .or_insert_with(|| World::new(DEFAULT_GRAVITY));
        }
        worlds.get_mut(&id).map(f)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::{Body, PhysicsScene, Shape};

    #[test]
    fn default_world_is_created_on_first_touch() {
        remove_world(DEFAULT_WORLD);
        let frame = with_world(DEFAULT_WORLD, |w| w.frame());
        assert_eq!(frame, Some(0));
    }

    #[test]
    fn unknown_world_is_none_and_created_worlds_are_independent() {
        assert_eq!(with_world(9999, |w| w.frame()), None);

        let a = create_world([0.0, -9.81, 0.0]);
        let b = create_world([0.0, 0.0, 0.0]);
        assert_ne!(a, b);

        with_world(a, |w| {
            w.reconcile(&PhysicsScene::create(
                [0.0, -9.81, 0.0],
                vec![Body::dynamic(
                    "x".to_string(),
                    Shape::Sphere { radius: 0.5 },
                )],
            ));
        });
        assert_eq!(
            with_world(a, |w| w.body_transform("x").is_some()),
            Some(true)
        );
        assert_eq!(
            with_world(b, |w| w.body_transform("x").is_some()),
            Some(false)
        );

        remove_world(a);
        assert_eq!(with_world(a, |w| w.frame()), None);
        remove_world(b);
    }

    #[test]
    fn active_world_defaults_and_scopes_with_nesting() {
        assert_eq!(active_world(), DEFAULT_WORLD);
        let a = create_world([0.0, -9.81, 0.0]);
        let b = create_world([0.0, 0.0, 0.0]);
        {
            let _outer = ActiveWorldScope::enter(a);
            assert_eq!(active_world(), a);
            {
                let _inner = ActiveWorldScope::enter(b);
                assert_eq!(active_world(), b);
            }
            assert_eq!(active_world(), a, "inner scope must restore the outer");
        }
        assert_eq!(active_world(), DEFAULT_WORLD, "scope must restore default");
        remove_world(a);
        remove_world(b);
    }
}
