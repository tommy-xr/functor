use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
    sync::{Arc, Weak},
};

use crate::asset::pipelines::HeightmapData;

thread_local! {
    /// Decoded terrain sources shared by the render and physics adapters on
    /// the runtime thread. The descriptor remains plain immutable data; this
    /// is only the shell-side hydration cache, analogous to GPU resources.
    static HEIGHTMAPS: RefCell<HashMap<TerrainSource, Weak<HeightmapData>>> =
        RefCell::new(HashMap::new());
    /// Sources requested by `Physics.heightfield` since the shell last drove
    /// terrain asset hydration. A set keeps repeated fixed substeps cheap.
    static HEIGHTMAP_REQUESTS: RefCell<BTreeSet<TerrainSource>> =
        RefCell::new(BTreeSet::new());
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TerrainSource {
    pub locator: String,
    #[serde(default)]
    pub while_pending: Vec<String>,
}

pub(crate) fn request_heightmap(source: TerrainSource) {
    HEIGHTMAP_REQUESTS.with(|requests| {
        requests.borrow_mut().insert(source);
    });
}

pub(crate) fn take_heightmap_requests() -> Vec<TerrainSource> {
    HEIGHTMAP_REQUESTS.with(|requests| {
        std::mem::take(&mut *requests.borrow_mut())
            .into_iter()
            .collect()
    })
}

pub(crate) fn publish_heightmap(source: &TerrainSource, data: Arc<HeightmapData>) {
    HEIGHTMAPS.with(|heightmaps| {
        let mut heightmaps = heightmaps.borrow_mut();
        let unchanged = heightmaps
            .get(source)
            .and_then(Weak::upgrade)
            // HeightmapData is Eq, so Arc equality first takes the pointer
            // fast path and only compares samples after a real reload.
            .is_some_and(|current| current == data);
        if !unchanged {
            // The asset pipeline and current physics declaration own the
            // samples. This lookup bridge must not pin a 32 MiB 4096² map
            // after its runtime/project has gone away.
            heightmaps.insert(source.clone(), Arc::downgrade(&data));
        }
    });
}

pub(crate) fn hydrated_heightmap(source: &TerrainSource) -> Option<Arc<HeightmapData>> {
    HEIGHTMAPS.with(|heightmaps| {
        let mut heightmaps = heightmaps.borrow_mut();
        let hydrated = heightmaps.get(source).and_then(Weak::upgrade);
        if hydrated.is_none() {
            heightmaps.remove(source);
        }
        hydrated
    })
}

/// The collision-relevant subset of a terrain descriptor.
///
/// Physics stores this rather than the full render descriptor so changing LOD,
/// colors, or vegetation does not rebuild a multi-megabyte collider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainGeometry {
    pub source: TerrainSource,
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
            source: self.source(),
            width: self.width,
            depth: self.depth,
            min_height: self.min_height,
            max_height: self.max_height,
        }
    }

    pub fn source(&self) -> TerrainSource {
        TerrainSource {
            locator: self.heightmap.clone(),
            while_pending: self.while_pending.clone(),
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

    #[test]
    fn hydration_registry_does_not_retain_heightmap_samples() {
        let source = TerrainSource {
            locator: "weak-registry-test.png".to_string(),
            while_pending: Vec::new(),
        };
        let data = Arc::new(HeightmapData::flat());
        let weak = Arc::downgrade(&data);
        publish_heightmap(&source, data.clone());
        assert_eq!(Arc::strong_count(&data), 1);

        drop(data);
        assert!(weak.upgrade().is_none());
        HEIGHTMAPS.with(|heightmaps| heightmaps.borrow_mut().remove(&source));
    }
}
