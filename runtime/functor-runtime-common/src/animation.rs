use cgmath::{Quaternion, Vector3};

pub struct Animation {
    pub name: String,
    pub channels: Vec<AnimationChannel>,
    pub duration: f32,
}

pub enum AnimationProperty {
    Translation,
    Rotation,
    Scale,
    // TODO: Morph target
    Weights,
}

pub struct AnimationChannel {
    pub target_node_index: usize,
    pub target_property: AnimationProperty,
    pub keyframes: Vec<Keyframe>,
    // interpolation: TODO?
}

#[derive(Clone)]
pub struct Keyframe {
    pub time: f32,
    pub value: AnimationValue,
}

#[derive(Clone)]
pub enum AnimationValue {
    Translation(Vector3<f32>),
    Rotation(Quaternion<f32>),
    Scale(Vector3<f32>),
    Weights(Vec<f32>),
}
