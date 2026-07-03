//! Declarative physics scene types (docs/physics.md).
//!
//! A [`PhysicsScene`] is the full set of bodies the game wants to exist this
//! frame — what `physicsScape model` will return in Phase 2. Pure, serializable
//! data; it crosses the dylib boundary as JSON (like `AudioScene`), and the
//! per-frame declared-scene history is exactly what a replay re-executes, so
//! these types carry no handles or live state.

use serde::{Deserialize, Serialize};

/// Standard gravity, Y-up (the coordinate convention — see CLAUDE.md).
pub const DEFAULT_GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];

/// Collision shape for a body. Deliberately independent of the render-side
/// `scene3d` shapes: physics extents are gameplay data, not visuals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    /// Axis-aligned box, given as *full* extents (width, height, depth).
    Cuboid { extents: [f32; 3] },
    Sphere { radius: f32 },
    /// Capsule along the local Y axis: a segment of `2 * half_height` with
    /// spherical caps of `radius`.
    Capsule { half_height: f32, radius: f32 },
}

/// How the body participates in simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyKind {
    /// Fully simulated: integrates under gravity, forces, and contacts.
    Dynamic,
    /// Position-driven: moved to the declared pose each frame, pushes dynamic
    /// bodies but is not pushed back (the shape `Remote` bodies take).
    Kinematic,
    /// Never moves ("fixed" — `static` is a reserved word). Ground, walls.
    Fixed,
}

/// Who simulates this body (docs/physics.md, Authority + divergence). Inert in
/// Phase 1 — recorded in the declaration so netcode phases can key off it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Authority {
    /// This instance simulates the body.
    Local,
    /// Another instance owns it; the named source drives the declared pose.
    Remote(String),
}

/// One declared body in a [`PhysicsScene`]. `tag` is the cross-frame identity
/// (like an `AudioSource` key): the same tag across frames is the same live
/// body. The divergence rule in `World::reconcile` compares a body against its
/// previous declaration — only *changed* fields are written to the live world,
/// so a dynamic body the game keeps declaring at its spawn pose still falls
/// freely, while declaring a *new* pose teleports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub tag: String,
    pub kind: BodyKind,
    pub shape: Shape,
    pub position: [f32; 3],
    /// Unit quaternion, `[x, y, z, w]`.
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    /// `None` uses Rapier's shape-density default.
    #[serde(default)]
    pub mass: Option<f32>,
    pub friction: f32,
    pub restitution: f32,
    /// A sensor detects overlaps but produces no contact forces.
    pub sensor: bool,
    pub authority: Authority,
}

impl Body {
    fn new(tag: String, kind: BodyKind, shape: Shape) -> Body {
        Body {
            tag,
            kind,
            shape,
            position: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0, 0.0, 0.0],
            mass: None,
            // Rapier's collider defaults.
            friction: 0.5,
            restitution: 0.0,
            sensor: false,
            authority: Authority::Local,
        }
    }

    /// A simulated body (`Local` authority).
    pub fn dynamic(tag: String, shape: Shape) -> Body {
        Body::new(tag, BodyKind::Dynamic, shape)
    }

    /// A position-driven body (the shape `Remote` bodies take).
    pub fn kinematic(tag: String, shape: Shape) -> Body {
        Body::new(tag, BodyKind::Kinematic, shape)
    }

    /// A body that never moves (ground, walls).
    pub fn fixed(tag: String, shape: Shape) -> Body {
        Body::new(tag, BodyKind::Fixed, shape)
    }

    pub fn at(mut self, position: [f32; 3]) -> Body {
        self.position = position;
        self
    }

    /// Orientation as a unit quaternion `[x, y, z, w]`.
    pub fn facing(mut self, rotation: [f32; 4]) -> Body {
        self.rotation = rotation;
        self
    }

    pub fn with_velocity(mut self, velocity: [f32; 3]) -> Body {
        self.velocity = velocity;
        self
    }

    pub fn with_mass(mut self, mass: f32) -> Body {
        self.mass = Some(mass);
        self
    }

    pub fn with_friction(mut self, friction: f32) -> Body {
        self.friction = friction;
        self
    }

    pub fn with_restitution(mut self, restitution: f32) -> Body {
        self.restitution = restitution;
        self
    }

    pub fn as_sensor(mut self) -> Body {
        self.sensor = true;
        self
    }

    pub fn with_authority(mut self, authority: Authority) -> Body {
        self.authority = authority;
        self
    }
}

/// The full set of bodies the game wants this frame — what `physicsScape model`
/// returns (Phase 2). Reconciled against the live world by `World::reconcile`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsScene {
    pub gravity: [f32; 3],
    pub bodies: Vec<Body>,
}

impl PhysicsScene {
    pub fn create(gravity: [f32; 3], bodies: Vec<Body>) -> PhysicsScene {
        PhysicsScene { gravity, bodies }
    }

    /// No bodies, standard gravity.
    pub fn empty() -> PhysicsScene {
        PhysicsScene {
            gravity: DEFAULT_GRAVITY,
            bodies: Vec::new(),
        }
    }
}
