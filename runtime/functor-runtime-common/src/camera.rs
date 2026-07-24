use cgmath::{perspective, vec3, InnerSpace, Matrix4, Point3, Quaternion, Rad, Rotation, Vector3};
use serde::{Deserialize, Serialize};

use crate::math::Angle;

/// A pose reported by a tracking system, in its right-handed local space.
///
/// The orientation is a unit quaternion in `[x, y, z, w]` order. Tracking
/// local `+X` is right, `+Y` is up, and `-Z` is forward, matching OpenXR.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackingPose {
    pub position: [f32; 3],
    pub orientation: [f32; 4],
}

impl TrackingPose {
    pub const IDENTITY: TrackingPose = TrackingPose {
        position: [0.0, 0.0, 0.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };

    pub fn new(position: [f32; 3], orientation: [f32; 4]) -> Self {
        Self {
            position,
            orientation,
        }
    }

    /// The center pose between two tracked eyes. Position is the midpoint;
    /// orientation is the shortest-path halfway rotation. Returns `None` for
    /// non-finite positions or invalid quaternions.
    pub fn midpoint(left: Self, right: Self) -> Option<Self> {
        let lp = left.position_vector()?;
        let rp = right.position_vector()?;
        let lq = left.unit_orientation()?;
        let mut rq = right.unit_orientation()?;
        if lq.dot(rq) < 0.0 {
            rq = -rq;
        }
        let orientation = lq.slerp(rq, 0.5).normalize();
        let position = (lp + rp) * 0.5;
        Some(Self::from_parts(position, orientation))
    }

    /// Express `self` relative to a tracking-space reference pose.
    ///
    /// The result is target-neutral rig-local data: its origin/orientation are
    /// the reference center eye, with OpenXR's +X right, +Y up, -Z forward
    /// convention. Controller snapshots use this form so games can map poses
    /// through the authored camera that belongs to the same model update.
    pub fn relative_to(self, reference: Self) -> Option<Self> {
        let (position, orientation) = Self::relative_parts(reference, self)?;
        Some(Self::from_parts(position, orientation))
    }

    fn position_vector(self) -> Option<Vector3<f32>> {
        self.position
            .iter()
            .all(|component| component.is_finite())
            .then(|| Vector3::from(self.position))
    }

    fn unit_orientation(self) -> Option<Quaternion<f32>> {
        let [x, y, z, w] = self.orientation;
        let q = Quaternion::new(w, x, y, z);
        let magnitude2 = q.magnitude2();
        (self
            .orientation
            .iter()
            .all(|component| component.is_finite())
            && magnitude2.is_normal())
        .then(|| q / magnitude2.sqrt())
    }

    fn from_parts(position: Vector3<f32>, orientation: Quaternion<f32>) -> Self {
        Self {
            position: position.into(),
            orientation: [
                orientation.v.x,
                orientation.v.y,
                orientation.v.z,
                orientation.s,
            ],
        }
    }

    fn relative_parts(reference: Self, tracked: Self) -> Option<(Vector3<f32>, Quaternion<f32>)> {
        let reference_position = reference.position_vector()?;
        let tracked_position = tracked.position_vector()?;
        let inverse_reference = reference.unit_orientation()?.conjugate();
        let tracked_orientation = tracked.unit_orientation()?;
        Some((
            inverse_reference.rotate_vector(tracked_position - reference_position),
            inverse_reference * tracked_orientation,
        ))
    }
}

/// A tracked pose mapped into the authored camera's world-space rig.
///
/// Controller/hand input can use the same mapping as the eye cameras, keeping
/// rendered hands and head motion in one coordinate system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MappedTrackingPose {
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
}

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

    /// Map a live tracking pose into this authored camera's local rig.
    ///
    /// `reference` is the tracking-space center-eye pose captured when the rig
    /// is established. At that pose, the mapped position and orientation equal
    /// this camera exactly. Subsequent room-scale translation and rotation are
    /// applied in the camera's local right/up/backward basis, so moving the
    /// authored camera moves the whole rig without discarding live tracking.
    pub fn map_tracking_pose(
        &self,
        reference: TrackingPose,
        tracked: TrackingPose,
    ) -> Option<MappedTrackingPose> {
        let eye = Vector3::from(self.eye);
        let target = Vector3::from(self.target);
        let authored_up = Vector3::from(self.up);
        let gaze = target - eye;
        let gaze_distance = gaze.magnitude();
        let right = gaze.cross(authored_up);
        if !eye.x.is_finite()
            || !eye.y.is_finite()
            || !eye.z.is_finite()
            || !gaze_distance.is_normal()
            || !right.magnitude().is_normal()
        {
            return None;
        }

        let forward = gaze / gaze_distance;
        let right = right.normalize();
        let up = right.cross(forward).normalize();
        let backward = -forward;
        let to_world = |v: Vector3<f32>| right * v.x + up * v.y + backward * v.z;

        let (local_offset, local_orientation) = TrackingPose::relative_parts(reference, tracked)?;
        let world_position = eye + to_world(local_offset);
        let world_forward =
            to_world(local_orientation.rotate_vector(-Vector3::unit_z())).normalize();
        let world_up = to_world(local_orientation.rotate_vector(Vector3::unit_y())).normalize();

        Some(MappedTrackingPose {
            position: world_position.into(),
            forward: world_forward.into(),
            up: world_up.into(),
        })
    }

    /// Compose one tracked eye onto this authored camera. Invalid tracking or
    /// a degenerate authored camera falls back to the authored camera for this
    /// frame rather than injecting NaNs into the renderer.
    ///
    /// The authored camera keeps its target distance, field-of-view value, and
    /// clip range. XR shells override only the optical projection with the
    /// runtime's exact per-eye FOV.
    pub fn compose_tracked_view(&self, reference: TrackingPose, tracked: TrackingPose) -> Camera {
        let Some(mapped) = self.map_tracking_pose(reference, tracked) else {
            return self.clone();
        };
        let distance = (Vector3::from(self.target) - Vector3::from(self.eye)).magnitude();
        let position = Vector3::from(mapped.position);
        let target = position + Vector3::from(mapped.forward) * distance;
        Camera {
            eye: mapped.position,
            target: target.into(),
            up: mapped.up,
            ..self.clone()
        }
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

    /// Build an asymmetric perspective projection from four view-space field-
    /// of-view angles. This is the projection form supplied per eye by XR
    /// runtimes, where the optical center usually does not sit at the middle
    /// of the render target.
    ///
    /// Angles are measured from the forward axis: left/down are negative and
    /// right/up are positive. The resulting matrix uses OpenGL's right-handed
    /// view convention and `[-1, 1]` clip-space depth, matching
    /// [`view_matrix`](Self::view_matrix) and the shared GLES renderer.
    pub fn projection_matrix_from_fov_angles(
        &self,
        angle_left: f32,
        angle_right: f32,
        angle_down: f32,
        angle_up: f32,
    ) -> Matrix4<f32> {
        let left = angle_left.tan();
        let right = angle_right.tan();
        let down = angle_down.tan();
        let up = angle_up.tan();
        let width = right - left;
        let height = up - down;
        let depth = self.far - self.near;

        // Column-major OpenGL projection. Writing this directly (instead of
        // cgmath::frustum) also preserves OpenXR's valid reversed-angle form,
        // where a negative width or height intentionally flips the image.
        Matrix4::new(
            2.0 / width,
            0.0,
            0.0,
            0.0,
            0.0,
            2.0 / height,
            0.0,
            0.0,
            (right + left) / width,
            (up + down) / height,
            -(self.far + self.near) / depth,
            -1.0,
            0.0,
            0.0,
            -(2.0 * self.far * self.near) / depth,
            0.0,
        )
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
    fn symmetric_fov_angles_match_the_standard_perspective() {
        let cam = Camera::default();
        let aspect = 1.37;
        let half_vertical = cam.fov_radians / 2.0;
        let half_horizontal = (half_vertical.tan() * aspect).atan();
        let symmetric = cam.projection_matrix(aspect);
        let from_angles = cam.projection_matrix_from_fov_angles(
            -half_horizontal,
            half_horizontal,
            -half_vertical,
            half_vertical,
        );

        let symmetric: &[f32; 16] = symmetric.as_ref();
        let from_angles: &[f32; 16] = from_angles.as_ref();
        for (actual, expected) in from_angles.iter().zip(symmetric.iter()) {
            assert!(approx(*actual, *expected), "{actual} != {expected}");
        }
    }

    #[test]
    fn asymmetric_fov_edges_map_to_clip_space_edges() {
        use cgmath::Vector4;

        let cam = Camera::default();
        let angles = (-0.82_f32, 0.71_f32, -0.76_f32, 0.88_f32);
        let projection =
            cam.projection_matrix_from_fov_angles(angles.0, angles.1, angles.2, angles.3);
        let near = cam.near;
        let project = |x: f32, y: f32| {
            let clip = projection * Vector4::new(x, y, -near, 1.0);
            [clip.x / clip.w, clip.y / clip.w]
        };

        assert!(approx(project(near * angles.0.tan(), 0.0)[0], -1.0));
        assert!(approx(project(near * angles.1.tan(), 0.0)[0], 1.0));
        assert!(approx(project(0.0, near * angles.2.tan())[1], -1.0));
        assert!(approx(project(0.0, near * angles.3.tan())[1], 1.0));
    }

    #[test]
    fn reversed_fov_angles_preserve_requested_clip_space_flip() {
        use cgmath::Vector4;

        let cam = Camera::default();
        let left = 0.71_f32;
        let right = -0.82_f32;
        let projection = cam.projection_matrix_from_fov_angles(left, right, -0.76, 0.88);
        let project_x = |angle: f32| {
            let clip = projection * Vector4::new(cam.near * angle.tan(), 0.0, -cam.near, 1.0);
            clip.x / clip.w
        };

        assert!(approx(project_x(left), -1.0));
        assert!(approx(project_x(right), 1.0));
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
    fn identity_tracking_pose_preserves_the_authored_camera() {
        let camera = Camera::look_at(
            [3.0, 2.0, -4.0],
            [7.0, 3.0, 2.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(67.0),
        );
        let composed = camera.compose_tracked_view(TrackingPose::IDENTITY, TrackingPose::IDENTITY);

        for i in 0..3 {
            assert!(approx(composed.eye[i], camera.eye[i]));
            assert!(approx(composed.target[i], camera.target[i]));
        }
        let authored_matrix = camera.view_matrix();
        let composed_matrix = composed.view_matrix();
        let authored_view: &[f32; 16] = authored_matrix.as_ref();
        let composed_view: &[f32; 16] = composed_matrix.as_ref();
        for (actual, expected) in composed_view.iter().zip(authored_view.iter()) {
            assert!(approx(*actual, *expected), "{actual} != {expected}");
        }
        assert_eq!(composed.fov_radians, camera.fov_radians);
        assert_eq!(composed.near, camera.near);
        assert_eq!(composed.far, camera.far);
    }

    #[test]
    fn tracked_eye_offsets_preserve_stereo_about_the_authored_camera() {
        let camera = Camera::first_person(
            [2.0, 3.0, 4.0],
            Angle::from_degrees(0.0),
            Angle::from_degrees(0.0),
            Angle::from_degrees(60.0),
        );
        let half_ipd = 0.032;
        let left_pose = TrackingPose::new([-half_ipd, 1.6, 0.0], [0.0, 0.0, 0.0, 1.0]);
        let right_pose = TrackingPose::new([half_ipd, 1.6, 0.0], [0.0, 0.0, 0.0, 1.0]);
        let reference = TrackingPose::midpoint(left_pose, right_pose).unwrap();
        let left = camera.compose_tracked_view(reference, left_pose);
        let right = camera.compose_tracked_view(reference, right_pose);
        let (expected_left, expected_right) = camera.stereo_eyes(half_ipd * 2.0);

        for i in 0..3 {
            assert!(approx(left.eye[i], expected_left.eye[i]));
            assert!(approx(right.eye[i], expected_right.eye[i]));
            assert!(approx(
                left.target[i] - left.eye[i],
                right.target[i] - right.eye[i]
            ));
        }
    }

    #[test]
    fn tracking_translation_uses_the_authored_camera_basis() {
        let camera = Camera::look_at(
            [10.0, 2.0, 20.0],
            [15.0, 2.0, 20.0], // authored forward = +X
            [0.0, 1.0, 0.0],
            Angle::from_degrees(45.0),
        );
        let tracked = TrackingPose::new([1.0, 0.5, -2.0], [0.0, 0.0, 0.0, 1.0]);
        let mapped = camera
            .map_tracking_pose(TrackingPose::IDENTITY, tracked)
            .unwrap();

        // For an authored +X gaze, local tracking +X/+Y/-Z map to world
        // +Z/+Y/+X respectively.
        assert!(approx(mapped.position[0], 12.0));
        assert!(approx(mapped.position[1], 2.5));
        assert!(approx(mapped.position[2], 21.0));
    }

    #[test]
    fn tracking_rotation_is_relative_to_the_reference_pose() {
        use cgmath::{Deg, Rotation3};

        let camera = Camera::first_person(
            [1.0, 2.0, 3.0],
            Angle::from_degrees(0.0),
            Angle::from_degrees(0.0),
            Angle::from_degrees(45.0),
        );
        let reference_q = Quaternion::from_angle_y(Deg(35.0));
        let relative_q = Quaternion::from_angle_y(Deg(20.0));
        let tracked_q = reference_q * relative_q;
        let pose =
            |q: Quaternion<f32>| TrackingPose::new([0.0, 0.0, 0.0], [q.v.x, q.v.y, q.v.z, q.s]);
        let composed = camera.compose_tracked_view(pose(reference_q), pose(tracked_q));
        let got = (Vector3::from(composed.target) - Vector3::from(composed.eye)).normalize();
        let yaw = 20.0_f32.to_radians();
        let expected = Vector3::new(yaw.sin(), 0.0, yaw.cos());

        assert!(got.dot(expected) > 0.9999, "forward = {got:?}");
    }

    #[test]
    fn tracking_pose_can_be_normalized_to_the_rig_reference() {
        use cgmath::Rotation3;

        let half_turn_y = Quaternion::from_angle_y(Rad(std::f32::consts::PI));
        let pose = |position, orientation: Quaternion<f32>| {
            TrackingPose::new(
                position,
                [
                    orientation.v.x,
                    orientation.v.y,
                    orientation.v.z,
                    orientation.s,
                ],
            )
        };
        let reference = pose([10.0, 2.0, 4.0], half_turn_y);
        // World -X from a 180-degree-yaw reference is rig-local +X.
        let tracked = pose([9.0, 2.5, 4.0], half_turn_y);
        let relative = tracked.relative_to(reference).unwrap();
        assert!(approx(relative.position[0], 1.0));
        assert!(approx(relative.position[1], 0.5));
        assert!(approx(relative.position[2], 0.0));
        assert!(approx(relative.orientation[0], 0.0));
        assert!(approx(relative.orientation[1], 0.0));
        assert!(approx(relative.orientation[2], 0.0));
        assert!(approx(relative.orientation[3].abs(), 1.0));
    }

    #[test]
    fn midpoint_handles_equivalent_opposite_quaternion_signs() {
        let left = TrackingPose::new([-0.03, 1.0, 2.0], [0.0, 0.0, 0.0, 1.0]);
        let right = TrackingPose::new([0.03, 1.0, 2.0], [0.0, 0.0, 0.0, -1.0]);
        let midpoint = TrackingPose::midpoint(left, right).unwrap();

        assert_eq!(midpoint.position, [0.0, 1.0, 2.0]);
        assert!(approx(midpoint.orientation[3].abs(), 1.0));
    }

    #[test]
    fn invalid_tracking_or_authored_camera_falls_back_without_nans() {
        let camera = Camera::default();
        let invalid = TrackingPose::new([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]);
        let fallback = camera.compose_tracked_view(TrackingPose::IDENTITY, invalid);
        assert_eq!(fallback.eye, camera.eye);
        assert_eq!(fallback.target, camera.target);

        let degenerate = Camera::look_at(
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(45.0),
        );
        let fallback =
            degenerate.compose_tracked_view(TrackingPose::IDENTITY, TrackingPose::IDENTITY);
        assert!(fallback.eye.iter().all(|component| component.is_finite()));
        assert!(fallback
            .target
            .iter()
            .all(|component| component.is_finite()));
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
