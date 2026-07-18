use cgmath::Matrix4;
use serde::{Deserialize, Serialize};

use super::MaterialDescription;

use crate::anim::AnimExpr;
use crate::scene3d::deserialize_matrix;
use crate::scene3d::serialize_matrix;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelHandle {
    File(String),
}

// Per-mesh selectors/overrides remain part of the serialized `ModelDescription`
// shape the renderer consumes; the F#-era authoring constructors
// (`MeshSelector::all` / `MeshOverride::material` / `ModelDescription::modify`)
// were removed with the F# framework — Functor Lang builds models with empty `overrides`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MeshSelector {
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MeshOverride {
    Material(MaterialDescription),
    #[serde(
        serialize_with = "serialize_matrix",
        deserialize_with = "deserialize_matrix"
    )]
    Transform(Matrix4<f32>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDescription {
    pub handle: ModelHandle,
    pub overrides: Vec<(MeshSelector, MeshOverride)>,
    /// The pose to render — `Scene.animate`'s expression. `None` keeps the
    /// legacy zero-config behavior: the first clip auto-plays on the game
    /// clock.
    #[serde(default)]
    pub animation: Option<AnimExpr>,
    /// Placeholder model locators tried in order WHILE `handle` is still
    /// loading (`Asset.whilePending`): the first loaded entry renders until
    /// the primary is ready. A FAILED primary renders the empty fallback
    /// like before — failure is not pending, and must stay visible. Skipped
    /// on the wire when empty, so plain models keep their JSON shape.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub while_pending: Vec<String>,
}
