#[derive(Clone, Copy)]
pub enum PixelFormat {
    RGB,
    RGBA,
}

#[derive(Clone)]
pub struct TextureData {
    pub bytes: std::vec::Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
}

pub trait TextureFormat {
    fn load(&self, buffer: &std::vec::Vec<u8>) -> TextureData;
}

pub struct FormatUsingImageCrate {
    image_format: image::ImageFormat,
}

impl TextureFormat for FormatUsingImageCrate {
    fn load(&self, buffer: &std::vec::Vec<u8>) -> TextureData {
        let img = image::load_from_memory_with_format(buffer, self.image_format)
            .expect("Failed to load texture");
        let mut data = img.to_rgba8().into_raw();
        apply_color_key(&mut data, img.width(), img.height());

        TextureData {
            bytes: data,
            width: img.width(),
            height: img.height(),
            format: PixelFormat::RGBA,
        }
    }
}

fn apply_color_key(pixels: &mut Vec<u8>, width: u32, height: u32) {
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

pub const PNG: FormatUsingImageCrate = FormatUsingImageCrate {
    image_format: image::ImageFormat::Png,
};
pub const JPEG: FormatUsingImageCrate = FormatUsingImageCrate {
    image_format: image::ImageFormat::Jpeg,
};
pub const GIF: FormatUsingImageCrate = FormatUsingImageCrate {
    image_format: image::ImageFormat::Gif,
};

pub fn extension_to_format(str: String) -> Option<Box<dyn TextureFormat>> {
    let lowercase_str = str.to_ascii_lowercase();
    match lowercase_str.as_str() {
        "png" => Some(Box::new(PNG)),
        "gif" => Some(Box::new(GIF)),
        "jpeg" => Some(Box::new(JPEG)),
        "jpg" => Some(Box::new(JPEG)),
        _ => None,
    }
}
