use fable_library_rust::NativeArray_::Array;
use serde::{Deserialize, Serialize};

use crate::{Camera, Light, Scene3D};

/// What a game's `draw3d` returns each frame: the camera, the scene to render,
/// and the lights affecting it. Intentionally a growable record (post-processing
/// etc. can be added later) so the render boundary signature doesn't churn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub camera: Camera,
    pub scene: Scene3D,
    #[serde(default)]
    pub lights: Vec<Light>,
}

impl Frame {
    /// Unlit frame (no lights): lit surfaces get only their ambient term until
    /// lights are supplied via `new_lit`.
    pub fn new(camera: Camera, scene: Scene3D) -> Frame {
        Frame {
            camera,
            scene,
            lights: vec![],
        }
    }

    pub fn new_lit(camera: Camera, scene: Scene3D, lights: Array<Light>) -> Frame {
        Frame {
            camera,
            scene,
            lights: lights.to_vec(),
        }
    }
}
