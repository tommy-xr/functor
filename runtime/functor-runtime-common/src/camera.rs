use cgmath::{perspective, vec3, Matrix4, Point3, Rad};
use serde::{Deserialize, Serialize};

use crate::math::Angle;

/// A camera description produced by the game's `draw3d`. Stores plain scalars
/// (so it serializes cleanly across the wasm boundary); the runtime turns it
/// into view/projection matrices at render time via `view_matrix` /
/// `projection_matrix`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Camera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
    /// Vertical field of view, in radians.
    pub fov_radians: f32,
    pub near: f32,
    pub far: f32,
}

impl Camera {
    /// Look from `eye` toward `target`. `fov` is the vertical field of view.
    pub fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3], fov: Angle) -> Camera {
        let fov: Rad<f32> = fov.into();
        Camera {
            eye,
            target,
            up,
            fov_radians: fov.0,
            near: 0.1,
            far: 100.0,
        }
    }

    /// First-person camera looking from `eye` along a direction given by `yaw`
    /// (rotation about +Y) and `pitch` (rotation about the local X axis).
    /// yaw = 0, pitch = 0 looks down +Z; positive pitch looks up.
    pub fn first_person(eye: [f32; 3], yaw: Angle, pitch: Angle, fov: Angle) -> Camera {
        let yaw: Rad<f32> = yaw.into();
        let pitch: Rad<f32> = pitch.into();
        let (sin_yaw, cos_yaw) = yaw.0.sin_cos();
        let (sin_pitch, cos_pitch) = pitch.0.sin_cos();
        let forward = [cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw];
        let target = [
            eye[0] + forward[0],
            eye[1] + forward[1],
            eye[2] + forward[2],
        ];
        Camera::look_at(eye, target, [0.0, 1.0, 0.0], fov)
    }

    pub fn view_matrix(&self) -> Matrix4<f32> {
        Matrix4::look_at_rh(
            Point3::new(self.eye[0], self.eye[1], self.eye[2]),
            Point3::new(self.target[0], self.target[1], self.target[2]),
            vec3(self.up[0], self.up[1], self.up[2]),
        )
    }

    pub fn projection_matrix(&self, aspect: f32) -> Matrix4<f32> {
        perspective(Rad(self.fov_radians), aspect, self.near, self.far)
    }
}

impl Default for Camera {
    fn default() -> Camera {
        Camera::look_at(
            [0.0, 0.0, -5.0],
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(45.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    // Resizing should reveal more of the scene horizontally, not stretch it:
    // a wider aspect leaves the vertical scale untouched and shrinks the
    // horizontal scale proportionally (cgmath puts f/aspect in m[0][0], f in
    // m[1][1]). This is the property that makes window/canvas resize correct.
    #[test]
    fn wider_aspect_widens_horizontally_without_stretching() {
        let cam = Camera::default();
        let square = cam.projection_matrix(1.0);
        let wide = cam.projection_matrix(2.0);

        // Vertical scale unchanged across aspect ratios.
        assert!(approx(wide.y.y, square.y.y));
        // Horizontal scale shrinks by exactly the aspect ratio (so geometry
        // keeps its proportions; you just see more to the sides).
        assert!(approx(wide.x.x, square.x.x / 2.0));
    }

    #[test]
    fn projection_uses_viewport_aspect() {
        let cam = Camera::default();
        let viewport = crate::Viewport::new(1600, 400); // aspect 4.0
        let direct = cam.projection_matrix(4.0);
        let via_viewport = cam.projection_matrix(viewport.aspect());
        assert!(approx(direct.x.x, via_viewport.x.x));
        assert!(approx(direct.y.y, via_viewport.y.y));
    }
}
