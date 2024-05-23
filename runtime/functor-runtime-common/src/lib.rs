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

pub mod geometry;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
