use cgmath::{Matrix4, SquareMatrix};
use serde::{Deserialize, Serialize};

use crate::{
    fog::Fog, render_target::RenderTargetDescriptor, skybox::SkyboxDescription, ui::View, Camera,
    Light, Scene3D, SceneObject, SpriteLayer,
};

fn is_false(value: &bool) -> bool {
    !*value
}

/// A named offscreen pass: `frame` (its own camera/scene/lights) is rendered
/// into `target`'s texture before the owning frame's main pass, and sampled via
/// `TextureDescription::RenderTarget(target.id)`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct RenderTargetPass {
    pub target: RenderTargetDescriptor,
    pub frame: Frame,
}

/// A named UI pass: `view` (a `Ui.*` tree) is painted into `target`'s texture
/// before the owning frame's main pass, and sampled via `Scene.screen` like
/// any render target. Display-only for now: interactive widgets render in
/// their resting state and their handlers are inert.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct UiTargetPass {
    pub target: RenderTargetDescriptor,
    pub view: View,
}

/// What a game's `draw` returns each frame: a 3D pass plus any ordered 2D
/// sprite layers. Intentionally a growable record (post-processing etc. can be
/// added later) so the render boundary signature doesn't churn.
///
/// `PartialEq` is the structural walk behind `Frame.equals`: every field —
/// camera, scene, lights (ordered), render-target passes (ordered), ui-target
/// passes (ordered), fog,
/// skybox, clear color, 2D layers (ordered), and the `pure_2d` marker. It
/// inherits [`Scene3D`]'s rules: floats compare exactly, assets compare by
/// locator, and animation compares as declared.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub camera: Camera,
    pub scene: Scene3D,
    #[serde(default)]
    pub lights: Vec<Light>,
    /// Offscreen passes rendered (in order) before the main pass. Nested
    /// targets inside a target's own frame are ignored (depth 1 for now).
    #[serde(default)]
    pub render_targets: Vec<RenderTargetPass>,
    /// UI views painted into render targets (in order) before the main pass
    /// (`Frame.withUiTarget`). Skipped when empty so older wire frames — and
    /// frames that never use the feature — serialize unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ui_targets: Vec<UiTargetPass>,
    /// Frame-level distance fog; its color also drives the pass's clear color.
    #[serde(default)]
    pub fog: Option<Fog>,
    /// A cubemap skybox drawn behind everything (fog does not apply to it).
    #[serde(default)]
    pub skybox: Option<SkyboxDescription>,
    /// Explicit background clear color (`Frame.withClearColor`). When set it
    /// wins over the fog-color-as-clear-color default; when `None` the clear
    /// color falls back to the fog color, else the engine default. It only
    /// paints the background — it does not affect fog blending.
    #[serde(default)]
    pub clear_color: Option<[f32; 3]>,
    /// Ordered center-origin, Y-up 2D passes. They render after the 3D scene;
    /// later layers appear above earlier ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sprite_layers: Vec<SpriteLayer>,
    /// Explicitly marks `Frame.create2D` output. Structural inspection cannot
    /// distinguish a 2D frame from an empty 3D world with a HUD/skybox.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pure_2d: bool,
}

impl Frame {
    /// Unlit frame (no lights): lit surfaces get only their ambient term until
    /// lights are supplied on the `lights` field.
    pub fn new(camera: Camera, scene: Scene3D) -> Frame {
        Frame {
            camera,
            scene,
            lights: vec![],
            render_targets: vec![],
            ui_targets: vec![],
            fog: None,
            skybox: None,
            clear_color: None,
            sprite_layers: vec![],
            pure_2d: false,
        }
    }

    /// A pure sprite frame produced by `Frame.create2D`.
    pub fn new_2d(layer: SpriteLayer) -> Frame {
        let empty = Scene3D {
            obj: SceneObject::Group(vec![]),
            xform: Matrix4::identity(),
        };
        let mut frame = Frame::new(Camera::default(), empty);
        frame.pure_2d = true;
        frame.sprite_layers.push(layer);
        frame
    }

    /// The background clear color for this frame's pass: the explicit
    /// `Frame.withClearColor` override when set, otherwise the fog color, else
    /// the engine default (`fog::clear_color`).
    pub fn resolved_clear_color(&self) -> [f32; 3] {
        self.clear_color
            .unwrap_or_else(|| crate::fog::clear_color(self.fog.as_ref()))
    }

    /// Whether the main view is the pure sprite frame produced by
    /// `Frame.create2D`. Mixed frames retain their real 3D pass and therefore
    /// use the runtime's 3D debug camera.
    pub fn is_pure_2d(&self) -> bool {
        self.pure_2d
    }

    /// Render `target_frame` into `target` each frame, before this frame's main
    /// pass. Subject-first so it pipes (`frame |> Frame.withRenderTarget(…)`);
    /// declaration order is render order.
    pub fn with_render_target(
        mut frame: Frame,
        target: RenderTargetDescriptor,
        target_frame: Frame,
    ) -> Frame {
        frame.render_targets.push(RenderTargetPass {
            target,
            frame: target_frame,
        });
        frame
    }

    /// Paint `view` (a `Ui.*` tree) into `target` each frame, before this
    /// frame's main pass, at the target's declared size. Subject-first so it
    /// pipes (`frame |> Frame.withUiTarget(…)`); declaration order is paint
    /// order, and — matching `withRenderTarget` — the FIRST declaration of an
    /// id wins (duplicates warn once and are skipped).
    pub fn with_ui_target(mut frame: Frame, target: RenderTargetDescriptor, view: View) -> Frame {
        frame.ui_targets.push(UiTargetPass { target, view });
        frame
    }

    /// Distance fog for this frame's forward pass (all forward materials,
    /// including emissive; the fog color becomes the clear color). Subject-
    /// first so it pipes (`frame |> Frame.withFog(fog)`).
    pub fn with_fog(mut frame: Frame, fog: Fog) -> Frame {
        frame.fog = Some(fog);
        frame
    }

    /// A cubemap skybox for this frame's pass, drawn behind everything right
    /// after the clear. Subject-first so it pipes
    /// (`frame |> Frame.withSkybox(sky)`).
    pub fn with_skybox(mut frame: Frame, skybox: SkyboxDescription) -> Frame {
        frame.skybox = Some(skybox);
        frame
    }

    /// Explicit background clear color, overriding the fog-color default.
    /// Subject-last so it pipes (`frame |> Frame.withClearColor(r, g, b)`).
    pub fn with_clear_color(mut frame: Frame, r: f32, g: f32, b: f32) -> Frame {
        frame.clear_color = Some([r, g, b]);
        frame
    }

    /// Add a 2D layer above the 3D pass and any earlier sprite layers.
    pub fn with_2d(mut frame: Frame, layer: SpriteLayer) -> Frame {
        frame.sprite_layers.push(layer);
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fog::Fog, Scene3D};

    fn bare() -> Frame {
        Frame::new(Camera::default(), Scene3D::cube())
    }

    #[test]
    fn resolved_clear_color_defaults_to_engine_default() {
        assert_eq!(bare().resolved_clear_color(), [0.1, 0.2, 0.3]);
    }

    #[test]
    fn pure_2d_is_distinct_from_3d_and_mixed_frames() {
        let layer = SpriteLayer {
            camera: crate::Camera2D::new(16.0, 9.0),
            scene: Scene3D::quad(),
        };
        assert!(Frame::new_2d(layer.clone()).is_pure_2d());
        assert!(!Frame::new(Camera::default(), Scene3D::cube()).is_pure_2d());
        let empty_3d = Scene3D {
            obj: SceneObject::Group(vec![]),
            xform: Matrix4::identity(),
        };
        assert!(!Frame::with_2d(Frame::new(Camera::default(), empty_3d), layer).is_pure_2d());
    }

    #[test]
    fn resolved_clear_color_falls_back_to_fog_color() {
        let frame = Frame::with_fog(bare(), Fog::linear(4.0, 30.0, 0.5, 0.6, 0.7));
        assert_eq!(frame.resolved_clear_color(), [0.5, 0.6, 0.7]);
    }

    #[test]
    fn explicit_clear_color_wins_over_fog() {
        let frame = Frame::with_fog(bare(), Fog::linear(4.0, 30.0, 0.5, 0.6, 0.7));
        let frame = Frame::with_clear_color(frame, 0.0, 0.0, 0.0);
        assert_eq!(frame.resolved_clear_color(), [0.0, 0.0, 0.0]);
        // The fog itself is untouched — only the background clear changed.
        assert!(frame.fog.is_some());
    }
}
