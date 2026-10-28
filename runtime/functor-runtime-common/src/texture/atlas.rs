//! `Texture.atlas`: packs images into one texture with a deterministic
//! skyline packer, padding each with copies of its edge texels so filtering
//! never reaches a neighbour.

use super::PixelData;

/// Where one image landed: its first texel (row 0, column 0) in the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: u32,
    pub y: u32,
}

/// Pack `images` into the smallest power-of-two square (from the one their
/// padded area needs, up to `max_size`) that holds them all. Taller images
/// go first (a stable sort, so equal heights keep input order); placements
/// come back in input order.
pub fn atlas(
    images: &[&PixelData],
    padding: u32,
    max_size: u32,
) -> Result<(PixelData, Vec<Placement>), String> {
    let Some(first) = images.first() else {
        return Err("images is empty".to_string());
    };
    check_max_size(max_size)?;
    if let Some(i) = images
        .iter()
        .position(|image| image.color_space() != first.color_space())
    {
        return Err(format!(
            "images[{i}] is {:?}, but images[0] is {:?}; an atlas has one colour space",
            images[i].color_space(),
            first.color_space()
        ));
    }
    // Each padded image must fit alone; checked in u64 so a huge `padding`
    // is an error, not an overflow.
    let padded: Vec<(u32, u32)> = images
        .iter()
        .enumerate()
        .map(|(i, image)| {
            let pad = |side: u32| side as u64 + 2 * padding as u64;
            let (w, h) = (pad(image.width()), pad(image.height()));
            if w.max(h) > max_size as u64 {
                return Err(format!(
                    "images[{i}] is {w}×{h} with padding, over maxSize {max_size}"
                ));
            }
            Ok((w as u32, h as u32))
        })
        .collect::<Result<_, _>>()?;
    let (side, slots) = pack(&padded, max_size).map_err(|area| {
        format!("images need more than {max_size}×{max_size} ({area} texels with padding)")
    })?;
    let mut rgba8 = vec![0u8; side as usize * side as usize * 4];
    let placements = images
        .iter()
        .zip(&slots)
        .map(|(image, &(x, y))| {
            blit(&mut rgba8, side, image, x, y, padding);
            Placement {
                x: x + padding,
                y: y + padding,
            }
        })
        .collect();
    Ok((
        PixelData::from_rgba8(side, side, first.color_space(), rgba8),
        placements,
    ))
}

/// `maxSize` for an atlas: a texture side (1 to 4096) and a power of two.
pub fn check_max_size(max_size: u32) -> Result<(), String> {
    super::check_side("maxSize", max_size as f64)?;
    if !max_size.is_power_of_two() {
        return Err(format!("maxSize must be a power of two, got {max_size}"));
    }
    Ok(())
}

/// Pack rectangles (each at most `max_size` a side) into the smallest
/// power-of-two square that holds them all: its side and each rectangle's
/// corner, in input order. `Err` is their total area when `max_size` is too
/// small.
pub fn pack(sizes: &[(u32, u32)], max_size: u32) -> Result<(u32, Vec<(u32, u32)>), u64> {
    let area: u64 = sizes.iter().map(|&(w, h)| w as u64 * h as u64).sum();
    if area > max_size as u64 * max_size as u64 {
        return Err(area);
    }
    let widest = sizes.iter().map(|&(w, h)| w.max(h)).max().unwrap_or(1);
    let mut side = widest
        .max((area as f64).sqrt().ceil() as u32)
        .next_power_of_two();
    loop {
        if side > max_size {
            return Err(area);
        }
        if let Some(slots) = skyline(sizes, side) {
            return Ok((side, slots));
        }
        side *= 2;
    }
}

/// Bottom-left skyline packing of `sizes` into a `side`² square, tallest
/// first; `None` if they don't fit. Slots are returned in input order.
fn skyline(sizes: &[(u32, u32)], side: u32) -> Option<Vec<(u32, u32)>> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(sizes[i].1));
    // The skyline: (x, y) segments left to right, each reaching the next's x.
    let mut segments: Vec<(u32, u32)> = vec![(0, 0)];
    let mut slots = vec![(0, 0); sizes.len()];
    for i in order {
        let (w, h) = sizes[i];
        // The lowest (then leftmost) spot: a run of segments from `start`
        // spanning `w`, resting on their highest point.
        let mut best: Option<(u32, usize)> = None;
        for start in 0..segments.len() {
            let x = segments[start].0;
            if x + w > side {
                break;
            }
            let end = x + w;
            let y = segments[start..]
                .iter()
                .take_while(|(sx, _)| *sx < end)
                .map(|&(_, sy)| sy)
                .max()
                .unwrap();
            if y + h <= side && best.map_or(true, |(by, _)| y < by) {
                best = Some((y, start));
            }
        }
        let (y, start) = best?;
        let x = segments[start].0;
        slots[i] = (x, y);
        // Replace the covered segments with the new top, keeping the
        // remainder of the last covered one.
        let end = x + w;
        let mut last = start;
        while last + 1 < segments.len() && segments[last + 1].0 < end {
            last += 1;
        }
        let tail_y = segments[last].1;
        let next_x = segments.get(last + 1).map_or(side, |s| s.0);
        let mut replacement = vec![(x, y + h)];
        if end < next_x {
            replacement.push((end, tail_y));
        }
        segments.splice(start..=last, replacement);
        // Merge level neighbours so the skyline stays short.
        segments.dedup_by(|later, earlier| later.1 == earlier.1);
    }
    Some(slots)
}

/// Copy `image` into the atlas with its padded corner at (`x`, `y`): the
/// image at `padding` in, the border repeating its nearest edge texel.
fn blit(atlas: &mut [u8], side: u32, image: &PixelData, x: u32, y: u32, padding: u32) {
    let (w, h) = (image.width() as i64, image.height() as i64);
    let src = image.rgba8();
    let pad = padding as i64;
    for row in -pad..h + pad {
        let sy = row.clamp(0, h - 1);
        let dy = (y as i64 + pad + row) as usize;
        let line = &src[(sy * w * 4) as usize..((sy + 1) * w * 4) as usize];
        let dst = (dy * side as usize + x as usize) * 4;
        // Left border, the row, right border.
        for k in 0..pad as usize {
            atlas[dst + k * 4..dst + k * 4 + 4].copy_from_slice(&line[..4]);
            let r = dst + (pad as usize + w as usize + k) * 4;
            atlas[r..r + 4].copy_from_slice(&line[line.len() - 4..]);
        }
        let start = dst + pad as usize * 4;
        atlas[start..start + line.len()].copy_from_slice(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture::{ColorSpace, PixelEncoding};

    /// A `w`×`h` image whose texels encode (image, x, y), so any misplaced
    /// copy shows.
    fn image(tag: u8, w: u32, h: u32) -> PixelData {
        let bytes: Vec<u8> = (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| [tag, x as u8, y as u8, 255]))
            .collect();
        PixelData::new(w, h, PixelEncoding::Rgba8, ColorSpace::Linear, &bytes).unwrap()
    }

    fn texel(atlas: &PixelData, x: u32, y: u32) -> &[u8] {
        let i = ((y * atlas.width() + x) * 4) as usize;
        &atlas.rgba8()[i..i + 4]
    }

    /// Deterministic sizes in 1..=32 (a tiny LCG).
    fn sizes(n: usize) -> Vec<(u32, u32)> {
        let mut state = 7u32;
        let mut next = || {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16) % 32 + 1
        };
        (0..n).map(|_| (next(), next())).collect()
    }

    #[test]
    fn packs_without_overlap_and_copies_every_texel() {
        let images: Vec<PixelData> = sizes(200)
            .iter()
            .enumerate()
            .map(|(i, &(w, h))| image(i as u8, w, h))
            .collect();
        let refs: Vec<&PixelData> = images.iter().collect();
        let padding = 2;
        let (atlas, placed) = atlas(&refs, padding, 4096).unwrap();
        let side = atlas.width();
        // Padded rectangles are in bounds and disjoint.
        let padded: Vec<[u32; 4]> = images
            .iter()
            .zip(&placed)
            .map(|(im, p)| {
                [
                    p.x - padding,
                    p.y - padding,
                    p.x + im.width() + padding,
                    p.y + im.height() + padding,
                ]
            })
            .collect();
        for (i, a) in padded.iter().enumerate() {
            assert!(a[2] <= side && a[3] <= side, "image {i} out of bounds");
            for b in &padded[i + 1..] {
                let apart = a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1];
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        // Every texel lands at its placement; the border repeats the edge.
        for (i, (im, p)) in images.iter().zip(&placed).enumerate() {
            let (w, h) = (im.width() as i64, im.height() as i64);
            let pad = padding as i64;
            for y in -pad..h + pad {
                for x in -pad..w + pad {
                    let (sx, sy) = (x.clamp(0, w - 1), y.clamp(0, h - 1));
                    let at = texel(&atlas, (p.x as i64 + x) as u32, (p.y as i64 + y) as u32);
                    assert_eq!(
                        at,
                        [i as u8, sx as u8, sy as u8, 255],
                        "image {i} at {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn packing_is_deterministic() {
        let images: Vec<PixelData> = sizes(50).iter().map(|&(w, h)| image(0, w, h)).collect();
        let refs: Vec<&PixelData> = images.iter().collect();
        let (a, pa) = atlas(&refs, 1, 1024).unwrap();
        let (b, pb) = atlas(&refs, 1, 1024).unwrap();
        assert_eq!((a.rgba8(), pa), (b.rgba8(), pb));
    }

    /// The design's medsci1 case: ~9.6k lightmaps, ~767k texels (~1.15M with
    /// 1 texel of padding), fits 2048².
    #[test]
    fn a_level_of_lightmaps_fits_2048() {
        let images: Vec<PixelData> = sizes(9_600)
            .iter()
            .map(|&(w, h)| image(0, w / 3 + 2, h / 3 + 6))
            .collect();
        let texels: u32 = images.iter().map(|im| im.width() * im.height()).sum();
        assert!((600_000..900_000).contains(&texels), "{texels} texels");
        let refs: Vec<&PixelData> = images.iter().collect();
        let start = std::time::Instant::now();
        let (atlas, _) = atlas(&refs, 1, 2048).unwrap();
        eprintln!("packed {} images in {:?}", images.len(), start.elapsed());
        assert_eq!(atlas.width(), 2048);
    }

    #[test]
    fn errors_name_the_problem() {
        let small = image(0, 4, 4);
        let srgb = PixelData::new(1, 1, PixelEncoding::Rgb8, ColorSpace::Srgb, &[0; 3]).unwrap();
        let err = |images: &[&PixelData], max| atlas(images, 1, max).err().unwrap();
        assert_eq!(err(&[], 64), "images is empty");
        assert_eq!(
            err(&[&small, &srgb], 64),
            "images[1] is Srgb, but images[0] is Linear; an atlas has one colour space"
        );
        assert_eq!(
            err(&[&small; 5], 8),
            "images need more than 8×8 (180 texels with padding)"
        );
        assert!(err(&[&small], 5000).starts_with("maxSize must be 1 to 4096"));
        assert_eq!(err(&[&small], 48), "maxSize must be a power of two, got 48");
        assert_eq!(
            atlas(&[&small], u32::MAX, 4096).err().unwrap(),
            "images[0] is 8589934594×8589934594 with padding, over maxSize 4096"
        );
    }
}
