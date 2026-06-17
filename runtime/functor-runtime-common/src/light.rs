use cgmath::{vec3, InnerSpace, Vector3};
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
}

/// The lighting terms the single-directional lit shader consumes today: summed
/// ambient color, plus the first directional light's normalized direction and
/// its color premultiplied by intensity. Additional directional lights are
/// ignored for now — the bounded multi-light array is a later step.
pub struct ResolvedLighting {
    pub ambient: Vector3<f32>,
    pub directional_dir: Vector3<f32>,
    pub directional_color: Vector3<f32>,
}

pub fn resolve_lighting(lights: &[Light]) -> ResolvedLighting {
    let mut ambient = vec3(0.0, 0.0, 0.0);
    let mut directional_dir = vec3(0.0, -1.0, 0.0);
    let mut directional_color = vec3(0.0, 0.0, 0.0);
    let mut have_directional = false;

    for light in lights {
        match light {
            Light::Ambient { color } => {
                ambient += vec3(color[0], color[1], color[2]);
            }
            Light::Directional {
                direction,
                color,
                intensity,
            } => {
                if !have_directional {
                    let d = vec3(direction[0], direction[1], direction[2]);
                    directional_dir = if d.magnitude2() > 1e-8 {
                        d.normalize()
                    } else {
                        vec3(0.0, -1.0, 0.0)
                    };
                    directional_color = vec3(color[0], color[1], color[2]) * *intensity;
                    have_directional = true;
                }
            }
        }
    }

    ResolvedLighting {
        ambient,
        directional_dir,
        directional_color,
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_lighting, Light};

    #[test]
    fn sums_ambient_and_takes_first_directional() {
        let lights = [
            Light::ambient(0.1, 0.1, 0.1),
            Light::ambient(0.2, 0.0, 0.0),
            Light::directional(0.0, -1.0, 0.0, 1.0, 1.0, 1.0, 2.0),
            Light::directional(1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0),
        ];
        let r = resolve_lighting(&lights);
        assert_eq!(r.ambient, cgmath::vec3(0.3, 0.1, 0.1));
        // First directional: color * intensity = (2,2,2).
        assert_eq!(r.directional_color, cgmath::vec3(2.0, 2.0, 2.0));
    }

    #[test]
    fn no_lights_is_black() {
        let r = resolve_lighting(&[]);
        assert_eq!(r.ambient, cgmath::vec3(0.0, 0.0, 0.0));
        assert_eq!(r.directional_color, cgmath::vec3(0.0, 0.0, 0.0));
    }
}
