//! The GL half of `Scene.text`: every label the walk queued this pass, in
//! one alpha-blended draw over the Inter MSDF atlas.
//!
//! The walk queues each text leaf with its placement already resolved
//! (facing labels against this pass's view), so the batch is one vertex
//! buffer per pass and eye, filled into a reused `Vec`: a frame of static
//! labels lays nothing out and allocates nothing once warm.

use std::mem::size_of;
use std::sync::Arc;

use cgmath::{Matrix4, Vector4};
use glow::HasContext;

use super::text_layout::GlyphQuad;
use crate::overlay_font;
use crate::shader::{Shader, ShaderType};
use crate::shader_program::{ShaderProgram, UniformLocation};
use crate::RenderContext;

/// One queued label: its glyphs, where they go, and their linear colour.
pub struct TextDraw {
    pub quads: Arc<[GlyphQuad]>,
    pub xform: Matrix4<f32>,
    pub color: [f32; 4],
}

/// Floats per vertex: position (3), atlas UV (2), linear colour (4).
const FLOATS: usize = 9;

/// The batch's vertices, four per glyph, into `out` (cleared first).
/// GL-free, so `frame_bench` can price it.
pub fn build_vertices(draws: &[TextDraw], out: &mut Vec<f32>) {
    out.clear();
    for draw in draws {
        for quad in draw.quads.iter() {
            let [l, b, r, t] = quad.rect;
            let [u0, v0, u1, v1] = quad.uv;
            // Counter-clockwise from the bottom left; V runs down the atlas.
            for (x, y, u, v) in [(l, b, u0, v1), (r, b, u1, v1), (r, t, u1, v0), (l, t, u0, v0)] {
                let p = draw.xform * Vector4::new(x, y, 0.0, 1.0);
                out.extend_from_slice(&[p.x, p.y, p.z, u, v]);
                out.extend_from_slice(&draw.color);
            }
        }
    }
}

const VERTEX_SHADER: &str = r#"
    layout (location = 0) in vec3 inPos;
    layout (location = 1) in highp vec2 inUv;
    layout (location = 2) in vec4 inColor;
    uniform mat4 viewProjection;
    out highp vec2 v_uv;
    out vec4 v_color;
    void main() {
        v_uv = inUv;
        v_color = inColor;
        gl_Position = viewProjection * vec4(inPos, 1.0);
    }
"#;

const FRAGMENT_SHADER: &str = r#"
    uniform sampler2D u_atlas;
    uniform bool u_linear_blend;
    in highp vec2 v_uv;
    in vec4 v_color;
    out vec4 fragColor;
    void main() {
        highp vec2 texels = functorMsdfTexels(v_uv);
        float coverage = functorMsdfCoverage(u_atlas, v_uv, texels, v_color.rgb, u_linear_blend);
        // Outside the outline: no colour and, under passthrough, no alpha.
        if (coverage <= 0.0) discard;
        fragColor = functorOutput(vec4(v_color.rgb, v_color.a * coverage));
    }
"#;

pub(super) struct TextRenderer {
    program: ShaderProgram,
    view_projection: UniformLocation,
    atlas_loc: UniformLocation,
    linear_blend: UniformLocation,
    output: crate::color_space::OutputEncodeUniform,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    // Held for the VAO, which records it as the element buffer.
    _ebo: glow::Buffer,
    atlas: glow::Texture,
    /// Glyphs the index buffer covers.
    glyph_capacity: usize,
    vertices: Vec<f32>,
}

impl TextRenderer {
    pub(super) fn new(gl: &glow::Context, shader_version: &str) -> Self {
        let vertex = Shader::build(gl, ShaderType::Vertex, VERTEX_SHADER, shader_version);
        let fragment = Shader::build(
            gl,
            ShaderType::Fragment,
            &format!("{}{FRAGMENT_SHADER}", overlay_font::msdf_glsl()),
            shader_version,
        );
        let program = ShaderProgram::link(gl, &vertex, &fragment);
        let counters = crate::gpu_counters::gpu_counters();
        unsafe {
            let vao = gl.create_vertex_array().expect("text VAO");
            let vbo = gl.create_buffer().expect("text vertices");
            let ebo = gl.create_buffer().expect("text indices");
            counters.vao_created();
            counters.buffer_created();
            counters.buffer_created();
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = (FLOATS * size_of::<f32>()) as i32;
            for (location, size, offset) in [(0, 3, 0), (1, 2, 3), (2, 4, 5)] {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(
                    location,
                    size,
                    glow::FLOAT,
                    false,
                    stride,
                    offset * size_of::<f32>() as i32,
                );
            }
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            TextRenderer {
                view_projection: program.get_uniform_location(gl, "viewProjection"),
                atlas_loc: program.get_uniform_location(gl, "u_atlas"),
                linear_blend: program.get_uniform_location(gl, "u_linear_blend"),
                output: crate::color_space::OutputEncodeUniform::get(&program, gl),
                program,
                vao,
                vbo,
                _ebo: ebo,
                atlas: overlay_font::upload_atlas(gl),
                glyph_capacity: 0,
                vertices: Vec::new(),
            }
        }
    }

    /// Draw `draws` in one call, far labels first: depth-tested, no depth
    /// write, blended over what is drawn.
    pub(super) fn draw(
        &mut self,
        ctx: &RenderContext,
        draws: &mut [TextDraw],
        projection: &Matrix4<f32>,
        view: &Matrix4<f32>,
    ) {
        // View-space z grows toward the camera: ascending is far to near.
        draws.sort_by(|a, b| {
            let depth = |d: &TextDraw| (view * d.xform.w).z;
            depth(a).total_cmp(&depth(b))
        });
        build_vertices(draws, &mut self.vertices);
        let glyphs = self.vertices.len() / (4 * FLOATS);
        if glyphs == 0 {
            return;
        }
        let gl = ctx.gl;
        let counters = crate::gpu_counters::gpu_counters();
        unsafe {
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            // A fresh store each pass (orphaning): the other eye's draw may
            // still read the last one, which a tiler would otherwise wait on.
            let bytes = crate::terrain_renderer::slice_bytes(&self.vertices);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
            counters.uploaded(bytes.len());
            // The quads' indices never change: grow them, never rewrite.
            if glyphs > self.glyph_capacity {
                let capacity = glyphs.next_power_of_two();
                let indices: Vec<u32> = (0..capacity as u32)
                    .flat_map(|g| [0, 1, 2, 0, 2, 3].map(|k| 4 * g + k))
                    .collect();
                let index_bytes = crate::terrain_renderer::slice_bytes(&indices);
                gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, index_bytes, glow::STATIC_DRAW);
                counters.uploaded(index_bytes.len());
                self.glyph_capacity = capacity;
            }

            let p = &self.program;
            p.use_program(gl);
            p.set_uniform_matrix4(gl, &self.view_projection, &(projection * view));
            p.set_uniform_1i(gl, &self.atlas_loc, 0);
            // A hardware sRGB target blends in linear light; the shader-encoded
            // targets (desktop, web) blend the encoded values.
            p.set_uniform_1i(gl, &self.linear_blend, !ctx.output_srgb_encode as i32);
            // The fragment's alpha is its edge coverage: write it, whatever
            // the pass writes for opaque surfaces.
            self.output.set(
                p,
                gl,
                ctx.output_srgb_encode,
                crate::color_space::OutputAlpha::Blend,
            );
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));

            // Face culling is never on in this engine: both sides draw. A
            // pass that owns blending (2D layers) keeps its own state.
            let owned = !ctx.pass_blends;
            if owned {
                gl.enable(glow::BLEND);
                gl.blend_func_separate(
                    glow::SRC_ALPHA,
                    glow::ONE_MINUS_SRC_ALPHA,
                    glow::ONE,
                    glow::ONE_MINUS_SRC_ALPHA,
                );
            }
            gl.depth_mask(false);
            gl.draw_elements(glow::TRIANGLES, (glyphs * 6) as i32, glow::UNSIGNED_INT, 0);
            gl.depth_mask(true);
            if owned {
                gl.disable(glow::BLEND);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
    }
}
