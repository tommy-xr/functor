use super::TextureData;

#[derive(Clone, Copy)]
pub enum PixelFormat {
    RGB,
    RGBA,
}

/// Why image bytes did not decode: the byte offset (0 where the decoder
/// doesn't say) and what was wrong there.
#[derive(Debug, PartialEq)]
pub struct ImageError {
    pub offset: usize,
    pub message: String,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at byte {}: {}", self.offset, self.message)
    }
}

/// An image file's RGBA8 pixels, top row first, recognised by content: PNG,
/// JPEG, GIF (its first frame) or PCX. Corrupt bytes are an Err, never a
/// panic — asset hot-reload can catch a file mid-write, and the render
/// thread polls asset futures (a panic aborts the runtime).
pub fn decode_image(bytes: &[u8]) -> Result<TextureData, ImageError> {
    if super::is_pcx(bytes) {
        return super::decode_pcx(bytes);
    }
    let error = |message: String| ImageError { offset: 0, message };
    match image::guess_format(bytes) {
        Ok(
            format @ (image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Gif),
        ) => image::load_from_memory_with_format(bytes, format)
            .map(TextureData::from_image)
            .map_err(|e| error(e.to_string())),
        _ => Err(error("not a PNG, JPEG, GIF or PCX image".to_string())),
    }
}

/// File textures' colour key: near-magenta and near-cyan texels become
/// transparent black.
pub fn apply_color_key(pixels: &mut Vec<u8>, width: u32, height: u32) {
    for x in 0..width {
        for y in 0..height {
            let pos = (((y * width) + x) * 4u32) as usize;

            let r = pixels[pos];
            let g = pixels[pos + 1usize];
            let b = pixels[pos + 2usize];
            let _a = pixels[pos + 3usize];

            if r > 250 && g < 5 && b > 250 || r < 5 && g > 250 && b > 250 {
                pixels[pos] = 0;
                pixels[pos + 1usize] = 0;
                pixels[pos + 2usize] = 0;
                pixels[pos + 3usize] = 0;
            }
        }
    }
}
