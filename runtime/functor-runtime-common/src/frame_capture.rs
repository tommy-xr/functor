//! Framebuffer readback and PNG encoding shared by runtime shells.
//!
//! OpenGL returns pixel rows from bottom to top, while image encoders expect
//! top-to-bottom rows. Keeping that conversion separate from readback makes
//! capture layout testable without a GL context.

use glow::HasContext;
use image::ImageFormat;
use std::io::Cursor;

const RGBA_BYTES_PER_PIXEL: usize = 4;

fn rgba_len(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 {
        return Err(format!(
            "RGBA image dimensions must be non-zero (got {width}x{height})"
        ));
    }

    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(RGBA_BYTES_PER_PIXEL))
        .ok_or_else(|| format!("RGBA image dimensions overflow ({width}x{height})"))
}

fn validate_rgba_len(label: &str, width: u32, height: u32, rgba: &[u8]) -> Result<usize, String> {
    let expected = rgba_len(width, height)?;
    if rgba.len() != expected {
        return Err(format!(
            "{label} RGBA buffer length mismatch for {width}x{height}: expected {expected} bytes, got {}",
            rgba.len()
        ));
    }
    Ok(expected)
}

/// Flip tightly packed RGBA rows from bottom-up to top-down order.
pub fn flip_rgba_rows(width: u32, height: u32, bottom_up_rgba: &[u8]) -> Result<Vec<u8>, String> {
    let len = validate_rgba_len("bottom-up", width, height, bottom_up_rgba)?;
    let stride = (width as usize)
        .checked_mul(RGBA_BYTES_PER_PIXEL)
        .ok_or_else(|| format!("RGBA row stride overflows for width {width}"))?;
    let mut top_down_rgba = vec![0; len];

    for output_row in 0..height as usize {
        let input_start = (height as usize - 1 - output_row) * stride;
        let output_start = output_row * stride;
        top_down_rgba[output_start..output_start + stride]
            .copy_from_slice(&bottom_up_rgba[input_start..input_start + stride]);
    }

    Ok(top_down_rgba)
}

/// Encode tightly packed, top-down RGBA pixels as PNG bytes.
pub fn encode_rgba_png(width: u32, height: u32, top_down_rgba: &[u8]) -> Result<Vec<u8>, String> {
    validate_rgba_len("top-down", width, height, top_down_rgba)?;
    let image = image::RgbaImage::from_raw(width, height, top_down_rgba.to_vec())
        .ok_or_else(|| format!("could not construct RGBA image for {width}x{height}"))?;
    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|error| format!("could not encode {width}x{height} RGBA image as PNG: {error}"))?;
    Ok(png)
}

/// Flip bottom-up RGBA pixels and encode them as PNG bytes.
pub fn encode_bottom_up_rgba_png(
    width: u32,
    height: u32,
    bottom_up_rgba: &[u8],
) -> Result<Vec<u8>, String> {
    let top_down_rgba = flip_rgba_rows(width, height, bottom_up_rgba)?;
    encode_rgba_png(width, height, &top_down_rgba)
}

/// Encode equal-size left and right eye buffers as one side-by-side PNG.
///
/// Both inputs are tightly packed RGBA in OpenGL's bottom-up row order. The
/// output is top-down and places the left eye before the right eye on every
/// row.
pub fn encode_stereo_side_by_side_png(
    eye_width: u32,
    eye_height: u32,
    left_bottom_up_rgba: &[u8],
    right_bottom_up_rgba: &[u8],
) -> Result<Vec<u8>, String> {
    let output_width = eye_width
        .checked_mul(2)
        .ok_or_else(|| format!("stereo output width overflows for eye width {eye_width}"))?;
    validate_rgba_len("left eye", eye_width, eye_height, left_bottom_up_rgba)?;
    validate_rgba_len("right eye", eye_width, eye_height, right_bottom_up_rgba)?;
    let output_len = rgba_len(output_width, eye_height)?;
    let eye_stride = (eye_width as usize)
        .checked_mul(RGBA_BYTES_PER_PIXEL)
        .ok_or_else(|| format!("RGBA row stride overflows for eye width {eye_width}"))?;
    let output_stride = eye_stride
        .checked_mul(2)
        .ok_or_else(|| format!("stereo RGBA row stride overflows for eye width {eye_width}"))?;
    let mut top_down_rgba = vec![0; output_len];

    for output_row in 0..eye_height as usize {
        let input_start = (eye_height as usize - 1 - output_row) * eye_stride;
        let output_start = output_row * output_stride;
        top_down_rgba[output_start..output_start + eye_stride]
            .copy_from_slice(&left_bottom_up_rgba[input_start..input_start + eye_stride]);
        top_down_rgba[output_start + eye_stride..output_start + output_stride]
            .copy_from_slice(&right_bottom_up_rgba[input_start..input_start + eye_stride]);
    }

    encode_rgba_png(output_width, eye_height, &top_down_rgba)
}

/// Read tightly packed, bottom-up RGBA pixels from the currently bound read
/// framebuffer.
///
/// The caller must ensure the GL context is current. `PACK_ALIGNMENT` is
/// temporarily set to one so readback does not depend on ambient GL state, and
/// is restored before returning.
pub unsafe fn read_bound_framebuffer_rgba(
    gl: &glow::Context,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let len = rgba_len(width, height)?;
    let gl_width = i32::try_from(width)
        .map_err(|_| format!("framebuffer width {width} exceeds OpenGL's i32 range"))?;
    let gl_height = i32::try_from(height)
        .map_err(|_| format!("framebuffer height {height} exceeds OpenGL's i32 range"))?;
    let mut pixels = vec![0; len];
    // GL errors are sticky. Clear any older renderer error so a failure below
    // can be attributed to this readback instead of silently returning a
    // zeroed/corrupt PNG with HTTP 200.
    while gl.get_error() != glow::NO_ERROR {}
    let previous_pack_alignment = gl.get_parameter_i32(glow::PACK_ALIGNMENT);

    if previous_pack_alignment != 1 {
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
    }
    gl.read_pixels(
        0,
        0,
        gl_width,
        gl_height,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        glow::PixelPackData::Slice(Some(&mut pixels)),
    );
    if previous_pack_alignment != 1 {
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, previous_pack_alignment);
    }
    let error = gl.get_error();
    if error != glow::NO_ERROR {
        return Err(format!("OpenGL framebuffer readback failed (0x{error:04x})"));
    }

    Ok(pixels)
}

/// Read the currently bound framebuffer and encode it as a top-down PNG.
pub unsafe fn encode_bound_framebuffer_png(
    gl: &glow::Context,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let bottom_up_rgba = read_bound_framebuffer_rgba(gl, width, height)?;
    encode_bottom_up_rgba_png(width, height, &bottom_up_rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    fn pixels(colors: &[[u8; 4]]) -> Vec<u8> {
        colors.iter().flatten().copied().collect()
    }

    fn decode_rgba(png: &[u8]) -> image::RgbaImage {
        image::load_from_memory_with_format(png, ImageFormat::Png)
            .unwrap()
            .into_rgba8()
    }

    #[test]
    fn flips_bottom_up_rows_without_reordering_pixels_within_a_row() {
        let bottom_up = pixels(&[BLUE, WHITE, RED, GREEN]);
        let flipped = flip_rgba_rows(2, 2, &bottom_up).unwrap();
        assert_eq!(flipped, pixels(&[RED, GREEN, BLUE, WHITE]));
    }

    #[test]
    fn one_row_flip_is_unchanged() {
        let row = pixels(&[RED, GREEN]);
        assert_eq!(flip_rgba_rows(2, 1, &row).unwrap(), row);
    }

    #[test]
    fn rejects_zero_dimensions_and_invalid_buffer_lengths() {
        assert!(flip_rgba_rows(0, 1, &[])
            .unwrap_err()
            .contains("dimensions must be non-zero"));
        let error = flip_rgba_rows(2, 2, &[0; 15]).unwrap_err();
        assert!(error.contains("expected 16 bytes, got 15"), "{error}");

        let error = encode_rgba_png(1, 1, &[0; 3]).unwrap_err();
        assert!(error.contains("expected 4 bytes, got 3"), "{error}");
    }

    #[test]
    fn rejects_dimension_arithmetic_overflow() {
        let error = flip_rgba_rows(u32::MAX, u32::MAX, &[]).unwrap_err();
        assert!(error.contains("dimensions overflow"), "{error}");

        let error = encode_stereo_side_by_side_png(u32::MAX, 1, &[], &[]).unwrap_err();
        assert!(error.contains("stereo output width overflows"), "{error}");
    }

    #[test]
    fn encodes_png_dimensions_and_top_down_pixels() {
        let top_down = pixels(&[RED, GREEN, BLUE, WHITE]);
        let png = encode_rgba_png(2, 2, &top_down).unwrap();
        let decoded = decode_rgba(&png);

        assert_eq!(decoded.dimensions(), (2, 2));
        assert_eq!(decoded.into_raw(), top_down);
    }

    #[test]
    fn bottom_up_png_encoder_flips_rows() {
        let bottom_up = pixels(&[BLUE, WHITE, RED, GREEN]);
        let png = encode_bottom_up_rgba_png(2, 2, &bottom_up).unwrap();
        assert_eq!(
            decode_rgba(&png).into_raw(),
            pixels(&[RED, GREEN, BLUE, WHITE])
        );
    }

    #[test]
    fn stereo_png_places_left_then_right_and_flips_each_eye() {
        // Each eye is 1x2, expressed bottom row first.
        let left = pixels(&[BLUE, RED]);
        let right = pixels(&[WHITE, GREEN]);
        let png = encode_stereo_side_by_side_png(1, 2, &left, &right).unwrap();
        let decoded = decode_rgba(&png);

        assert_eq!(decoded.dimensions(), (2, 2));
        assert_eq!(decoded.into_raw(), pixels(&[RED, GREEN, BLUE, WHITE]));
    }

    #[test]
    fn stereo_png_rejects_invalid_eye_lengths_independently() {
        let error = encode_stereo_side_by_side_png(1, 1, &[0; 3], &[0; 4]).unwrap_err();
        assert!(error.contains("left eye"), "{error}");
        assert!(error.contains("expected 4 bytes, got 3"), "{error}");

        let error = encode_stereo_side_by_side_png(1, 1, &[0; 4], &[0; 3]).unwrap_err();
        assert!(error.contains("right eye"), "{error}");
        assert!(error.contains("expected 4 bytes, got 3"), "{error}");
    }
}
