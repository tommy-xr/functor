use fable_library_rust::NativeArray_::Array;
use fable_library_rust::String_::LrcStr;
use serde::{Deserialize, Serialize};

use super::MaterialDescription;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelHandle {
    File(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MeshSelector {
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MeshOverride {
    Material(MaterialDescription),
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

    pub fn with_overrides(
        model: &ModelDescription,
        overrides: Array<(MeshSelector, MeshOverride)>,
    ) -> ModelDescription {
        ModelDescription {
            handle: model.handle.clone(),
            overrides: overrides.to_vec(),
        }
    }
}
