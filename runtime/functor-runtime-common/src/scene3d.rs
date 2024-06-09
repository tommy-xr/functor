use cgmath::{vec3, Matrix4, SquareMatrix};
use serde::{Deserialize, Serialize};

use crate::geometry::Geometry;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Shape {
    Cube,
    Sphere,
    Cylinder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SceneObject {
    Geometry(Shape),
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

    pub fn transform(self, xform: Matrix4<f32>) -> Self {
        Scene3D {
            xform: self.xform * xform,
            ..self
        }
    }

    pub fn translate_y(self, y: f32) -> Self {
        self.transform(Matrix4::from_translation(vec3(0.0, y, 0.0)))
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
