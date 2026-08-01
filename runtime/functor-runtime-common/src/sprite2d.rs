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

    /// The ideal, continuous aspect-fitted rectangle shared by rendering and
    /// pointer mapping.
    ///
    /// Coordinates are bottom-left-origin like [`Viewport`]. Keeping this
    /// calculation in floating point makes its half-open bounds authoritative
    /// for logical pointer coordinates and invariant across device-pixel
    /// ratios. [`Self::fitted_viewport`] rounds only when GL requires an
    /// integer raster viewport, which may move a physical edge by a subpixel
    /// in logical space.
    fn fitted_rect(&self, width: f32, height: f32) -> Option<[f32; 4]> {
        if width <= 0.0
            || height <= 0.0
            || !width.is_finite()
            || !height.is_finite()
            || self.width <= 0.0
            || self.height <= 0.0
            || !self.width.is_finite()
            || !self.height.is_finite()
        {
            return None;
        }
        let camera_aspect = self.width / self.height;
        if camera_aspect <= 0.0 || !camera_aspect.is_finite() {
            return None;
        }
        let surface_aspect = width / height;
        if surface_aspect > camera_aspect {
            let fitted_width = height * camera_aspect;
            Some([(width - fitted_width) * 0.5, 0.0, fitted_width, height])
        } else {
            let fitted_height = width / camera_aspect;
            Some([0.0, (height - fitted_height) * 0.5, width, fitted_height])
        }
    }

    /// Fit the camera's declared aspect inside `viewport`, preserving its
    /// bottom-left offset for sub-rectangle views (stereo, per eye).
    pub(crate) fn fitted_viewport(&self, viewport: Viewport) -> Viewport {
        let Some([_, _, width, height]) =
            self.fitted_rect(viewport.width as f32, viewport.height as f32)
        else {
            return viewport;
        };
        let fitted_width = (width.round() as u32).clamp(1, viewport.width);
        let fitted_height = (height.round() as u32).clamp(1, viewport.height);
        Viewport::with_offset(
            viewport.x + (viewport.width - fitted_width) / 2,
            viewport.y + (viewport.height - fitted_height) / 2,
            fitted_width,
            fitted_height,
        )
    }

    /// Map a top-left-origin logical surface point into this camera's world.
    ///
    /// Returns `None` for points in aspect-fit letterbox/pillarbox bars, for
    /// points outside the surface, or for a degenerate surface. Surface
    /// dimensions must share the point's coordinate space (window points on
    /// desktop, CSS pixels on web), not framebuffer pixels. Bar rejection uses
    /// the ideal half-open logical fit; it deliberately does not inherit the
    /// renderer's device-pixel rounding.
    pub fn to_world(
        &self,
        x: f32,
        y: f32,
        surface_width: f32,
        surface_height: f32,
    ) -> Option<[f32; 2]> {
        if !x.is_finite()
            || !y.is_finite()
            || self.zoom <= 0.0
            || !self.zoom.is_finite()
            || !self.center[0].is_finite()
            || !self.center[1].is_finite()
        {
            return None;
        }
        let [left, bottom, width, height] = self.fitted_rect(surface_width, surface_height)?;
        let top = surface_height - bottom - height;
        if x < left || x >= left + width || y < top || y >= top + height {
            return None;
        }
        let normalized_x = (x - left) / width;
        let normalized_y = (y - top) / height;
        Some([
            self.center[0] + (normalized_x - 0.5) * self.width / self.zoom,
            self.center[1] + (0.5 - normalized_y) * self.height / self.zoom,
        ])
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

    #[test]
    fn to_world_maps_matching_surface_with_top_left_input() {
        let camera = Camera2D::new(16.0, 9.0);
        assert_eq!(
            camera.to_world(800.0, 450.0, 1600.0, 900.0),
            Some([0.0, 0.0])
        );
        assert_eq!(camera.to_world(0.0, 0.0, 1600.0, 900.0), Some([-8.0, 4.5]));
    }

    #[test]
    fn to_world_rejects_letterbox_and_pillarbox_bars() {
        let camera = Camera2D::new(16.0, 9.0);
        // A square surface has horizontal bars: ideal fitted content is the
        // half-open interval y=[218.75, 781.25), so 781.25 is the first
        // rejected coordinate.
        assert_eq!(camera.to_world(500.0, 100.0, 1000.0, 1000.0), None);
        assert_eq!(camera.to_world(500.0, 218.74, 1000.0, 1000.0), None);
        assert!(camera
            .to_world(500.0, 218.75, 1000.0, 1000.0)
            .is_some());
        assert_eq!(camera.to_world(500.0, 781.25, 1000.0, 1000.0), None);
        assert_eq!(
            camera.to_world(500.0, 500.0, 1000.0, 1000.0),
            Some([0.0, 0.0])
        );

        // A 2:1 surface has vertical bars: fitted content is
        // x=111.11..1888.89.
        assert_eq!(camera.to_world(50.0, 500.0, 2000.0, 1000.0), None);
        assert_eq!(
            camera.to_world(1000.0, 500.0, 2000.0, 1000.0),
            Some([0.0, 0.0])
        );
    }

    #[test]
    fn to_world_applies_pan_and_zoom() {
        let camera = Camera2D::new(20.0, 10.0)
            .with_center(4.0, -3.0)
            .with_zoom(2.0);
        assert_eq!(
            camera.to_world(100.0, 50.0, 200.0, 100.0),
            Some([4.0, -3.0])
        );
        assert_eq!(camera.to_world(0.0, 0.0, 200.0, 100.0), Some([-1.0, -0.5]));
    }

    #[test]
    fn to_world_is_retina_scale_invariant_and_tracks_resize() {
        let camera = Camera2D::new(16.0, 9.0);
        let one_x = camera.to_world(300.0, 400.0, 900.0, 1000.0);
        let retina = camera.to_world(600.0, 800.0, 1800.0, 2000.0);
        assert_eq!(one_x, retina);

        // Pin the intentional raster-boundary distinction: GL must round this
        // 506.25-high ideal fit to integer pixels, while logical picking keeps
        // the continuous [246.875, 753.125) interval. Scaling both the point
        // and logical surface leaves even that boundary-near pick unchanged.
        assert_eq!(
            camera.fitted_viewport(Viewport::new(900, 1000)),
            Viewport::with_offset(0, 247, 900, 506)
        );
        let logical_edge = camera.to_world(450.0, 753.0, 900.0, 1000.0);
        let scaled_edge = camera.to_world(900.0, 1506.0, 1800.0, 2000.0);
        assert!(logical_edge.is_some());
        assert_eq!(logical_edge, scaled_edge);

        let before = camera.to_world(450.0, 500.0, 900.0, 1000.0);
        let after = camera.to_world(800.0, 450.0, 1600.0, 900.0);
        assert_eq!(before, Some([0.0, 0.0]));
        assert_eq!(after, Some([0.0, 0.0]));
    }

    #[test]
    fn to_world_rejects_degenerate_and_outside_surfaces() {
        let camera = Camera2D::new(16.0, 9.0);
        assert_eq!(camera.to_world(0.0, 0.0, 0.0, 900.0), None);
        assert_eq!(camera.to_world(-0.25, 0.0, 1600.0, 900.0), None);
        assert_eq!(camera.to_world(-1.0, 0.0, 1600.0, 900.0), None);
        assert_eq!(camera.to_world(1600.0, 0.0, 1600.0, 900.0), None);
        assert_eq!(camera.to_world(f32::NAN, 0.0, 1600.0, 900.0), None);
    }
}
