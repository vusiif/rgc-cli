use log::debug;
use xcap::Window;

use crate::capture::window::find_window;
use crate::cli::{CoordMode, MaskEffect, MaskRegion, MaskWindow, Region};

/// Resolve a mask region from user coordinates (logical or physical) to physical canvas pixels.
/// Clamps to canvas dimensions. If the region is completely outside, returns None.
#[allow(dead_code)]
pub fn resolve_mask_region(
    region: &Region,
    coords: CoordMode,
    scale_factor: f32,
    canvas_w: u32,
    canvas_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    let (phys_x, phys_y, phys_w, phys_h) = match coords {
        CoordMode::Physical => (
            region.x as f64,
            region.y as f64,
            region.width as f64,
            region.height as f64,
        ),
        CoordMode::Logical => {
            let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
                scale_factor as f64
            } else {
                1.0
            };
            (
                (region.x as f64 * scale).round(),
                (region.y as f64 * scale).round(),
                (region.width as f64 * scale).round(),
                (region.height as f64 * scale).round(),
            )
        }
    };

    let x0 = phys_x.clamp(0.0, canvas_w as f64) as u32;
    let y0 = phys_y.clamp(0.0, canvas_h as f64) as u32;
    let x1 = (phys_x + phys_w).clamp(0.0, canvas_w as f64) as u32;
    let y1 = (phys_y + phys_h).clamp(0.0, canvas_h as f64) as u32;

    if x0 < x1 && y0 < y1 {
        Some((x0, y0, x1 - x0, y1 - y0))
    } else {
        None
    }
}

/// Compute the intersection of a window's physical bounds with the target monitor's physical canvas.
/// Returns (x, y, width, height) relative to the captured image canvas (after optional crop offset).
#[allow(dead_code, clippy::too_many_arguments)]
pub fn resolve_window_mask_bounds(
    win_x: i32,
    win_y: i32,
    win_w: u32,
    win_h: u32,
    mon_x: i32,
    mon_y: i32,
    crop_offset_x: u32,
    crop_offset_y: u32,
    canvas_w: u32,
    canvas_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    let rel_x = (win_x - mon_x) - crop_offset_x as i32;
    let rel_y = (win_y - mon_y) - crop_offset_y as i32;

    let x0 = rel_x.clamp(0, canvas_w as i32) as u32;
    let y0 = rel_y.clamp(0, canvas_h as i32) as u32;
    let x1 = (rel_x + win_w as i32).clamp(0, canvas_w as i32) as u32;
    let y1 = (rel_y + win_h as i32).clamp(0, canvas_h as i32) as u32;

    if x0 < x1 && y0 < y1 {
        Some((x0, y0, x1 - x0, y1 - y0))
    } else {
        None
    }
}

/// Build concrete pixel redactions from manual mask regions and window masks.
#[allow(clippy::too_many_arguments)]
pub fn build_redactions(
    canvas_w: u32,
    canvas_h: u32,
    scale_factor: f32,
    crop_offset: (u32, u32),
    monitor_origin: Option<(i32, i32)>,
    coords: CoordMode,
    manual_masks: &[MaskRegion],
    window_masks: &[MaskWindow],
    windows: Option<&[Window]>,
) -> Vec<(u32, u32, u32, u32, MaskEffect)> {
    let mut redactions = Vec::new();

    // 1. Process manual mask regions
    for mm in manual_masks {
        let (phys_x, phys_y, phys_w, phys_h) = match coords {
            CoordMode::Physical => (
                mm.region.x as f64,
                mm.region.y as f64,
                mm.region.width as f64,
                mm.region.height as f64,
            ),
            CoordMode::Logical => {
                let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
                    scale_factor as f64
                } else {
                    1.0
                };
                (
                    (mm.region.x as f64 * scale).round(),
                    (mm.region.y as f64 * scale).round(),
                    (mm.region.width as f64 * scale).round(),
                    (mm.region.height as f64 * scale).round(),
                )
            }
        };

        let img_x = phys_x - crop_offset.0 as f64;
        let img_y = phys_y - crop_offset.1 as f64;

        let x0 = img_x.clamp(0.0, canvas_w as f64) as u32;
        let y0 = img_y.clamp(0.0, canvas_h as f64) as u32;
        let x1 = (img_x + phys_w).clamp(0.0, canvas_w as f64) as u32;
        let y1 = (img_y + phys_h).clamp(0.0, canvas_h as f64) as u32;

        if x0 < x1 && y0 < y1 {
            let w = x1 - x0;
            let h = y1 - y0;
            debug!(
                "Resolved manual mask: {:?} -> x={}, y={}, w={}, h={}, effect={:?}",
                mm.region, x0, y0, w, h, mm.effect
            );
            redactions.push((x0, y0, w, h, mm.effect));
        }
    }

    // 2. Process window masks
    if !window_masks.is_empty() {
        if let Some((mon_x, mon_y)) = monitor_origin {
            if let Some(win_list) = windows {
                for wm in window_masks {
                    match find_window(win_list, &wm.query) {
                        Ok(target_win) => {
                            if target_win.is_minimized().unwrap_or(false) {
                                debug!("Mask window target '{}' is minimized, skipping", wm.query);
                                continue;
                            }
                            let win_x = target_win.x().unwrap_or(0);
                            let win_y = target_win.y().unwrap_or(0);
                            let win_w = target_win.width().unwrap_or(0);
                            let win_h = target_win.height().unwrap_or(0);

                            if let Some((x, y, w, h)) = resolve_window_mask_bounds(
                                win_x,
                                win_y,
                                win_w,
                                win_h,
                                mon_x,
                                mon_y,
                                crop_offset.0,
                                crop_offset.1,
                                canvas_w,
                                canvas_h,
                            ) {
                                debug!(
                                    "Resolved window mask '{}' [bounds {},{} {}x{}]: x={}, y={}, w={}, h={}, effect={:?}",
                                    wm.query, win_x, win_y, win_w, win_h, x, y, w, h, wm.effect
                                );
                                redactions.push((x, y, w, h, wm.effect));
                            } else {
                                debug!(
                                    "Window mask '{}' does not intersect captured screen area",
                                    wm.query
                                );
                            }
                        }
                        Err(e) => {
                            debug!(
                                "Window mask target '{}' not found on desktop: {}",
                                wm.query, e
                            );
                        }
                    }
                }
            }
        }
    }

    redactions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_mask_region_dpi_150() {
        let region = Region {
            x: 100,
            y: 200,
            width: 300,
            height: 400,
        };

        // 150% DPI scale on 2560x1440 canvas
        let res = resolve_mask_region(&region, CoordMode::Logical, 1.5, 2560, 1440).unwrap();
        assert_eq!(res, (150, 300, 450, 600));

        // Physical mode: 1:1 without scaling
        let res_phys = resolve_mask_region(&region, CoordMode::Physical, 1.5, 2560, 1440).unwrap();
        assert_eq!(res_phys, (100, 200, 300, 400));
    }

    #[test]
    fn test_resolve_mask_region_clamping() {
        let out_region = Region {
            x: -50,
            y: -20,
            width: 100,
            height: 100,
        };
        // Clamped at (0, 0)
        let res = resolve_mask_region(&out_region, CoordMode::Physical, 1.0, 500, 500).unwrap();
        assert_eq!(res, (0, 0, 50, 80));

        // Completely outside -> returns None
        let far_out = Region {
            x: 600,
            y: 600,
            width: 100,
            height: 100,
        };
        assert!(resolve_mask_region(&far_out, CoordMode::Physical, 1.0, 500, 500).is_none());
    }

    #[test]
    fn test_resolve_window_mask_bounds() {
        // Monitor at (0, 0), resolution 2560x1440, no crop
        let bounds =
            resolve_window_mask_bounds(100, 200, 800, 600, 0, 0, 0, 0, 2560, 1440).unwrap();
        assert_eq!(bounds, (100, 200, 800, 600));

        // Monitor at (2560, 0), window at (2600, 100) -> relative to monitor is (40, 100)
        let bounds_m2 =
            resolve_window_mask_bounds(2600, 100, 500, 400, 2560, 0, 0, 0, 1920, 1080).unwrap();
        assert_eq!(bounds_m2, (40, 100, 500, 400));

        // Crop offset (50, 50) on 2560x1440
        let bounds_crop =
            resolve_window_mask_bounds(100, 200, 800, 600, 0, 0, 50, 50, 1000, 800).unwrap();
        assert_eq!(bounds_crop, (50, 150, 800, 600));

        // Window outside monitor
        let bounds_none = resolve_window_mask_bounds(3000, 2000, 500, 400, 0, 0, 0, 0, 2560, 1440);
        assert!(bounds_none.is_none());
    }

    #[test]
    fn test_build_redactions_manual_and_window() {
        let manual = vec![MaskRegion {
            region: Region {
                x: 10,
                y: 20,
                width: 30,
                height: 40,
            },
            effect: MaskEffect::Transparent,
        }];
        let window_masks = Vec::new();

        let redactions = build_redactions(
            500,
            500,
            1.5,
            (0, 0),
            Some((0, 0)),
            CoordMode::Logical,
            &manual,
            &window_masks,
            None,
        );

        assert_eq!(redactions.len(), 1);
        assert_eq!(redactions[0], (15, 30, 45, 60, MaskEffect::Transparent));
    }
}
