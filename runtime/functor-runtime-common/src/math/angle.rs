use cgmath::Rad;

pub struct Angle {
    ang: cgmath::Rad<f32>,
}

impl Angle {
    pub fn from_degrees(angle: f32) -> Angle {
        Angle {
            ang: cgmath::Deg(angle).into(),
        }
    }

    pub fn from_radians(angle: f32) -> Angle {
        Angle {
            ang: cgmath::Rad(angle),
        }
    }
}

impl Into<Rad<f32>> for Angle {
    fn into(self) -> Rad<f32> {
        self.ang
    }
}
