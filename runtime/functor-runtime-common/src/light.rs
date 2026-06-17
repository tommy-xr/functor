use serde::{Deserialize, Serialize};

/// A light source. Pure data in the `Frame`, so lights serialize for `/scene`
/// introspection. Colors/directions are plain `[f32; 3]` (Serialize-friendly).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Light {
    /// Uniform light from every direction: its color is added to every lit
    /// surface regardless of orientation.
    Ambient { color: [f32; 3] },
    /// A distant "sun": parallel rays travelling along `direction`. `intensity`
    /// scales `color`.
    Directional {
        direction: [f32; 3],
        color: [f32; 3],
        intensity: f32,
    },
    /// An omnidirectional point light at `position`, fading to nothing by
    /// `range` (world units).
    Point {
        position: [f32; 3],
        color: [f32; 3],
        intensity: f32,
        range: f32,
    },
    /// A cone of light from `position` aimed along `direction`, with a soft edge
    /// at `cone_angle` (radians from the axis) and distance falloff to `range`.
    Spot {
        position: [f32; 3],
        direction: [f32; 3],
        color: [f32; 3],
        intensity: f32,
        range: f32,
        cone_angle: f32,
    },
}

impl Light {
    pub fn ambient(r: f32, g: f32, b: f32) -> Light {
        Light::Ambient { color: [r, g, b] }
    }

    pub fn directional(
        dx: f32,
        dy: f32,
        dz: f32,
        r: f32,
        g: f32,
        b: f32,
        intensity: f32,
    ) -> Light {
        Light::Directional {
            direction: [dx, dy, dz],
            color: [r, g, b],
            intensity,
        }
    }

    pub fn point(px: f32, py: f32, pz: f32, r: f32, g: f32, b: f32, intensity: f32, range: f32) -> Light {
        Light::Point {
            position: [px, py, pz],
            color: [r, g, b],
            intensity,
            range,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spot(
        px: f32,
        py: f32,
        pz: f32,
        dx: f32,
        dy: f32,
        dz: f32,
        r: f32,
        g: f32,
        b: f32,
        intensity: f32,
        range: f32,
        cone_angle: f32,
    ) -> Light {
        Light::Spot {
            position: [px, py, pz],
            direction: [dx, dy, dz],
            color: [r, g, b],
            intensity,
            range,
            cone_angle,
        }
    }
}

/// The maximum lights a single `LitMaterial` shader evaluates per draw. Beyond
/// this the extra lights are dropped (the multi-pass additive path is a later
/// step). Must match `MAX_LIGHTS` in the lit shader.
pub const MAX_LIGHTS: usize = 8;

// Light type tags, shared with the lit shader's `lightType[]`.
const TYPE_AMBIENT: i32 = 0;
const TYPE_DIRECTIONAL: i32 = 1;
const TYPE_POINT: i32 = 2;
const TYPE_SPOT: i32 = 3;

/// Lights packed into fixed-length, flattened arrays for upload to the lit
/// shader's bounded uniform arrays. `color` is premultiplied by intensity.
pub struct LightUniforms {
    pub count: i32,
    pub types: [i32; MAX_LIGHTS],
    pub colors: [f32; 3 * MAX_LIGHTS],
    pub positions: [f32; 3 * MAX_LIGHTS],
    pub directions: [f32; 3 * MAX_LIGHTS],
    pub ranges: [f32; MAX_LIGHTS],
    pub cone_cos: [f32; MAX_LIGHTS],
}

/// Pack up to `MAX_LIGHTS` lights into flattened arrays for the lit shader.
pub fn pack_lights(lights: &[Light]) -> LightUniforms {
    let mut u = LightUniforms {
        count: 0,
        types: [TYPE_AMBIENT; MAX_LIGHTS],
        colors: [0.0; 3 * MAX_LIGHTS],
        positions: [0.0; 3 * MAX_LIGHTS],
        directions: [0.0; 3 * MAX_LIGHTS],
        ranges: [1.0; MAX_LIGHTS],
        cone_cos: [-1.0; MAX_LIGHTS],
    };

    for light in lights.iter().take(MAX_LIGHTS) {
        let i = u.count as usize;
        let c3 = i * 3;
        let mut set_color = |col: &[f32; 3], intensity: f32| {
            u.colors[c3] = col[0] * intensity;
            u.colors[c3 + 1] = col[1] * intensity;
            u.colors[c3 + 2] = col[2] * intensity;
        };
        match light {
            Light::Ambient { color } => {
                u.types[i] = TYPE_AMBIENT;
                set_color(color, 1.0);
            }
            Light::Directional {
                direction,
                color,
                intensity,
            } => {
                u.types[i] = TYPE_DIRECTIONAL;
                set_color(color, *intensity);
                u.directions[c3..c3 + 3].copy_from_slice(direction);
            }
            Light::Point {
                position,
                color,
                intensity,
                range,
            } => {
                u.types[i] = TYPE_POINT;
                set_color(color, *intensity);
                u.positions[c3..c3 + 3].copy_from_slice(position);
                u.ranges[i] = *range;
            }
            Light::Spot {
                position,
                direction,
                color,
                intensity,
                range,
                cone_angle,
            } => {
                u.types[i] = TYPE_SPOT;
                set_color(color, *intensity);
                u.positions[c3..c3 + 3].copy_from_slice(position);
                u.directions[c3..c3 + 3].copy_from_slice(direction);
                u.ranges[i] = *range;
                u.cone_cos[i] = cone_angle.cos();
            }
        }
        u.count += 1;
    }

    u
}

#[cfg(test)]
mod tests {
    use super::{pack_lights, Light, MAX_LIGHTS};

    #[test]
    fn packs_each_type_with_premultiplied_color() {
        let lights = [
            Light::ambient(0.1, 0.2, 0.3),
            Light::directional(0.0, -1.0, 0.0, 1.0, 1.0, 1.0, 2.0),
            Light::point(1.0, 2.0, 3.0, 1.0, 0.0, 0.0, 4.0, 10.0),
            Light::spot(0.0, 5.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 20.0, 0.0),
        ];
        let u = pack_lights(&lights);
        assert_eq!(u.count, 4);
        assert_eq!(u.types[0..4], [0, 1, 2, 3]);
        // Directional color premultiplied by intensity 2.
        assert_eq!(&u.colors[3..6], &[2.0, 2.0, 2.0]);
        // Point: position + range; color * intensity 4.
        assert_eq!(&u.positions[6..9], &[1.0, 2.0, 3.0]);
        assert_eq!(u.ranges[2], 10.0);
        assert_eq!(&u.colors[6..9], &[4.0, 0.0, 0.0]);
        // Spot: cone_cos = cos(0) = 1.
        assert!((u.cone_cos[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn empty_is_zero_count() {
        assert_eq!(pack_lights(&[]).count, 0);
    }

    #[test]
    fn caps_at_max_lights() {
        let many = vec![Light::ambient(0.1, 0.1, 0.1); MAX_LIGHTS + 4];
        assert_eq!(pack_lights(&many).count, MAX_LIGHTS as i32);
    }
}
