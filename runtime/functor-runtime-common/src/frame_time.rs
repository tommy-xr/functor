#[derive(Clone)]
pub struct FrameTime {
    // tts - total time in seconds
    pub tts: f64,

    // dts - delta time in seconds for this frame
    pub dts: f64,
}
