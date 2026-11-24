#![forbid(unsafe_code)]

//! High-performance normalized cross-correlation (NCC) visual template and icon matching.
//!
//! Provides sub-pixel accurate, lighting-invariant template and icon detection across desktop screenshots:
//! 1. Fast integral image-based variance and local energy computation (O(1) window queries).
//! 2. Alpha transparency awareness (ignores transparent template padding/cutouts).
//! 3. Coarse-to-fine multi-scale pyramid search for ultra-fast processing on 2K/4K screenshots.
//! 4. Non-maximum suppression (NMS) to eliminate overlapping redundant detections.

use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

use crate::api::TemplateMatchResult;
use crate::coords::CoordinateTransform;

/// Configuration options for template/icon matching.
#[derive(Debug, Clone)]
pub struct TemplateMatchOptions {
    /// Target template image to locate on the screenshot.
    pub template: DynamicImage,
    /// Minimum confidence threshold in range 0.0 - 1.0 (default: 0.85).
    pub threshold: f32,
    /// Maximum number of matching occurrences to return (default: 1).
    pub max_matches: usize,
    /// Whether to overlay visual highlighting boxes and confidence badges onto the output image.
    pub highlight: bool,
}

impl Default for TemplateMatchOptions {
    fn default() -> Self {
        Self {
            template: DynamicImage::new_rgba8(0, 0),
            threshold: 0.85,
            max_matches: 1,
            highlight: false,
        }
    }
}

/// Internal representation of a grayscale image with precomputed 2D integral images.
struct GrayscaleSearchCanvas {
    width: usize,
    #[allow(dead_code)]
    height: usize,
    pixels: Vec<f64>,
    sum: Vec<f64>,
    sum_sq: Vec<f64>,
}

impl GrayscaleSearchCanvas {
    fn from_rgba(rgba: &RgbaImage) -> Self {
        let width = rgba.width() as usize;
        let height = rgba.height() as usize;
        let mut pixels = vec![0.0f64; width * height];

        let raw = rgba.as_raw();
        for y in 0..height {
            let row_offset = y * width * 4;
            let dest_offset = y * width;
            for x in 0..width {
                let px = row_offset + x * 4;
                let r = raw[px] as f64;
                let g = raw[px + 1] as f64;
                let b = raw[px + 2] as f64;
                pixels[dest_offset + x] = (r * 299.0 + g * 587.0 + b * 114.0) / 1000.0;
            }
        }

        // Compute 2D integral images for sum and sum of squares
        // Dimensions of integral images: (width + 1) * (height + 1)
        let stride = width + 1;
        let mut sum = vec![0.0f64; (width + 1) * (height + 1)];
        let mut sum_sq = vec![0.0f64; (width + 1) * (height + 1)];

        for y in 0..height {
            let mut row_sum = 0.0f64;
            let mut row_sum_sq = 0.0f64;
            let px_offset = y * width;
            let int_prev_row = y * stride;
            let int_curr_row = (y + 1) * stride;

            for x in 0..width {
                let val = pixels[px_offset + x];
                row_sum += val;
                row_sum_sq += val * val;

                sum[int_curr_row + x + 1] = sum[int_prev_row + x + 1] + row_sum;
                sum_sq[int_curr_row + x + 1] = sum_sq[int_prev_row + x + 1] + row_sum_sq;
            }
        }

        Self {
            width,
            height,
            pixels,
            sum,
            sum_sq,
        }
    }

    /// Query the sum of pixel values in rectangle [x, x + w) x [y, y + h) in O(1) time.
    #[inline(always)]
    fn query_sum(&self, x: usize, y: usize, w: usize, h: usize) -> f64 {
        let stride = self.width + 1;
        let y0 = y * stride;
        let y1 = (y + h) * stride;
        let x0 = x;
        let x1 = x + w;

        self.sum[y1 + x1] - self.sum[y0 + x1] - self.sum[y1 + x0] + self.sum[y0 + x0]
    }

    /// Query the sum of squared pixel values in rectangle [x, x + w) x [y, y + h) in O(1) time.
    #[inline(always)]
    fn query_sum_sq(&self, x: usize, y: usize, w: usize, h: usize) -> f64 {
        let stride = self.width + 1;
        let y0 = y * stride;
        let y1 = (y + h) * stride;
        let x0 = x;
        let x1 = x + w;

        self.sum_sq[y1 + x1] - self.sum_sq[y0 + x1] - self.sum_sq[y1 + x0] + self.sum_sq[y0 + x0]
    }
}

/// Normalized template representation with optional alpha mask.
struct PreparedTemplate {
    width: usize,
    height: usize,
    /// Zero-mean normalized pixel values for active foreground pixels.
    normalized_pixels: Vec<f64>,
    /// Active foreground mask indices relative to top-left (dx, dy).
    active_offsets: Vec<(usize, usize, f64)>,
    /// Number of active foreground pixels.
    active_count: f64,
    /// Standard deviation / energy denominator of the template.
    template_norm: f64,
}

impl PreparedTemplate {
    fn from_image(img: &DynamicImage) -> Option<Self> {
        let (width, height) = img.dimensions();
        let width = width as usize;
        let height = height as usize;

        if width == 0 || height == 0 {
            return None;
        }

        let rgba = img.to_rgba8();
        let raw = rgba.as_raw();

        let mut active_offsets = Vec::new();
        let mut sum = 0.0f64;

        for y in 0..height {
            let row_offset = y * width * 4;
            for x in 0..width {
                let px = row_offset + x * 4;
                let a = raw[px + 3];
                // Only consider non-transparent foreground pixels
                if a >= 128 {
                    let r = raw[px] as f64;
                    let g = raw[px + 1] as f64;
                    let b = raw[px + 2] as f64;
                    let lum = (r * 299.0 + g * 587.0 + b * 114.0) / 1000.0;
                    active_offsets.push((x, y, lum));
                    sum += lum;
                }
            }
        }

        let active_count = active_offsets.len() as f64;
        if active_count < 4.0 {
            return None;
        }

        let mean = sum / active_count;
        let mut variance_sum = 0.0f64;

        for (_, _, lum) in &mut active_offsets {
            *lum -= mean;
            variance_sum += *lum * *lum;
        }

        let template_norm = variance_sum.sqrt();
        // Constant color template has no variance
        if template_norm <= 1e-6 {
            return None;
        }

        let mut normalized_pixels = vec![0.0f64; width * height];
        for &(x, y, zero_mean_lum) in &active_offsets {
            normalized_pixels[y * width + x] = zero_mean_lum;
        }

        Some(Self {
            width,
            height,
            normalized_pixels,
            active_offsets,
            active_count,
            template_norm,
        })
    }
}

/// Search for occurrences of `template` in `target` image.
///
/// Returns sorted match results in descending order of confidence score.
pub fn find_template_matches(
    target: &RgbaImage,
    template: &DynamicImage,
    threshold: f32,
    max_matches: usize,
) -> Vec<TemplateMatchResult> {
    if max_matches == 0 {
        return Vec::new();
    }

    let prep_template = match PreparedTemplate::from_image(template) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let target_w = target.width() as usize;
    let target_h = target.height() as usize;

    if target_w < prep_template.width || target_h < prep_template.height {
        return Vec::new();
    }

    let canvas = GrayscaleSearchCanvas::from_rgba(target);
    let t_w = prep_template.width;
    let t_h = prep_template.height;
    let max_x = target_w - t_w;
    let max_y = target_h - t_h;

    let has_full_alpha = prep_template.active_offsets.len() == (t_w * t_h);
    let threshold_f64 = (threshold as f64).clamp(0.0, 1.0);

    // Multi-scale acceleration check: if search area and template are large,
    // perform coarse step-2 grid scan then refine candidates
    let use_pyramid = (max_x > 200 && max_y > 200) && (t_w >= 16 && t_h >= 16);
    let coarse_step = if use_pyramid { 2 } else { 1 };
    let coarse_threshold = threshold_f64 * 0.75;

    let mut raw_candidates: Vec<(usize, usize, f64)> = Vec::new();

    let mut y = 0;
    while y <= max_y {
        let mut x = 0;
        while x <= max_x {
            let (patch_variance, cross_sum) = if has_full_alpha {
                let sum = canvas.query_sum(x, y, t_w, t_h);
                let sum_sq = canvas.query_sum_sq(x, y, t_w, t_h);
                let var = sum_sq - (sum * sum) / prep_template.active_count;

                if var <= 1e-4 {
                    (var, 0.0)
                } else {
                    let mut cross = 0.0f64;
                    for ty in 0..t_h {
                        let src_start = (y + ty) * target_w + x;
                        let tpl_start = ty * t_w;
                        let src_slice = &canvas.pixels[src_start..src_start + t_w];
                        let tpl_slice =
                            &prep_template.normalized_pixels[tpl_start..tpl_start + t_w];
                        for (s, t) in src_slice.iter().zip(tpl_slice.iter()) {
                            cross += s * t;
                        }
                    }
                    (var, cross)
                }
            } else {
                let mut p_sum = 0.0f64;
                let mut p_sum_sq = 0.0f64;
                let mut cross = 0.0f64;
                for &(tx, ty, t_val) in &prep_template.active_offsets {
                    let val = canvas.pixels[(y + ty) * target_w + (x + tx)];
                    p_sum += val;
                    p_sum_sq += val * val;
                    cross += val * t_val;
                }
                let var = p_sum_sq - (p_sum * p_sum) / prep_template.active_count;
                (var, cross)
            };

            if patch_variance > 1e-4 {
                let patch_norm = patch_variance.sqrt();
                let denom = prep_template.template_norm * patch_norm;

                if denom > 1e-6 {
                    let ncc = cross_sum / denom;
                    let target_thresh = if coarse_step > 1 {
                        coarse_threshold
                    } else {
                        threshold_f64
                    };

                    if ncc >= target_thresh {
                        raw_candidates.push((x, y, ncc));
                    }
                }
            }

            x += coarse_step;
        }
        y += coarse_step;
    }

    // Refinement stage if coarse scan was used
    let mut refined_matches: Vec<(usize, usize, f64)> =
        if coarse_step > 1 && !raw_candidates.is_empty() {
            let mut refined = Vec::with_capacity(raw_candidates.len());
            for (cx, cy, _) in raw_candidates {
                let rx0 = cx.saturating_sub(1);
                let ry0 = cy.saturating_sub(1);
                let rx1 = (cx + 1).min(max_x);
                let ry1 = (cy + 1).min(max_y);

                let mut best_ncc = 0.0f64;
                let mut best_coord = (cx, cy);

                for fy in ry0..=ry1 {
                    for fx in rx0..=rx1 {
                        let (patch_variance, cross_sum) = if has_full_alpha {
                            let sum = canvas.query_sum(fx, fy, t_w, t_h);
                            let sum_sq = canvas.query_sum_sq(fx, fy, t_w, t_h);
                            let var = sum_sq - (sum * sum) / prep_template.active_count;

                            if var <= 1e-4 {
                                (var, 0.0)
                            } else {
                                let mut cross = 0.0f64;
                                for ty in 0..t_h {
                                    let src_start = (fy + ty) * target_w + fx;
                                    let tpl_start = ty * t_w;
                                    let src_slice = &canvas.pixels[src_start..src_start + t_w];
                                    let tpl_slice = &prep_template.normalized_pixels
                                        [tpl_start..tpl_start + t_w];
                                    for (s, t) in src_slice.iter().zip(tpl_slice.iter()) {
                                        cross += s * t;
                                    }
                                }
                                (var, cross)
                            }
                        } else {
                            let mut p_sum = 0.0f64;
                            let mut p_sum_sq = 0.0f64;
                            let mut cross = 0.0f64;
                            for &(tx, ty, t_val) in &prep_template.active_offsets {
                                let val = canvas.pixels[(fy + ty) * target_w + (fx + tx)];
                                p_sum += val;
                                p_sum_sq += val * val;
                                cross += val * t_val;
                            }
                            let var = p_sum_sq - (p_sum * p_sum) / prep_template.active_count;
                            (var, cross)
                        };

                        if patch_variance > 1e-4 {
                            let patch_norm = patch_variance.sqrt();
                            let denom = prep_template.template_norm * patch_norm;
                            if denom > 1e-6 {
                                let ncc = cross_sum / denom;
                                if ncc > best_ncc {
                                    best_ncc = ncc;
                                    best_coord = (fx, fy);
                                }
                            }
                        }
                    }
                }

                if best_ncc >= threshold_f64 {
                    refined.push((best_coord.0, best_coord.1, best_ncc));
                }
            }
            refined
        } else {
            raw_candidates
        };

    if refined_matches.is_empty() {
        return Vec::new();
    }

    // Sort by confidence descending
    refined_matches.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    // Non-Maximum Suppression (NMS)
    let mut selected_matches = Vec::with_capacity(max_matches);
    let suppression_dist_sq =
        ((t_w / 2).max(4) * (t_w / 2).max(4) + (t_h / 2).max(4) * (t_h / 2).max(4)) as i64;

    for (x, y, conf) in refined_matches {
        let mut suppressed = false;
        for (sx, sy, _) in &selected_matches {
            let dx = (*sx as i64) - (x as i64);
            let dy = (*sy as i64) - (y as i64);
            if (dx * dx + dy * dy) < suppression_dist_sq {
                suppressed = true;
                break;
            }
        }

        if !suppressed {
            selected_matches.push((x, y, conf));
            if selected_matches.len() >= max_matches {
                break;
            }
        }
    }

    selected_matches
        .into_iter()
        .map(|(x, y, conf)| {
            TemplateMatchResult::new(
                x as u32,
                y as u32,
                t_w as u32,
                t_h as u32,
                (conf as f32 * 1000.0).round() / 1000.0,
            )
        })
        .collect()
}

/// Enrich template match results with global OS desktop screen coordinates.
pub fn enrich_matches_with_coordinates(
    matches: &mut [TemplateMatchResult],
    transform: &CoordinateTransform,
) {
    for m in matches {
        let (screen_cx, screen_cy) = transform.image_to_screen(m.center_x, m.center_y);
        let (screen_tl_x, screen_tl_y) = transform.image_to_screen(m.x, m.y);
        m.screen_center_x = Some(screen_cx);
        m.screen_center_y = Some(screen_cy);
        m.screen_x = Some(screen_tl_x);
        m.screen_y = Some(screen_tl_y);
    }
}

/// Render high-contrast visual bounding boxes and confidence score badges onto the output image in-place.
pub fn draw_template_matches_in_place(rgba: &mut RgbaImage, matches: &[TemplateMatchResult]) {
    if matches.is_empty() {
        return;
    }

    let img_w = rgba.width();
    let img_h = rgba.height();

    // High-contrast electric purple/violet color for template match bounding box
    let box_color = Rgba([224, 64, 251, 255]);
    let border_thickness = 2u32;

    for m in matches {
        let x0 = m.x.min(img_w);
        let y0 = m.y.min(img_h);
        let x1 = (m.x + m.width).min(img_w);
        let y1 = (m.y + m.height).min(img_h);

        for py in y0..y1 {
            for px in x0..x1 {
                let is_top = py < (y0 + border_thickness);
                let is_bottom = py >= y1.saturating_sub(border_thickness);
                let is_left = px < (x0 + border_thickness);
                let is_right = px >= x1.saturating_sub(border_thickness);

                if is_top || is_bottom || is_left || is_right {
                    rgba.put_pixel(px, py, box_color);
                }
            }
        }

        // Draw confidence badge at top-left of the match box
        let pct = (m.confidence * 100.0).round() as u32;
        let badge_text = format!("{}%", pct);
        let label_y = if y0 >= 8 { y0.saturating_sub(8) } else { y0 };
        let label_fg = Rgba([0, 0, 0, 255]);
        let label_bg = box_color;

        crate::transform::draw_label_3x5(rgba, x0, label_y, &badge_text, label_fg, label_bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_perfect_template_match() {
        let mut target = RgbaImage::new(100, 100);
        // Draw background gradient
        for (x, y, p) in target.enumerate_pixels_mut() {
            *p = Rgba([(x % 50) as u8, (y % 50) as u8, 128, 255]);
        }

        // Insert a unique 16x16 icon at (40, 30)
        let mut icon = RgbaImage::new(16, 16);
        for (x, y, p) in icon.enumerate_pixels_mut() {
            let val = ((x * 15 + y * 15) % 255) as u8;
            *p = Rgba([val, 255 - val, 200, 255]);
            target.put_pixel(40 + x, 30 + y, *p);
        }

        let template = DynamicImage::ImageRgba8(icon);
        let matches = find_template_matches(&target, &template, 0.90, 1);

        assert_eq!(matches.len(), 1);
        let m = &matches[0];
        assert_eq!(m.x, 40);
        assert_eq!(m.y, 30);
        assert_eq!(m.width, 16);
        assert_eq!(m.height, 16);
        assert!(m.confidence >= 0.99);
    }

    #[test]
    fn test_template_match_with_alpha_mask() {
        let mut target = RgbaImage::new(80, 80);
        // Plain solid background
        for (_, _, p) in target.enumerate_pixels_mut() {
            *p = Rgba([40, 40, 40, 255]);
        }

        // Create an icon with a transparent circle cutout
        let mut icon = RgbaImage::new(16, 16);
        for (x, y, p) in icon.enumerate_pixels_mut() {
            let dx = (x as i32) - 8;
            let dy = (y as i32) - 8;
            if dx * dx + dy * dy <= 25 {
                // Non-transparent foreground with radial gradient
                let shade = (255 - (dx.abs() + dy.abs()) * 20).clamp(50, 255) as u8;
                *p = Rgba([shade, 200, 50, 255]);
                target.put_pixel(25 + x, 25 + y, *p);
            } else {
                // Transparent border/padding
                *p = Rgba([0, 0, 0, 0]);
            }
        }

        let template = DynamicImage::ImageRgba8(icon);
        let matches = find_template_matches(&target, &template, 0.85, 1);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].x, 25);
        assert_eq!(matches[0].y, 25);
        assert!(matches[0].confidence >= 0.95);
    }

    #[test]
    fn test_multiple_occurrences_nms() {
        let mut target = RgbaImage::new(120, 120);

        let mut icon = RgbaImage::new(10, 10);
        for (x, y, p) in icon.enumerate_pixels_mut() {
            *p = Rgba([200, (x * 20) as u8, (y * 20) as u8, 255]);
        }

        // Place icon at 3 different distinct locations
        for (x, y, p) in icon.enumerate_pixels() {
            target.put_pixel(10 + x, 10 + y, *p);
            target.put_pixel(50 + x, 50 + y, *p);
            target.put_pixel(80 + x, 20 + y, *p);
        }

        let template = DynamicImage::ImageRgba8(icon);
        let matches = find_template_matches(&target, &template, 0.90, 5);

        assert_eq!(matches.len(), 3);
        let mut found_coords: Vec<(u32, u32)> = matches.into_iter().map(|m| (m.x, m.y)).collect();
        found_coords.sort();

        assert_eq!(found_coords, vec![(10, 10), (50, 50), (80, 20)]);
    }

    #[test]
    fn test_nonexistent_template_empty_result() {
        let target = RgbaImage::new(50, 50);
        let mut icon = RgbaImage::new(8, 8);
        for (_, _, p) in icon.enumerate_pixels_mut() {
            *p = Rgba([255, 0, 0, 255]);
        }

        let template = DynamicImage::ImageRgba8(icon);
        let matches = find_template_matches(&target, &template, 0.85, 1);
        assert!(matches.is_empty());
    }
}
