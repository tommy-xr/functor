use cgmath::{ortho, Matrix4};
use serde::{Deserialize, Serialize};

use crate::{math::Angle, Camera, Scene3D, Viewport};

/// A center-origin, Y-up camera for a 2D sprite layer.
///
/// `width` × `height` is the visible world extent at zoom 1. The renderer
/// preserves that aspect ratio and letterboxes inside the shell viewport, so
/// sprites never stretch when the window or canvas changes shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera2D {
    pub width: f32,
    pub height: f32,
    pub center: [f32; 2],
    pub zoom: f32,
}

impl Camera2D {
    pub fn new(width: f32, height: f32) -> Camera2D {
        Camera2D {
            width,
            height,
            center: [0.0, 0.0],
            zoom: 1.0,
        }
    }

    pub fn with_center(mut self, x: f32, y: f32) -> Camera2D {
        self.center = [x, y];
        self
    }

    pub fn with_zoom(mut self, zoom: f32) -> Camera2D {
        self.zoom = zoom;
        self
    }

    /// The ordinary 3D camera supplying the sprite pass's view transform.
    /// Its perspective fields are unused because the pass supplies
    /// [`Camera2D::projection_matrix`] explicitly.
    pub(crate) fn render_camera(&self) -> Camera {
        Camera::look_at(
            [self.center[0], self.center[1], 1.0],
            [self.center[0], self.center[1], 0.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(45.0),
        )
    }

    pub(crate) fn projection_matrix(&self) -> Matrix4<f32> {
        let half_width = self.width / (2.0 * self.zoom);
        let half_height = self.height / (2.0 * self.zoom);
        ortho(
            -half_width,
            half_width,
            -half_height,
            half_height,
            0.1,
            10.0,
        )
    }

    /// Fit the camera's declared aspect inside `viewport`, preserving its
    /// bottom-left offset for stereo/netsim panes.
    pub(crate) fn fitted_viewport(&self, viewport: Viewport) -> Viewport {
        if viewport.width == 0 || viewport.height == 0 {
            return viewport;
        }
        let camera_aspect = self.width / self.height;
        let viewport_aspect = viewport.aspect();
        if viewport_aspect > camera_aspect {
            let width =
                ((viewport.height as f32 * camera_aspect).round() as u32).clamp(1, viewport.width);
            Viewport::with_offset(
                viewport.x + (viewport.width - width) / 2,
                viewport.y,
                width,
                viewport.height,
            )
        } else {
            let height =
                ((viewport.width as f32 / camera_aspect).round() as u32).clamp(1, viewport.height);
            Viewport::with_offset(
                viewport.x,
                viewport.y + (viewport.height - height) / 2,
                viewport.width,
                height,
            )
        }
    }
}

/// One ordered 2D pass attached to a frame. Layers render after the 3D pass,
/// in declaration order; later layers appear on top.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpriteLayer {
    pub camera: Camera2D,
    pub scene: Scene3D,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_viewport_is_pillarboxed_without_losing_its_offset() {
        let fitted =
            Camera2D::new(16.0, 9.0).fitted_viewport(Viewport::with_offset(100, 20, 2000, 900));
        assert_eq!(fitted, Viewport::with_offset(300, 20, 1600, 900));
    }

    #[test]
    fn tall_viewport_is_letterboxed() {
        let fitted = Camera2D::new(16.0, 9.0).fitted_viewport(Viewport::new(900, 1000));
        assert_eq!(fitted, Viewport::with_offset(0, 247, 900, 506));
    }

    #[test]
    fn extreme_aspects_keep_a_visible_pixel() {
        let very_tall = Camera2D::new(1.0, f32::MAX).fitted_viewport(Viewport::new(100, 100));
        assert_eq!(very_tall.width, 1);
        assert_eq!(very_tall.height, 100);

        let very_wide = Camera2D::new(f32::MAX, 1.0).fitted_viewport(Viewport::new(100, 100));
        assert_eq!(very_wide.width, 100);
        assert_eq!(very_wide.height, 1);
    }
}
