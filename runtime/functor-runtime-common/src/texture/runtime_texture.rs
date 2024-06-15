use crate::RenderContext;

pub trait RuntimeTexture {
    fn bind(&self, index: u32, render_context: &RenderContext);
}
