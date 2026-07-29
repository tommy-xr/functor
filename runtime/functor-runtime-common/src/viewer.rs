use cgmath::{InnerSpace, Quaternion, Rad, Rotation, Rotation3, Vector3};

use crate::{Camera, Camera2D, Frame};

/// The navigation behavior selected from the current frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugCameraMode {
    /// A free-flying 3D camera with mouse look and WASD + Q/E translation.
    Fps,
    /// A pure 2D frame whose sprite cameras pan and zoom together.
    Pan2d,
}

#[derive(Clone, Debug)]
enum DebugView {
    Fps(Camera),
    Pan2d(Vec<Camera2D>),
}

/// A runtime-owned observer camera for inspecting a game from another view.
///
/// Detaching snapshots the current frame, then keeps all navigation in this
/// shell-owned value. The authored frame, game model, recorded input, and
/// replay remain untouched. A 3D or mixed frame gets an FPS view; a pure 2D
/// frame gets pan/zoom over its sprite cameras.
#[derive(Clone, Debug, Default)]
pub struct DebugCamera {
    view: Option<DebugView>,
    focal_distance: f32,
}

impl DebugCamera {
    const LOOK_RADIANS_PER_PIXEL: f32 = 0.005;
    const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 - 0.01;
    const MIN_DISTANCE: f32 = 0.000_001;
    const FPS_UNITS_PER_SECOND_PER_FOCAL_UNIT: f32 = 1.5;
    const MIN_FOV: f32 = 15.0_f32.to_radians();
    const MAX_FOV: f32 = 120.0_f32.to_radians();
    const PAN_FRACTION_PER_PIXEL: f32 = 0.002;
    const PAN_VIEWPORTS_PER_SECOND: f32 = 0.6;
    const MIN_2D_ZOOM: f32 = 0.01;
    const MAX_2D_ZOOM: f32 = 100.0;

    /// Snapshot the frame's main view. Returns false for invalid camera data,
    /// leaving the controller attached.
    pub fn detach(&mut self, frame: &Frame) -> bool {
        if frame.is_pure_2d() {
            if frame
                .sprite_layers
                .iter()
                .any(|layer| !Self::valid_2d_camera(&layer.camera))
            {
                self.reattach();
                return false;
            }
            self.focal_distance = 1.0;
            self.view = Some(DebugView::Pan2d(
                frame
                    .sprite_layers
                    .iter()
                    .map(|layer| layer.camera.clone())
                    .collect(),
            ));
            return true;
        }

        let offset = Vector3::from(frame.camera.target) - Vector3::from(frame.camera.eye);
        let distance = offset.magnitude();
        let up = Vector3::from(frame.camera.up);
        if !distance.is_finite()
            || distance < Self::MIN_DISTANCE
            || !frame.camera.eye.into_iter().all(f32::is_finite)
            || !frame.camera.target.into_iter().all(f32::is_finite)
            || !frame.camera.up.into_iter().all(f32::is_finite)
            || up.magnitude2() <= f32::EPSILON
            || offset.cross(up).magnitude2() <= f32::EPSILON
            || !frame.camera.fov_radians.is_finite()
            || frame.camera.fov_radians <= 0.0
            || frame.camera.fov_radians >= std::f32::consts::PI
            || !frame.camera.near.is_finite()
            || !frame.camera.far.is_finite()
            || frame.camera.near <= 0.0
            || frame.camera.near >= frame.camera.far
        {
            self.reattach();
            return false;
        }
        self.focal_distance = distance;
        self.view = Some(DebugView::Fps(frame.camera.clone()));
        true
    }

    pub fn reattach(&mut self) {
        self.view = None;
    }

    pub fn is_detached(&self) -> bool {
        self.view.is_some()
    }

    pub fn mode(&self) -> Option<DebugCameraMode> {
        match self.view {
            Some(DebugView::Fps(_)) => Some(DebugCameraMode::Fps),
            Some(DebugView::Pan2d(_)) => Some(DebugCameraMode::Pan2d),
            None => None,
        }
    }

    /// A debug view cannot silently change dimensions underneath its snapshot.
    /// Shells reattach when a live game switches between pure 2D and 3D/mixed.
    pub fn is_compatible(&self, frame: &Frame) -> bool {
        match &self.view {
            Some(DebugView::Fps(_)) => !frame.is_pure_2d(),
            Some(DebugView::Pan2d(cameras)) => {
                frame.is_pure_2d() && cameras.len() == frame.sprite_layers.len()
            }
            None => true,
        }
    }

    /// Number of projected frames that can share one pinned observer view.
    ///
    /// A preview is temporal, so compatibility is a prefix: once a future
    /// switches between 2D and 3D (or changes a pure-2D layer count), later
    /// frames must not be stitched back in after the incompatible boundary.
    pub fn compatible_prefix_len<'a>(
        anchor: &Frame,
        frames: impl IntoIterator<Item = &'a Frame>,
    ) -> usize {
        frames
            .into_iter()
            .take_while(|candidate| {
                if anchor.is_pure_2d() {
                    candidate.is_pure_2d()
                        && candidate.sprite_layers.len() == anchor.sprite_layers.len()
                } else {
                    !candidate.is_pure_2d()
                }
            })
            .count()
    }

    /// The effective main 3D view camera. A 2D debug view leaves the frame's
    /// dummy/main 3D camera alone.
    pub fn camera<'a>(&'a self, authored: &'a Camera) -> &'a Camera {
        match &self.view {
            Some(DebugView::Fps(camera)) => camera,
            Some(DebugView::Pan2d(_)) | None => authored,
        }
    }

    /// Overrides for the main frame's ordered 2D layers, when panning a pure
    /// 2D frame. Mixed frames keep their authored overlay cameras.
    pub fn sprite_cameras(&self) -> Option<&[Camera2D]> {
        match &self.view {
            Some(DebugView::Pan2d(cameras)) => Some(cameras),
            Some(DebugView::Fps(_)) | None => None,
        }
    }

    /// Apply pointer-lock motion: mouse-look in 3D, pan in 2D.
    pub fn look(&mut self, dx: f32, dy: f32) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        match &mut self.view {
            Some(DebugView::Fps(camera)) => Self::fps_look(camera, dx, dy),
            Some(DebugView::Pan2d(cameras)) => {
                for camera in cameras {
                    let world_units_per_pixel =
                        camera.height / camera.zoom * Self::PAN_FRACTION_PER_PIXEL;
                    camera.center[0] += dx * world_units_per_pixel;
                    camera.center[1] -= dy * world_units_per_pixel;
                }
            }
            None => {}
        }
    }

    /// Move with WASD and Q/E. FPS motion uses local forward/right and world
    /// up; 2D maps WASD to vertical/horizontal pan and ignores Q/E.
    pub fn move_local(
        &mut self,
        forward: f32,
        right: f32,
        vertical: f32,
        elapsed_seconds: f32,
    ) {
        if !forward.is_finite()
            || !right.is_finite()
            || !vertical.is_finite()
            || !elapsed_seconds.is_finite()
            || elapsed_seconds <= 0.0
        {
            return;
        }
        let elapsed_seconds = elapsed_seconds.min(0.05);
        match &mut self.view {
            Some(DebugView::Fps(camera)) => {
                let forward_axis = Vector3::from(camera.target) - Vector3::from(camera.eye);
                let up = Vector3::from(camera.up);
                if forward_axis.magnitude2() <= f32::EPSILON
                    || up.magnitude2() <= f32::EPSILON
                {
                    return;
                }
                let forward_axis = forward_axis.normalize();
                let right_axis = forward_axis.cross(up);
                if right_axis.magnitude2() <= f32::EPSILON {
                    return;
                }
                let velocity = forward_axis * forward
                    + right_axis.normalize() * right
                    + Vector3::unit_y() * vertical;
                if velocity.magnitude2() <= f32::EPSILON {
                    return;
                }
                let distance = self.focal_distance.max(Self::MIN_DISTANCE)
                    * Self::FPS_UNITS_PER_SECOND_PER_FOCAL_UNIT
                    * elapsed_seconds;
                let translation = velocity.normalize() * distance;
                camera.eye = (Vector3::from(camera.eye) + translation).into();
                camera.target = (Vector3::from(camera.target) + translation).into();
            }
            Some(DebugView::Pan2d(cameras)) => {
                let magnitude = (forward * forward + right * right).sqrt();
                if magnitude <= f32::EPSILON {
                    return;
                }
                let scale = magnitude.max(1.0);
                for camera in cameras {
                    camera.center[0] += right / scale * camera.width / camera.zoom
                        * Self::PAN_VIEWPORTS_PER_SECOND
                        * elapsed_seconds;
                    camera.center[1] += forward / scale * camera.height / camera.zoom
                        * Self::PAN_VIEWPORTS_PER_SECOND
                        * elapsed_seconds;
                }
            }
            None => {}
        }
    }

    /// Wheel zoom: change the debug lens FOV in 3D and `Camera2D.zoom` in 2D.
    /// Positive steps zoom in.
    pub fn zoom(&mut self, steps: f32) {
        if !steps.is_finite() {
            return;
        }
        match &mut self.view {
            Some(DebugView::Fps(camera)) => {
                let proposed = camera.fov_radians * (-steps * 0.08).exp();
                camera.fov_radians = Self::move_toward_band(
                    camera.fov_radians,
                    proposed,
                    Self::MIN_FOV,
                    Self::MAX_FOV,
                );
            }
            Some(DebugView::Pan2d(cameras)) => {
                for camera in cameras {
                    let proposed = camera.zoom * (steps * 0.12).exp();
                    camera.zoom = Self::move_toward_band(
                        camera.zoom,
                        proposed,
                        Self::MIN_2D_ZOOM,
                        Self::MAX_2D_ZOOM,
                    );
                }
            }
            None => {}
        }
    }

    fn fps_look(camera: &mut Camera, dx: f32, dy: f32) {
        let eye = Vector3::from(camera.eye);
        let mut direction = Vector3::from(camera.target) - eye;
        let mut up = Vector3::from(camera.up);
        let distance = direction.magnitude();
        if distance <= f32::EPSILON {
            return;
        }

        let yaw = Quaternion::from_axis_angle(
            Vector3::unit_y(),
            Rad(-dx * Self::LOOK_RADIANS_PER_PIXEL),
        );
        direction = yaw.rotate_vector(direction);
        up = yaw.rotate_vector(up);

        let current_pitch = (direction.y / distance).clamp(-1.0, 1.0).asin();
        let proposed_pitch = current_pitch - dy * Self::LOOK_RADIANS_PER_PIXEL;
        let next_pitch = Self::move_toward_band(
            current_pitch,
            proposed_pitch,
            -Self::MAX_PITCH,
            Self::MAX_PITCH,
        );
        let right_axis = direction.cross(up);
        if right_axis.magnitude2() > f32::EPSILON {
            let pitch = Quaternion::from_axis_angle(
                right_axis.normalize(),
                Rad(next_pitch - current_pitch),
            );
            direction = pitch.rotate_vector(direction);
            up = pitch.rotate_vector(up);
        }

        camera.target = (eye + direction).into();
        camera.up = up.into();
    }

    fn valid_2d_camera(camera: &Camera2D) -> bool {
        camera.width.is_finite()
            && camera.width > 0.0
            && camera.height.is_finite()
            && camera.height > 0.0
            && camera.center.into_iter().all(f32::is_finite)
            && camera.zoom.is_finite()
            && camera.zoom > 0.0
    }

    fn move_toward_band(current: f32, proposed: f32, min: f32, max: f32) -> f32 {
        if proposed < current {
            proposed.max(min.min(current))
        } else if proposed > current {
            proposed.min(max.max(current))
        } else {
            current
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DebugCamera, DebugCameraMode};
    use crate::{math::Angle, Camera, Camera2D, Frame, Scene3D, SpriteLayer};

    fn camera() -> Camera {
        Camera::look_at(
            [0.0, 2.0, -6.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            Angle::from_degrees(60.0),
        )
    }

    fn frame_3d() -> Frame {
        Frame::new(camera(), Scene3D::cube())
    }

    fn frame_2d() -> Frame {
        Frame::new_2d(SpriteLayer {
            camera: Camera2D::new(32.0, 18.0),
            scene: Scene3D::quad(),
        })
    }

    fn distance(camera: &Camera) -> f32 {
        camera
            .eye
            .iter()
            .zip(camera.target)
            .map(|(eye, target)| (eye - target).powi(2))
            .sum::<f32>()
            .sqrt()
    }

    #[test]
    fn frame_kind_selects_fps_pan2d_and_fps_for_mixed_content() {
        let mut debug = DebugCamera::default();
        assert!(debug.detach(&frame_3d()));
        assert_eq!(debug.mode(), Some(DebugCameraMode::Fps));

        assert!(debug.detach(&frame_2d()));
        assert_eq!(debug.mode(), Some(DebugCameraMode::Pan2d));

        let mixed = Frame::with_2d(
            frame_3d(),
            SpriteLayer {
                camera: Camera2D::new(10.0, 10.0),
                scene: Scene3D::quad(),
            },
        );
        assert!(debug.detach(&mixed));
        assert_eq!(debug.mode(), Some(DebugCameraMode::Fps));
    }

    #[test]
    fn attached_view_tracks_authored_and_fps_view_snapshots_it() {
        let frame = frame_3d();
        let mut debug = DebugCamera::default();
        assert!(std::ptr::eq(debug.camera(&frame.camera), &frame.camera));
        assert!(debug.detach(&frame));

        let mut moved_authored = frame.camera.clone();
        moved_authored.eye = [99.0, 99.0, 99.0];
        assert_eq!(debug.camera(&moved_authored).eye, frame.camera.eye);

        debug.reattach();
        assert_eq!(debug.camera(&moved_authored).eye, moved_authored.eye);
    }

    #[test]
    fn fps_look_turns_in_place_and_preserves_focal_distance() {
        let frame = frame_3d();
        let mut debug = DebugCamera::default();
        assert!(debug.detach(&frame));
        let before_distance = distance(debug.camera(&frame.camera));
        let before_eye = debug.camera(&frame.camera).eye;

        debug.look(120.0, -40.0);
        let effective = debug.camera(&frame.camera);
        assert_eq!(effective.eye, before_eye);
        assert_ne!(effective.target, frame.camera.target);
        assert!(
            effective.target[0] < frame.camera.target[0],
            "moving the mouse right must look right"
        );
        assert!(
            effective.target[1] > frame.camera.target[1],
            "moving the mouse up must look up"
        );
        assert!((distance(effective) - before_distance).abs() < 0.001);
        assert!(effective.target.into_iter().all(f32::is_finite));
        assert!(effective.up.into_iter().all(f32::is_finite));
    }

    #[test]
    fn wasd_and_qe_move_eye_and_target_together() {
        let frame = frame_3d();
        let mut debug = DebugCamera::default();
        assert!(debug.detach(&frame));
        let before = debug.camera(&frame.camera).clone();

        debug.move_local(1.0, 1.0, 1.0, 1.0 / 60.0);
        let effective = debug.camera(&frame.camera);
        assert_ne!(effective.eye, before.eye);
        assert!(
            effective.eye[0] < before.eye[0],
            "D must move along the established camera-right basis"
        );
        assert!(effective.eye[1] > before.eye[1], "E must move upward");
        for axis in 0..3 {
            let eye_delta = effective.eye[axis] - before.eye[axis];
            let target_delta = effective.target[axis] - before.target[axis];
            assert!((eye_delta - target_delta).abs() < 0.000_001);
        }
        assert_eq!(frame.camera.eye, camera().eye);
        assert_eq!(frame.camera.target, camera().target);
    }

    #[test]
    fn fps_wheel_changes_only_the_debug_fov() {
        let frame = frame_3d();
        let mut debug = DebugCamera::default();
        assert!(debug.detach(&frame));
        let before_eye = debug.camera(&frame.camera).eye;
        let before_fov = debug.camera(&frame.camera).fov_radians;

        debug.zoom(1.0);
        let effective = debug.camera(&frame.camera);
        assert_eq!(effective.eye, before_eye);
        assert!(effective.fov_radians < before_fov);
        assert_eq!(frame.camera.fov_radians, camera().fov_radians);
    }

    #[test]
    fn pan2d_mouse_wasd_and_wheel_change_only_private_sprite_cameras() {
        let frame = frame_2d();
        let mut debug = DebugCamera::default();
        assert!(debug.detach(&frame));
        let authored = frame.sprite_layers[0].camera.clone();

        debug.look(20.0, -10.0);
        debug.move_local(1.0, 1.0, 0.0, 1.0 / 60.0);
        debug.zoom(1.0);
        let effective = &debug.sprite_cameras().unwrap()[0];
        assert_ne!(effective.center, authored.center);
        assert!(effective.zoom > authored.zoom);
        assert_eq!(frame.sprite_layers[0].camera, authored);
    }

    #[test]
    fn pan2d_pointer_axes_share_one_world_scale_and_zoom_approaches_bounds() {
        let mut frame = frame_2d();
        frame.sprite_layers[0].camera.zoom = 250.0;
        let mut debug = DebugCamera::default();
        assert!(debug.detach(&frame));

        debug.look(10.0, 10.0);
        let panned = &debug.sprite_cameras().unwrap()[0];
        assert!((panned.center[0].abs() - panned.center[1].abs()).abs() < 0.000_001);

        debug.zoom(-1.0);
        let zoom = debug.sprite_cameras().unwrap()[0].zoom;
        assert!(
            zoom > DebugCamera::MAX_2D_ZOOM && zoom < 250.0,
            "an authored out-of-band zoom moves toward the supported range without snapping"
        );
    }

    #[test]
    fn dimension_changes_make_a_live_debug_snapshot_incompatible() {
        let mut debug = DebugCamera::default();
        assert!(debug.detach(&frame_3d()));
        assert!(!debug.is_compatible(&frame_2d()));
        assert!(debug.is_compatible(&frame_3d()));

        let two_layers = Frame::with_2d(
            frame_2d(),
            SpriteLayer {
                camera: Camera2D::new(10.0, 10.0),
                scene: Scene3D::quad(),
            },
        );
        assert!(debug.detach(&two_layers));
        assert!(!debug.is_compatible(&frame_2d()));
        assert!(!debug.is_compatible(&frame_3d()));
    }

    #[test]
    fn pinned_preview_stops_at_the_first_incompatible_view_layout() {
        let anchor_3d = frame_3d();
        let projected_3d = frame_3d();
        let projected_2d = frame_2d();
        let later_3d = frame_3d();
        assert_eq!(
            DebugCamera::compatible_prefix_len(
                &anchor_3d,
                [&projected_3d, &projected_2d, &later_3d]
            ),
            1
        );

        let anchor_2d = frame_2d();
        let matching_2d = frame_2d();
        let changed_layers = Frame::with_2d(
            frame_2d(),
            SpriteLayer {
                camera: Camera2D::new(10.0, 10.0),
                scene: Scene3D::quad(),
            },
        );
        assert_eq!(
            DebugCamera::compatible_prefix_len(
                &anchor_2d,
                [&matching_2d, &changed_layers, &matching_2d]
            ),
            1
        );
    }

    #[test]
    fn invalid_camera_data_refuses_detach_without_partial_state() {
        let mut invalid = frame_3d();
        invalid.camera.eye = invalid.camera.target;
        let mut debug = DebugCamera::default();
        assert!(!debug.detach(&invalid));
        assert!(!debug.is_detached());

        let mut invalid_2d = frame_2d();
        invalid_2d.sprite_layers[0].camera.zoom = f32::NAN;
        assert!(!debug.detach(&invalid_2d));
        assert!(!debug.is_detached());
    }
}
