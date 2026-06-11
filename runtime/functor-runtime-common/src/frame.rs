use serde::{Deserialize, Serialize};

use crate::{Camera, Scene3D};

/// What a game's `draw3d` returns each frame: the camera plus the scene to
/// render. Intentionally a growable record (lights, post-processing, etc. can
/// be added later) so the render boundary signature doesn't churn each time.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub camera: Camera,
    pub scene: Scene3D,
}

impl Frame {
    pub fn new(camera: Camera, scene: Scene3D) -> Frame {
        Frame { camera, scene }
    }
}
