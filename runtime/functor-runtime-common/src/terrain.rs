use serde::{Deserialize, Serialize};

/// The collision-relevant subset of a terrain descriptor.
///
/// Physics stores this rather than the full render descriptor so changing LOD,
/// colors, or vegetation does not rebuild a multi-megabyte collider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainGeometry {
    pub heightmap: String,
    pub width: f32,
    pub depth: f32,
    pub min_height: f32,
    pub max_height: f32,
}

/// Height/slope-driven terrain colors evaluated per fragment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainLayers {
    pub low: [f32; 3],
    pub high: [f32; 3],
    pub rock: [f32; 3],
    pub snow: [f32; 3],
    pub snow_height: f32,
}

/// GPU-instanced grass clusters around the camera.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainGrass {
    /// Approximate spacing between clusters in terrain-local world units.
    pub spacing: f32,
    /// Camera-centered draw radius in terrain-local world units.
    pub distance: f32,
    pub blade_height: f32,
    pub color: [f32; 3],
}

/// A finite heightfield terrain shared by the rendering and physics protocols.
///
/// Heights are normalized unsigned samples from `heightmap`: black maps to
/// `min_height`, white maps to `max_height`. The terrain is centered on the
/// local origin, spans `width × depth` in XZ, and remains ordinary immutable
/// protocol data so the render and physics adapters cannot drift.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainDescription {
    pub heightmap: String,
    #[serde(default)]
    pub while_pending: Vec<String>,
    pub width: f32,
    pub depth: f32,
    pub min_height: f32,
    pub max_height: f32,
    pub max_pixel_error: f32,
    pub color: [f32; 3],
    #[serde(default)]
    pub layers: Option<TerrainLayers>,
    #[serde(default)]
    pub grass: Option<TerrainGrass>,
}

impl TerrainDescription {
    pub const DEFAULT_MAX_PIXEL_ERROR: f32 = 2.0;
    pub const DEFAULT_COLOR: [f32; 3] = [0.34, 0.48, 0.24];

    pub fn heightmap(
        heightmap: String,
        while_pending: Vec<String>,
        width: f32,
        depth: f32,
        min_height: f32,
        max_height: f32,
    ) -> Self {
        Self {
            heightmap,
            while_pending,
            width,
            depth,
            min_height,
            max_height,
            max_pixel_error: Self::DEFAULT_MAX_PIXEL_ERROR,
            color: Self::DEFAULT_COLOR,
            layers: None,
            grass: None,
        }
    }

    pub fn with_max_pixel_error(mut self, max_pixel_error: f32) -> Self {
        self.max_pixel_error = max_pixel_error;
        self
    }

    pub fn with_color(mut self, color: [f32; 3]) -> Self {
        self.color = color;
        self.layers = None;
        self
    }

    pub fn with_layers(mut self, layers: TerrainLayers) -> Self {
        self.layers = Some(layers);
        self
    }

    pub fn with_grass(mut self, grass: TerrainGrass) -> Self {
        self.grass = Some(grass);
        self
    }

    pub fn geometry(&self) -> TerrainGeometry {
        TerrainGeometry {
            heightmap: self.heightmap.clone(),
            width: self.width,
            depth: self.depth,
            min_height: self.min_height,
            max_height: self.max_height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_description_roundtrips_through_the_protocol() {
        let terrain = TerrainDescription::heightmap(
            "island.png".to_string(),
            vec!["island-low.png".to_string()],
            4000.0,
            4000.0,
            -80.0,
            520.0,
        )
        .with_max_pixel_error(3.0)
        .with_layers(TerrainLayers {
            low: [0.2, 0.4, 0.1],
            high: [0.3, 0.5, 0.2],
            rock: [0.25, 0.22, 0.2],
            snow: [0.9, 0.95, 1.0],
            snow_height: 300.0,
        })
        .with_grass(TerrainGrass {
            spacing: 8.0,
            distance: 300.0,
            blade_height: 2.2,
            color: [0.15, 0.32, 0.08],
        });
        let json = serde_json::to_string(&terrain).unwrap();
        assert_eq!(
            serde_json::from_str::<TerrainDescription>(&json).unwrap(),
            terrain
        );
    }
}
