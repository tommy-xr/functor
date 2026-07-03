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

    /// Split this camera into (left, right) eye cameras for stereo rendering,
    /// separated by `ipd` (inter-pupillary distance, world units) along the
    /// camera's right vector. Both eye and target shift together, keeping the
    /// two gaze axes parallel (no toe-in — convergence comes from the viewer's
    /// eyes, not the projection). Falls back to the original camera for a
    /// degenerate gaze (eye == target, or looking exactly along `up`), so a
    /// bad frame can't inject NaNs into the view matrix.
    pub fn stereo_eyes(&self, ipd: f32) -> (Camera, Camera) {
        let forward = vec3(
            self.target[0] - self.eye[0],
            self.target[1] - self.eye[1],
            self.target[2] - self.eye[2],
        );
        let right = forward.cross(vec3(self.up[0], self.up[1], self.up[2]));
        let len = (right.x * right.x + right.y * right.y + right.z * right.z).sqrt();
        if !len.is_normal() {
            return (self.clone(), self.clone());
        }
        let half = ipd / 2.0;
        let shift = [
            right.x / len * half,
            right.y / len * half,
            right.z / len * half,
        ];
        let shifted = |sign: f32| Camera {
            eye: [
                self.eye[0] + sign * shift[0],
                self.eye[1] + sign * shift[1],
                self.eye[2] + sign * shift[2],
            ],
            target: [
                self.target[0] + sign * shift[0],
                self.target[1] + sign * shift[1],
                self.target[2] + sign * shift[2],
            ],
            ..self.clone()
        };
        (shifted(-1.0), shifted(1.0))
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

    // Eyes must sit `ipd` apart along the camera's right vector, with both
    // gaze axes parallel to the original — the property that makes SBS stereo
    // fuse correctly instead of toeing in.
    #[test]
    fn stereo_eyes_are_ipd_apart_with_parallel_gaze() {
        // Looking down +Z from the origin, up = +Y, so right = -X… verify by
        // construction instead: separation vector must have length ipd and be
        // perpendicular to both forward and up.
        let cam = Camera::look_at(
            [1.0, 2.0, 3.0],
            [4.0, 2.5, 7.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(60.0),
        );
        let ipd = 0.064;
        let (l, r) = cam.stereo_eyes(ipd);

        let sep = [
            r.eye[0] - l.eye[0],
            r.eye[1] - l.eye[1],
            r.eye[2] - l.eye[2],
        ];
        let sep_len = (sep[0] * sep[0] + sep[1] * sep[1] + sep[2] * sep[2]).sqrt();
        assert!(approx(sep_len, ipd));

        // Gaze direction identical for both eyes (parallel axes)…
        for i in 0..3 {
            let lg = l.target[i] - l.eye[i];
            let rg = r.target[i] - r.eye[i];
            let og = cam.target[i] - cam.eye[i];
            assert!(approx(lg, og));
            assert!(approx(rg, og));
        }
        // …and the separation is horizontal (perpendicular to up).
        assert!(approx(sep[1], 0.0));
        // Original camera sits at the midpoint.
        for i in 0..3 {
            assert!(approx((l.eye[i] + r.eye[i]) / 2.0, cam.eye[i]));
        }
    }

    // A degenerate gaze (target == eye, or forward parallel to up) must not
    // produce NaN cameras — fall back to the mono camera for that frame.
    #[test]
    fn stereo_eyes_degenerate_gaze_falls_back_to_mono() {
        let stacked = Camera::look_at(
            [0.0, 0.0, 0.0],
            [0.0, 5.0, 0.0], // looking straight along up
            [0.0, 1.0, 0.0],
            Angle::from_degrees(45.0),
        );
        let zero = Camera::look_at(
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0], // eye == target
            [0.0, 1.0, 0.0],
            Angle::from_degrees(45.0),
        );
        for cam in [stacked, zero] {
            let (l, r) = cam.stereo_eyes(0.064);
            for v in [l.eye, l.target, r.eye, r.target] {
                assert!(v.iter().all(|c| c.is_finite()));
            }
            assert_eq!(l.eye, cam.eye);
            assert_eq!(r.eye, cam.eye);
        }
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
