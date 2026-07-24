use serde::{Deserialize, Serialize};

use crate::{
    fog::Fog, render_target::RenderTargetDescriptor, skybox::SkyboxDescription, Camera, Light,
    Scene3D, SpriteLayer,
};

/// A named offscreen pass: `frame` (its own camera/scene/lights) is rendered
/// into `target`'s texture before the owning frame's main pass, and sampled via
/// `TextureDescription::RenderTarget(target.id)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderTargetPass {
    pub target: RenderTargetDescriptor,
    pub frame: Frame,
}

/// What a game's `draw` returns each frame: a 3D pass plus any ordered 2D
/// sprite layers. Intentionally a growable record (post-processing etc. can be
/// added later) so the render boundary signature doesn't churn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub camera: Camera,
    pub scene: Scene3D,
    #[serde(default)]
    pub lights: Vec<Light>,
    /// Offscreen passes rendered (in order) before the main pass. Nested
    /// targets inside a target's own frame are ignored (depth 1 for now).
    #[serde(default)]
    pub render_targets: Vec<RenderTargetPass>,
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
            fog: None,
            skybox: None,
            clear_color: None,
            sprite_layers: vec![],
        }
    }

    /// The background clear color for this frame's pass: the explicit
    /// `Frame.withClearColor` override when set, otherwise the fog color, else
    /// the engine default (`fog::clear_color`).
    pub fn resolved_clear_color(&self) -> [f32; 3] {
        self.clear_color
            .unwrap_or_else(|| crate::fog::clear_color(self.fog.as_ref()))
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
