use cgmath::Matrix4;

use crate::shader_program::ShaderProgram;
use crate::shader_program::UniformLocation;
use crate::RenderContext;

use super::Material;

const VERTEX_SHADER_SOURCE: &str = r#"
        #define MAX_JOINTS 200

        layout (location = 0) in vec3 inPos;
        layout (location = 1) in vec2 inTex;
        layout (location = 2) in vec4 inJointIndices;
        layout (location = 3) in vec4 inWeights;

        uniform mat4 jointTransforms[MAX_JOINTS];
        uniform mat4 world;
        uniform mat4 view;
        uniform mat4 projection;

        out vec2 texCoord;
        out vec3 weights;

        void main() {

            // Compute the skinning matrix
            mat4 skinMatrix = 
                inWeights.x * jointTransforms[int(inJointIndices.x)] +
                inWeights.y * jointTransforms[int(inJointIndices.y)] +
                inWeights.z * jointTransforms[int(inJointIndices.z)] +
                inWeights.w * jointTransforms[int(inJointIndices.w)];

            texCoord = inTex;
            weights = inJointIndices.xyz;

            // Apply the skinning transformation
            vec4 skinnedPos = skinMatrix * vec4(inPos, 1.0);

            gl_Position = projection * view * world * skinnedPos;
        }
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
        out vec4 fragColor;

        in vec2 texCoord;
        in vec3 weights;

        uniform sampler2D texture1;

        void main() {
            //fragColor = vec4(texCoord.x, texCoord.y, 0.0, 1.0);
            fragColor = texture(texture1, texCoord);
            //fragColor = vec4(weights.x / 50.0, weights.y / 50.0, weights.z / 50.0, 1.0);
        }
"#;

struct Uniforms {
    world_loc: UniformLocation,
    view_loc: UniformLocation,
    projection_loc: UniformLocation,
    texture_loc: UniformLocation,
    joint_transforms_loc: UniformLocation,
}

// TODO: We'll have to re-think this pattern
// Maybe we need a shader repository or something to pull from
static mut SHADER_PROGRAM: Option<(ShaderProgram, Uniforms)> = None;

pub struct SkinnedMaterial;

use crate::shader::Shader;
use crate::shader::ShaderType;

impl Material for SkinnedMaterial {
    fn initialize(&mut self, ctx: &RenderContext) {
        unsafe {
            if SHADER_PROGRAM.is_none() {
                let vertex_shader = Shader::build(
                    ctx.gl,
                    ShaderType::Vertex,
                    VERTEX_SHADER_SOURCE,
                    ctx.shader_version,
                );

                // fragment shader
                let fragment_shader = Shader::build(
                    ctx.gl,
                    ShaderType::Fragment,
                    FRAGMENT_SHADER_SOURCE,
                    ctx.shader_version,
                );
                // link shaders

                let shader = crate::shader_program::ShaderProgram::link(
                    &ctx.gl,
                    &vertex_shader,
                    &fragment_shader,
                );

                let uniforms = Uniforms {
                    world_loc: shader.get_uniform_location(ctx.gl, "world"),
                    view_loc: shader.get_uniform_location(ctx.gl, "view"),
                    projection_loc: shader.get_uniform_location(ctx.gl, "projection"),
                    texture_loc: shader.get_uniform_location(ctx.gl, "texture1"),
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
            // TODO: Find another approach to do this - maybe a shader repository?
            #[allow(static_mut_refs)]
            if let Some((shader, uniforms)) = &SHADER_PROGRAM {
                let p = shader;
                p.use_program(ctx.gl);

                p.set_uniform_matrix4(ctx.gl, &uniforms.world_loc, world_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.view_loc, view_matrix);
                p.set_uniform_matrix4(ctx.gl, &uniforms.projection_loc, projection_matrix);
                p.set_uniform_1i(ctx.gl, &uniforms.texture_loc, 0);

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

impl SkinnedMaterial {
    pub fn create() -> Box<dyn Material> {
        Box::new(SkinnedMaterial)
    }
}
