use std::collections::VecDeque;

use image::{DynamicImage, Rgba};

use crate::api::UiElement;

const PALETTE: [Rgba<u8>; 6] = [
    Rgba([0, 229, 255, 255]),  // Vivid Cyan
    Rgba([118, 255, 3, 255]),  // Electric Lime
    Rgba([255, 214, 0, 255]),  // Bright Gold
    Rgba([255, 64, 129, 255]), // Hot Pink / Coral
    Rgba([41, 121, 255, 255]), // Electric Blue
    Rgba([224, 64, 251, 255]), // Bright Violet
];

/// Detect interactive UI elements (buttons, inputs, cards) using edge gradient analysis
/// and morphological clustering directly from an RGBA image.
/// Uses a 3-row sliding cache to achieve O(1) row memory overhead and maximum L1/L2 cache locality.
pub fn detect_ui_elements_from_rgba(
    rgba: &image::RgbaImage,
    threshold: u8,
    max_elements: usize,
) -> Vec<UiElement> {
    let width = rgba.width();
    let height = rgba.height();
    if width < 30 || height < 20 || max_elements == 0 {
        return Vec::new();
    }

    let grad_threshold = (threshold as u32).max(5) * 2;

    // Block size for spatial clustering: 4px for standard, 8px for 4K
    let block_size: u32 = if width >= 2560 || height >= 1440 {
        8
    } else {
        4
    };
    let grid_w = width.div_ceil(block_size);
    let grid_h = height.div_ceil(block_size);

    // Step 1: Detect edge pixels using 3-row sliding luminance cache
    let mut block_edges = vec![0u16; (grid_w * grid_h) as usize];

    let w = width as usize;
    let raw_bytes = rgba.as_raw();
    let stride = w * 4;

    let fill_lum_row = |row: u32, buf: &mut [u8]| {
        let offset = (row as usize) * stride;
        let slice = &raw_bytes[offset..offset + stride];
        for (x, out_val) in buf.iter_mut().enumerate().take(w) {
            let px = x * 4;
            let r = slice[px] as u32;
            let g = slice[px + 1] as u32;
            let b = slice[px + 2] as u32;
            *out_val = ((r * 299 + g * 587 + b * 114) / 1000) as u8;
        }
    };

    let mut prev_lum = vec![0u8; w];
    let mut curr_lum = vec![0u8; w];
    let mut next_lum = vec![0u8; w];

    fill_lum_row(0, &mut prev_lum);
    fill_lum_row(1, &mut curr_lum);

    for y in 1..height.saturating_sub(1) {
        fill_lum_row(y + 1, &mut next_lum);
        let by = y / block_size;
        let by_stride = (by * grid_w) as usize;

        for x in 1..width.saturating_sub(1) {
            let xi = x as usize;
            let dx = (curr_lum[xi + 1] as i32 - curr_lum[xi - 1] as i32).unsigned_abs();
            let dy = (next_lum[xi] as i32 - prev_lum[xi] as i32).unsigned_abs();

            if dx + dy >= grad_threshold {
                let bx = x / block_size;
                block_edges[by_stride + bx as usize] =
                    block_edges[by_stride + bx as usize].saturating_add(1);
            }
        }

        std::mem::swap(&mut prev_lum, &mut curr_lum);
        std::mem::swap(&mut curr_lum, &mut next_lum);
    }

    // Step 2: Binary block mask with threshold
    let mut active_blocks = vec![false; (grid_w * grid_h) as usize];
    for by in 0..grid_h {
        for bx in 0..grid_w {
            let idx = (by * grid_w + bx) as usize;
            if block_edges[idx] >= 2 {
                active_blocks[idx] = true;
            }
        }
    }

    // Morphological dilation (connects borders with internal button text/icons)
    let mut dilated = active_blocks.clone();
    for by in 0..grid_h {
        for bx in 0..grid_w {
            if active_blocks[(by * grid_w + bx) as usize] {
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let ny = by as i32 + dy;
                        let nx = bx as i32 + dx;
                        if ny >= 0 && ny < grid_h as i32 && nx >= 0 && nx < grid_w as i32 {
                            dilated[(ny as u32 * grid_w + nx as u32) as usize] = true;
                        }
                    }
                }
            }
        }
    }

    // Step 3: Connected Component Labeling on block grid (BFS)
    let mut visited = vec![false; (grid_w * grid_h) as usize];
    let mut raw_boxes = Vec::new();

    for start_by in 0..grid_h {
        for start_bx in 0..grid_w {
            let start_idx = (start_by * grid_w + start_bx) as usize;
            if !dilated[start_idx] || visited[start_idx] {
                continue;
            }

            let mut min_bx = start_bx;
            let mut max_bx = start_bx;
            let mut min_by = start_by;
            let mut max_by = start_by;
            let mut block_count = 0usize;

            let mut queue = VecDeque::new();
            queue.push_back((start_bx, start_by));
            visited[start_idx] = true;

            while let Some((cx, cy)) = queue.pop_front() {
                block_count += 1;
                min_bx = min_bx.min(cx);
                max_bx = max_bx.max(cx);
                min_by = min_by.min(cy);
                max_by = max_by.max(cy);

                // 8-neighbor exploration
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = cx as i32 + dx;
                        let ny = cy as i32 + dy;
                        if nx >= 0 && nx < grid_w as i32 && ny >= 0 && ny < grid_h as i32 {
                            let n_idx = (ny as u32 * grid_w + nx as u32) as usize;
                            if dilated[n_idx] && !visited[n_idx] {
                                visited[n_idx] = true;
                                queue.push_back((nx as u32, ny as u32));
                            }
                        }
                    }
                }
            }

            // Convert block coords to pixel coords
            let px_x = min_bx * block_size;
            let px_y = min_by * block_size;
            let px_w = ((max_bx - min_bx + 1) * block_size).min(width.saturating_sub(px_x));
            let px_h = ((max_by - min_by + 1) * block_size).min(height.saturating_sub(px_y));

            // Filtering heuristics for clickable UI elements
            if px_w < 20 || px_h < 14 {
                continue;
            }
            if px_w > (width as f32 * 0.95) as u32 && px_h > (height as f32 * 0.95) as u32 {
                // Entire screen / background window border
                continue;
            }
            let aspect = px_w as f32 / px_h as f32;
            if !(0.05..=25.0).contains(&aspect) {
                // Single line dividers
                continue;
            }

            raw_boxes.push((px_x, px_y, px_w, px_h, block_count));
        }
    }

    // Step 4: Non-Maximum Suppression (NMS) & duplicate suppression
    let mut filtered_boxes = Vec::new();
    // Sort by area descending
    raw_boxes.sort_by_key(|b| std::cmp::Reverse(b.2 * b.3));

    for &(x1, y1, w1, h1, _) in &raw_boxes {
        let mut keep = true;
        for &(x2, y2, w2, h2, _) in &filtered_boxes {
            let ix0 = x1.max(x2);
            let iy0 = y1.max(y2);
            let ix1 = (x1 + w1).min(x2 + w2);
            let iy1 = (y1 + h1).min(y2 + h2);

            if ix1 > ix0 && iy1 > iy0 {
                let inter_area = (ix1 - ix0) * (iy1 - iy0);
                let union_area = (w1 * h1) + (w2 * h2) - inter_area;
                let iou = inter_area as f32 / union_area as f32;

                if iou > 0.65 {
                    keep = false;
                    break;
                }
            }
        }
        if keep {
            filtered_boxes.push((x1, y1, w1, h1, 0));
        }
    }

    // Step 5: Natural reading order sorting (top-to-bottom in row bands, then left-to-right)
    filtered_boxes.sort_by(|a, b| {
        let band_threshold = 24u32;
        let y_diff = (a.1 as i32 - b.1 as i32).unsigned_abs();
        if y_diff <= band_threshold {
            a.0.cmp(&b.0)
        } else {
            a.1.cmp(&b.1)
        }
    });

    // Step 6: Construct UiElement list with 1-based IDs capped at max_elements
    let mut elements = Vec::with_capacity(filtered_boxes.len().min(max_elements));
    for (idx, &(x, y, w, h, _)) in filtered_boxes.iter().take(max_elements).enumerate() {
        elements.push(UiElement {
            id: (idx + 1) as u32,
            x,
            y,
            width: w,
            height: h,
            center_x: x + w / 2,
            center_y: y + h / 2,
        });
    }

    elements
}

/// Detect interactive UI elements (buttons, inputs, cards) using edge gradient analysis
/// and morphological clustering on a dynamic image.
pub fn detect_ui_elements(
    image: &DynamicImage,
    threshold: u8,
    max_elements: usize,
) -> Vec<UiElement> {
    let cow = crate::transform::borrow_or_convert_rgba8(image);
    detect_ui_elements_from_rgba(&cow, threshold, max_elements)
}

/// Render high-contrast Set-of-Mark (SoM) visual bounding boxes and numbered badges onto an RGBA canvas in-place.
pub fn draw_som_overlay_in_place(rgba: &mut image::RgbaImage, elements: &[UiElement]) {
    if elements.is_empty() {
        return;
    }

    let img_w = rgba.width();
    let img_h = rgba.height();

    for elem in elements {
        let color = PALETTE[((elem.id.saturating_sub(1)) as usize) % PALETTE.len()];
        let border_thickness = 2u32;

        let x0 = elem.x.min(img_w);
        let y0 = elem.y.min(img_h);
        let x1 = (elem.x + elem.width).min(img_w);
        let y1 = (elem.y + elem.height).min(img_h);

        // Draw 2px rectangular border
        for py in y0..y1 {
            for px in x0..x1 {
                let is_top = py < (y0 + border_thickness);
                let is_bottom = py >= y1.saturating_sub(border_thickness);
                let is_left = px < (x0 + border_thickness);
                let is_right = px >= x1.saturating_sub(border_thickness);

                if is_top || is_bottom || is_left || is_right {
                    rgba.put_pixel(px, py, color);
                }
            }
        }

        // Draw numbered badge at top-left of the box
        let badge_text = format!("{}", elem.id);
        let label_y = if y0 >= 8 { y0.saturating_sub(8) } else { y0 };
        let label_fg = Rgba([0, 0, 0, 255]); // Black text
        let label_bg = color; // Matching vivid badge background

        crate::transform::draw_label_3x5(rgba, x0, label_y, &badge_text, label_fg, label_bg);
    }
}

/// Render high-contrast Set-of-Mark (SoM) visual bounding boxes and numbered badges onto the image.
pub fn draw_som_overlay(image: DynamicImage, elements: &[UiElement]) -> DynamicImage {
    if elements.is_empty() {
        return image;
    }

    let mut rgba = image.into_rgba8();
    draw_som_overlay_in_place(&mut rgba, elements);
    DynamicImage::ImageRgba8(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    #[test]
    fn test_detect_ui_elements_synthetic() {
        // Create 400x300 dark gray canvas
        let mut img = RgbaImage::from_pixel(400, 300, Rgba([30, 30, 30, 255]));

        // Draw button 1 at (50, 50) with size 100x40 (bright white)
        for y in 50..90 {
            for x in 50..150 {
                img.put_pixel(x, y, Rgba([240, 240, 240, 255]));
            }
        }

        // Draw button 2 at (220, 50) with size 120x40 (bright cyan)
        for y in 50..90 {
            for x in 220..340 {
                img.put_pixel(x, y, Rgba([0, 200, 255, 255]));
            }
        }

        let dynamic = DynamicImage::ImageRgba8(img);
        let elements = detect_ui_elements(&dynamic, 25, 10);

        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0].id, 1);
        assert_eq!(elements[1].id, 2);

        // Check center coords of element 1 (around x=100, y=70)
        assert!((elements[0].center_x as i32 - 100).abs() <= 8);
        assert!((elements[0].center_y as i32 - 70).abs() <= 8);

        // Check center coords of element 2 (around x=280, y=70)
        assert!((elements[1].center_x as i32 - 280).abs() <= 8);
        assert!((elements[1].center_y as i32 - 70).abs() <= 8);
    }

    #[test]
    fn test_draw_som_overlay() {
        let base = DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 200, Rgba([0, 0, 0, 255])));
        let elements = vec![UiElement {
            id: 1,
            x: 20,
            y: 20,
            width: 80,
            height: 40,
            center_x: 60,
            center_y: 40,
        }];

        let overlaid = draw_som_overlay(base, &elements);
        let rgba = overlaid.into_rgba8();

        // Border at (20, 20) should have cyan color from PALETTE[0]
        assert_eq!(rgba.get_pixel(20, 20), &PALETTE[0]);
    }

    #[test]
    fn test_detect_ui_elements_tiny_canvas() {
        let tiny = DynamicImage::ImageRgba8(RgbaImage::new(10, 10));
        let elements = detect_ui_elements(&tiny, 25, 10);
        assert!(elements.is_empty());
    }
}
