use serde::{Deserialize, Serialize};

/// The drawable surface size in pixels. The runtimes query their window /
/// canvas each frame and build one of these; rendering derives the GL viewport
/// and the camera's projection aspect from it, so resizing "just works".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    pub fn new(width: u32, height: u32) -> Viewport {
        Viewport { width, height }
    }

    /// Width-to-height ratio for the perspective projection. Guards against a
    /// zero (or zero-height) surface — which happens transiently while a window
    /// is minimized or a canvas hasn't been laid out yet — so we never feed a
    /// NaN/infinity into the projection matrix.
    pub fn aspect(&self) -> f32 {
        if self.width == 0 || self.height == 0 {
            1.0
        } else {
            self.width as f32 / self.height as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_is_width_over_height() {
        assert_eq!(Viewport::new(800, 600).aspect(), 800.0 / 600.0);
        assert_eq!(Viewport::new(1920, 1080).aspect(), 1920.0 / 1080.0);
    }

    #[test]
    fn aspect_guards_degenerate_sizes() {
        // No NaN/infinity while minimized or before first layout.
        assert_eq!(Viewport::new(800, 0).aspect(), 1.0);
        assert_eq!(Viewport::new(0, 600).aspect(), 1.0);
        assert_eq!(Viewport::new(0, 0).aspect(), 1.0);
        assert!(Viewport::new(1024, 0).aspect().is_finite());
    }
}
