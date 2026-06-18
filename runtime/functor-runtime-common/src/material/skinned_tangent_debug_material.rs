use cgmath::Matrix4;

use crate::shader_program::ShaderProgram;
use crate::shader_program::UniformLocation;
use crate::RenderContext;

use super::Material;

// The skinned counterpart of `TangentDebugMaterial`: deforms the tangent by the
// same joint blend as the position (rotation only — `mat3`), rotates it into
// world space, and visualizes it as RGB. Used by `DebugRenderMode::Tangents` for
// skinned glTF meshes; static meshes use the non-skinned variant.
const VERTEX_SHADER_SOURCE: &str = r#"
        #define MAX_JOINTS 200

        layout (location = 0) in vec3 inPos;
        layout (location = 1) in vec2 inTex;
        layout (location = 2) in vec3 inNormal;
        layout (location = 3) in vec4 inTangent;
        layout (location = 4) in vec4 inJointIndices;
        layout (location = 5) in vec4 inWeights;

        uniform mat4 jointTransforms[MAX_JOINTS];
        uniform mat4 world;
        uniform mat4 view;
        uniform mat4 projection;

        out vec3 worldTangent;

        void main() {
            mat4 skinMatrix =
                inWeights.x * jointTransforms[int(inJointIndices.x)] +
                inWeights.y * jointTransforms[int(inJointIndices.y)] +
                inWeights.z * jointTransforms[int(inJointIndices.z)] +
                inWeights.w * jointTransforms[int(inJointIndices.w)];

            worldTangent = mat3(world) * mat3(skinMatrix) * inTangent.xyz;

            vec4 skinnedPos = skinMatrix * vec4(inPos, 1.0);
            gl_Position = projection * view * world * skinnedPos;
        }
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec3 worldTangent;

        void main() {
            vec3 t = normalize(worldTangent);
            fragColor = vec4(t * 0.5 + 0.5, 1.0);
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    joint_transforms_loc: UniformLocation,
}

static mut SHADER_PROGRAM: Option<(ShaderProgram, Uniforms)> = None;

pub struct SkinnedTangentDebugMaterial;

use crate::shader::Shader;
use crate::shader::ShaderType;

impl Material for SkinnedTangentDebugMaterial {
    fn initialize(&mut self, ctx: &RenderContext) {
        unsafe {
            #[allow(static_mut_refs)]
            if SHADER_PROGRAM.is_none() {
                let vertex_shader = Shader::build(
                    ctx.gl,
                    ShaderType::Vertex,
                    VERTEX_SHADER_SOURCE,
                    ctx.shader_version,
                );

                let fragment_shader = Shader::build(
                    ctx.gl,
                    ShaderType::Fragment,
                    FRAGMENT_SHADER_SOURCE,
                    ctx.shader_version,
                );

                let shader = crate::shader_program::ShaderProgram::link(
                    &ctx.gl,
                    &vertex_shader,
                    &fragment_shader,
                );

                let uniforms = Uniforms {
                    world_loc: shader.get_uniform_location(ctx.gl, "world"),
                    view_loc: shader.get_uniform_location(ctx.gl, "view"),
                    projection_loc: shader.get_uniform_location(ctx.gl, "projection"),
                    joint_transforms_loc: shader.get_uniform_location(ctx.gl, "jointTransforms"),
                };

                SHADER_PROGRAM = Some((shader, uniforms));
            }
        }
    }

    fn draw_opaque(
        &self,
        ctx: &RenderContext,
        projection_matrix: &Matrix4<f32>,
        view_matrix: &Matrix4<f32>,
        world_matrix: &Matrix4<f32>,
        skinning_data: &[Matrix4<f32>],
    ) -> bool {
        unsafe {
            #[allow(static_mut_refs)]
            if let Some((shader, uniforms)) = &SHADER_PROGRAM {
                let p = shader;
                p.use_program(ctx.gl);

                p.set_uniform_matrix4(ctx.gl, &uniforms.world_loc, world_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.view_loc, view_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.projection_loc, projection_matrix);

                let num_joints = skinning_data.len();
                let mut joint_matrices = Vec::with_capacity(num_joints * 16);
                for i in 0..num_joints {
                    let matrix_array: &[f32; 16] = skinning_data[i].as_ref();
                    joint_matrices.extend_from_slice(matrix_array);
                }

                p.set_uniform_matrix4fv(ctx.gl, &uniforms.joint_transforms_loc, &joint_matrices);
            }
        }

        true
    }
}

impl SkinnedTangentDebugMaterial {
    pub fn create() -> Box<dyn Material> {
        Box::new(SkinnedTangentDebugMaterial)
    }
}
