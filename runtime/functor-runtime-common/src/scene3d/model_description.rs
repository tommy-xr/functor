use cgmath::Matrix4;
use fable_library_rust::String_::LrcStr;
use serde::{Deserialize, Serialize};

use super::MaterialDescription;

use crate::scene3d::deserialize_matrix;
use crate::scene3d::serialize_matrix;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelHandle {
    File(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MeshSelector {
    All,
}

impl MeshSelector {
    pub fn all() -> MeshSelector {
        MeshSelector::All
    }
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

impl MeshOverride {
    pub fn material(description: MaterialDescription) -> MeshOverride {
        MeshOverride::Material(description)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDescription {
    pub handle: ModelHandle,
    pub overrides: Vec<(MeshSelector, MeshOverride)>,
}

impl ModelDescription {
    pub fn file(s: LrcStr) -> ModelDescription {
        ModelDescription {
            handle: ModelHandle::File(s.to_string()),
            overrides: Vec::new(),
        }
    }

    pub fn modify(
        model: ModelDescription,
        selector: MeshSelector,
        override_: MeshOverride,
    ) -> ModelDescription {
        let mut overrides = model.overrides.clone();
        overrides.push((selector, override_));
        ModelDescription {
            handle: model.handle,
            overrides,
        }
    }
}
