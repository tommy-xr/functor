use fable_library_rust::String_::LrcStr;

#[derive(Debug)]
pub enum TextureDescription {
    File(String),
}

impl TextureDescription {
    pub fn file(s: LrcStr) -> TextureDescription {
        TextureDescription::File(s.to_string())
    }
}
