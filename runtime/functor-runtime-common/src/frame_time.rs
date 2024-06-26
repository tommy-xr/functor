use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct FrameTime {
    // tts - total time in seconds
    pub tts: f32,

    // dts - delta time in seconds for this frame
    pub dts: f32,
}
