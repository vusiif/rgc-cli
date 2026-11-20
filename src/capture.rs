use image::{DynamicImage, RgbaImage};
use log::debug;
use thiserror::Error;
use xcap::{Monitor, Window};

use crate::cli::{CoordMode, Region};

#[derive(Error, Debug)]
pub enum CaptureError {
    #[error("No monitors found on system")]
    NoMonitorsFound,

    #[error("Monitor index {0} is out of bounds (found {1} monitors)")]
    MonitorNotFound(usize, usize),

    #[error("Failed to list monitors: {0}")]
    ListMonitorsFailed(#[source] xcap::XCapError),

    #[error("Failed to query monitor properties: {0}")]
    MonitorPropertyFailed(#[source] xcap::XCapError),

    #[error("Failed to capture monitor: {0}")]
    CaptureFailed(#[source] xcap::XCapError),

    #[error("Failed to list windows: {0}")]
    ListWindowsFailed(#[source] xcap::XCapError),

    #[error("No window found matching '{0}'")]
    WindowNotFound(String),

    #[error("Cannot capture window #{0} ('{1}') because it is minimized")]
    WindowMinimized(u32, String),

    #[error("Failed to capture window: {0}")]
    WindowCaptureFailed(#[source] xcap::XCapError),

    #[error("Invalid scale factor {0}: scale factor must be a finite, positive number")]
    InvalidScaleFactor(f32),

    #[error(
        "Specified crop region (x={x}, y={y}, w={width}, h={height}) is out of monitor bounds ({monitor_w}x{monitor_h})"
    )]
    RegionOutOfBounds {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        monitor_w: u32,
        monitor_h: u32,
    },
}

impl From<CaptureError> for crate::output::ApiError {
    fn from(err: CaptureError) -> Self {
        use crate::output::{ApiError, ErrorCode};
        match err {
            CaptureError::NoMonitorsFound => {
                ApiError::new(ErrorCode::NoMonitor, "No monitors found on system.")
                    .with_stage("monitor_selection")
                    .with_suggestion("Ensure at least one active display is connected and powered on.")
            }
            CaptureError::MonitorNotFound(req, avail) => ApiError::new(
                ErrorCode::MonitorNotFound,
                format!("Monitor index {req} is out of bounds (found {avail} monitors)."),
            )
            .with_stage("monitor_selection")
            .with_suggestion("Run 'rgc-cli --list-monitors' to see all valid monitor indices."),
            CaptureError::ListMonitorsFailed(e) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Failed to list monitors: {e}"),
            )
            .with_stage("monitor_enumeration")
            .with_suggestion("Verify display server permissions or graphics drivers."),
            CaptureError::MonitorPropertyFailed(e) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Failed to query monitor properties: {e}"),
            )
            .with_stage("monitor_properties")
            .with_suggestion("Verify display driver stability."),
            CaptureError::CaptureFailed(e) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Failed to capture monitor: {e}"),
            )
            .with_stage("screen_capture")
            .with_suggestion("Check display capture permissions in OS privacy settings."),
            CaptureError::ListWindowsFailed(e) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Failed to list windows: {e}"),
            )
            .with_stage("window_enumeration")
            .with_suggestion("Ensure application window server permissions are granted."),
            CaptureError::WindowNotFound(pattern) => ApiError::new(
                ErrorCode::WindowNotFound,
                format!("No window found matching '{pattern}'."),
            )
            .with_stage("window_selection")
            .with_suggestion("Run 'rgc-cli --list-windows' to inspect all active window titles and IDs."),
            CaptureError::WindowMinimized(id, title) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Cannot capture window #{id} ('{title}') because it is minimized."),
            )
            .with_stage("window_capture")
            .with_suggestion("Restore or un-minimize the window before attempting capture."),
            CaptureError::WindowCaptureFailed(e) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Failed to capture window: {e}"),
            )
            .with_stage("window_capture")
            .with_suggestion("Check OS screen/window capture privacy permissions."),
            CaptureError::InvalidScaleFactor(factor) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Invalid scale factor {factor}: must be a finite, positive number."),
            )
            .with_stage("dpi_scaling")
            .with_suggestion("Check OS display scaling settings."),
            CaptureError::RegionOutOfBounds {
                x,
                y,
                width,
                height,
                monitor_w,
                monitor_h,
            } => ApiError::new(
                ErrorCode::RegionOutOfBounds,
                format!(
                    "Specified crop region (x={x}, y={y}, w={width}, h={height}) is out of monitor bounds ({monitor_w}x{monitor_h})."
                ),
            )
            .with_stage("region_validation")
            .with_suggestion("Adjust region coordinates to fit within the selected monitor boundaries."),
        }
    }
}

/// Information about a connected display/monitor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MonitorInfo {
    pub index: usize,
    pub name: String,
    pub is_primary: bool,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
}

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
            width,
            height,
            scale_factor,
        });
    }

    Ok(infos)
}

/// Information about an active application window.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_minimized: bool,
}

/// Retrieve all open desktop windows from the system.
pub fn get_windows() -> Result<Vec<Window>, CaptureError> {
    Window::all().map_err(CaptureError::ListWindowsFailed)
}

/// Retrieve friendly display information for all visible, non-empty desktop windows.
pub fn get_window_infos() -> Result<Vec<WindowInfo>, CaptureError> {
    let windows = get_windows()?;
    let mut infos = Vec::with_capacity(windows.len());

    for w in windows {
        let id = match w.id() {
            Ok(id) => id,
            Err(_) => continue,
        };
        let title = w.title().unwrap_or_default();
        let app_name = w.app_name().unwrap_or_default();
        let width = w.width().unwrap_or(0);
        let height = w.height().unwrap_or(0);
        let x = w.x().unwrap_or(0);
        let y = w.y().unwrap_or(0);
        let is_minimized = w.is_minimized().unwrap_or(false);

        // Filter out zero-sized invisible utility windows or empty system background handles
        if width == 0 || height == 0 || (title.is_empty() && app_name.is_empty()) {
            continue;
        }

        infos.push(WindowInfo {
            id,
            title,
            app_name,
            x,
            y,
            width,
            height,
            is_minimized,
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

/// Resolve crop region from user coordinates (logical or physical) to physical monitor pixels.
/// Employs overflow-safe arithmetic, scale validation, and bounds checking.
pub fn resolve_crop_region(
    region: &Region,
    coords: CoordMode,
    scale_factor: f32,
    monitor_width: u32,
    monitor_height: u32,
) -> Result<(u32, u32, u32, u32), CaptureError> {
    if region.x < 0 || region.y < 0 {
        return Err(CaptureError::RegionOutOfBounds {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
            monitor_w: monitor_width,
            monitor_h: monitor_height,
        });
    }

    let (phys_x, phys_y, phys_w, phys_h) = match coords {
        CoordMode::Physical => (
            region.x as u32,
            region.y as u32,
            region.width,
            region.height,
        ),
        CoordMode::Logical => {
            if !scale_factor.is_finite() || scale_factor <= 0.0 {
                return Err(CaptureError::InvalidScaleFactor(scale_factor));
            }

            let scale = scale_factor as f64;
            let phys_x_f = (region.x as f64 * scale).round();
            let phys_y_f = (region.y as f64 * scale).round();
            let phys_w_f = (region.width as f64 * scale).round();
            let phys_h_f = (region.height as f64 * scale).round();

            if phys_x_f < 0.0
                || phys_y_f < 0.0
                || phys_w_f <= 0.0
                || phys_h_f <= 0.0
                || phys_x_f > u32::MAX as f64
                || phys_y_f > u32::MAX as f64
                || phys_w_f > u32::MAX as f64
                || phys_h_f > u32::MAX as f64
            {
                return Err(CaptureError::RegionOutOfBounds {
                    x: phys_x_f.clamp(i32::MIN as f64, i32::MAX as f64) as i32,
                    y: phys_y_f.clamp(i32::MIN as f64, i32::MAX as f64) as i32,
                    width: phys_w_f.clamp(0.0, u32::MAX as f64) as u32,
                    height: phys_h_f.clamp(0.0, u32::MAX as f64) as u32,
                    monitor_w: monitor_width,
                    monitor_h: monitor_height,
                });
            }

            (
                phys_x_f as u32,
                phys_y_f as u32,
                phys_w_f as u32,
                phys_h_f as u32,
            )
        }
    };

    debug!(
        "Resolved crop region: coords={:?}, input={:?}, scale={} -> physical(x={}, y={}, w={}, h={}) on display({}x{})",
        coords, region, scale_factor, phys_x, phys_y, phys_w, phys_h, monitor_width, monitor_height
    );

    // Overflow-safe bounds check:
    // 1. Starting coordinate must be within monitor dimensions.
    // 2. Width/Height must not exceed the remaining room from the start coordinate.
    // 3. Width/Height must be greater than zero.
    let is_valid = phys_x < monitor_width
        && phys_y < monitor_height
        && phys_w <= monitor_width - phys_x
        && phys_h <= monitor_height - phys_y
        && phys_w > 0
        && phys_h > 0;

    if !is_valid {
        return Err(CaptureError::RegionOutOfBounds {
            x: phys_x as i32,
            y: phys_y as i32,
            width: phys_w,
            height: phys_h,
            monitor_w: monitor_width,
            monitor_h: monitor_height,
        });
    }

    Ok((phys_x, phys_y, phys_w, phys_h))
}

/// Backward compatibility alias for resolve_crop_region in Logical mode.
#[allow(dead_code)]
pub fn logical_to_physical_region(
    region: &Region,
    scale_factor: f32,
    monitor_width: u32,
    monitor_height: u32,
) -> Result<(u32, u32, u32, u32), CaptureError> {
    resolve_crop_region(
        region,
        CoordMode::Logical,
        scale_factor,
        monitor_width,
        monitor_height,
    )
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

/// Capture a screenshot from a specific monitor index (or primary by default),
/// optionally cropping to a specified region according to the chosen coordinate mode.
pub fn capture_screen(
    monitor_index: Option<usize>,
    region: Option<&Region>,
    coords: CoordMode,
) -> Result<DynamicImage, CaptureError> {
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
    debug!(
        "Selected monitor #{}: name={:?}, primary={:?}",
        target_idx,
        target_monitor.name(),
        target_monitor.is_primary()
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
        let scale_factor = target_monitor.scale_factor().unwrap_or(1.0);
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

    if let Some((x, y, w, h)) = resolved_crop {
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

        debug!(
            "Cropping image to physical region: x={}, y={}, w={}, h={}",
            x, y, w, h
        );
        let cropped = image::imageops::crop_imm(&raw_image, x, y, w, h).to_image();
        Ok(DynamicImage::ImageRgba8(cropped))
    } else {
        Ok(DynamicImage::ImageRgba8(raw_image))
    }
}

/// Find a window matching the query string:
/// 1. If query is numeric, attempts exact match against window ID.
/// 2. Exact case-insensitive match on window title or application name.
/// 3. Substring case-insensitive match on window title or application name (prioritizing non-minimized windows).
pub fn find_window<'a>(windows: &'a [Window], query: &str) -> Result<&'a Window, CaptureError> {
    let query_trimmed = query.trim();

    // 1. If numeric, first check if any window has this exact ID
    if let Ok(target_id) = query_trimmed.parse::<u32>() {
        for w in windows {
            if let Ok(id) = w.id() {
                if id == target_id {
                    return Ok(w);
                }
            }
        }
    }

    // 2. Exact match on title or app_name (case-insensitive)
    let query_lower = query_trimmed.to_lowercase();
    for w in windows {
        let title = w.title().unwrap_or_default().to_lowercase();
        let app = w.app_name().unwrap_or_default().to_lowercase();
        if title == query_lower || app == query_lower {
            return Ok(w);
        }
    }

    // 3. Substring match on title or app_name (case-insensitive)
    let mut matching_windows = Vec::new();
    for w in windows {
        let title = w.title().unwrap_or_default().to_lowercase();
        let app = w.app_name().unwrap_or_default().to_lowercase();
        if title.contains(&query_lower) || app.contains(&query_lower) {
            matching_windows.push(w);
        }
    }

    // Prioritize non-minimized window if multiple matched
    if let Some(&first_active) = matching_windows
        .iter()
        .find(|w| !w.is_minimized().unwrap_or(false))
    {
        return Ok(first_active);
    }

    if let Some(&first) = matching_windows.first() {
        return Ok(first);
    }

    Err(CaptureError::WindowNotFound(query.to_string()))
}

/// Capture screenshot of an application window identified by ID or title substring.
pub fn capture_window(query: &str) -> Result<DynamicImage, CaptureError> {
    let windows = get_windows()?;
    let target = find_window(&windows, query)?;

    let is_minimized = target.is_minimized().unwrap_or(false);
    let title = target.title().unwrap_or_else(|_| "Untitled".to_string());
    let id = target.id().unwrap_or(0);

    if is_minimized {
        return Err(CaptureError::WindowMinimized(id, title));
    }

    debug!("Capturing window id={}, title={:?}", id, title);
    let raw = target
        .capture_image()
        .map_err(CaptureError::WindowCaptureFailed)?;
    Ok(DynamicImage::ImageRgba8(raw))
}

/// Capture screenshot of a window and optionally crop to a specified sub-region.
pub fn capture_window_with_crop(
    query: &str,
    region: Option<&Region>,
) -> Result<DynamicImage, CaptureError> {
    let img = capture_window(query)?;
    if let Some(r) = region {
        let actual_w = img.width();
        let actual_h = img.height();
        if r.x < 0 || r.y < 0 {
            return Err(CaptureError::RegionOutOfBounds {
                x: r.x,
                y: r.y,
                width: r.width,
                height: r.height,
                monitor_w: actual_w,
                monitor_h: actual_h,
            });
        }
        let rx = r.x as u32;
        let ry = r.y as u32;
        if rx >= actual_w || ry >= actual_h || r.width > actual_w - rx || r.height > actual_h - ry {
            return Err(CaptureError::RegionOutOfBounds {
                x: r.x,
                y: r.y,
                width: r.width,
                height: r.height,
                monitor_w: actual_w,
                monitor_h: actual_h,
            });
        }
        debug!(
            "Cropping window image to region: x={}, y={}, w={}, h={}",
            rx, ry, r.width, r.height
        );
        let cropped = image::imageops::crop_imm(&img, rx, ry, r.width, r.height).to_image();
        Ok(DynamicImage::ImageRgba8(cropped))
    } else {
        Ok(img)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_window_nonexistent() {
        let err = capture_window("nonexistent_test_window_query_123456").unwrap_err();
        match err {
            CaptureError::WindowNotFound(_) | CaptureError::ListWindowsFailed(_) => {}
            other => panic!("Expected WindowNotFound or ListWindowsFailed, got: {other:?}"),
        }
    }

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

    #[test]
    fn test_resolve_crop_region_dpi_matrix() {
        let region = Region {
            x: 100,
            y: 100,
            width: 200,
            height: 200,
        };

        // 1.0x Scale (standard 1080p display)
        let (x1, y1, w1, h1) =
            resolve_crop_region(&region, CoordMode::Logical, 1.0, 1920, 1080).unwrap();
        assert_eq!((x1, y1, w1, h1), (100, 100, 200, 200));

        // 1.25x Scale (125% DPI)
        let (x125, y125, w125, h125) =
            resolve_crop_region(&region, CoordMode::Logical, 1.25, 1920, 1080).unwrap();
        assert_eq!((x125, y125, w125, h125), (125, 125, 250, 250));

        // 1.5x Scale (150% DPI, e.g. 1440p)
        let (x15, y15, w15, h15) =
            resolve_crop_region(&region, CoordMode::Logical, 1.5, 2560, 1440).unwrap();
        assert_eq!((x15, y15, w15, h15), (150, 150, 300, 300));

        // 2.0x Scale (200% Retina / 4K UHD)
        let (x2, y2, w2, h2) =
            resolve_crop_region(&region, CoordMode::Logical, 2.0, 3840, 2160).unwrap();
        assert_eq!((x2, y2, w2, h2), (200, 200, 400, 400));
    }

    #[test]
    fn test_resolve_crop_region_physical_mode() {
        let region = Region {
            x: 50,
            y: 60,
            width: 800,
            height: 600,
        };

        // In physical mode, scale_factor is ignored and coordinates map 1:1 to pixels
        let (x, y, w, h) =
            resolve_crop_region(&region, CoordMode::Physical, 2.5, 1920, 1080).unwrap();
        assert_eq!((x, y, w, h), (50, 60, 800, 600));

        // Edge boundary in physical mode
        let edge = Region {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let (ex, ey, ew, eh) =
            resolve_crop_region(&edge, CoordMode::Physical, 1.0, 1920, 1080).unwrap();
        assert_eq!((ex, ey, ew, eh), (0, 0, 1920, 1080));

        // 1 pixel out of bounds in physical mode
        let out_edge = Region {
            x: 0,
            y: 0,
            width: 1921,
            height: 1080,
        };
        assert!(resolve_crop_region(&out_edge, CoordMode::Physical, 1.0, 1920, 1080).is_err());
    }

    #[test]
    fn test_logical_to_physical_region() {
        let region = Region {
            x: 100,
            y: 100,
            width: 200,
            height: 200,
        };

        // Scale 1.5x on 1920x1080 display
        let (x, y, w, h) = logical_to_physical_region(&region, 1.5, 1920, 1080).unwrap();
        assert_eq!(x, 150);
        assert_eq!(y, 150);
        assert_eq!(w, 300);
        assert_eq!(h, 300);

        // Edge-aligned boundary: exactly fits monitor (100 + 1820 = 1920, 100 + 980 = 1080)
        let edge_region = Region {
            x: 100,
            y: 100,
            width: 1820,
            height: 980,
        };
        let (ex, ey, ew, eh) = logical_to_physical_region(&edge_region, 1.0, 1920, 1080).unwrap();
        assert_eq!((ex, ey, ew, eh), (100, 100, 1820, 980));

        // 1 pixel out of bounds
        let out_by_one = Region {
            x: 100,
            y: 100,
            width: 1821,
            height: 980,
        };
        assert!(logical_to_physical_region(&out_by_one, 1.0, 1920, 1080).is_err());

        // Arithmetic overflow safety with u32::MAX width
        let overflow_region = Region {
            x: 10,
            y: 10,
            width: u32::MAX,
            height: 100,
        };
        assert!(logical_to_physical_region(&overflow_region, 1.0, 1920, 1080).is_err());

        // Negative coordinates preserve accurate x/y in error
        let neg_region = Region {
            x: -25,
            y: -10,
            width: 100,
            height: 100,
        };
        match logical_to_physical_region(&neg_region, 1.0, 1920, 1080).unwrap_err() {
            CaptureError::RegionOutOfBounds { x, y, .. } => {
                assert_eq!(x, -25);
                assert_eq!(y, -10);
            }
            other => panic!("Expected RegionOutOfBounds, got: {other:?}"),
        }

        // Invalid scale factors (zero, negative, NaN, infinite)
        assert!(logical_to_physical_region(&region, 0.0, 1920, 1080).is_err());
        assert!(logical_to_physical_region(&region, -1.0, 1920, 1080).is_err());
        assert!(logical_to_physical_region(&region, f32::NAN, 1920, 1080).is_err());
        assert!(logical_to_physical_region(&region, f32::INFINITY, 1920, 1080).is_err());
    }
}
