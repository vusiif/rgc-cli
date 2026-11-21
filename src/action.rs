#![forbid(unsafe_code)]

//! Action feedback overlays and action verification.
//!
//! Provides visual overlays for mouse clicks (concentric expanding ripples, target dots)
//! and drag trajectories (directional dashed arrows with start anchors and arrowheads),
//! as well as localized visual change verification around interaction targets.

use image::{DynamicImage, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

use crate::transform::{blend_pixel, draw_label_3x5, DiffBBox};

/// Visual click marker targeting a coordinate on the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickMarker {
    pub x: u32,
    pub y: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Visual drag trajectory between two screen coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragMarker {
    pub start_x: u32,
    pub start_y: u32,
    pub end_x: u32,
    pub end_y: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Outcome of localized action verification around a target coordinate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionVerificationResult {
    pub verified: bool,
    pub target_x: u32,
    pub target_y: u32,
    pub radius: u32,
    pub changed_pixels: u64,
    pub total_roi_pixels: u64,
    pub change_percent: f64,
    pub threshold: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<DiffBBox>,
}

/// Parse a click marker from string `X,Y` or `X,Y,LABEL`.
pub fn parse_click_marker(s: &str) -> Result<ClickMarker, String> {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    if parts.len() < 2 || parts.len() > 3 {
        return Err(format!(
            "Invalid click format '{s}': expected 'X,Y' or 'X,Y,LABEL'"
        ));
    }

    let x: u32 = parts[0]
        .parse()
        .map_err(|e| format!("Invalid click x coordinate '{}': {e}", parts[0]))?;
    let y: u32 = parts[1]
        .parse()
        .map_err(|e| format!("Invalid click y coordinate '{}': {e}", parts[1]))?;

    let label = if parts.len() == 3 && !parts[2].is_empty() {
        Some(parts[2].to_string())
    } else {
        None
    };

    Ok(ClickMarker { x, y, label })
}

/// Parse a drag trajectory from string `X1,Y1,X2,Y2` or `X1,Y1,X2,Y2,LABEL`.
pub fn parse_drag_marker(s: &str) -> Result<DragMarker, String> {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    if parts.len() < 4 || parts.len() > 5 {
        return Err(format!(
            "Invalid drag format '{s}': expected 'X1,Y1,X2,Y2' or 'X1,Y1,X2,Y2,LABEL'"
        ));
    }

    let start_x: u32 = parts[0]
        .parse()
        .map_err(|e| format!("Invalid drag start_x '{}': {e}", parts[0]))?;
    let start_y: u32 = parts[1]
        .parse()
        .map_err(|e| format!("Invalid drag start_y '{}': {e}", parts[1]))?;
    let end_x: u32 = parts[2]
        .parse()
        .map_err(|e| format!("Invalid drag end_x '{}': {e}", parts[2]))?;
    let end_y: u32 = parts[3]
        .parse()
        .map_err(|e| format!("Invalid drag end_y '{}': {e}", parts[3]))?;

    let label = if parts.len() == 5 && !parts[4].is_empty() {
        Some(parts[4].to_string())
    } else {
        None
    };

    Ok(DragMarker {
        start_x,
        start_y,
        end_x,
        end_y,
        label,
    })
}

/// Draw a ring of specified radius and thickness centered at `(cx, cy)`.
fn draw_circle_ring(
    img: &mut RgbaImage,
    cx: i32,
    cy: i32,
    radius: i32,
    thickness: i32,
    color: Rgba<u8>,
) {
    let w = img.width() as i32;
    let h = img.height() as i32;

    let r_outer = radius + thickness / 2;
    let r_inner = (radius - thickness / 2).max(0);
    let r_outer_sq = r_outer * r_outer;
    let r_inner_sq = r_inner * r_inner;

    let min_x = (cx - r_outer).clamp(0, w - 1);
    let max_x = (cx + r_outer).clamp(0, w - 1);
    let min_y = (cy - r_outer).clamp(0, h - 1);
    let max_y = (cy + r_outer).clamp(0, h - 1);

    for py in min_y..=max_y {
        let dy = py - cy;
        let dy_sq = dy * dy;
        for px in min_x..=max_x {
            let dx = px - cx;
            let dist_sq = dx * dx + dy_sq;
            if dist_sq <= r_outer_sq && dist_sq >= r_inner_sq {
                blend_pixel(img, px as u32, py as u32, color);
            }
        }
    }
}

/// Draw a solid filled circle of specified radius centered at `(cx, cy)`.
fn draw_filled_circle(img: &mut RgbaImage, cx: i32, cy: i32, radius: i32, color: Rgba<u8>) {
    let w = img.width() as i32;
    let h = img.height() as i32;
    let r_sq = radius * radius;

    let min_x = (cx - radius).clamp(0, w - 1);
    let max_x = (cx + radius).clamp(0, w - 1);
    let min_y = (cy - radius).clamp(0, h - 1);
    let max_y = (cy + radius).clamp(0, h - 1);

    for py in min_y..=max_y {
        let dy = py - cy;
        let dy_sq = dy * dy;
        for px in min_x..=max_x {
            let dx = px - cx;
            if dx * dx + dy_sq <= r_sq {
                blend_pixel(img, px as u32, py as u32, color);
            }
        }
    }
}

/// Draw a line segment between two points with optional dashed pattern and thickness.
fn draw_line_segment(
    img: &mut RgbaImage,
    p0: (i32, i32),
    p1: (i32, i32),
    thickness: i32,
    color: Rgba<u8>,
    dashed: bool,
) {
    let (x0, y0) = p0;
    let (x1, y1) = p1;
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx - dy;

    let mut curr_x = x0;
    let mut curr_y = y0;
    let mut step = 0;

    let half_th = thickness / 2;

    loop {
        // In dashed mode: draw 8 steps, skip 4 steps
        let draw_pixel = !dashed || ((step / 4) % 3 != 2);

        if draw_pixel {
            for ox in -half_th..=half_th {
                for oy in -half_th..=half_th {
                    let px = curr_x + ox;
                    let py = curr_y + oy;
                    if px >= 0 && px < img.width() as i32 && py >= 0 && py < img.height() as i32 {
                        blend_pixel(img, px as u32, py as u32, color);
                    }
                }
            }
        }

        if curr_x == x1 && curr_y == y1 {
            break;
        }

        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            curr_x += sx;
        }
        if e2 < dx {
            err += dx;
            curr_y += sy;
        }
        step += 1;
    }
}

/// Draw a triangular directional arrowhead pointing at `tip`.
fn draw_arrowhead(
    img: &mut RgbaImage,
    from: (i32, i32),
    tip: (i32, i32),
    arrow_len: f32,
    wing_angle_rad: f32,
    color: Rgba<u8>,
) {
    let (from_x, from_y) = from;
    let (tip_x, tip_y) = tip;
    let dx = (tip_x - from_x) as f32;
    let dy = (tip_y - from_y) as f32;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist < 2.0 {
        return;
    }

    let dir_x = dx / dist;
    let dir_y = dy / dist;

    // Wing angles relative to reverse direction
    let angle = dy.atan2(dx);
    let left_angle = angle + std::f32::consts::PI - wing_angle_rad;
    let right_angle = angle + std::f32::consts::PI + wing_angle_rad;

    let left_x = (tip_x as f32 + arrow_len * left_angle.cos()).round() as i32;
    let left_y = (tip_y as f32 + arrow_len * left_angle.sin()).round() as i32;

    let right_x = (tip_x as f32 + arrow_len * right_angle.cos()).round() as i32;
    let right_y = (tip_y as f32 + arrow_len * right_angle.sin()).round() as i32;

    // Draw lines from tip to wings and base
    draw_line_segment(img, (tip_x, tip_y), (left_x, left_y), 2, color, false);
    draw_line_segment(img, (tip_x, tip_y), (right_x, right_y), 2, color, false);
    draw_line_segment(img, (left_x, left_y), (right_x, right_y), 2, color, false);

    // Draw small filled center triangle for solid visibility
    let mid_base_x = (left_x + right_x) / 2;
    let mid_base_y = (left_y + right_y) / 2;
    draw_line_segment(
        img,
        (tip_x, tip_y),
        (mid_base_x, mid_base_y),
        2,
        color,
        false,
    );

    // Arrowhead tip accent
    draw_filled_circle(img, tip_x, tip_y, 2, color);
    let _ = (dir_x, dir_y);
}

/// Render click ripple and target indicators onto an RGBA canvas.
pub fn draw_click_marker(img: &mut RgbaImage, marker: &ClickMarker) {
    let cx = marker.x as i32;
    let cy = marker.y as i32;

    // Palette: Vibrant cyan concentric ripples with white/dark crosshairs
    let color_outer_ring = Rgba([0, 220, 255, 140]);
    let color_inner_ring = Rgba([0, 220, 255, 210]);
    let color_center_dot = Rgba([255, 255, 255, 255]);
    let color_center_shadow = Rgba([0, 0, 0, 220]);
    let color_ticks = Rgba([255, 60, 60, 230]);

    // Concentric expanding rings
    draw_circle_ring(img, cx, cy, 18, 2, color_outer_ring);
    draw_circle_ring(img, cx, cy, 10, 2, color_inner_ring);

    // Cardinal tick marks
    draw_line_segment(img, (cx - 14, cy), (cx - 6, cy), 2, color_ticks, false);
    draw_line_segment(img, (cx + 6, cy), (cx + 14, cy), 2, color_ticks, false);
    draw_line_segment(img, (cx, cy - 14), (cx, cy - 6), 2, color_ticks, false);
    draw_line_segment(img, (cx, cy + 6), (cx, cy + 14), 2, color_ticks, false);

    // Solid center dot with dark drop shadow border
    draw_filled_circle(img, cx, cy, 4, color_center_shadow);
    draw_filled_circle(img, cx, cy, 2, color_center_dot);

    // Optional text badge
    let label = marker
        .label
        .as_deref()
        .map(|l| format!("{l} ({},{})", marker.x, marker.y))
        .unwrap_or_else(|| format!("({},{})", marker.x, marker.y));

    let fg_text = Rgba([255, 255, 255, 255]);
    let bg_text = Rgba([10, 20, 30, 220]);
    let badge_x = (marker.x + 16).min(img.width().saturating_sub(40));
    let badge_y = if marker.y >= 16 {
        marker.y - 12
    } else {
        marker.y + 12
    };
    draw_label_3x5(img, badge_x, badge_y, &label, fg_text, bg_text);
}

/// Render a drag gesture trajectory onto an RGBA canvas.
pub fn draw_drag_marker(img: &mut RgbaImage, marker: &DragMarker) {
    let x0 = marker.start_x as i32;
    let y0 = marker.start_y as i32;
    let x1 = marker.end_x as i32;
    let y1 = marker.end_y as i32;

    // Palette: Emerald start point, high-contrast amber dashed path, bright orange arrowhead
    let color_start_ring = Rgba([30, 230, 120, 230]);
    let color_start_dot = Rgba([255, 255, 255, 255]);
    let color_path_shadow = Rgba([0, 0, 0, 180]);
    let color_path_core = Rgba([255, 180, 0, 240]);
    let color_arrowhead = Rgba([255, 120, 0, 255]);

    // Start anchor: concentric green dot
    draw_circle_ring(img, x0, y0, 8, 2, color_start_ring);
    draw_filled_circle(img, x0, y0, 3, color_start_dot);

    // Directional dashed trajectory path with drop-shadow halo
    draw_line_segment(img, (x0, y0), (x1, y1), 4, color_path_shadow, true);
    draw_line_segment(img, (x0, y0), (x1, y1), 2, color_path_core, true);

    // Directional arrowhead at destination
    draw_arrowhead(img, (x0, y0), (x1, y1), 14.0, 0.45, color_arrowhead);

    // Destination target dot
    draw_filled_circle(img, x1, y1, 3, color_arrowhead);

    // Optional label near midpoint
    if let Some(ref lbl) = marker.label {
        let mid_x = ((marker.start_x + marker.end_x) / 2)
            .saturating_add(6)
            .min(img.width().saturating_sub(40));
        let mid_y = ((marker.start_y + marker.end_y) / 2)
            .saturating_sub(10)
            .min(img.height().saturating_sub(15));
        let fg_text = Rgba([255, 255, 255, 255]);
        let bg_text = Rgba([20, 20, 20, 220]);
        draw_label_3x5(img, mid_x, mid_y, lbl, fg_text, bg_text);
    }
}

/// Composite click markers and drag trajectories onto a dynamic image.
pub fn draw_action_overlays(
    image: DynamicImage,
    clicks: &[ClickMarker],
    drags: &[DragMarker],
) -> DynamicImage {
    if clicks.is_empty() && drags.is_empty() {
        return image;
    }

    let mut rgba = image.into_rgba8();

    for drag in drags {
        draw_drag_marker(&mut rgba, drag);
    }

    for click in clicks {
        draw_click_marker(&mut rgba, click);
    }

    DynamicImage::ImageRgba8(rgba)
}

/// Verify whether visual changes occurred in a localized ROI around `(target_x, target_y)`.
pub fn verify_action_change(
    before: &DynamicImage,
    after: &DynamicImage,
    target_x: u32,
    target_y: u32,
    radius: u32,
    threshold: u8,
) -> ActionVerificationResult {
    let b_rgba = before.to_rgba8();
    let a_rgba = after.to_rgba8();

    let bw = b_rgba.width();
    let bh = b_rgba.height();
    let aw = a_rgba.width();
    let ah = a_rgba.height();

    let w = bw.min(aw);
    let h = bh.min(ah);

    let (min_x, max_x, min_y, max_y) = if radius == 0 {
        (0, w, 0, h)
    } else {
        (
            target_x.saturating_sub(radius).min(w),
            target_x.saturating_add(radius + 1).min(w),
            target_y.saturating_sub(radius).min(h),
            target_y.saturating_add(radius + 1).min(h),
        )
    };

    let r_sq = (radius as i64) * (radius as i64);
    let mut total_roi_pixels = 0u64;
    let mut changed_pixels = 0u64;

    let mut diff_min_x = u32::MAX;
    let mut diff_max_x = 0u32;
    let mut diff_min_y = u32::MAX;
    let mut diff_max_y = 0u32;

    for y in min_y..max_y {
        let dy = (y as i64) - (target_y as i64);
        let dy_sq = dy * dy;
        for x in min_x..max_x {
            let dx = (x as i64) - (target_x as i64);
            if radius > 0 && (dx * dx + dy_sq > r_sq) {
                continue;
            }

            total_roi_pixels += 1;

            let bp = b_rgba.get_pixel(x, y);
            let ap = a_rgba.get_pixel(x, y);

            let is_diff = bp[0].abs_diff(ap[0]) > threshold
                || bp[1].abs_diff(ap[1]) > threshold
                || bp[2].abs_diff(ap[2]) > threshold
                || bp[3].abs_diff(ap[3]) > threshold;

            if is_diff {
                changed_pixels += 1;
                diff_min_x = diff_min_x.min(x);
                diff_max_x = diff_max_x.max(x);
                diff_min_y = diff_min_y.min(y);
                diff_max_y = diff_max_y.max(y);
            }
        }
    }

    let change_percent = if total_roi_pixels > 0 {
        ((changed_pixels as f64) / (total_roi_pixels as f64)) * 100.0
    } else {
        0.0
    };

    let bbox = if changed_pixels > 0 && diff_min_x <= diff_max_x && diff_min_y <= diff_max_y {
        Some(DiffBBox {
            x: diff_min_x,
            y: diff_min_y,
            width: diff_max_x - diff_min_x + 1,
            height: diff_max_y - diff_min_y + 1,
        })
    } else {
        None
    };

    // Consider verified if at least 5 pixels changed or >= 0.1% of ROI changed
    let verified = changed_pixels >= 5;

    ActionVerificationResult {
        verified,
        target_x,
        target_y,
        radius,
        changed_pixels,
        total_roi_pixels,
        change_percent,
        threshold,
        bbox,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_click_marker() {
        let c1 = parse_click_marker("150,300").unwrap();
        assert_eq!(
            c1,
            ClickMarker {
                x: 150,
                y: 300,
                label: None
            }
        );

        let c2 = parse_click_marker("200, 400 , submit_btn").unwrap();
        assert_eq!(
            c2,
            ClickMarker {
                x: 200,
                y: 400,
                label: Some("submit_btn".to_string())
            }
        );

        assert!(parse_click_marker("invalid").is_err());
        assert!(parse_click_marker("150").is_err());
        assert!(parse_click_marker("150,abc").is_err());
    }

    #[test]
    fn test_parse_drag_marker() {
        let d1 = parse_drag_marker("100,200,300,400").unwrap();
        assert_eq!(
            d1,
            DragMarker {
                start_x: 100,
                start_y: 200,
                end_x: 300,
                end_y: 400,
                label: None
            }
        );

        let d2 = parse_drag_marker("10, 20, 30, 40, swipe_right").unwrap();
        assert_eq!(
            d2,
            DragMarker {
                start_x: 10,
                start_y: 20,
                end_x: 30,
                end_y: 40,
                label: Some("swipe_right".to_string())
            }
        );

        assert!(parse_drag_marker("100,200,300").is_err());
        assert!(parse_drag_marker("100,200,300,abc").is_err());
    }

    #[test]
    fn test_draw_click_marker_synthetic() {
        let img = DynamicImage::new_rgba8(200, 200);
        let clicks = vec![ClickMarker {
            x: 100,
            y: 100,
            label: Some("click".to_string()),
        }];
        let marked = draw_action_overlays(img, &clicks, &[]);
        let rgba = marked.to_rgba8();

        // Center pixel should be white or center dot
        let center = rgba.get_pixel(100, 100);
        assert_eq!(center[0], 255);
        assert_eq!(center[1], 255);
        assert_eq!(center[2], 255);
    }

    #[test]
    fn test_draw_drag_marker_synthetic() {
        let img = DynamicImage::new_rgba8(300, 300);
        let drags = vec![DragMarker {
            start_x: 50,
            start_y: 50,
            end_x: 250,
            end_y: 250,
            label: Some("drag".to_string()),
        }];
        let marked = draw_action_overlays(img, &[], &drags);
        let rgba = marked.to_rgba8();

        // Start anchor center should be marked
        let start_center = rgba.get_pixel(50, 50);
        assert_eq!(start_center[3], 255);

        // End tip should have arrowhead color
        let end_tip = rgba.get_pixel(250, 250);
        assert_eq!(end_tip[0], 255);
    }

    #[test]
    fn test_verify_action_change_in_roi() {
        let before = DynamicImage::new_rgba8(200, 200);
        let mut after_rgba = before.to_rgba8();

        // Change a 10x10 block around (100, 100)
        for y in 95..105 {
            for x in 95..105 {
                after_rgba.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        let after = DynamicImage::ImageRgba8(after_rgba);

        // Verification centered at (100, 100) with radius 30 should succeed
        let res = verify_action_change(&before, &after, 100, 100, 30, 10);
        assert!(res.verified);
        assert_eq!(res.changed_pixels, 100);
        assert!(res.bbox.is_some());
        let bbox = res.bbox.unwrap();
        assert_eq!(bbox.x, 95);
        assert_eq!(bbox.y, 95);
        assert_eq!(bbox.width, 10);
        assert_eq!(bbox.height, 10);
    }

    #[test]
    fn test_verify_action_change_outside_roi() {
        let before = DynamicImage::new_rgba8(200, 200);
        let mut after_rgba = before.to_rgba8();

        // Change a block far away at (180, 180)
        for y in 175..185 {
            for x in 175..185 {
                after_rgba.put_pixel(x, y, Rgba([0, 255, 0, 255]));
            }
        }
        let after = DynamicImage::ImageRgba8(after_rgba);

        // Verification centered at (50, 50) with radius 20 should detect NO changes
        let res = verify_action_change(&before, &after, 50, 50, 20, 10);
        assert!(!res.verified);
        assert_eq!(res.changed_pixels, 0);
        assert!(res.bbox.is_none());
    }

    #[test]
    fn test_action_boundaries_no_panic() {
        let img = DynamicImage::new_rgba8(50, 50);
        let clicks = vec![
            ClickMarker {
                x: 0,
                y: 0,
                label: None,
            },
            ClickMarker {
                x: 49,
                y: 49,
                label: Some("edge".to_string()),
            },
        ];
        let drags = vec![DragMarker {
            start_x: 0,
            start_y: 0,
            end_x: 49,
            end_y: 49,
            label: Some("diagonal".to_string()),
        }];

        let result = draw_action_overlays(img, &clicks, &drags);
        assert_eq!(result.width(), 50);
        assert_eq!(result.height(), 50);
    }
}
