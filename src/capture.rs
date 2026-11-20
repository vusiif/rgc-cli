use image::{DynamicImage, RgbaImage};
use log::debug;
use thiserror::Error;
use xcap::{Monitor, Window};

use crate::cli::{CoordMode, MaskEffect, MaskRegion, MaskWindow, Region};

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
    pub x: i32,
    pub y: i32,
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

/// Screen capture output along with display metrics and crop offsets for downstream processing.
#[derive(Debug)]
pub struct CapturedScreen {
    pub image: DynamicImage,
    pub scale_factor: f32,
    pub crop_offset: (u32, u32),
    pub monitor_origin: (i32, i32),
}

/// Window capture output along with display metrics and crop offsets for downstream processing.
#[derive(Debug)]
pub struct CapturedWindow {
    pub image: DynamicImage,
    pub scale_factor: f32,
    pub crop_offset: (u32, u32),
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

        debug!(
            "Cropping image to physical region: x={}, y={}, w={}, h={}",
            x, y, w, h
        );
        let cropped = image::imageops::crop_imm(&raw_image, x, y, w, h).to_image();
        (DynamicImage::ImageRgba8(cropped), (x, y))
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

    struct MonItem {
        monitor: Monitor,
        x: i32,
        y: i32,
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
        let rel_x = (item.x - min_x) as i64;
        let rel_y = (item.y - min_y) as i64;
        image::imageops::overlay(&mut canvas, &raw, rel_x, rel_y);
    }

    let (image, crop_offset) = if let Some(reg) = region {
        let (crop_x, crop_y, crop_w, crop_h) =
            resolve_crop_region(reg, coords, primary_scale, total_w, total_h)?;
        let cropped = image::imageops::crop_imm(&canvas, crop_x, crop_y, crop_w, crop_h).to_image();
        (DynamicImage::ImageRgba8(cropped), (crop_x, crop_y))
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

/// Capture screenshot of an application window along with display metrics and crop offsets.
pub fn capture_window_with_details(
    query: &str,
    region: Option<&Region>,
) -> Result<CapturedWindow, CaptureError> {
    let img = capture_window(query)?;
    let scale_factor = get_monitors()
        .ok()
        .and_then(|m| m.first().and_then(|mon| mon.scale_factor().ok()))
        .unwrap_or(1.0);

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
        Ok(CapturedWindow {
            image: DynamicImage::ImageRgba8(cropped),
            scale_factor,
            crop_offset: (rx, ry),
        })
    } else {
        Ok(CapturedWindow {
            image: img,
            scale_factor,
            crop_offset: (0, 0),
        })
    }
}

/// Capture screenshot of a window and optionally crop to a specified sub-region.
#[allow(dead_code)]
pub fn capture_window_with_crop(
    query: &str,
    region: Option<&Region>,
) -> Result<DynamicImage, CaptureError> {
    capture_window_with_details(query, region).map(|c| c.image)
}

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
    let y1 = (rel_y + win_h as i32).clamp(0, canvas_w as i32) as u32;

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
    fn test_capture_window_nonexistent() {
        let err = capture_window("nonexistent_test_window_query_123456").unwrap_err();
        match err {
            CaptureError::WindowNotFound(_) | CaptureError::ListWindowsFailed(_) => {}
            other => panic!("Expected WindowNotFound or ListWindowsFailed, got: {other:?}"),
        }
    }

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
