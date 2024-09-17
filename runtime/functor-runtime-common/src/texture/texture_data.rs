use image::DynamicImage;

use super::PixelFormat;

#[derive(Clone)]
pub struct TextureData {
    pub bytes: std::vec::Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
}

impl TextureData {
    pub fn from_image(image: DynamicImage) -> TextureData {
        let bytes = image.to_rgba8();

        TextureData {
            bytes: bytes.to_vec(),
            width: image.width(),
            height: image.height(),
            format: PixelFormat::RGBA,
        }
    }

    pub fn checkerboard_pattern(width: u32, height: u32, color: [u8; 4]) -> TextureData {
        let mut bytes = Vec::with_capacity((width * height * 4) as usize);

        for y in 0..height {
            for x in 0..width {
                let is_white = (x + y) % 2 == 0;
                let color = if is_white {
                    color
                } else {
                    [0, 0, 0, 255] // Black with full opacity
                };

                bytes.extend_from_slice(&color);
            }
        }

        TextureData {
            bytes,
            width,
            height,
            format: PixelFormat::RGBA,
        }
    }
}
