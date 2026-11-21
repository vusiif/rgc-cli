use image::{DynamicImage, RgbaImage};
use log::debug;
use xcap::Monitor;

use crate::capture::crop::resolve_crop_region;
use crate::capture::screen::{capture_screen_with_details, get_monitors};
use crate::capture::types::{CaptureError, CapturedScreen};
use crate::cli::{CoordMode, Region};

struct MonItem {
    monitor: Monitor,
    x: i32,
    y: i32,
}

/// Capture all connected monitors stitched together into a unified virtual desktop panorama.
pub fn capture_all_monitors_with_details(
    region: Option<&Region>,
    coords: CoordMode,
) -> Result<CapturedScreen, CaptureError> {
    let monitors = get_monitors()?;
    if monitors.is_empty() {
        return Err(CaptureError::NoMonitorsFound);
    }

    if monitors.len() == 1 {
        return capture_screen_with_details(Some(0), region, coords);
    }

    let mut items = Vec::with_capacity(monitors.len());
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    let mut primary_scale = 1.0f32;

    for m in monitors {
        let w = m.width().map_err(CaptureError::MonitorPropertyFailed)?;
        let h = m.height().map_err(CaptureError::MonitorPropertyFailed)?;
        if w == 0 || h == 0 {
            continue;
        }
        let x = m.x().unwrap_or(0);
        let y = m.y().unwrap_or(0);

        if m.is_primary().unwrap_or(false) {
            primary_scale = m.scale_factor().unwrap_or(1.0);
        }

        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x.saturating_add(w as i32));
        max_y = max_y.max(y.saturating_add(h as i32));

        items.push(MonItem { monitor: m, x, y });
    }

    if items.is_empty() {
        return Err(CaptureError::NoMonitorsFound);
    }

    let total_w = (max_x - min_x).max(1) as u32;
    let total_h = (max_y - min_y).max(1) as u32;

    debug!(
        "Stitching multi-monitor panorama: virtual desktop ({}, {})..({}, {}) = {}x{}, displays: {}",
        min_x,
        min_y,
        max_x,
        max_y,
        total_w,
        total_h,
        items.len()
    );

    let mut canvas = RgbaImage::new(total_w, total_h);

    for item in items {
        let raw = item
            .monitor
            .capture_image()
            .map_err(CaptureError::CaptureFailed)?;
        let rel_x = (item.x - min_x) as usize;
        let rel_y = (item.y - min_y) as usize;
        let rw = raw.width() as usize;
        let rh = raw.height() as usize;
        let cw = canvas.width() as usize;
        let ch = canvas.height() as usize;

        let copy_w = rw.min(cw.saturating_sub(rel_x));
        let copy_h = rh.min(ch.saturating_sub(rel_y));

        let raw_slice = raw.as_raw();
        let canvas_slice = canvas.as_mut();

        for row in 0..copy_h {
            let src_start = row * rw * 4;
            let src_end = src_start + copy_w * 4;
            let dst_start = ((rel_y + row) * cw + rel_x) * 4;
            let dst_end = dst_start + copy_w * 4;
            canvas_slice[dst_start..dst_end].copy_from_slice(&raw_slice[src_start..src_end]);
        }
    }

    let (image, crop_offset) = if let Some(reg) = region {
        let (crop_x, crop_y, crop_w, crop_h) =
            resolve_crop_region(reg, coords, primary_scale, total_w, total_h)?;
        if crop_x == 0 && crop_y == 0 && crop_w == total_w && crop_h == total_h {
            (DynamicImage::ImageRgba8(canvas), (0, 0))
        } else {
            let cropped =
                image::imageops::crop_imm(&canvas, crop_x, crop_y, crop_w, crop_h).to_image();
            (DynamicImage::ImageRgba8(cropped), (crop_x, crop_y))
        }
    } else {
        (DynamicImage::ImageRgba8(canvas), (0, 0))
    };

    Ok(CapturedScreen {
        image,
        scale_factor: primary_scale,
        crop_offset,
        monitor_origin: (min_x, min_y),
    })
}

/// Capture all connected monitors stitched together into a unified virtual desktop panorama.
#[allow(dead_code)]
pub fn capture_all_monitors(
    region: Option<&Region>,
    coords: CoordMode,
) -> Result<DynamicImage, CaptureError> {
    capture_all_monitors_with_details(region, coords).map(|c| c.image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_all_monitors_headless_safe() {
        // In headless CI or graphical sessions, capture_all_monitors must either succeed or return a valid CaptureError, never panic.
        match capture_all_monitors(None, CoordMode::Logical) {
            Ok(img) => {
                assert!(img.width() > 0);
                assert!(img.height() > 0);
            }
            Err(CaptureError::NoMonitorsFound)
            | Err(CaptureError::ListMonitorsFailed(_))
            | Err(CaptureError::CaptureFailed(_)) => {}
            Err(other) => panic!("Unexpected error from capture_all_monitors: {other:?}"),
        }
    }
}
