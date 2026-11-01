//! PCX images (ZSoft Paintbrush, run-length encoded) decoded to RGBA8, top
//! row first: 8 bits per channel, as one plane of indices into the 256-colour
//! palette that ends the file, or three planes (red, green, blue).

use super::{ImageError, PixelFormat, TextureData};

const HEADER: usize = 128;
// A 12 byte, then 256 RGB triples, at the end of an indexed image.
const PALETTE: usize = 769;

/// Whether `bytes` start as a run-length encoded PCX: byte 0 is 10, byte 2 is 1.
pub fn is_pcx(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER && bytes[0] == 10 && bytes[2] == 1
}

fn fail<T>(offset: usize, message: String) -> Result<T, ImageError> {
    Err(ImageError { offset, message })
}

pub fn decode_pcx(bytes: &[u8]) -> Result<TextureData, ImageError> {
    if !is_pcx(bytes) {
        return fail(0, "not a run-length encoded PCX".to_string());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let (bits, planes, stride) = (bytes[3], bytes[65] as usize, u16_at(66));
    if bits != 8 {
        return fail(3, format!("{bits} bits per channel: only 8 decode"));
    }
    if planes != 1 && planes != 3 {
        return fail(
            65,
            format!("{planes} planes: only 1 (palette) or 3 (RGB) decode"),
        );
    }
    let (x0, y0, x1, y1) = (u16_at(4), u16_at(6), u16_at(8), u16_at(10));
    if x1 < x0 || y1 < y0 {
        return fail(4, format!("window {x0},{y0} to {x1},{y1} is empty"));
    }
    let (width, height) = (x1 - x0 + 1, y1 - y0 + 1);
    // Bounded sides keep the sizes below within a 32-bit usize (wasm).
    for (name, side) in [("width", width), ("height", height)] {
        super::check_side(name, side as f64).or_else(|e| fail(4, e))?;
    }
    if stride < width {
        return fail(
            66,
            format!("rows of {stride} bytes are narrower than the width {width}"),
        );
    }
    let end = if planes == 1 {
        let at = bytes.len().saturating_sub(PALETTE);
        if at < HEADER || bytes[at] != 12 {
            return fail(at, "no 256-colour palette at the end".to_string());
        }
        at
    } else {
        bytes.len()
    };
    // Each row is `planes` runs of `stride` bytes; the first `width` of each
    // are kept, a row at a time, so padding is never held.
    let mut rle = Rle {
        src: &bytes[HEADER..end],
        at: 0,
        run: 0,
        value: 0,
    };
    let mut line = vec![0u8; width * planes];
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for plane in line.chunks_exact_mut(width) {
            for x in 0..stride {
                let Some(b) = rle.next() else {
                    return fail(
                        HEADER + rle.at,
                        format!("pixel data ends in row {y} of {height}"),
                    );
                };
                if x < width {
                    plane[x] = b;
                }
            }
        }
        for x in 0..width {
            let [r, g, b] = if planes == 1 {
                let i = end + 1 + 3 * line[x] as usize;
                [bytes[i], bytes[i + 1], bytes[i + 2]]
            } else {
                [line[x], line[width + x], line[2 * width + x]]
            };
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    Ok(TextureData {
        bytes: rgba,
        width: width as u32,
        height: height as u32,
        format: PixelFormat::RGBA,
    })
}

/// A PCX run-length stream, byte by byte: a byte ≥ 0xC0 repeats the next one
/// (its low 6 bits) times, any other byte is itself. Runs may cross rows.
struct Rle<'a> {
    src: &'a [u8],
    at: usize,
    run: u8,
    value: u8,
}

impl Rle<'_> {
    /// The next byte, or `None` (with `at` where it ran out) at the end.
    fn next(&mut self) -> Option<u8> {
        while self.run == 0 {
            let b = *self.src.get(self.at)?;
            if b < 0xC0 {
                self.at += 1;
                return Some(b);
            }
            self.value = *self.src.get(self.at + 1)?;
            self.run = b & 0x3F;
            self.at += 2;
        }
        self.run -= 1;
        Some(self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PCX with `planes` planes of `stride`-byte rows over the window
    /// (0, 0)–(`width` − 1, `height` − 1), `rle` after the header, then
    /// `tail` (the palette, for one plane).
    fn pcx(width: u16, height: u16, planes: u8, stride: u16, rle: &[u8], tail: &[u8]) -> Vec<u8> {
        let mut file = vec![0u8; HEADER];
        file[..4].copy_from_slice(&[10, 5, 1, 8]);
        file[8..10].copy_from_slice(&(width - 1).to_le_bytes());
        file[10..12].copy_from_slice(&(height - 1).to_le_bytes());
        file[65] = planes;
        file[66..68].copy_from_slice(&stride.to_le_bytes());
        file.extend_from_slice(rle);
        file.extend_from_slice(tail);
        file
    }

    // Index i is (i, 255 − i, 7).
    fn palette() -> Vec<u8> {
        std::iter::once(12)
            .chain((0..=255u8).flat_map(|i| [i, 255 - i, 7]))
            .collect()
    }

    fn rgba(texels: &[[u8; 3]]) -> Vec<u8> {
        texels
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect()
    }

    /// Indices through the palette; an odd width keeps the first `width` of
    /// each padded row; a run of index ≥ 0xC0 is how such an index is stored.
    #[test]
    fn indexed_rows_drop_their_padding() {
        // Row 0: a run of three 1s, then padding. Row 1: 0, 2, 193, padding.
        let file = pcx(3, 2, 1, 4, &[0xC3, 1, 0, 0, 2, 0xC1, 193, 0], &palette());
        let image = decode_pcx(&file).unwrap();
        assert_eq!((image.width, image.height), (3, 2));
        assert_eq!(
            image.bytes,
            rgba(&[
                [1, 254, 7],
                [1, 254, 7],
                [1, 254, 7],
                [0, 255, 7],
                [2, 253, 7],
                [193, 62, 7]
            ])
        );
    }

    /// A run may cross into the next row and past the last; a zero-length
    /// run (0xC0) still consumes its value byte.
    #[test]
    fn runs_cross_rows_and_stop_at_the_end() {
        // 2×2, no padding: a run of three 5s (rows 0 and 1), a zero run of
        // 9, then a run of four 6s of which one is used.
        let file = pcx(2, 2, 1, 2, &[0xC3, 5, 0xC0, 9, 0xC4, 6], &palette());
        assert_eq!(
            decode_pcx(&file).unwrap().bytes,
            rgba(&[[5, 250, 7], [5, 250, 7], [5, 250, 7], [6, 249, 7]])
        );
    }

    /// Three planes: each row is its red, green then blue bytes; no palette.
    #[test]
    fn rgb_planes_interleave_per_row() {
        // 1×2, rows padded to 2 bytes per plane.
        let file = pcx(
            1,
            2,
            3,
            2,
            &[10, 0, 20, 0, 30, 0, 0xC2, 40, 0xC2, 50, 0xC2, 60],
            &[],
        );
        let image = decode_pcx(&file).unwrap();
        assert_eq!((image.width, image.height), (1, 2));
        assert_eq!(image.bytes, rgba(&[[10, 20, 30], [40, 50, 60]]));
    }

    #[test]
    fn bad_files_say_what_and_where() {
        let err = |file: &[u8]| decode_pcx(file).err().unwrap().to_string();
        assert_eq!(err(b"\x89PNG"), "at byte 0: not a run-length encoded PCX");
        let good = pcx(3, 2, 1, 4, &[0xC3, 1, 0, 0, 2, 0xC1, 193, 0], &palette());
        let with = |at: usize, value: u8| {
            let mut file = good.clone();
            file[at] = value;
            err(&file)
        };
        assert_eq!(with(3, 4), "at byte 3: 4 bits per channel: only 8 decode");
        assert_eq!(
            with(65, 4),
            "at byte 65: 4 planes: only 1 (palette) or 3 (RGB) decode"
        );
        assert_eq!(
            with(66, 2),
            "at byte 66: rows of 2 bytes are narrower than the width 3"
        );
        assert_eq!(with(4, 5), "at byte 4: window 5,0 to 2,1 is empty");
        assert_eq!(
            with(good.len() - PALETTE, 0),
            "at byte 136: no 256-colour palette at the end"
        );
        // Cut short: a row and a half of pixels, and a run missing its value.
        for rle in [&[0xC3, 1, 0, 0, 2][..], &[0xC3, 1, 0, 0, 2, 0xC1]] {
            let file = pcx(3, 2, 1, 4, rle, &palette());
            assert_eq!(err(&file), "at byte 133: pixel data ends in row 1 of 2");
        }
        // Sides past 4096 (65536 here, where 3 planes of 32768-byte rows
        // would overflow a 32-bit size).
        let mut huge = pcx(1, 1, 3, 32768, &[], &[]);
        huge[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            err(&huge),
            "at byte 4: height must be 1 to 4096 (the largest texture every target supports), got 65536"
        );
        assert_eq!(
            err(&pcx(3, 2, 1, 4, &[], &[])[..HEADER]),
            "at byte 0: no 256-colour palette at the end"
        );
    }
}
