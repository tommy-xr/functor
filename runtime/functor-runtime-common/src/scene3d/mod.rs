use std::{cell::RefCell, sync::Arc};

use cgmath::{point3, vec3, vec4, Matrix4, SquareMatrix, Transform};
use serde::{Deserialize, Serialize};

use fable_library_rust::NativeArray_::Array;

use crate::{
    asset::{
        self,
        pipelines::{ModelPipeline, TexturePipeline},
        AssetHandle, BuiltAssetPipeline,
    },
    geometry::{self, Geometry, Mesh},
    material::{ColorMaterial, Material, SkinnedMaterial},
    math::Angle,
    model::{Model, Skeleton},
    texture::{RuntimeTexture, Texture2D},
    RenderContext,
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
    cylinder: RefCell<Mesh>,
    sphere: RefCell<Mesh>,
}

impl SceneContext {
    pub fn new() -> SceneContext {
        SceneContext {
            cube: RefCell::new(geometry::Cube::create()),
            sphere: RefCell::new(geometry::Sphere::create()),
            cylinder: RefCell::new(geometry::Cylinder::create()),
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

    pub fn cylinder() -> Self {
        Scene3D {
            obj: SceneObject::Geometry(Shape::Cylinder),
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
        match &self.obj {
            SceneObject::Model(model_description) => {
                let mut basic_material = SkinnedMaterial::create();
                basic_material.initialize(&render_context);

                match &model_description.handle {
                    ModelHandle::File(str) => {
                        let model: Arc<AssetHandle<Model>> = render_context
                            .asset_cache
                            .load_asset_with_pipeline(scene_context.model_pipeline.clone(), str);

                        let hydrated_model = model.get();

                        let matrix = world_matrix * self.xform;

                        // println!("SKELETON: {:#?}", hydrated_model.skeleton);
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
                                let maybe_animation =
                                    hydrated_model.animations.get(animation_index);
                                let joints = if let Some(animation) = maybe_animation {
                                    let time = render_context.frame_time.tts % animation.duration;
                                    let animated_skeleton = Skeleton::animate(
                                        &hydrated_model.skeleton,
                                        animation,
                                        time,
                                    );
                                    animated_skeleton.get_skinning_transforms()
                                } else {
                                    let mut joints = Vec::new();

                                    for i in 0..50 {
                                        joints.push(Matrix4::identity());
                                    }
                                    joints
                                };

                                // Bind textures
                                mesh.base_color_texture.bind(0, &render_context);
                                basic_material.draw_opaque(
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

                        // TEMPORARY: Render joints
                        let maybe_animation = hydrated_model.animations.get(animation_index);
                        if let Some(animation) = maybe_animation {
                            let time = render_context.frame_time.tts % animation.duration;
                            let animated_skeleton =
                                Skeleton::animate(&hydrated_model.skeleton, animation, time);

                            println!(
                                "Animating {} {} {}",
                                animation.name, animation.duration, time,
                            );

                            let joints = animated_skeleton.get_transforms();
                            for joint_transform in joints {
                                let mut color_material =
                                    ColorMaterial::create(vec4(0.0, 1.0, 0.0, 1.0));
                                color_material.initialize(&render_context);

                                let xform = &(matrix * joint_transform);
                                let point = xform.transform_point(point3(0.0, 0.0, 0.0));
                                let xform2 =
                                    Matrix4::from_translation(vec3(point.x, point.y, point.z))
                                        * Matrix4::from_scale(0.1);

                                color_material.draw_opaque(
                                    &render_context,
                                    projection_matrix,
                                    view_matrix,
                                    &xform2,
                                    &[],
                                );

                                scene_context.sphere.borrow().draw(render_context.gl);
                            }
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
                current_material.draw_opaque(
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
                current_material.draw_opaque(
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
                current_material.draw_opaque(
                    &render_context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );

                scene_context.sphere.borrow_mut().draw(&render_context.gl);
            }
        }
    }
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
