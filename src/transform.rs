use image::DynamicImage;
use log::debug;
use serde::{Deserialize, Serialize};

pub use crate::cli::MaskEffect;

/// Bounding box of detected visual differences.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffBBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Structured summary of visual diff comparison against a reference image.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub changed: bool,
    pub changed_pixels: u64,
    pub total_pixels: u64,
    pub changed_percent: f64,
    pub threshold: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<DiffBBox>,
}

/// Borrows `DynamicImage` as `&RgbaImage` with zero allocation if already RGBA8,
/// or converts to owned `RgbaImage` if in a different color space.
#[inline]
pub fn borrow_or_convert_rgba8<'a>(
    image: &'a DynamicImage,
) -> std::borrow::Cow<'a, image::RgbaImage> {
    if let Some(rgba) = image.as_rgba8() {
        std::borrow::Cow::Borrowed(rgba)
    } else {
        std::borrow::Cow::Owned(image.to_rgba8())
    }
}

/// Applies scaling factor or max dimension constraint to captured image.
pub fn apply_image_transforms(
    image: DynamicImage,
    scale: Option<f64>,
    max_dimension: Option<u32>,
) -> DynamicImage {
    if let Some(factor) = scale {
        let new_w = ((image.width() as f64) * factor).round().max(1.0) as u32;
        let new_h = ((image.height() as f64) * factor).round().max(1.0) as u32;
        debug!(
            "Resizing image from {}x{} to {}x{} (scale: {:.2})",
            image.width(),
            image.height(),
            new_w,
            new_h,
            factor
        );
        return image.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3);
    }
    if let Some(max_dim) = max_dimension {
        if image.width() > max_dim || image.height() > max_dim {
            debug!(
                "Constraining image to max dimension {}px from {}x{}",
                max_dim,
                image.width(),
                image.height()
            );
            return image.resize(max_dim, max_dim, image::imageops::FilterType::Lanczos3);
        }
    }
    image
}

/// Apply a single redaction mask to an RGBA image in-place.
pub fn apply_redaction(
    img: &mut image::RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    effect: MaskEffect,
) {
    let img_w = img.width();
    let img_h = img.height();

    // Defensive clipping to image boundaries
    let x0 = x.min(img_w);
    let y0 = y.min(img_h);
    let x1 = (x.saturating_add(width)).min(img_w);
    let y1 = (y.saturating_add(height)).min(img_h);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    match effect {
        MaskEffect::Transparent => {
            let stride = (img_w as usize) * 4;
            let row_len = ((x1 - x0) as usize) * 4;
            let raw = img.as_mut();
            for py in y0..y1 {
                let off = (py as usize) * stride + (x0 as usize) * 4;
                raw[off..off + row_len].fill(0);
            }
        }
        MaskEffect::Blackout => {
            let stride = (img_w as usize) * 4;
            let row_len = ((x1 - x0) as usize) * 4;
            let raw = img.as_mut();
            for py in y0..y1 {
                let off = (py as usize) * stride + (x0 as usize) * 4;
                for px in raw[off..off + row_len].chunks_exact_mut(4) {
                    px.copy_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
        MaskEffect::Pixelate => {
            let block_size: u32 = 12;
            let mut by = y0;
            while by < y1 {
                let mut bx = x0;
                let b_h = (by + block_size).min(y1) - by;
                while bx < x1 {
                    let b_w = (bx + block_size).min(x1) - bx;
                    let count = (b_w * b_h) as u64;

                    // Compute average RGBA across block
                    let mut sum = [0u64; 4];
                    for py in by..(by + b_h) {
                        for px in bx..(bx + b_w) {
                            let p = img.get_pixel(px, py);
                            for c in 0..4 {
                                sum[c] += p[c] as u64;
                            }
                        }
                    }

                    let avg_pixel = image::Rgba([
                        (sum[0] / count) as u8,
                        (sum[1] / count) as u8,
                        (sum[2] / count) as u8,
                        (sum[3] / count) as u8,
                    ]);

                    for py in by..(by + b_h) {
                        for px in bx..(bx + b_w) {
                            img.put_pixel(px, py, avg_pixel);
                        }
                    }

                    bx += block_size;
                }
                by += block_size;
            }
        }
        MaskEffect::Blur => {
            let sub_w = x1 - x0;
            let sub_h = y1 - y0;
            if sub_w == 0 || sub_h == 0 {
                return;
            }

            let sub_img = image::imageops::crop_imm(img, x0, y0, sub_w, sub_h).to_image();
            let dyn_sub = DynamicImage::ImageRgba8(sub_img);

            // 8x downsample + bilinear upsample provides fast, zero-dependency visual blur
            let down_w = (sub_w / 8).max(1);
            let down_h = (sub_h / 8).max(1);
            let downscaled =
                dyn_sub.resize_exact(down_w, down_h, image::imageops::FilterType::Triangle);
            let blurred = downscaled
                .resize_exact(sub_w, sub_h, image::imageops::FilterType::Triangle)
                .to_rgba8();

            let b_raw = blurred.as_raw();
            let img_raw = img.as_mut();
            let b_stride = (sub_w as usize) * 4;
            let img_stride = (img_w as usize) * 4;
            for py in 0..sub_h as usize {
                let b_start = py * b_stride;
                let b_end = b_start + b_stride;
                let img_start = (y0 as usize + py) * img_stride + (x0 as usize) * 4;
                let img_end = img_start + b_stride;
                img_raw[img_start..img_end].copy_from_slice(&b_raw[b_start..b_end]);
            }
        }
    }
}

/// Apply a list of redaction masks to a DynamicImage.
pub fn apply_redactions(
    image: DynamicImage,
    masks: &[(u32, u32, u32, u32, MaskEffect)],
) -> DynamicImage {
    if masks.is_empty() {
        return image;
    }

    let mut rgba = image.into_rgba8();
    for &(x, y, w, h, effect) in masks {
        debug!(
            "Applying redaction: x={}, y={}, w={}, h={}, effect={:?}",
            x, y, w, h, effect
        );
        apply_redaction(&mut rgba, x, y, w, h, effect);
    }
    DynamicImage::ImageRgba8(rgba)
}

// 3x5 bitmap font for '0'..='9', ',', and '-' (used for visual grounding grid coordinates)
#[rustfmt::skip]
const FONT_3X5_DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], [0b010, 0b110, 0b010, 0b010, 0b111], // 0, 1
    [0b111, 0b001, 0b111, 0b100, 0b111], [0b111, 0b001, 0b111, 0b001, 0b111], // 2, 3
    [0b101, 0b101, 0b111, 0b001, 0b001], [0b111, 0b100, 0b111, 0b001, 0b111], // 4, 5
    [0b111, 0b100, 0b111, 0b101, 0b111], [0b111, 0b001, 0b010, 0b010, 0b010], // 6, 7
    [0b111, 0b101, 0b111, 0b101, 0b111], [0b111, 0b101, 0b111, 0b001, 0b111], // 8, 9
];

#[rustfmt::skip]
const FONT_3X5_LETTERS: [[u8; 5]; 26] = [
    [0b010, 0b101, 0b111, 0b101, 0b101], [0b110, 0b101, 0b110, 0b101, 0b110], // A, B
    [0b011, 0b100, 0b100, 0b100, 0b011], [0b110, 0b101, 0b101, 0b101, 0b110], // C, D
    [0b111, 0b100, 0b110, 0b100, 0b111], [0b111, 0b100, 0b110, 0b100, 0b100], // E, F
    [0b011, 0b100, 0b101, 0b101, 0b011], [0b101, 0b101, 0b111, 0b101, 0b101], // G, H
    [0b111, 0b010, 0b010, 0b010, 0b111], [0b001, 0b001, 0b001, 0b101, 0b010], // I, J
    [0b101, 0b110, 0b100, 0b110, 0b101], [0b100, 0b100, 0b100, 0b100, 0b111], // K, L
    [0b101, 0b111, 0b101, 0b101, 0b101], [0b110, 0b101, 0b101, 0b101, 0b101], // M, N
    [0b010, 0b101, 0b101, 0b101, 0b010], [0b110, 0b101, 0b110, 0b100, 0b100], // O, P
    [0b010, 0b101, 0b101, 0b110, 0b011], [0b110, 0b101, 0b110, 0b101, 0b101], // Q, R
    [0b011, 0b100, 0b010, 0b001, 0b110], [0b111, 0b010, 0b010, 0b010, 0b010], // S, T
    [0b101, 0b101, 0b101, 0b101, 0b010], [0b101, 0b101, 0b101, 0b010, 0b010], // U, V
    [0b101, 0b101, 0b101, 0b111, 0b101], [0b101, 0b101, 0b010, 0b101, 0b101], // W, X
    [0b101, 0b101, 0b010, 0b010, 0b010], [0b111, 0b001, 0b010, 0b100, 0b111], // Y, Z
];
const FONT_3X5_COMMA: [u8; 5] = [0b000, 0b000, 0b000, 0b010, 0b100];
const FONT_3X5_MINUS: [u8; 5] = [0b000, 0b000, 0b111, 0b000, 0b000];
const FONT_3X5_SPACE: [u8; 5] = [0b000, 0b000, 0b000, 0b000, 0b000];
const FONT_3X5_COLON: [u8; 5] = [0b000, 0b010, 0b000, 0b010, 0b000];
const FONT_3X5_DOT: [u8; 5] = [0b000, 0b000, 0b000, 0b000, 0b010];
const FONT_3X5_SLASH: [u8; 5] = [0b001, 0b001, 0b010, 0b100, 0b100];
const FONT_3X5_UNDERSCORE: [u8; 5] = [0b000, 0b000, 0b000, 0b000, 0b111];
const FONT_3X5_PIPE: [u8; 5] = [0b010, 0b010, 0b010, 0b010, 0b010];

fn get_glyph_3x5(ch: char) -> Option<&'static [u8; 5]> {
    match ch {
        '0'..='9' => Some(&FONT_3X5_DIGITS[(ch as usize) - ('0' as usize)]),
        'A'..='Z' => Some(&FONT_3X5_LETTERS[(ch as usize) - ('A' as usize)]),
        'a'..='z' => Some(&FONT_3X5_LETTERS[(ch.to_ascii_uppercase() as usize) - ('A' as usize)]),
        ',' => Some(&FONT_3X5_COMMA),
        '-' => Some(&FONT_3X5_MINUS),
        ':' => Some(&FONT_3X5_COLON),
        '.' => Some(&FONT_3X5_DOT),
        '/' => Some(&FONT_3X5_SLASH),
        '_' => Some(&FONT_3X5_UNDERSCORE),
        '|' => Some(&FONT_3X5_PIPE),
        ' ' => Some(&FONT_3X5_SPACE),
        _ => None,
    }
}

/// Alpha-blends a source pixel over the destination image at (x, y).
#[inline]
pub(crate) fn blend_pixel(img: &mut image::RgbaImage, x: u32, y: u32, src: image::Rgba<u8>) {
    if x >= img.width() || y >= img.height() {
        return;
    }
    if src[3] == 255 {
        img.put_pixel(x, y, src);
        return;
    }
    if src[3] == 0 {
        return;
    }

    let dst = img.get_pixel(x, y);
    let sa = src[3] as u32;
    let da = dst[3] as u32;
    let inv_sa = 255 - sa;

    let out_a = sa + (da * inv_sa) / 255;
    if out_a == 0 {
        img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
        return;
    }

    let c = |i: usize| ((src[i] as u32 * sa + dst[i] as u32 * da * inv_sa / 255) / out_a) as u8;
    img.put_pixel(x, y, image::Rgba([c(0), c(1), c(2), out_a as u8]));
}

/// Draws coordinate label using 3x5 font with background padding for high contrast.
pub fn draw_label_3x5(
    img: &mut image::RgbaImage,
    start_x: u32,
    start_y: u32,
    text: &str,
    fg: image::Rgba<u8>,
    bg: image::Rgba<u8>,
) {
    let img_w = img.width();
    let img_h = img.height();

    let text_len = text.len() as u32;
    if text_len == 0 {
        return;
    }
    let total_w = text_len * 4;
    let total_h = 7;

    let bx0 = start_x.min(img_w);
    let by0 = start_y.min(img_h);
    let bx1 = (start_x + total_w + 1).min(img_w);
    let by1 = (start_y + total_h).min(img_h);

    for py in by0..by1 {
        for px in bx0..bx1 {
            blend_pixel(img, px, py, bg);
        }
    }

    let mut cur_x = start_x + 1;
    let cur_y = start_y + 1;

    for ch in text.chars() {
        if let Some(g) = get_glyph_3x5(ch) {
            for (row, &bits) in g.iter().enumerate() {
                let py = cur_y + row as u32;
                if py >= img_h {
                    continue;
                }
                for col in 0..3 {
                    if (bits & (1 << (2 - col))) != 0 {
                        let px = cur_x + col;
                        if px < img_w {
                            img.put_pixel(px, py, fg);
                        }
                    }
                }
            }
        }
        cur_x += 4;
    }
}

/// In-place variant of visual grounding grid overlay.
pub fn apply_grid_overlay_in_place(rgba: &mut image::RgbaImage, step: u32) {
    if step < 10 {
        return;
    }

    let w = rgba.width();
    let h = rgba.height();

    let grid_color = image::Rgba([0, 170, 255, 90]);
    let cross_color = image::Rgba([255, 70, 70, 220]);
    let fg_text = image::Rgba([255, 255, 255, 255]);
    let bg_text = image::Rgba([0, 0, 0, 190]);

    // Vertical grid lines
    let mut x = step;
    while x < w {
        for y in 0..h {
            blend_pixel(rgba, x, y, grid_color);
        }
        x += step;
    }

    // Horizontal grid lines
    let mut y = step;
    while y < h {
        for x_coord in 0..w {
            blend_pixel(rgba, x_coord, y, grid_color);
        }
        y += step;
    }

    // Intersection anchors and coordinate labels
    let mut iy = step;
    while iy < h {
        let mut ix = step;
        while ix < w {
            for dx in ix.saturating_sub(2)..=(ix + 2).min(w.saturating_sub(1)) {
                rgba.put_pixel(dx, iy, cross_color);
            }
            for dy in iy.saturating_sub(2)..=(iy + 2).min(h.saturating_sub(1)) {
                rgba.put_pixel(ix, dy, cross_color);
            }

            let label = format!("{ix},{iy}");
            draw_label_3x5(rgba, ix + 2, iy + 2, &label, fg_text, bg_text);

            ix += step;
        }
        iy += step;
    }
}

/// Overlay visual grounding grid lines and coordinate anchors for AI agent localization.
pub fn apply_grid_overlay(image: DynamicImage, step: u32) -> DynamicImage {
    if step < 10 {
        return image;
    }

    let mut rgba = image.into_rgba8();
    apply_grid_overlay_in_place(&mut rgba, step);
    DynamicImage::ImageRgba8(rgba)
}

/// Compare current image against a reference image and calculate bounding box of differences.
/// Optimizes performance via zero-copy borrowing and contiguous row comparisons.
pub fn calculate_image_diff(
    current: &DynamicImage,
    reference: &DynamicImage,
    threshold: u8,
) -> DiffResult {
    let curr_cow = borrow_or_convert_rgba8(current);
    let ref_cow = borrow_or_convert_rgba8(reference);

    let cw = curr_cow.width();
    let ch = curr_cow.height();
    let rw = ref_cow.width();
    let rh = ref_cow.height();

    let total_pixels = (cw as u64) * (ch as u64);
    if total_pixels == 0 {
        return DiffResult {
            changed: false,
            changed_pixels: 0,
            total_pixels: 0,
            changed_percent: 0.0,
            threshold,
            bbox: None,
        };
    }

    let mut changed_pixels = 0u64;
    let mut min_x = u32::MAX;
    let mut max_x = 0u32;
    let mut min_y = u32::MAX;
    let mut max_y = 0u32;

    if cw == rw && ch == rh {
        let curr_raw = curr_cow.as_raw();
        let ref_raw = ref_cow.as_raw();
        let stride = (cw as usize) * 4;

        for y in 0..ch {
            let row_offset = (y as usize) * stride;
            let c_row = &curr_raw[row_offset..row_offset + stride];
            let r_row = &ref_raw[row_offset..row_offset + stride];

            if c_row == r_row {
                continue;
            }

            for (x_usize, (cp, rp)) in c_row.chunks_exact(4).zip(r_row.chunks_exact(4)).enumerate()
            {
                let is_diff = cp[0].abs_diff(rp[0]) > threshold
                    || cp[1].abs_diff(rp[1]) > threshold
                    || cp[2].abs_diff(rp[2]) > threshold
                    || cp[3].abs_diff(rp[3]) > threshold;

                if is_diff {
                    let x = x_usize as u32;
                    changed_pixels += 1;
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
    } else {
        for y in 0..ch {
            for x in 0..cw {
                let is_diff = if x < rw && y < rh {
                    let cp = curr_cow.get_pixel(x, y);
                    let rp = ref_cow.get_pixel(x, y);
                    (0..4).any(|c| cp[c].abs_diff(rp[c]) > threshold)
                } else {
                    true
                };

                if is_diff {
                    changed_pixels += 1;
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
    }

    if changed_pixels > 0 && min_x <= max_x && min_y <= max_y {
        let width = max_x - min_x + 1;
        let height = max_y - min_y + 1;
        let percent =
            ((changed_pixels as f64) / (total_pixels as f64) * 100.0 * 1000.0).round() / 1000.0;
        DiffResult {
            changed: true,
            changed_pixels,
            total_pixels,
            changed_percent: percent,
            threshold,
            bbox: Some(DiffBBox {
                x: min_x,
                y: min_y,
                width,
                height,
            }),
        }
    } else {
        DiffResult {
            changed: false,
            changed_pixels: 0,
            total_pixels,
            changed_percent: 0.0,
            threshold,
            bbox: None,
        }
    }
}

/// Crop image to the difference bounding box with optional padding margin.
pub fn crop_to_diff_bbox(image: DynamicImage, bbox: &DiffBBox, padding: u32) -> DynamicImage {
    let w = image.width();
    let h = image.height();

    let px = bbox.x.saturating_sub(padding);
    let py = bbox.y.saturating_sub(padding);
    let pw = (bbox.width + padding * 2).min(w.saturating_sub(px));
    let ph = (bbox.height + padding * 2).min(h.saturating_sub(py));

    if pw == 0 || ph == 0 {
        return image;
    }

    image.crop_imm(px, py, pw, ph)
}

/// Draw a prominent bounding box around visual differences on an RGBA image in-place.
pub fn highlight_diff_bbox_in_place(
    rgba: &mut image::RgbaImage,
    bbox: &DiffBBox,
    stroke_width: u32,
    box_color: Option<image::Rgba<u8>>,
) {
    let img_w = rgba.width();
    let img_h = rgba.height();

    let stroke = stroke_width.clamp(1, 10);
    let color = box_color.unwrap_or(image::Rgba([255, 30, 30, 255])); // High-visibility red
    let label_bg = image::Rgba([0, 0, 0, 210]);
    let label_fg = image::Rgba([255, 255, 255, 255]);

    let x0 = bbox.x.min(img_w);
    let y0 = bbox.y.min(img_h);
    let x1 = (bbox.x.saturating_add(bbox.width)).min(img_w);
    let y1 = (bbox.y.saturating_add(bbox.height)).min(img_h);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    // Top and bottom borders
    for py in y0..(y0 + stroke).min(y1) {
        for px in x0..x1 {
            rgba.put_pixel(px, py, color);
        }
    }
    let b_top = y1.saturating_sub(stroke).max(y0);
    for py in b_top..y1 {
        for px in x0..x1 {
            rgba.put_pixel(px, py, color);
        }
    }
    // Left and right borders
    let r_left = x1.saturating_sub(stroke).max(x0);
    for py in y0..y1 {
        for px in x0..(x0 + stroke).min(x1) {
            rgba.put_pixel(px, py, color);
        }
        for px in r_left..x1 {
            rgba.put_pixel(px, py, color);
        }
    }

    // Small label tag
    let label = format!("DIFF:{}x{}", bbox.width, bbox.height);
    let label_y = if y0 >= 10 { y0 - 9 } else { y0 + stroke + 1 };
    draw_label_3x5(rgba, x0, label_y, &label, label_fg, label_bg);
}

/// Draw a prominent bounding box around visual differences on the image.
pub fn highlight_diff_bbox(
    image: DynamicImage,
    bbox: &DiffBBox,
    stroke_width: u32,
    box_color: Option<image::Rgba<u8>>,
) -> DynamicImage {
    let mut rgba = image.into_rgba8();
    highlight_diff_bbox_in_place(&mut rgba, bbox, stroke_width, box_color);
    DynamicImage::ImageRgba8(rgba)
}

/// Watermark position options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WatermarkPosition {
    #[default]
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
}

/// Overlay timestamp and/or custom tag badge onto RGBA image in-place.
pub fn apply_watermark_and_tag_in_place(
    rgba: &mut image::RgbaImage,
    timestamp: Option<&str>,
    tag: Option<&str>,
    position: WatermarkPosition,
) {
    let mut parts = Vec::new();
    if let Some(t) = timestamp.filter(|s| !s.is_empty()) {
        parts.push(t);
    }
    if let Some(tg) = tag.filter(|s| !s.is_empty()) {
        parts.push(tg);
    }
    if parts.is_empty() {
        return;
    }

    let text = parts.join(" | ");
    let img_w = rgba.width();
    let img_h = rgba.height();

    // 4 pixels per character + padding
    let badge_w = (text.len() as u32) * 4 + 6;
    let badge_h = 11; // 7px text + 4px padding

    if img_w < badge_w || img_h < badge_h {
        return;
    }

    let margin = 8u32;
    let (start_x, start_y) = match position {
        WatermarkPosition::BottomRight => (
            img_w.saturating_sub(badge_w + margin),
            img_h.saturating_sub(badge_h + margin),
        ),
        WatermarkPosition::BottomLeft => (margin, img_h.saturating_sub(badge_h + margin)),
        WatermarkPosition::TopRight => (img_w.saturating_sub(badge_w + margin), margin),
        WatermarkPosition::TopLeft => (margin, margin),
    };

    let fg = image::Rgba([255, 255, 255, 255]);
    let bg = image::Rgba([0, 0, 0, 200]);

    draw_label_3x5(rgba, start_x, start_y, &text, fg, bg);
}

/// Overlay timestamp and/or custom tag badge onto image.
pub fn apply_watermark_and_tag(
    image: DynamicImage,
    timestamp: Option<&str>,
    tag: Option<&str>,
    position: WatermarkPosition,
) -> DynamicImage {
    let mut rgba = image.into_rgba8();
    apply_watermark_and_tag_in_place(&mut rgba, timestamp, tag, position);
    DynamicImage::ImageRgba8(rgba)
}

/// Draw an overlaid mouse cursor arrow (with optional focus halo ring) onto an RGBA canvas in-place.
/// Uses pure Safe Rust, alpha-blends with the existing background, and clamps to canvas boundaries.
pub fn draw_cursor_overlay_in_place(
    rgba: &mut image::RgbaImage,
    cursor_x: i32,
    cursor_y: i32,
    highlight: bool,
) {
    let width = rgba.width() as i32;
    let height = rgba.height() as i32;

    // Helper closure to blend a pixel safely
    let mut blend_px = |x: i32, y: i32, color: image::Rgba<u8>| {
        if x < 0 || y < 0 || x >= width || y >= height {
            return;
        }
        let alpha = color[3] as f32 / 255.0;
        if alpha <= 0.0 {
            return;
        }
        let px = rgba.get_pixel_mut(x as u32, y as u32);
        if alpha >= 1.0 {
            *px = color;
            return;
        }
        let inv_alpha = 1.0 - alpha;
        px[0] = (color[0] as f32 * alpha + px[0] as f32 * inv_alpha).round() as u8;
        px[1] = (color[1] as f32 * alpha + px[1] as f32 * inv_alpha).round() as u8;
        px[2] = (color[2] as f32 * alpha + px[2] as f32 * inv_alpha).round() as u8;
        px[3] = (color[3] as f32 + px[3] as f32 * (1.0 - color[3] as f32 / 255.0)).min(255.0) as u8;
    };

    // 1. Draw highlight focus ring if requested
    if highlight {
        let ring_inner = 14;
        let ring_outer = 18;
        let ring_inner_sq = ring_inner * ring_inner;
        let ring_outer_sq = ring_outer * ring_outer;
        let ring_color = image::Rgba([255, 179, 0, 210]); // Vivid Amber / Golden highlight
        let crosshair_color = image::Rgba([255, 87, 34, 230]); // Deep Orange crosshair markers

        for dy in -ring_outer..=ring_outer {
            for dx in -ring_outer..=ring_outer {
                let d_sq = dx * dx + dy * dy;
                if d_sq >= ring_inner_sq && d_sq <= ring_outer_sq {
                    blend_px(cursor_x + dx, cursor_y + dy, ring_color);
                }
            }
        }

        // Draw subtle 3px tick marks on 4 axes: left, right, top, bottom
        for i in 19..=22 {
            blend_px(cursor_x + i, cursor_y, crosshair_color);
            blend_px(cursor_x - i, cursor_y, crosshair_color);
            blend_px(cursor_x, cursor_y + i, crosshair_color);
            blend_px(cursor_x, cursor_y - i, crosshair_color);
        }
    }

    // 2. Overlay 11x17 classic cursor arrow bitmap
    // 0 = transparent, 1 = black border (#000000), 2 = white interior (#FFFFFF)
    #[rustfmt::skip]
    const ARROW_PATTERN: [[u8; 11]; 17] = [
        [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        [1, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0], [1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0],
        [1, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0], [1, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0],
        [1, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0], [1, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0],
        [1, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0], [1, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0],
        [1, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1], [1, 2, 2, 1, 2, 2, 1, 0, 0, 0, 0],
        [1, 2, 1, 0, 1, 2, 2, 1, 0, 0, 0], [1, 1, 0, 0, 1, 2, 2, 1, 0, 0, 0],
        [1, 0, 0, 0, 0, 1, 2, 2, 1, 0, 0], [0, 0, 0, 0, 0, 1, 2, 2, 1, 0, 0],
        [0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0],
    ];

    let black = image::Rgba([0, 0, 0, 255]);
    let white = image::Rgba([255, 255, 255, 255]);

    for (row_idx, row) in ARROW_PATTERN.iter().enumerate() {
        for (col_idx, &val) in row.iter().enumerate() {
            let px_x = cursor_x + col_idx as i32;
            let px_y = cursor_y + row_idx as i32;
            match val {
                1 => blend_px(px_x, px_y, black),
                2 => blend_px(px_x, px_y, white),
                _ => {}
            }
        }
    }
}

/// Draw an overlaid mouse cursor arrow (with optional focus halo ring) onto the canvas.
/// Uses pure Safe Rust, alpha-blends with the existing background, and clamps to canvas boundaries.
pub fn draw_cursor_overlay(
    image: DynamicImage,
    cursor_x: i32,
    cursor_y: i32,
    highlight: bool,
) -> DynamicImage {
    let mut rgba = image.into_rgba8();
    draw_cursor_overlay_in_place(&mut rgba, cursor_x, cursor_y, highlight);
    DynamicImage::ImageRgba8(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn test_apply_image_transforms() {
        let img = DynamicImage::ImageRgba8(RgbaImage::new(100, 200));

        // Test scale
        let scaled = apply_image_transforms(img.clone(), Some(0.5), None);
        assert_eq!(scaled.width(), 50);
        assert_eq!(scaled.height(), 100);

        // Test max_dimension
        let constrained = apply_image_transforms(img.clone(), None, Some(80));
        assert_eq!(constrained.width(), 40);
        assert_eq!(constrained.height(), 80);

        // Test identity
        let unchanged = apply_image_transforms(img.clone(), None, None);
        assert_eq!(unchanged.width(), 100);
        assert_eq!(unchanged.height(), 200);
    }

    #[test]
    fn test_apply_redaction_blackout() {
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        apply_redaction(&mut img, 10, 10, 20, 20, MaskEffect::Blackout);

        assert_eq!(img.get_pixel(10, 10), &Rgba([0, 0, 0, 255]));
        assert_eq!(img.get_pixel(29, 29), &Rgba([0, 0, 0, 255]));
        assert_eq!(img.get_pixel(9, 9), &Rgba([255, 255, 255, 255]));
        assert_eq!(img.get_pixel(30, 30), &Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn test_apply_redaction_transparent() {
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([255, 0, 0, 255]));
        apply_redaction(&mut img, 5, 5, 10, 10, MaskEffect::Transparent);

        assert_eq!(img.get_pixel(5, 5), &Rgba([0, 0, 0, 0]));
        assert_eq!(img.get_pixel(14, 14), &Rgba([0, 0, 0, 0]));
        assert_eq!(img.get_pixel(4, 4), &Rgba([255, 0, 0, 255]));
        assert_eq!(img.get_pixel(15, 15), &Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn test_apply_redaction_pixelate_and_blur() {
        let mut img = RgbaImage::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                img.put_pixel(x, y, Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255]));
            }
        }

        // Test pixelate
        apply_redaction(&mut img, 10, 10, 30, 30, MaskEffect::Pixelate);
        let p1 = *img.get_pixel(10, 10);
        let p2 = *img.get_pixel(15, 15);
        assert_eq!(p1, p2);

        // Test blur
        apply_redaction(&mut img, 50, 50, 40, 40, MaskEffect::Blur);

        // Test out of bounds / zero dimension defense
        apply_redaction(&mut img, 200, 200, 50, 50, MaskEffect::Transparent);
        apply_redaction(&mut img, 10, 10, 0, 0, MaskEffect::Transparent);
    }

    #[test]
    fn test_apply_redactions_wrapper() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            50,
            50,
            image::Rgba([100, 100, 100, 255]),
        ));
        let masks = vec![
            (0, 0, 10, 10, MaskEffect::Transparent),
            (20, 20, 10, 10, MaskEffect::Blackout),
        ];
        let rgba = apply_redactions(img, &masks).into_rgba8();
        assert_eq!(rgba.get_pixel(0, 0), &image::Rgba([0, 0, 0, 0]));
        assert_eq!(rgba.get_pixel(20, 20), &image::Rgba([0, 0, 0, 255]));
        assert_eq!(rgba.get_pixel(40, 40), &image::Rgba([100, 100, 100, 255]));
    }

    #[test]
    fn test_apply_grid_overlay() {
        let img =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 200, image::Rgba([0, 0, 0, 255])));
        let with_grid = apply_grid_overlay(img, 50);
        let rgba = with_grid.into_rgba8();

        // Pixel at (50, 50) is crosshair center
        assert_eq!(rgba.get_pixel(50, 50), &image::Rgba([255, 70, 70, 220]));
        // Label starts at (52, 52), background is dark alpha blend, character pixels are white
        assert_ne!(rgba.get_pixel(53, 53), &image::Rgba([0, 0, 0, 255]));
        // Step below 10 is ignored
        let orig =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(50, 50, image::Rgba([1, 2, 3, 255])));
        let unchanged = apply_grid_overlay(orig.clone(), 5);
        assert_eq!(orig.into_rgba8(), unchanged.into_rgba8());
    }

    #[test]
    fn test_calculate_image_diff_identical() {
        let img1 = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            100,
            100,
            image::Rgba([128, 128, 128, 255]),
        ));
        let img2 = img1.clone();
        let diff = calculate_image_diff(&img1, &img2, 16);
        assert!(!diff.changed);
        assert_eq!(diff.changed_pixels, 0);
        assert_eq!(diff.total_pixels, 10000);
        assert_eq!(diff.changed_percent, 0.0);
        assert_eq!(diff.bbox, None);
    }

    #[test]
    fn test_calculate_image_diff_modified_and_crop() {
        let img1 = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            100,
            100,
            image::Rgba([128, 128, 128, 255]),
        ));
        let mut img2_rgba = RgbaImage::from_pixel(100, 100, image::Rgba([128, 128, 128, 255]));

        // Modify a 20x10 rectangle from (30, 40) to (49, 49)
        for y in 40..50 {
            for x in 30..50 {
                img2_rgba.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        let img2 = DynamicImage::ImageRgba8(img2_rgba);

        let diff = calculate_image_diff(&img2, &img1, 16);
        assert!(diff.changed);
        assert_eq!(diff.changed_pixels, 200);
        assert_eq!(diff.total_pixels, 10000);
        assert_eq!(diff.changed_percent, 2.0);
        assert_eq!(
            diff.bbox,
            Some(DiffBBox {
                x: 30,
                y: 40,
                width: 20,
                height: 10
            })
        );

        // Test crop to diff bbox with padding=5
        let cropped = crop_to_diff_bbox(img2, diff.bbox.as_ref().unwrap(), 5);
        assert_eq!(cropped.width(), 30); // 20 + 5*2
        assert_eq!(cropped.height(), 20); // 10 + 5*2
    }

    #[test]
    fn test_highlight_diff_bbox() {
        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 255])));
        let bbox = DiffBBox {
            x: 20,
            y: 20,
            width: 30,
            height: 30,
        };
        let highlighted = highlight_diff_bbox(base, &bbox, 2, None);
        let rgba = highlighted.into_rgba8();

        // Top border at (20, 20) should be colored bright red
        assert_eq!(rgba.get_pixel(20, 20), &image::Rgba([255, 30, 30, 255]));
        // Bottom border at (20, 49) should be bright red
        assert_eq!(rgba.get_pixel(20, 49), &image::Rgba([255, 30, 30, 255]));
    }

    #[test]
    fn test_highlight_diff_bbox_with_crop_mapping() {
        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(500, 500, image::Rgba([0, 0, 0, 255])));
        let bbox = DiffBBox {
            x: 200,
            y: 250,
            width: 50,
            height: 40,
        };
        let padding = 8u32;
        let crop_x = bbox.x.saturating_sub(padding);
        let crop_y = bbox.y.saturating_sub(padding);

        let cropped = crop_to_diff_bbox(base, &bbox, padding);
        let mut rgba = cropped.into_rgba8();

        // Local bbox mapped to cropped image coordinates
        let local_bbox = DiffBBox {
            x: bbox.x.saturating_sub(crop_x),
            y: bbox.y.saturating_sub(crop_y),
            width: bbox.width,
            height: bbox.height,
        };

        highlight_diff_bbox_in_place(&mut rgba, &local_bbox, 2, None);

        // Highlight top-left border should be at local coordinates (8, 8)
        assert_eq!(local_bbox.x, 8);
        assert_eq!(local_bbox.y, 8);
        assert_eq!(rgba.get_pixel(8, 8), &image::Rgba([255, 30, 30, 255]));
        // Bottom-right border at (57, 47)
        assert_eq!(rgba.get_pixel(57, 47), &image::Rgba([255, 30, 30, 255]));
    }

    #[test]
    fn test_apply_watermark_and_tag() {
        let base = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            200,
            100,
            image::Rgba([50, 50, 50, 255]),
        ));
        let with_wm = apply_watermark_and_tag(
            base,
            Some("2026-11-20"),
            Some("STEP-1"),
            WatermarkPosition::BottomRight,
        );
        let rgba = with_wm.into_rgba8();

        // Check that watermark area at (120, 83) was modified
        let sample = rgba.get_pixel(120, 83);
        assert_ne!(sample, &image::Rgba([50, 50, 50, 255]));
    }

    #[test]
    fn test_get_glyph_3x5_ascii() {
        for ch in ['A', 'z', '5', ':', '/', '|', '_', ' '] {
            assert!(get_glyph_3x5(ch).is_some());
        }
        assert!(get_glyph_3x5('~').is_none());
    }

    #[test]
    fn test_draw_cursor_overlay_standard() {
        let base = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            100,
            100,
            image::Rgba([128, 128, 128, 255]),
        ));
        let with_cursor = draw_cursor_overlay(base, 20, 20, false);
        let rgba = with_cursor.into_rgba8();

        // Tip of the cursor at (20, 20) is black border
        assert_eq!(rgba.get_pixel(20, 20), &image::Rgba([0, 0, 0, 255]));
        // Interior of cursor arrow at (21, 22) is white interior
        assert_eq!(rgba.get_pixel(21, 22), &image::Rgba([255, 255, 255, 255]));
        // Outside the cursor bounding box should be unchanged
        assert_eq!(rgba.get_pixel(10, 10), &image::Rgba([128, 128, 128, 255]));
    }

    #[test]
    fn test_draw_cursor_overlay_highlight() {
        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 255])));
        let with_highlight = draw_cursor_overlay(base, 50, 50, true);
        let rgba = with_highlight.into_rgba8();

        // Cursor tip at (50, 50)
        assert_eq!(rgba.get_pixel(50, 50), &image::Rgba([0, 0, 0, 255]));
        // Ring radius ~16 pixels right at (66, 50) should be modified (amber ring)
        let ring_px = rgba.get_pixel(66, 50);
        assert_ne!(ring_px, &image::Rgba([0, 0, 0, 255]));
        assert!(ring_px[0] > 100); // Amber has high red
    }
}
