use fable_library_rust::String_::LrcStr;
use serde::{Deserialize, Serialize};

/// The six faces of a cubemap skybox, by asset path (+X, -X, +Y, -Y, +Z, -Z —
/// the GL `TEXTURE_CUBE_MAP_POSITIVE_X + i` upload order). Frame-level: drawn
/// behind everything right after the pass's clear, so render-target inner
/// frames can carry their own sky. While the faces load, the pass's clear
/// color shows; a face that fails to load disables the skybox with one
/// warning. Fog does NOT apply to the skybox — it IS the horizon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkyboxDescription {
    pub px: String,
    pub nx: String,
    pub py: String,
    pub ny: String,
    pub pz: String,
    pub nz: String,
}

impl SkyboxDescription {
    pub fn new(
        px: impl Into<String>,
        nx: impl Into<String>,
        py: impl Into<String>,
        ny: impl Into<String>,
        pz: impl Into<String>,
        nz: impl Into<String>,
    ) -> SkyboxDescription {
        SkyboxDescription {
            px: px.into(),
            nx: nx.into(),
            py: py.into(),
            ny: ny.into(),
            pz: pz.into(),
            nz: nz.into(),
        }
    }

    /// F# boundary constructor (the `TextureDescription::file` shim style).
    pub fn files(
        px: LrcStr,
        nx: LrcStr,
        py: LrcStr,
        ny: LrcStr,
        pz: LrcStr,
        nz: LrcStr,
    ) -> SkyboxDescription {
        SkyboxDescription::new(
            px.to_string(),
            nx.to_string(),
            py.to_string(),
            ny.to_string(),
            pz.to_string(),
            nz.to_string(),
        )
    }

    /// The faces in GL upload order (`TEXTURE_CUBE_MAP_POSITIVE_X + i`).
    pub fn faces(&self) -> [&str; 6] {
        [&self.px, &self.nx, &self.py, &self.ny, &self.pz, &self.nz]
    }
}

// The skybox draw's shaders (used by `SceneContext::draw_skybox`). The unit
// cube's positions double as the cubemap sample direction; the view matrix
// arrives translation-stripped, so the box is glued to the camera.
pub const SKYBOX_VERTEX_SHADER_SOURCE: &str = r#"
        layout (location = 0) in vec3 inPos;

        uniform mat4 view;        // rotation-only (translation stripped CPU-side)
        uniform mat4 projection;

        out vec3 texDir;

        void main() {
            texDir = inPos;
            vec4 pos = projection * view * vec4(inPos, 1.0);
            // z = w -> NDC depth exactly 1.0: never near/far-clipped regardless
            // of the camera's near plane (the cube's size is irrelevant), and
            // it passes the LEQUAL depth test against the cleared 1.0.
            gl_Position = pos.xyww;
        }
"#;

pub const SKYBOX_FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec3 texDir;

        uniform samplerCube skybox;

        void main() {
            fragColor = texture(skybox, texDir);
        }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_are_in_gl_upload_order() {
        let sky = SkyboxDescription::new("px", "nx", "py", "ny", "pz", "nz");
        assert_eq!(sky.faces(), ["px", "nx", "py", "ny", "pz", "nz"]);
    }
}
