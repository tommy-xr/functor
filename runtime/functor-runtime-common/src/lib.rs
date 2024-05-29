use std::any::Any;

#[derive(Debug, Clone)]
pub enum Scene3D {
    Cube,
    Sphere,
}

impl Scene3D {
    pub fn cube() -> Self {
        Self::Cube
    }

    pub fn sphere() -> Self {
        Self::Sphere
    }
}

pub fn add(left: usize, right: usize) -> usize {
    left + right
}

pub struct OpaqueState {
    any: Box<dyn Any>,
}

impl OpaqueState {
    pub fn new<T: 'static>(obj: T) -> OpaqueState {
        OpaqueState { any: Box::new(obj) }
    }

    pub fn coerce<T: 'static + Clone + Sized>(opaque_state: OpaqueState) -> T {
        unsafe {
            // Convert Box<dyn Any> to raw pointer
            let raw = Box::into_raw(opaque_state.any);
            // Convert the raw pointer to Box<T>
            let boxed_t: Box<T> = Box::from_raw(raw as *mut T);
            // Unbox to get the inner T
            *boxed_t
        }
    }
}

pub mod geometry;
pub mod material;
mod shader;
mod shader_program;
pub mod texture;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
