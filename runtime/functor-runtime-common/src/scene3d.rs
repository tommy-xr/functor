use cgmath::{vec3, Matrix4, SquareMatrix};
use serde::{Deserialize, Serialize};

use fable_library_rust::NativeArray_::Array;

const CRATE_BYTES: &[u8] = include_bytes!("../included-assets/crate.png");

use crate::{
    geometry::{self, Geometry},
    material::BasicMaterial,
    math::Angle,
    texture::{
        PixelFormat, RuntimeTexture, Texture2D, TextureData, TextureFormat, TextureOptions, PNG,
    },
    RenderContext,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Shape {
    Cube,
    Sphere,
    Cylinder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SceneObject {
    Geometry(Shape),
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

    fn create_checkerboard_pattern(width: u32, height: u32, color: [u8; 4]) -> TextureData {
        let mut bytes = Vec::with_capacity((width * height * 4) as usize);

        for y in 0..height {
            for x in 0..width {
                let is_white = (x + y) % 2 == 0;
                let color = if is_white {
                    color
                } else {
                    [0, 0, 0, 255] // Black with full opacity
                };

                bytes.extend_from_slice(&color);
            }
        }

        TextureData {
            bytes,
            width,
            height,
            format: PixelFormat::RGBA,
        }
    }
    pub fn render(
        &self,
        context: &RenderContext,
        world_matrix: &Matrix4<f32>,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
    ) {
        let texture_data1 = Self::create_checkerboard_pattern(8, 8, [255, 0, 0, 255]);
        let texture_data2 = Self::create_checkerboard_pattern(8, 8, [0, 0, 255, 255]);
        // let texture_data = PNG.load(&CRATE_BYTES.to_vec());
        let texture1 = Texture2D::init_from_data(texture_data1, TextureOptions::default());
        let texture2 = Texture2D::init_from_data(texture_data2, TextureOptions::default());
        texture1.bind(0, &context);
        texture2.bind(1, &context);

        // HACK: Since we're initializing the textures every frame,
        // reset the bindings because they might've been paved by initialization...
        texture1.bind(0, &context);
        texture2.bind(1, &context);
        // TODO: Factor out to pass in current_material
        let mut basic_material = BasicMaterial::create();
        basic_material.initialize(&context);

        let skinning_data = vec![];
        match &self.obj {
            SceneObject::Group(items) => {
                let new_world_matrix = world_matrix * self.xform;
                for item in items.into_iter() {
                    item.render(
                        &context,
                        &new_world_matrix,
                        &projection_matrix,
                        &view_matrix,
                    )
                }
            }
            SceneObject::Geometry(Shape::Cube) => {
                let xform = world_matrix * self.xform;
                basic_material.draw_opaque(
                    &context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );
                let mut cube = geometry::Cube::create();
                cube.draw(&context.gl);
            }
            SceneObject::Geometry(Shape::Cylinder) => {
                let xform = world_matrix * self.xform;
                basic_material.draw_opaque(
                    &context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );

                let mut cylinder = geometry::Cylinder::create();
                cylinder.draw(&context.gl);
            }
            SceneObject::Geometry(Shape::Sphere) => {
                let xform = world_matrix * self.xform;
                basic_material.draw_opaque(
                    &context,
                    &projection_matrix,
                    &view_matrix,
                    &xform,
                    &skinning_data,
                );

                let mut sphere = geometry::Sphere::create();
                sphere.draw(&context.gl);
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
