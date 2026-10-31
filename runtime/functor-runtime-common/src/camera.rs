use cgmath::{perspective, vec3, InnerSpace, Matrix4, Point3, Quaternion, Rad, Rotation, Vector3};
use serde::{Deserialize, Serialize};

use crate::math::Angle;
use crate::runtime_settings::{floor_locked_requested, passthrough_requested, GameConfig};

/// A pose reported by a tracking system, in its right-handed local space.
///
/// The orientation is a unit quaternion in `[x, y, z, w]` order. Tracking
/// local `+X` is right, `+Y` is up, and `-Z` is forward, matching OpenXR.
///
/// Deserialization fills missing fields from [`TrackingPose::IDENTITY`], so a
/// scripted/injected pose can name only the part it cares about.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, deny_unknown_fields)]
pub struct TrackingPose {
    pub position: [f32; 3],
    pub orientation: [f32; 4],
}

/// The identity pose — NOT a field-wise zero: an all-zero quaternion is not a
/// rotation. Hand-written for that reason, and so partial debug-injected poses
/// (`{"position":[…]}`) default to an upright, forward-facing orientation.
impl Default for TrackingPose {
    fn default() -> Self {
        TrackingPose::IDENTITY
    }
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

    /// This pose stood upright (yaw only) at `height` above the tracking
    /// floor — the rig reference for room-locked mixed reality. Centering on
    /// it, in a floor-origin (STAGE) space, maps the real floor to world
    /// `y = 0` under a level authored camera whose eye is `height` high, and
    /// keeps the room level however the head was tilted when it was taken.
    /// Idempotent for a fixed `height`.
    /// Gazing near-vertically (a look down at a desk), the heading comes from
    /// the top of the head instead. `None` only for an invalid pose.
    pub fn floor_locked(self, height: f32) -> Option<Self> {
        let position = self.position_vector()?;
        let q = self.unit_orientation()?;
        let forward = q.rotate_vector(-Vector3::unit_z());
        let crown = q.rotate_vector(Vector3::unit_y()) * -forward.y.signum();
        let orientation = heading(forward).or_else(|| heading(crown))?;
        Some(Self::from_parts(
            Vector3::new(position.x, height, position.z),
            orientation,
        ))
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

/// The standing eye height, in metres above the floor, that a floor-locked rig
/// assumes where tracking has no floor, and the desktop `--emulate-xr` head's
/// default.
pub const STANDING_EYE_HEIGHT: f32 = 1.6;

/// The reference tracked poses are taken relative to, from the center-eye
/// pose the rig centred on. Shells derive it every frame, so a `config`
/// change applies without a recentre; it is also idempotent.
///
/// - `Xr.floorLocked`: the floor under the head, level and yaw-only.
///   `Frame.camera` stands for that floor point, so rig-local `head.y` is the
///   eye's height above the floor, and a recentre moves only yaw and XZ. A
///   floor-origin (STAGE) space puts the floor at tracking `y = 0`. Without
///   one (LOCAL, whose origin is the head at launch or recentre), the floor is
///   assumed [`STANDING_EYE_HEIGHT`] below that origin.
/// - `Xr.passthrough` on STAGE: level and yaw-only at `authored_eye_y`, so the
///   real floor is world `y = 0` under a level authored camera.
/// - Otherwise the pose itself, tilt included.
pub fn rig_reference(
    centre: TrackingPose,
    config: Option<GameConfig>,
    authored_eye_y: Option<f32>,
    floor_origin: bool,
) -> TrackingPose {
    let height = if floor_locked_requested(config) {
        Some(if floor_origin {
            0.0
        } else {
            -STANDING_EYE_HEIGHT
        })
    } else if floor_origin && passthrough_requested(config) {
        authored_eye_y
    } else {
        None
    };
    height
        .and_then(|height| centre.floor_locked(height))
        .unwrap_or(centre)
}

/// Reduce an orientation to its yaw-only component, for a recenter
/// reference. Recentering must zero *where you're facing* but keep pitch/roll
/// gravity-anchored — a full-pose reference taken with a tilted head would
/// bake that tilt in as a permanently crooked horizon. Returns `None` when the
/// gaze is within ~11° of vertical (yaw is ill-defined staring at the
/// ceiling); callers keep or retake their reference.
pub fn yaw_only(q: Quaternion<f32>) -> Option<Quaternion<f32>> {
    heading(q.rotate_vector(-Vector3::unit_z()))
}

/// The yaw about +Y that turns -Z toward `f`'s horizontal part; `None` when
/// `f` is within ~11° of vertical.
fn heading(f: Vector3<f32>) -> Option<Quaternion<f32>> {
    use cgmath::Rotation3;
    if (f.x * f.x + f.z * f.z).sqrt() < 0.2 {
        return None;
    }
    // At identity the gaze is -z; from_angle_y(θ) carries -z to
    // (-sin θ, 0, -cos θ), so θ recovers as atan2(-x, -z).
    Some(Quaternion::from_angle_y(Rad(f32::atan2(-f.x, -f.z))))
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

impl MappedTrackingPose {
    /// The rigid transform placing an authored local frame at this pose:
    /// origin at `position`, local `+Z` along `forward`, `+Y` along `up`
    /// (re-orthogonalized against `forward`). Right-handed, so local `+X`
    /// lands on `up × forward`, the pose's left.
    ///
    /// `None` for non-finite input, a zero `forward`/`up`, or an `up` within
    /// ~0.06° of parallel to `forward` (no defined roll).
    pub fn to_matrix(&self) -> Option<Matrix4<f32>> {
        let position = Vector3::from(self.position);
        let forward = Vector3::from(self.forward);
        let up = Vector3::from(self.up);
        let finite = position.x.is_finite() && position.y.is_finite() && position.z.is_finite();
        if !finite || !forward.magnitude().is_normal() || !up.magnitude().is_normal() {
            return None;
        }
        let z = forward.normalize();
        let x = up.normalize().cross(z);
        if !(x.magnitude() > 1e-3) {
            return None;
        }
        let x = x.normalize();
        let y = z.cross(x);
        Some(Matrix4::from_cols(
            x.extend(0.0),
            y.extend(0.0),
            z.extend(0.0),
            position.extend(1.0),
        ))
    }
}

/// A world-space ray derived from a point on a camera's logical surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldRay {
    pub origin: [f32; 3],
    pub direction: [f32; 3],
}

/// The authored camera's normalized world-space axes.
///
/// Kept crate-private so tracking, picking, and debug visualization share the
/// exact same handedness and degeneracy rules without exposing cgmath through
/// the public protocol surface.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CameraBasis {
    pub(crate) eye: Vector3<f32>,
    pub(crate) forward: Vector3<f32>,
    pub(crate) right: Vector3<f32>,
    pub(crate) up: Vector3<f32>,
}

/// A camera description produced by the game's `draw3d`. Stores plain scalars
/// (so it serializes cleanly across the wasm boundary); the runtime turns it
/// into view/projection matrices at render time via `view_matrix` /
/// `projection_matrix`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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

    /// A camera at `position` looking along `forward`, rolled so `up` points
    /// up on screen (e.g. a `MappedTrackingPose`). The authored basis matches
    /// [`Camera::first_person`]: forward `+Z` with up `+Y` is yaw = pitch = 0.
    ///
    /// `forward` need not be unit length; `up` is re-orthogonalized against
    /// it. Returns `None` for non-finite input, a zero `forward`/`up`, or an
    /// `up` within ~0.06° of parallel to `forward` (no defined roll).
    pub fn from_pose(position: [f32; 3], forward: [f32; 3], up: [f32; 3], fov: Angle) -> Option<Camera> {
        let eye = Vector3::from(position);
        let forward = Vector3::from(forward);
        let up = Vector3::from(up);
        let finite = |v: Vector3<f32>| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
        if !finite(eye) || !forward.magnitude().is_normal() || !up.magnitude().is_normal() {
            return None;
        }
        let forward = forward.normalize();
        let right = forward.cross(up.normalize());
        if !(right.magnitude() > 1e-3) {
            return None;
        }
        let up = right.normalize().cross(forward);
        Some(Camera::look_at(eye.into(), (eye + forward).into(), up.into(), fov))
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

    /// Return the normalized authored axes, or `None` when they cannot define
    /// a finite view basis.
    pub(crate) fn world_basis(&self) -> Option<CameraBasis> {
        let eye = Vector3::from(self.eye);
        let gaze = Vector3::from(self.target) - eye;
        let authored_up = Vector3::from(self.up);
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
        Some(CameraBasis {
            eye,
            forward,
            right,
            up,
        })
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
        let CameraBasis {
            eye,
            forward,
            right,
            up,
        } = self.world_basis()?;
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

    /// Map a top-left-origin logical surface point through this camera's
    /// perspective projection.
    ///
    /// The ray begins at the camera eye and its direction is normalized.
    /// Position and surface extent must share one coordinate space (window
    /// points or CSS pixels), so scaling both by a device-pixel ratio leaves
    /// the result unchanged. Returns `None` for an outside/degenerate surface
    /// point or a degenerate camera.
    pub fn to_world_ray(
        &self,
        x: f32,
        y: f32,
        surface_width: f32,
        surface_height: f32,
    ) -> Option<WorldRay> {
        if !x.is_finite()
            || !y.is_finite()
            || !surface_width.is_finite()
            || !surface_height.is_finite()
            || !(self.fov_radians > 0.0 && self.fov_radians < std::f32::consts::PI)
            || surface_width <= 0.0
            || surface_height <= 0.0
            || x < 0.0
            || y < 0.0
            || x >= surface_width
            || y >= surface_height
        {
            return None;
        }

        let CameraBasis {
            eye,
            forward,
            right,
            up,
        } = self.world_basis()?;
        let aspect = surface_width / surface_height;
        let half_fov_tan = (self.fov_radians * 0.5).tan();
        if !aspect.is_normal()
            || !half_fov_tan.is_normal()
            || half_fov_tan <= 0.0
        {
            return None;
        }

        let ndc_x = 2.0 * x / surface_width - 1.0;
        let ndc_y = 1.0 - 2.0 * y / surface_height;
        let direction = (forward
            + right * (ndc_x * half_fov_tan * aspect)
            + up * (ndc_y * half_fov_tan))
            .normalize();
        (direction.x.is_finite() && direction.y.is_finite() && direction.z.is_finite())
            .then_some(WorldRay {
                origin: eye.into(),
                direction: direction.into(),
            })
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
        // An infinite far plane (OpenXR depth images) takes the limit.
        let (z_scale, z_offset) = if self.far.is_finite() {
            (
                -(self.far + self.near) / depth,
                -(2.0 * self.far * self.near) / depth,
            )
        } else {
            (-1.0, -2.0 * self.near)
        };

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
            z_scale,
            -1.0,
            0.0,
            0.0,
            z_offset,
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
    use cgmath::Rotation3;

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
    fn center_surface_point_rays_along_the_camera_gaze() {
        let camera = Camera::look_at(
            [4.0, 2.0, -3.0],
            [7.0, 4.0, 5.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(55.0),
        );
        let ray = camera.to_world_ray(800.0, 450.0, 1600.0, 900.0).unwrap();
        let expected = (Vector3::from(camera.target) - Vector3::from(camera.eye)).normalize();
        let direction = Vector3::from(ray.direction);

        assert_eq!(ray.origin, camera.eye);
        assert!(direction.dot(expected) > 0.99999);
        assert!(approx(direction.magnitude(), 1.0));
    }

    #[test]
    fn surface_corners_follow_top_left_input_and_projection_aspect() {
        let camera = Camera::first_person(
            [0.0, 0.0, 0.0],
            Angle::from_degrees(0.0),
            Angle::from_degrees(0.0),
            Angle::from_degrees(90.0),
        );
        let top_left = Vector3::from(
            camera
                .to_world_ray(0.0, 0.0, 200.0, 100.0)
                .unwrap()
                .direction,
        );
        let near_bottom_right = Vector3::from(
            camera
                .to_world_ray(199.999, 99.999, 200.0, 100.0)
                .unwrap()
                .direction,
        );

        // Looking down +Z means this camera's screen-right axis is -X.
        assert!(top_left.x > 0.0);
        assert!(top_left.y > 0.0);
        assert!(top_left.z > 0.0);
        assert!(near_bottom_right.x < 0.0);
        assert!(near_bottom_right.y < 0.0);
        assert!(near_bottom_right.z > 0.0);
        // The 2:1 surface has twice the horizontal half-span at a 90° fov.
        assert!(approx((top_left.x / top_left.z).abs(), 2.0));
        assert!(approx((top_left.y / top_left.z).abs(), 1.0));
    }

    #[test]
    fn world_ray_is_logical_scale_invariant_and_tracks_resize() {
        let camera = Camera::default();
        let one_x = camera.to_world_ray(300.0, 200.0, 900.0, 1000.0).unwrap();
        let retina = camera
            .to_world_ray(600.0, 400.0, 1800.0, 2000.0)
            .unwrap();
        for i in 0..3 {
            assert!(approx(one_x.direction[i], retina.direction[i]));
        }

        let resized = camera.to_world_ray(800.0, 450.0, 1600.0, 900.0).unwrap();
        let expected = (Vector3::from(camera.target) - Vector3::from(camera.eye)).normalize();
        assert!(Vector3::from(resized.direction).dot(expected) > 0.99999);
    }

    #[test]
    fn world_ray_projects_back_to_the_requested_surface_point() {
        use cgmath::Vector4;

        let camera = Camera::look_at(
            [4.0, 2.0, -3.0],
            [7.0, 4.0, 5.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(55.0),
        );
        let (x, y, width, height) = (123.0, 456.0, 1600.0, 900.0);
        let ray = camera.to_world_ray(x, y, width, height).unwrap();
        let point = Vector3::from(ray.origin) + Vector3::from(ray.direction) * 10.0;
        let clip = camera.projection_matrix(width / height)
            * camera.view_matrix()
            * Vector4::new(point.x, point.y, point.z, 1.0);

        assert!(approx(clip.x / clip.w, 2.0 * x / width - 1.0));
        assert!(approx(clip.y / clip.w, 1.0 - 2.0 * y / height));
    }

    #[test]
    fn world_ray_rejects_outside_surfaces_and_degenerate_cameras() {
        let camera = Camera::default();
        assert!(camera.to_world_ray(0.0, 0.0, 0.0, 900.0).is_none());
        assert!(camera.to_world_ray(-0.1, 0.0, 1600.0, 900.0).is_none());
        assert!(camera
            .to_world_ray(1600.0, 450.0, 1600.0, 900.0)
            .is_none());
        assert!(camera
            .to_world_ray(800.0, 900.0, 1600.0, 900.0)
            .is_none());

        let zero_gaze = Camera::look_at(
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(45.0),
        );
        let parallel_up = Camera::look_at(
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(45.0),
        );
        assert!(zero_gaze
            .to_world_ray(10.0, 10.0, 100.0, 100.0)
            .is_none());
        assert!(parallel_up
            .to_world_ray(10.0, 10.0, 100.0, 100.0)
            .is_none());

        for degrees in [0.0, 180.0, 360.0, 450.0] {
            let invalid_fov = Camera::look_at(
                [0.0, 0.0, -5.0],
                [0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                Angle::from_degrees(degrees),
            );
            assert!(invalid_fov
                .to_world_ray(50.0, 50.0, 100.0, 100.0)
                .is_none());
        }
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
    fn an_infinite_far_plane_maps_near_to_minus_one_and_infinity_to_one() {
        use cgmath::Vector4;

        let cam = Camera {
            near: 0.1,
            far: f32::INFINITY,
            ..Camera::default()
        };
        let projection = cam.projection_matrix_from_fov_angles(-0.8, 0.8, -0.8, 0.8);
        let ndc_z = |distance: f32| {
            let clip = projection * Vector4::new(0.0, 0.0, -distance, 1.0);
            clip.z / clip.w
        };
        assert!(approx(ndc_z(0.1), -1.0));
        assert!(approx(ndc_z(2.0), 1.0 - 2.0 * 0.1 / 2.0));
        assert!(ndc_z(1.0e6) < 1.0 && ndc_z(1.0e6) > 0.999);
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
    fn floor_locked_keeps_heading_and_drops_tilt() {
        // Head 1.7 m up, turned 90° left (forward = -X) and pitched down 30°.
        let yaw = Quaternion::from_angle_y(Rad(std::f32::consts::FRAC_PI_2));
        let pitch = Quaternion::from_angle_x(Rad(-std::f32::consts::FRAC_PI_6));
        let q = yaw * pitch;
        let head = TrackingPose::new([0.3, 1.7, -0.2], [q.v.x, q.v.y, q.v.z, q.s]);
        let locked = head.floor_locked(1.6).unwrap();
        assert_eq!(locked.position, [0.3, 1.6, -0.2]);
        let forward = locked
            .unit_orientation()
            .unwrap()
            .rotate_vector(-Vector3::unit_z());
        assert!((forward - Vector3::new(-1.0, 0.0, 0.0)).magnitude() < 1e-5);
        // Idempotent, and the stage floor maps to rig-local y = -height.
        let again = locked.floor_locked(1.6).unwrap();
        assert_eq!(again.position, locked.position);
        let dot: f32 = (0..4).map(|i| again.orientation[i] * locked.orientation[i]).sum();
        assert!((dot.abs() - 1.0).abs() < 1e-5);
        // Looking straight down at a desk, the crown of the head gives the heading.
        let down = yaw * Quaternion::from_angle_x(Rad(-std::f32::consts::FRAC_PI_2));
        let desk = TrackingPose::new([0.0, 1.2, 0.0], [down.v.x, down.v.y, down.v.z, down.s]);
        let forward = desk
            .floor_locked(1.2)
            .unwrap()
            .unit_orientation()
            .unwrap()
            .rotate_vector(-Vector3::unit_z());
        assert!((forward - Vector3::new(-1.0, 0.0, 0.0)).magnitude() < 1e-4);
        let floor = TrackingPose::new([0.3, 0.0, -0.2], locked.orientation);
        let local = floor.relative_to(locked).unwrap();
        assert!((local.position[1] + 1.6).abs() < 1e-6);
    }

    fn floor_locked_config(passthrough: bool) -> Option<GameConfig> {
        Some(GameConfig {
            xr: crate::runtime_settings::XrConfig {
                passthrough,
                floor_locked: true,
                ..Default::default()
            },
            ..GameConfig::default()
        })
    }

    /// A head turned 40° left, pitched 30° down and rolled 10°: how a user
    /// might be standing when the rig centres.
    fn tilted_head(position: [f32; 3]) -> TrackingPose {
        let q = Quaternion::from_angle_y(Rad(40f32.to_radians()))
            * Quaternion::from_angle_x(Rad(-30f32.to_radians()))
            * Quaternion::from_angle_z(Rad(10f32.to_radians()));
        TrackingPose::new(position, [q.v.x, q.v.y, q.v.z, q.s])
    }

    fn up_of(pose: TrackingPose) -> Vector3<f32> {
        pose.unit_orientation()
            .unwrap()
            .rotate_vector(Vector3::unit_y())
    }

    #[test]
    fn floor_locked_rig_levels_a_tilted_head_over_the_floor() {
        let head = tilted_head([0.3, 1.7, -0.2]);
        let reference = rig_reference(head, floor_locked_config(false), Some(1.6), true);
        // Floor y = 0, up = gravity.
        assert_eq!(reference.position, [0.3, 0.0, -0.2]);
        assert!((up_of(reference) - Vector3::unit_y()).magnitude() < 1e-5);
        // Rig-local head.y is the eye height; the floor is rig-local y = 0.
        let local = head.relative_to(reference).unwrap();
        assert!((local.position[1] - 1.7).abs() < 1e-5);
        assert!(local.position[0].abs() < 1e-5 && local.position[2].abs() < 1e-5);
        let floor = TrackingPose::new([1.0, 0.0, 2.0], head.orientation);
        assert!(floor.relative_to(reference).unwrap().position[1].abs() < 1e-5);
        // Through a level camera, the eye is `authored eye + head.y` high.
        let camera = Camera::look_at(
            [5.0, 0.5, 2.0],
            [5.0, 0.5, 1.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(60.0),
        );
        let mapped = camera.map_tracking_pose(reference, head).unwrap();
        assert!((mapped.position[1] - 2.2).abs() < 1e-5);
        // Idempotent: shells re-apply it every frame.
        let again = rig_reference(reference, floor_locked_config(false), Some(1.6), true);
        assert_eq!(again.position, reference.position);
        assert!((up_of(again) - up_of(reference)).magnitude() < 1e-6);
        // It also wins over passthrough's authored-eye height.
        let both = rig_reference(head, floor_locked_config(true), Some(1.6), true);
        assert_eq!(both.position, reference.position);
    }

    #[test]
    fn without_floor_locked_the_rig_keeps_the_full_head_pose() {
        let head = tilted_head([0.3, 1.7, -0.2]);
        for config in [None, Some(GameConfig::default())] {
            for floor_origin in [true, false] {
                let reference = rig_reference(head, config, Some(1.6), floor_origin);
                assert_eq!(reference, head);
                // So head.y starts at 0 and "up" carries the tilt.
                let local = head.relative_to(reference).unwrap();
                assert!(local.position.iter().all(|c| c.abs() < 1e-6));
                assert!((up_of(reference) - Vector3::unit_y()).magnitude() > 0.1);
            }
        }
    }

    #[test]
    fn floor_locked_recentre_keeps_height_and_moves_only_yaw() {
        let config = floor_locked_config(false);
        let first = rig_reference(tilted_head([0.3, 1.7, -0.2]), config, None, true);
        // Recentre while crouched, elsewhere, facing another way, tilted.
        let q = Quaternion::from_angle_y(Rad(-100f32.to_radians()))
            * Quaternion::from_angle_x(Rad(20f32.to_radians()));
        let crouched = TrackingPose::new([1.0, 1.1, 0.5], [q.v.x, q.v.y, q.v.z, q.s]);
        let second = rig_reference(crouched, config, None, true);
        assert_eq!(second.position[1], first.position[1]);
        assert!((up_of(second) - Vector3::unit_y()).magnitude() < 1e-5);
        // Height survives: the crouched head still reads 1.1 m, not 0.
        let local = crouched.relative_to(second).unwrap();
        assert!((local.position[1] - 1.1).abs() < 1e-5);
    }

    #[test]
    fn floor_locked_without_a_stage_assumes_a_standing_eye_height() {
        // LOCAL's origin is the head at launch: the floor is unknown.
        let head = tilted_head([0.0, 0.05, 0.0]);
        let reference = rig_reference(head, floor_locked_config(false), Some(1.6), false);
        assert_eq!(reference.position, [0.0, -STANDING_EYE_HEIGHT, 0.0]);
        assert!((up_of(reference) - Vector3::unit_y()).magnitude() < 1e-5);
        let local = head.relative_to(reference).unwrap();
        assert!((local.position[1] - (STANDING_EYE_HEIGHT + 0.05)).abs() < 1e-5);
        let again = rig_reference(reference, floor_locked_config(false), Some(1.6), false);
        assert_eq!(again.position, reference.position);
    }

    #[test]
    fn passthrough_rig_locks_to_the_authored_eye_on_stage_only() {
        let config = Some(GameConfig {
            xr: crate::runtime_settings::XrConfig {
                passthrough: true,
                ..Default::default()
            },
            ..GameConfig::default()
        });
        let head = tilted_head([0.3, 1.7, -0.2]);
        assert_eq!(
            rig_reference(head, config, Some(1.6), true).position,
            [0.3, 1.6, -0.2]
        );
        assert_eq!(rig_reference(head, config, Some(1.6), false), head);
        assert_eq!(rig_reference(head, config, None, true), head);
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

    fn assert_same_view(a: &Camera, b: &Camera) {
        let (a, b) = (a.view_matrix(), b.view_matrix());
        let (a, b): (&[f32; 16], &[f32; 16]) = (a.as_ref(), b.as_ref());
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 1e-5, "{a:?} != {b:?}");
        }
    }

    /// Where a world point lands on a square viewport, in NDC (+x right, +y up).
    fn ndc(camera: &Camera, p: [f32; 3]) -> (f32, f32) {
        let clip = camera.projection_matrix(1.0) * camera.view_matrix() * Point3::from(p).to_homogeneous();
        (clip.x / clip.w, clip.y / clip.w)
    }

    // The authored pose basis is first_person's: +Z forward and +Y up is
    // yaw = pitch = 0, and yaw/pitch poses reproduce first_person exactly.
    #[test]
    fn from_pose_matches_first_person_for_unrolled_poses() {
        let eye = [2.0, 1.5, -3.0];
        let fov = Angle::from_degrees(12.0);
        let pose = |f: [f32; 3], u: [f32; 3]| Camera::from_pose(eye, f, u, fov).unwrap();
        let fp = |yaw: f32, pitch: f32| {
            Camera::first_person(eye, Angle::from_degrees(yaw), Angle::from_degrees(pitch), fov)
        };
        assert_same_view(&pose([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]), &fp(0.0, 0.0));
        // Unnormalized forward and a non-orthogonal up give the same camera.
        assert_same_view(&pose([0.0, 0.0, 5.0], [0.0, 2.0, 0.3]), &fp(0.0, 0.0));
        let (s, c) = 30f32.to_radians().sin_cos();
        assert_same_view(&pose([s, 0.0, c], [0.0, 1.0, 0.0]), &fp(30.0, 0.0));
        assert_same_view(&pose([0.0, s, c], [0.0, c, -s]), &fp(0.0, 30.0));
        let p = pose([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        assert_eq!(p.fov_radians, fov.radians());
        assert_eq!((p.near, p.far), (0.1, 100.0));
    }

    // Roll: the screen's up follows `up`. Rolled 90° so up is world +X (the
    // left of a +Z gaze), a point to the left shows at the TOP and a point
    // above shows on the RIGHT. A mirrored basis flips one of the two.
    #[test]
    fn from_pose_rolls_the_view_about_forward() {
        let camera = Camera::from_pose([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], Angle::from_degrees(60.0)).unwrap();
        let (x, y) = ndc(&camera, [1.0, 0.0, 5.0]);
        assert!(x.abs() < 1e-5 && y > 0.1, "left point at ({x}, {y})");
        let (x, y) = ndc(&camera, [0.0, 1.0, 5.0]);
        assert!(x > 0.1 && y.abs() < 1e-5, "upper point at ({x}, {y})");

        // Unrolled, for contrast: left is screen-left, above is screen-top.
        let level = Camera::from_pose([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], Angle::from_degrees(60.0)).unwrap();
        assert!(ndc(&level, [1.0, 0.0, 5.0]).0 < -0.1);
        assert!(ndc(&level, [0.0, 1.0, 5.0]).1 > 0.1);
    }

    // Straight up or down is a valid aim (first_person degenerates there):
    // `up` alone sets which way the image is turned.
    #[test]
    fn from_pose_aims_vertically() {
        let fov = Angle::from_degrees(60.0);
        let up = Camera::from_pose([0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0], fov).unwrap();
        let (x, y) = ndc(&up, [0.0, 10.0, 0.0]);
        assert!(x.abs() < 1e-5 && y.abs() < 1e-5);
        assert!(ndc(&up, [0.0, 10.0, -1.0]).1 > 0.05);
        let down = Camera::from_pose([0.0; 3], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], fov).unwrap();
        assert!(ndc(&down, [0.0, -10.0, 1.0]).1 > 0.05);
    }

    #[test]
    fn from_pose_rejects_degenerate_poses() {
        let fov = Angle::from_degrees(60.0);
        let pose = |p: [f32; 3], f: [f32; 3], u: [f32; 3]| Camera::from_pose(p, f, u, fov);
        assert!(pose([0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]).is_none());
        assert!(pose([0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.0, -3.0]).is_none());
        assert!(pose([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1e-4, 1.0]).is_none());
        assert!(pose([0.0; 3], [0.0; 3], [0.0, 1.0, 0.0]).is_none());
        assert!(pose([0.0; 3], [0.0, 0.0, 1.0], [0.0; 3]).is_none());
        assert!(pose([f32::NAN, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]).is_none());
        assert!(pose([0.0; 3], [0.0, f32::INFINITY, 1.0], [0.0, 1.0, 0.0]).is_none());
        // Nearly, but not, parallel still defines a roll.
        assert!(pose([0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.01, 1.0]).is_some());
    }
}
