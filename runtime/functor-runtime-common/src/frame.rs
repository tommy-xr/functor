use fable_library_rust::NativeArray_::Array;
use serde::{Deserialize, Serialize};

use crate::{render_target::RenderTargetDescriptor, Camera, Light, Scene3D};

/// A named offscreen pass: `frame` (its own camera/scene/lights) is rendered
/// into `target`'s texture before the owning frame's main pass, and sampled via
/// `TextureDescription::RenderTarget(target.id)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderTargetPass {
    pub target: RenderTargetDescriptor,
    pub frame: Frame,
}

/// What a game's `draw3d` returns each frame: the camera, the scene to render,
/// and the lights affecting it. Intentionally a growable record (post-processing
/// etc. can be added later) so the render boundary signature doesn't churn.
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
}

impl Frame {
    /// Unlit frame (no lights): lit surfaces get only their ambient term until
    /// lights are supplied via `new_lit`.
    pub fn new(camera: Camera, scene: Scene3D) -> Frame {
        Frame {
            camera,
            scene,
            lights: vec![],
            render_targets: vec![],
        }
    }

    pub fn new_lit(camera: Camera, scene: Scene3D, lights: Array<Light>) -> Frame {
        Frame {
            camera,
            scene,
            lights: lights.to_vec(),
            render_targets: vec![],
        }
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
}
