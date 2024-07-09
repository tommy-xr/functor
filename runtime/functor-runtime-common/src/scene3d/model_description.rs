use fable_library_rust::String_::LrcStr;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelDescription {
    File(String),
}

impl ModelDescription {
    pub fn file(s: LrcStr) -> ModelDescription {
        ModelDescription::File(s.to_string())
    }
}
