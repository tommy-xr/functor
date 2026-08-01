use cgmath::{InnerSpace, Quaternion, Rad, Rotation, Rotation3, Vector3};

use crate::{physics::DebugLine, Camera, Camera2D, DebugRenderMode, Frame};

/// The navigation behavior selected from the current frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugCameraMode {
    /// A free-flying 3D camera with mouse look and WASD + Q/E translation.
    Fps,
    /// A 3D camera that rotates around its current target.
    Orbit,
    /// A pure 2D frame whose sprite cameras pan and zoom together.
    Pan2d,
}

impl DebugCameraMode {
    pub fn label(self) -> &'static str {
        match self {
            DebugCameraMode::Fps => "FPS",
            DebugCameraMode::Orbit => "Orbit",
            DebugCameraMode::Pan2d => "Pan 2D",
        }
    }

    pub fn from_index(index: u32) -> Option<Self> {
        match index {
            0 => Some(DebugCameraMode::Fps),
            1 => Some(DebugCameraMode::Orbit),
            2 => Some(DebugCameraMode::Pan2d),
            _ => None,
        }
    }

    pub fn index(self) -> u32 {
        match self {
            DebugCameraMode::Fps => 0,
            DebugCameraMode::Orbit => 1,
            DebugCameraMode::Pan2d => 2,
        }
    }
}

/// The global material override used by a detached debug view.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DebugMaterialMode {
    #[default]
    Shaded,
    Normals,
    Tangents,
    Transparent,
}

impl DebugMaterialMode {
    pub fn label(self) -> &'static str {
        match self {
            DebugMaterialMode::Shaded => "Shaded",
            DebugMaterialMode::Normals => "Normals",
            DebugMaterialMode::Tangents => "Tangents",
            DebugMaterialMode::Transparent => "Transparent",
        }
    }

    pub fn from_index(index: u32) -> Option<Self> {
        match index {
            0 => Some(DebugMaterialMode::Shaded),
            1 => Some(DebugMaterialMode::Normals),
            2 => Some(DebugMaterialMode::Tangents),
            3 => Some(DebugMaterialMode::Transparent),
            _ => None,
        }
    }

    pub fn index(self) -> u32 {
        match self {
            DebugMaterialMode::Shaded => 0,
            DebugMaterialMode::Normals => 1,
            DebugMaterialMode::Tangents => 2,
            DebugMaterialMode::Transparent => 3,
        }
    }

    pub fn render_mode(self) -> DebugRenderMode {
        match self {
            DebugMaterialMode::Shaded => DebugRenderMode::Default,
            DebugMaterialMode::Normals => DebugRenderMode::Normals,
            DebugMaterialMode::Tangents => DebugRenderMode::Tangents,
            DebugMaterialMode::Transparent => DebugRenderMode::Transparent,
        }
    }
}

/// Shell-owned diagnostic presentation applied only while the debug camera is
/// detached. These settings never enter the game model, input log, or replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DebugPresentation {
    pub material: DebugMaterialMode,
    pub physics: bool,
    pub authored_camera_frustum: bool,
    pub show_game_ui: bool,
}

/// The diagnostic render policy after combining the shell's launch flags with
/// the current debug-camera state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DebugDiagnostics {
    pub render_mode: DebugRenderMode,
    pub physics: bool,
    pub authored_camera_frustum: bool,
}

impl Default for DebugPresentation {
    fn default() -> Self {
        Self {
            material: DebugMaterialMode::Shaded,
            physics: false,
            authored_camera_frustum: false,
            show_game_ui: true,
        }
    }
}

impl DebugPresentation {
    /// Preserve the existing launch-time debug mode when a session first
    /// detaches, while splitting material replacement from line overlays.
    pub fn from_render_mode(mode: DebugRenderMode) -> Self {
        match mode {
            DebugRenderMode::Default => Self::default(),
            DebugRenderMode::Normals => Self {
                material: DebugMaterialMode::Normals,
                ..Self::default()
            },
            DebugRenderMode::Tangents => Self {
                material: DebugMaterialMode::Tangents,
                ..Self::default()
            },
            DebugRenderMode::Transparent => Self {
                material: DebugMaterialMode::Transparent,
                ..Self::default()
            },
            DebugRenderMode::Physics => Self {
                physics: true,
                ..Self::default()
            },
        }
    }

    /// Resolve shell-owned diagnostics without letting them leak into a pure
    /// 2D detached view. While attached, preserve the existing launch-time
    /// `--debug-render` behavior.
    pub fn diagnostics(
        self,
        detached: bool,
        pure_2d: bool,
        launch_mode: DebugRenderMode,
    ) -> DebugDiagnostics {
        if detached {
            if pure_2d {
                return DebugDiagnostics {
                    render_mode: DebugRenderMode::Default,
                    physics: false,
                    authored_camera_frustum: false,
                };
            }
            return DebugDiagnostics {
                render_mode: self.material.render_mode(),
                physics: self.physics,
                authored_camera_frustum: self.authored_camera_frustum,
            };
        }

        DebugDiagnostics {
            render_mode: launch_mode,
            physics: matches!(launch_mode, DebugRenderMode::Physics),
            authored_camera_frustum: false,
        }
    }
}

#[derive(Clone, Debug)]
enum DebugView {
    Fps(Camera),
    Orbit(Camera),
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
            Some(DebugView::Orbit(_)) => Some(DebugCameraMode::Orbit),
            Some(DebugView::Pan2d(_)) => Some(DebugCameraMode::Pan2d),
            None => None,
        }
    }

    /// Change navigation behavior without changing the effective camera.
    /// Dimensionality is fixed by the frame that was detached.
    pub fn set_mode(&mut self, mode: DebugCameraMode) -> bool {
        self.view = match (self.view.take(), mode) {
            (Some(DebugView::Fps(camera)), DebugCameraMode::Orbit) => {
                Some(DebugView::Orbit(camera))
            }
            (Some(DebugView::Orbit(camera)), DebugCameraMode::Fps) => Some(DebugView::Fps(camera)),
            (Some(view @ DebugView::Fps(_)), DebugCameraMode::Fps)
            | (Some(view @ DebugView::Orbit(_)), DebugCameraMode::Orbit)
            | (Some(view @ DebugView::Pan2d(_)), DebugCameraMode::Pan2d) => Some(view),
            (view, _) => {
                self.view = view;
                return false;
            }
        };
        true
    }

    /// A debug view cannot silently change dimensions underneath its snapshot.
    /// Shells reattach when a live game switches between pure 2D and 3D/mixed.
    pub fn is_compatible(&self, frame: &Frame) -> bool {
        match &self.view {
            Some(DebugView::Fps(_)) | Some(DebugView::Orbit(_)) => !frame.is_pure_2d(),
            Some(DebugView::Pan2d(cameras)) => {
                frame.is_pure_2d() && cameras.len() == frame.sprite_layers.len()
            }
            None => true,
        }
    }

    /// The effective main 3D view camera. A 2D debug view leaves the frame's
    /// dummy/main 3D camera alone.
    pub fn camera<'a>(&'a self, authored: &'a Camera) -> &'a Camera {
        match &self.view {
            Some(DebugView::Fps(camera)) | Some(DebugView::Orbit(camera)) => camera,
            Some(DebugView::Pan2d(_)) | None => authored,
        }
    }

    /// Overrides for the main frame's ordered 2D layers, when panning a pure
    /// 2D frame. Mixed frames keep their authored overlay cameras.
    pub fn sprite_cameras(&self) -> Option<&[Camera2D]> {
        match &self.view {
            Some(DebugView::Pan2d(cameras)) => Some(cameras),
            Some(DebugView::Fps(_)) | Some(DebugView::Orbit(_)) | None => None,
        }
    }

    /// Apply pointer-lock motion: mouse-look in 3D, pan in 2D.
    pub fn look(&mut self, dx: f32, dy: f32) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        match &mut self.view {
            Some(DebugView::Fps(camera)) => Self::fps_look(camera, dx, dy),
            Some(DebugView::Orbit(camera)) => Self::orbit_look(camera, dx, dy),
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
    pub fn move_local(&mut self, forward: f32, right: f32, vertical: f32, elapsed_seconds: f32) {
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
            Some(DebugView::Fps(camera)) | Some(DebugView::Orbit(camera)) => {
                let forward_axis = Vector3::from(camera.target) - Vector3::from(camera.eye);
                let up = Vector3::from(camera.up);
                if forward_axis.magnitude2() <= f32::EPSILON || up.magnitude2() <= f32::EPSILON {
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
            Some(DebugView::Orbit(camera)) => {
                let target = Vector3::from(camera.target);
                let offset = Vector3::from(camera.eye) - target;
                let distance = offset.magnitude();
                if distance > Self::MIN_DISTANCE {
                    let proposed = distance * (-steps * 0.12).exp();
                    let next = proposed.max(Self::MIN_DISTANCE);
                    camera.eye = (target + offset / distance * next).into();
                    self.focal_distance = next;
                }
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

    pub fn fov_degrees(&self) -> Option<f32> {
        match &self.view {
            Some(DebugView::Fps(camera)) | Some(DebugView::Orbit(camera)) => {
                Some(camera.fov_radians.to_degrees())
            }
            Some(DebugView::Pan2d(_)) | None => None,
        }
    }

    pub fn zoom_2d(&self) -> Option<f32> {
        match &self.view {
            Some(DebugView::Pan2d(cameras)) => cameras.first().map(|camera| camera.zoom),
            Some(DebugView::Fps(_)) | Some(DebugView::Orbit(_)) | None => None,
        }
    }

    pub fn set_fov_degrees(&mut self, degrees: f32) {
        if !degrees.is_finite() {
            return;
        }
        let radians = degrees.to_radians().clamp(Self::MIN_FOV, Self::MAX_FOV);
        match &mut self.view {
            Some(DebugView::Fps(camera)) | Some(DebugView::Orbit(camera)) => {
                camera.fov_radians = radians;
            }
            Some(DebugView::Pan2d(_)) | None => {}
        }
    }

    fn fps_look(camera: &mut Camera, dx: f32, dy: f32) {
        let eye = Vector3::from(camera.eye);
        let Some((direction, up)) = Self::look_orientation(camera, dx, dy) else {
            return;
        };
        camera.target = (eye + direction).into();
        camera.up = up.into();
    }

    fn orbit_look(camera: &mut Camera, dx: f32, dy: f32) {
        let target = Vector3::from(camera.target);
        let Some((direction, up)) = Self::look_orientation(camera, dx, dy) else {
            return;
        };
        camera.eye = (target - direction).into();
        camera.up = up.into();
    }

    fn look_orientation(camera: &Camera, dx: f32, dy: f32) -> Option<(Vector3<f32>, Vector3<f32>)> {
        let eye = Vector3::from(camera.eye);
        let mut direction = Vector3::from(camera.target) - eye;
        let mut up = Vector3::from(camera.up);
        let distance = direction.magnitude();
        if distance <= f32::EPSILON {
            return None;
        }

        let yaw =
            Quaternion::from_axis_angle(Vector3::unit_y(), Rad(-dx * Self::LOOK_RADIANS_PER_PIXEL));
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

        Some((direction, up))
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

/// The authored camera's actual near/far view volume as world-space lines.
///
/// The shell draws these through the detached camera, so the game continues
/// culling through `authored` while the observer can inspect its exact volume.
pub fn camera_frustum_lines(authored: &Camera, aspect: f32) -> Vec<DebugLine> {
    if !aspect.is_finite()
        || aspect <= 0.0
        || !authored.fov_radians.is_finite()
        || authored.fov_radians <= 0.0
        || authored.fov_radians >= std::f32::consts::PI
        || !authored.near.is_finite()
        || !authored.far.is_finite()
        || authored.near <= 0.0
        || authored.near >= authored.far
    {
        return Vec::new();
    }

    let Some(basis) = authored.world_basis() else {
        return Vec::new();
    };

    let half_tan = (authored.fov_radians * 0.5).tan();
    let corners = |distance: f32| {
        let center = basis.eye + basis.forward * distance;
        let half_height = half_tan * distance;
        let half_width = half_height * aspect;
        [
            center - basis.right * half_width - basis.up * half_height,
            center + basis.right * half_width - basis.up * half_height,
            center + basis.right * half_width + basis.up * half_height,
            center - basis.right * half_width + basis.up * half_height,
        ]
    };
    let near = corners(authored.near);
    let far = corners(authored.far);
    let color = [0.91, 0.35, 0.72, 1.0];
    let line = |a: Vector3<f32>, b: Vector3<f32>| DebugLine {
        a: a.into(),
        b: b.into(),
        color,
    };

    let mut lines = Vec::with_capacity(12);
    for index in 0..4 {
        let next = (index + 1) % 4;
        lines.push(line(near[index], near[next]));
        lines.push(line(far[index], far[next]));
        lines.push(line(near[index], far[index]));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{
        camera_frustum_lines, DebugCamera, DebugCameraMode, DebugMaterialMode, DebugPresentation,
    };
    use crate::{math::Angle, Camera, Camera2D, Frame, Scene3D, SpriteLayer};
    use cgmath::{InnerSpace, Vector3};

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
    fn fps_and_orbit_switch_without_moving_the_camera() {
        let frame = frame_3d();
        let mut debug = DebugCamera::default();
        assert!(debug.detach(&frame));
        let before = debug.camera(&frame.camera).clone();

        assert!(debug.set_mode(DebugCameraMode::Orbit));
        assert_eq!(debug.mode(), Some(DebugCameraMode::Orbit));
        assert_eq!(debug.camera(&frame.camera).eye, before.eye);
        assert_eq!(debug.camera(&frame.camera).target, before.target);

        debug.look(100.0, -20.0);
        let orbit = debug.camera(&frame.camera);
        assert_ne!(orbit.eye, before.eye);
        assert_eq!(orbit.target, before.target);
        assert!((distance(orbit) - distance(&before)).abs() < 0.001);

        assert!(debug.set_mode(DebugCameraMode::Fps));
        assert_eq!(debug.mode(), Some(DebugCameraMode::Fps));
        assert!(!debug.set_mode(DebugCameraMode::Pan2d));
        assert_eq!(debug.mode(), Some(DebugCameraMode::Fps));
    }

    #[test]
    fn orbit_wheel_dollies_without_changing_fov() {
        let frame = frame_3d();
        let mut debug = DebugCamera::default();
        assert!(debug.detach(&frame));
        assert!(debug.set_mode(DebugCameraMode::Orbit));
        let before = debug.camera(&frame.camera).clone();

        debug.zoom(1.0);
        let effective = debug.camera(&frame.camera);
        assert!(distance(effective) < distance(&before));
        assert_eq!(effective.target, before.target);
        assert_eq!(effective.fov_radians, before.fov_radians);
    }

    #[test]
    fn fov_control_and_presentation_stay_shell_owned() {
        let frame = frame_3d();
        let mut debug = DebugCamera::default();
        assert!(debug.detach(&frame));
        debug.set_fov_degrees(75.0);
        assert!((debug.fov_degrees().unwrap() - 75.0).abs() < 0.001);
        assert_ne!(
            debug.camera(&frame.camera).fov_radians,
            frame.camera.fov_radians
        );

        let presentation = DebugPresentation {
            material: DebugMaterialMode::Normals,
            physics: true,
            authored_camera_frustum: true,
            show_game_ui: false,
        };
        assert_eq!(
            presentation.material.render_mode(),
            crate::DebugRenderMode::Normals
        );
        assert_eq!(
            DebugMaterialMode::from_index(3),
            Some(DebugMaterialMode::Transparent)
        );
        assert_eq!(
            DebugMaterialMode::Transparent.render_mode(),
            crate::DebugRenderMode::Transparent
        );
        assert_eq!(
            DebugPresentation::from_render_mode(crate::DebugRenderMode::Transparent).material,
            DebugMaterialMode::Transparent
        );
        assert_eq!(
            presentation.diagnostics(true, false, crate::DebugRenderMode::Default),
            super::DebugDiagnostics {
                render_mode: crate::DebugRenderMode::Normals,
                physics: true,
                authored_camera_frustum: true,
            }
        );
        assert_eq!(
            presentation.diagnostics(true, true, crate::DebugRenderMode::Tangents),
            super::DebugDiagnostics {
                render_mode: crate::DebugRenderMode::Default,
                physics: false,
                authored_camera_frustum: false,
            }
        );
        assert_eq!(
            presentation.diagnostics(false, false, crate::DebugRenderMode::Physics),
            super::DebugDiagnostics {
                render_mode: crate::DebugRenderMode::Physics,
                physics: true,
                authored_camera_frustum: false,
            }
        );
    }

    #[test]
    fn authored_frustum_uses_actual_clip_planes() {
        let camera = camera();
        let lines = camera_frustum_lines(&camera, 16.0 / 9.0);
        assert_eq!(lines.len(), 12);
        assert!(lines.iter().all(|line| {
            line.a.into_iter().all(f32::is_finite) && line.b.into_iter().all(f32::is_finite)
        }));

        let eye = Vector3::from(camera.eye);
        let forward = (Vector3::from(camera.target) - eye).normalize();
        let near_distance = (Vector3::from(lines[0].a) - eye).dot(forward);
        let far_distance = (Vector3::from(lines[1].a) - eye).dot(forward);
        assert!((near_distance - camera.near).abs() < 0.001);
        assert!((far_distance - camera.far).abs() < 0.001);
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
