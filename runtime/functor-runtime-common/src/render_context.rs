pub struct RenderContext<'a> {
    pub gl: &'a glow::Context,
    pub shader_version: &'a str,
}
