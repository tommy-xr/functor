use std::collections::HashMap;
use std::{cell::RefCell, sync::Arc};

use cgmath::{vec3, Matrix4, SquareMatrix};
use serde::{Deserialize, Serialize};

use fable_library_rust::NativeArray_::Array;

use crate::{
    asset::{
        self,
        pipelines::{ModelPipeline, TexturePipeline},
        AssetHandle, BuiltAssetPipeline,
    },
    geometry::{self, Geometry},
    material::{
        BasicMaterial, Material, NormalDebugMaterial, SkinnedMaterial, SkinnedNormalDebugMaterial,
    },
    math::Angle,
    model::{Model, Skeleton},
    texture::{RuntimeTexture, Texture2D},
    DebugRenderMode, RenderContext,
};

mod material_description;
mod model_description;
mod texture_description;

pub use material_description::*;
pub use model_description::*;
pub use texture_description::*;

pub struct SceneContext {
    model_pipeline: Arc<BuiltAssetPipeline<Model>>,
    texture_pipeline: Arc<BuiltAssetPipeline<Texture2D>>,
    cube: RefCell<Box<dyn Geometry>>,
    cylinder: RefCell<Box<dyn Geometry>>,
    sphere: RefCell<Box<dyn Geometry>>,
    quad: RefCell<Box<dyn Geometry>>,
    plane: RefCell<Box<dyn Geometry>>,
    // Heightmaps are parameterized, so they're cached by a content hash (rows,
    // cols, heights) — static terrain builds its GL mesh once and reuses it.
    heightmaps: RefCell<HashMap<u64, Box<dyn Geometry>>>,
}

impl SceneContext {
    pub fn new() -> SceneContext {
        SceneContext {
            cube: RefCell::new(geometry::Cube::create()),
            sphere: RefCell::new(geometry::Sphere::create()),
            cylinder: RefCell::new(geometry::Cylinder::create()),
            quad: RefCell::new(geometry::Quad::create()),
            plane: RefCell::new(geometry::Plane::create()),
            heightmaps: RefCell::new(HashMap::new()),
            texture_pipeline: asset::build_pipeline(Box::new(TexturePipeline)),
            model_pipeline: asset::build_pipeline(Box::new(ModelPipeline)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Shape {
    Cube,
    Sphere,
    Cylinder,
    Quad,
    Plane,
    /// A subdivided XZ grid displaced by per-vertex heights (row-major,
    /// length `rows * cols`).
    Heightmap {
        rows: u32,
        cols: u32,
        heights: Vec<f32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SceneObject {
    Geometry(Shape),
    Model(ModelDescription),
    Material(MaterialDescription, Vec<Scene3D>),
    Group(Vec<Scene3D>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene3D {
    pub obj: SceneObject,
    #[serde(
        serialize_with = "serialize_matrix",
        deserialize_with = "deserialize_matrix"
    )]
    pub xform: Matrix4<f32>,
}

impl Scene3D {
    pub fn cube() -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Cube),
            xform: Matrix4::identity(),
        }
    }

    pub fn sphere() -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Sphere),
            xform: Matrix4::identity(),
        }
    }

    pub fn quad() -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Quad),
            xform: Matrix4::identity(),
        }
    }

    pub fn plane() -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Plane),
            xform: Matrix4::identity(),
        }
    }

    pub fn cylinder() -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Cylinder),
            xform: Matrix4::identity(),
        }
    }

    pub fn heightmap(rows: i32, cols: i32, heights: Array<f32>) -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Heightmap {
                rows: rows.max(0) as u32,
                cols: cols.max(0) as u32,
                heights: heights.to_vec(),
            }),
            xform: Matrix4::identity(),
        }
    }

    pub fn material(material: MaterialDescription, items: Array<Scene3D>) -> Self {
        Scene3D {
            obj: SceneObject::Material(material, items.to_vec()),
            xform: Matrix4::identity(),
        }
    }

    pub fn model(model: ModelDescription) -> Self {
        Scene3D {
            obj: SceneObject::Model(model),
            xform: Matrix4::identity(),
        }
    }

    pub fn group(items: Array<Scene3D>) -> Self {
        Scene3D {
            obj: SceneObject::Group(items.to_vec()),
            xform: Matrix4::identity(),
        }
    }

    pub fn transform(self, xform: Matrix4<f32>) -> Self {
        Scene3D {
            xform: self.xform * xform,
            ..self
        }
    }

    pub fn scale_x(self, x: f32) -> Self {
        self.transform(Matrix4::from_nonuniform_scale(x, 1.0, 1.0))
    }
    pub fn scale_y(self, y: f32) -> Self {
        self.transform(Matrix4::from_nonuniform_scale(1.0, y, 1.0))
    }
    pub fn scale_z(self, z: f32) -> Self {
        self.transform(Matrix4::from_nonuniform_scale(1.0, 1.0, z))
    }

    pub fn translate_x(self, x: f32) -> Self {
        self.transform(Matrix4::from_translation(vec3(x, 0.0, 0.0)))
    }

    pub fn translate_y(self, y: f32) -> Self {
        self.transform(Matrix4::from_translation(vec3(0.0, y, 0.0)))
    }

    pub fn translate_z(self, z: f32) -> Self {
        self.transform(Matrix4::from_translation(vec3(0.0, 0.0, z)))
    }

    pub fn rotate_x(self, ang: Angle) -> Self {
        self.transform(Matrix4::from_angle_x(ang))
    }
    pub fn rotate_y(self, ang: Angle) -> Self {
        self.transform(Matrix4::from_angle_y(ang))
    }
    pub fn rotate_z(self, ang: Angle) -> Self {
        self.transform(Matrix4::from_angle_z(ang))
    }

    pub fn render(
        &self,
        render_context: &RenderContext,
        scene_context: &SceneContext,
        world_matrix: &Matrix4<f32>,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
        current_material: &Box<dyn Material>,
    ) {
        let skinning_data = vec![];

        // In a debug render mode, primitive geometry is drawn with a diagnostic
        // shader instead of its own material. glTF models use the skinned vertex
        // format (no normals until that import lands), so they're left on their
        // own materials and aren't overridden here.
        let debug_material: Option<Box<dyn Material>> = match render_context.debug_render_mode {
            DebugRenderMode::Default => None,
            DebugRenderMode::Normals => {
                let mut m = NormalDebugMaterial::create();
                m.initialize(render_context);
                Some(m)
            }
        };
        let geometry_material = debug_material.as_ref().unwrap_or(current_material);

        match &self.obj {
            SceneObject::Model(model_description) => {
                match &model_description.handle {
                    ModelHandle::File(str) => {
                        let model: Arc<AssetHandle<Model>> = render_context
                            .asset_cache
                            .load_asset_with_pipeline(scene_context.model_pipeline.clone(), str);

                        let hydrated_model = model.get();

                        let matrix = world_matrix * self.xform;

                        // Skinned models pay for the joint-matrix uniform array;
                        // static models (no skeleton) render with the basic
                        // material instead. In a debug render mode, both swap to
                        // the matching diagnostic material (the skinned variant
                        // deforms the normal by the joint blend).
                        let is_skinned = hydrated_model.skeleton.get_joint_count() > 0;
                        let normals_debug =
                            render_context.debug_render_mode == DebugRenderMode::Normals;
                        let mut model_material: Box<dyn Material> = match (is_skinned, normals_debug)
                        {
                            (true, false) => SkinnedMaterial::create(),
                            (false, false) => BasicMaterial::create(),
                            (true, true) => SkinnedNormalDebugMaterial::create(),
                            (false, true) => NormalDebugMaterial::create(),
                        };
                        model_material.initialize(&render_context);

                        let animation_index = 0;

                        for mesh in hydrated_model.meshes.iter() {
                            // Go through selectors, and adjust
                            // let override_material_description = Some(MaterialDescription::Texture(
                            //     TextureDescription::File("vr_glove_color.jpg".to_string()),
                            // ));

                            let mut override_material_description: Option<&MaterialDescription> =
                                None;

                            let mut matrix = matrix * mesh.transform;

                            for (_selector, override_) in &model_description.overrides {
                                match override_ {
                                    MeshOverride::Material(material) => {
                                        override_material_description = Some(material);
                                    }
                                    MeshOverride::Transform(xform) => {
                                        matrix = matrix * xform;
                                    }
                                }
                            }

                            // A debug render mode overrides everything — ignore
                            // per-mesh material selectors so the whole model is
                            // visualized (and skinned meshes still get joints).
                            if normals_debug {
                                override_material_description = None;
                            }

                            if let Some(material_description) = override_material_description {
                                let material =
                                    material_description.get(render_context, scene_context);

                                material.draw_opaque(
                                    &render_context,
                                    projection_matrix,
                                    view_matrix,
                                    &matrix,
                                    &[],
                                );
                            } else {
                                let joints = if is_skinned {
                                    match hydrated_model.animations.get(animation_index) {
                                        Some(animation) => {
                                            let time = render_context.frame_time.tts
                                                % animation.duration;
                                            let animated_skeleton = Skeleton::animate(
                                                &hydrated_model.skeleton,
                                                animation,
                                                time,
                                            );
                                            animated_skeleton.get_skinning_transforms()
                                        }
                                        None => vec![Matrix4::identity(); 50],
                                    }
                                } else {
                                    vec![]
                                };

                                // Bind textures
                                mesh.base_color_texture.bind(0, &render_context);
                                model_material.draw_opaque(
                                    &render_context,
                                    projection_matrix,
                                    view_matrix,
                                    &matrix,
                                    &joints,
                                );
                            };

                            // TODO: Bring back drawing
                            mesh.mesh.draw(&render_context.gl)
                        }
                    }
                }
            }

            SceneObject::Material(material_description, items) => {
                let material = material_description.get(render_context, scene_context);
                for item in items.into_iter() {
                    item.render(
                        &render_context,
                        &scene_context,
                        &world_matrix,
                        &projection_matrix,
                        &view_matrix,
                        &material,
                    )
                }
            }

            SceneObject::Group(items) => {
                let new_world_matrix = world_matrix * self.xform;
                for item in items.into_iter() {
                    item.render(
                        &render_context,
                        &scene_context,
                        &new_world_matrix,
                        &projection_matrix,
                        &view_matrix,
                        current_material,
                    )
                }
            }
            SceneObject::Geometry(Shape::Cube) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );
                scene_context.cube.borrow_mut().draw(&render_context.gl);
            }
            SceneObject::Geometry(Shape::Cylinder) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );

                scene_context.cylinder.borrow_mut().draw(&render_context.gl);
            }
            SceneObject::Geometry(Shape::Sphere) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );

                scene_context.sphere.borrow_mut().draw(&render_context.gl);
            }
            SceneObject::Geometry(Shape::Quad) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );
                scene_context.quad.borrow_mut().draw(&render_context.gl);
            }
            SceneObject::Geometry(Shape::Plane) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );
                scene_context.plane.borrow_mut().draw(&render_context.gl);
            }
            SceneObject::Geometry(Shape::Heightmap { rows, cols, heights }) => {
                let xform = world_matrix * self.xform;
                geometry_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );
                // Build the grid mesh once per unique (rows, cols, heights) and
                // cache it; static terrain reuses the same GL buffers each frame.
                let key = heightmap_key(*rows, *cols, heights);
                scene_context
                    .heightmaps
                    .borrow_mut()
                    .entry(key)
                    .or_insert_with(|| {
                        geometry::Heightmap::create(*rows as usize, *cols as usize, heights)
                    });
                scene_context.heightmaps.borrow()[&key].draw(&render_context.gl);
            }
        }
    }
}

/// Content hash of a heightmap's parameters, used to cache its built GL mesh.
fn heightmap_key(rows: u32, cols: u32, heights: &[f32]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rows.hash(&mut hasher);
    cols.hash(&mut hasher);
    for h in heights {
        h.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

fn serialize_matrix<S>(matrix: &Matrix4<f32>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let array: [[f32; 4]; 4] = [
        [matrix.x[0], matrix.x[1], matrix.x[2], matrix.x[3]],
        [matrix.y[0], matrix.y[1], matrix.y[2], matrix.y[3]],
        [matrix.z[0], matrix.z[1], matrix.z[2], matrix.z[3]],
        [matrix.w[0], matrix.w[1], matrix.w[2], matrix.w[3]],
    ];
    array.serialize(serializer)
}

fn deserialize_matrix<'de, D>(deserializer: D) -> Result<Matrix4<f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let array = <[[f32; 4]; 4]>::deserialize(deserializer)?;
    Ok(Matrix4::new(
        array[0][0],
        array[0][1],
        array[0][2],
        array[0][3],
        array[1][0],
        array[1][1],
        array[1][2],
        array[1][3],
        array[2][0],
        array[2][1],
        array[2][2],
        array[2][3],
        array[3][0],
        array[3][1],
        array[3][2],
        array[3][3],
    ))
}
