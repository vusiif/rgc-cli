use image::{DynamicImage, RgbaImage};
use log::debug;
use xcap::Monitor;

use crate::capture::crop::resolve_crop_region;
use crate::capture::types::{CaptureError, CapturedScreen, MonitorInfo};
use crate::cli::{CoordMode, Region};

/// Retrieve all connected monitors from the system.
pub fn get_monitors() -> Result<Vec<Monitor>, CaptureError> {
    Monitor::all().map_err(CaptureError::ListMonitorsFailed)
}

/// Retrieve friendly display information for all connected monitors.
pub fn get_monitor_infos() -> Result<Vec<MonitorInfo>, CaptureError> {
    let monitors = get_monitors()?;
    let mut infos = Vec::with_capacity(monitors.len());

    for (index, monitor) in monitors.iter().enumerate() {
        let name = monitor
            .name()
            .unwrap_or_else(|_| format!("Monitor #{index}"));
        let is_primary = monitor.is_primary().unwrap_or(false);
        let x = monitor.x().unwrap_or(0);
        let y = monitor.y().unwrap_or(0);
        let width = monitor
            .width()
            .map_err(CaptureError::MonitorPropertyFailed)?;
        let height = monitor
            .height()
            .map_err(CaptureError::MonitorPropertyFailed)?;
        let scale_factor = monitor.scale_factor().unwrap_or(1.0);

        infos.push(MonitorInfo {
            index,
            name,
            is_primary,
            x,
            y,
            width,
            height,
            scale_factor,
        });
    }

    Ok(infos)
}

/// Find the primary monitor index, falling back to index 0 if not explicitly designated.
pub fn find_primary_monitor_index(monitors: &[Monitor]) -> usize {
    for (idx, m) in monitors.iter().enumerate() {
        if m.is_primary().unwrap_or(false) {
            return idx;
        }
    }
    0
}

/// Capture the primary monitor and return raw RGBA image data in memory.
#[allow(dead_code)]
pub fn capture_primary_monitor() -> Result<RgbaImage, CaptureError> {
    let monitors = get_monitors()?;
    if monitors.is_empty() {
        return Err(CaptureError::NoMonitorsFound);
    }

    let primary_idx = find_primary_monitor_index(&monitors);
    debug!("Capturing primary monitor at index {}", primary_idx);

    monitors[primary_idx]
        .capture_image()
        .map_err(CaptureError::CaptureFailed)
}

/// Capture a screenshot with complete display metrics, origin coordinates, and crop offsets.
pub fn capture_screen_with_details(
    monitor_index: Option<usize>,
    region: Option<&Region>,
    coords: CoordMode,
) -> Result<CapturedScreen, CaptureError> {
    let monitors = get_monitors()?;
    if monitors.is_empty() {
        return Err(CaptureError::NoMonitorsFound);
    }

    let target_idx = match monitor_index {
        Some(idx) => {
            if idx >= monitors.len() {
                return Err(CaptureError::MonitorNotFound(idx, monitors.len()));
            }
            idx
        }
        None => find_primary_monitor_index(&monitors),
    };

    let target_monitor = &monitors[target_idx];
    let scale_factor = target_monitor.scale_factor().unwrap_or(1.0);
    let mon_x = target_monitor.x().unwrap_or(0);
    let mon_y = target_monitor.y().unwrap_or(0);

    debug!(
        "Selected monitor #{}: name={:?}, primary={:?}, scale_factor={}, origin=({}, {})",
        target_idx,
        target_monitor.name(),
        target_monitor.is_primary(),
        scale_factor,
        mon_x,
        mon_y
    );

    // Fast pre-validation: validate crop region against reported monitor bounds before taking full screenshot
    // to avoid unnecessary GPU/desktop capture and 30-50MB frame buffer allocation if out of bounds.
    let resolved_crop = if let Some(reg) = region {
        let width = target_monitor
            .width()
            .map_err(CaptureError::MonitorPropertyFailed)?;
        let height = target_monitor
            .height()
            .map_err(CaptureError::MonitorPropertyFailed)?;
        Some(resolve_crop_region(
            reg,
            coords,
            scale_factor,
            width,
            height,
        )?)
    } else {
        None
    };

    let raw_image = target_monitor
        .capture_image()
        .map_err(CaptureError::CaptureFailed)?;

    let (image, crop_offset) = if let Some((x, y, w, h)) = resolved_crop {
        let actual_w = raw_image.width();
        let actual_h = raw_image.height();
        if x >= actual_w || y >= actual_h || w > actual_w - x || h > actual_h - y {
            return Err(CaptureError::RegionOutOfBounds {
                x: x as i32,
                y: y as i32,
                width: w,
                height: h,
                monitor_w: actual_w,
                monitor_h: actual_h,
            });
        }

        if x == 0 && y == 0 && w == actual_w && h == actual_h {
            (DynamicImage::ImageRgba8(raw_image), (0, 0))
        } else {
            debug!(
                "Cropping image to physical region: x={}, y={}, w={}, h={}",
                x, y, w, h
            );
            let cropped = image::imageops::crop_imm(&raw_image, x, y, w, h).to_image();
            (DynamicImage::ImageRgba8(cropped), (x, y))
        }
    } else {
        (DynamicImage::ImageRgba8(raw_image), (0, 0))
    };

    Ok(CapturedScreen {
        image,
        scale_factor,
        crop_offset,
        monitor_origin: (mon_x, mon_y),
    })
}

/// Capture a screenshot from a specific monitor index (or primary by default),
/// optionally cropping to a specified region according to the chosen coordinate mode.
pub fn capture_screen(
    monitor_index: Option<usize>,
    region: Option<&Region>,
    coords: CoordMode,
) -> Result<DynamicImage, CaptureError> {
    capture_screen_with_details(monitor_index, region, coords).map(|c| c.image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires graphical desktop session / physical monitor"]
    fn test_list_and_capture_primary() {
        let monitors = get_monitors().expect("Should query monitors");
        assert!(!monitors.is_empty());

        let infos = get_monitor_infos().expect("Should query monitor info");
        assert_eq!(infos.len(), monitors.len());

        let primary_img =
            capture_primary_monitor().expect("Capture primary monitor should succeed");
        assert!(primary_img.width() > 0);
        assert!(primary_img.height() > 0);
    }

    #[test]
    #[ignore = "requires graphical desktop session / physical monitor"]
    fn test_capture_screen_with_region() {
        let region = Region {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };

        let captured = capture_screen(Some(0), Some(&region), CoordMode::Logical)
            .expect("Capture with region should succeed");
        assert!(captured.width() >= 100);
        assert!(captured.height() >= 100);
    }

    #[test]
    fn test_capture_screen_invalid_monitor_index() {
        match capture_screen(Some(9999), None, CoordMode::Logical) {
            Err(CaptureError::MonitorNotFound(idx, _)) => assert_eq!(idx, 9999),
            Err(CaptureError::NoMonitorsFound) | Err(CaptureError::ListMonitorsFailed(_)) => {
                // In headless CI environments (e.g. GitHub Actions), no physical displays are connected.
            }
            Ok(_) => panic!("Expected capture_screen to fail with invalid monitor index"),
            Err(other) => panic!("Expected MonitorNotFound or NoMonitorsFound, got: {other:?}"),
        }
    }
}
