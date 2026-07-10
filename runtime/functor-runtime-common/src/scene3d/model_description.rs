use cgmath::Matrix4;
use serde::{Deserialize, Serialize};

use super::MaterialDescription;

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
}
