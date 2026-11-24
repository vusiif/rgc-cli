use std::time::{Duration, Instant};

use image::DynamicImage;
use log::debug;
use xcap::Window;

use crate::capture::screen::get_monitors;
use crate::capture::types::{CaptureError, CapturedWindow, WindowInfo};
use crate::cli::Region;

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

/// Find a window matching the query string:
/// 1. If query is numeric, attempts exact match against window ID.
/// 2. Exact case-insensitive match on window title or application name.
/// 3. Substring case-insensitive match on window title or application name (prioritizing non-minimized windows).
///
/// Single-pass implementation caches titles/app names to minimize Win32 IPC system calls.
pub fn find_window<'a>(windows: &'a [Window], query: &str) -> Result<&'a Window, CaptureError> {
    let query_trimmed = query.trim();
    let numeric_target_id = query_trimmed.parse::<u32>().ok();
    let query_lower = query_trimmed.to_lowercase();

    let mut exact_match = None;
    let mut matching_windows = Vec::new();

    for w in windows {
        // Fast numeric ID match if query was an integer
        if let Some(target_id) = numeric_target_id {
            if let Ok(id) = w.id() {
                if id == target_id {
                    return Ok(w);
                }
            }
        }

        // Cache title and app_name in single pass to avoid redundant Win32 GetWindowTextW IPC calls
        let title = w.title().unwrap_or_default().to_lowercase();
        let app = w.app_name().unwrap_or_default().to_lowercase();

        if exact_match.is_none() && (title == query_lower || app == query_lower) {
            exact_match = Some(w);
            continue;
        }

        if title.contains(&query_lower) || app.contains(&query_lower) {
            matching_windows.push(w);
        }
    }

    if let Some(exact) = exact_match {
        return Ok(exact);
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

/// Compute non-transparent bounding box and client area inset to trim system drop shadows and titlebars.
pub fn calculate_client_area_bounds(
    rgba: &image::RgbaImage,
    scale_factor: f32,
    remove_titlebar: bool,
) -> (u32, u32, u32, u32) {
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 {
        return (0, 0, w, h);
    }

    // 1. Detect non-transparent content boundary using edge-shrink scan
    let stride = (w as usize) * 4;
    let raw = rgba.as_raw();

    // Find top-most row with alpha >= 32
    let mut min_y = h;
    for y in 0..h {
        let row = &raw[(y as usize) * stride..((y as usize) + 1) * stride];
        if row.chunks_exact(4).any(|px| px[3] >= 32) {
            min_y = y;
            break;
        }
    }

    if min_y == h {
        return (0, 0, w, h);
    }

    // Find bottom-most row with alpha >= 32
    let mut max_y = min_y;
    for y in (min_y..h).rev() {
        let row = &raw[(y as usize) * stride..((y as usize) + 1) * stride];
        if row.chunks_exact(4).any(|px| px[3] >= 32) {
            max_y = y;
            break;
        }
    }

    // Find left-most and right-most x across min_y..=max_y
    let mut min_x = w;
    let mut max_x = 0;
    for y in min_y..=max_y {
        let row = &raw[(y as usize) * stride..((y as usize) + 1) * stride];
        // Scan left edge only up to current min_x
        for (x, px) in row.chunks_exact(4).enumerate().take(min_x as usize) {
            if px[3] >= 32 {
                min_x = x as u32;
                break;
            }
        }
        // Scan right edge only down to current max_x
        for (x_offset, px) in row.chunks_exact(4).rev().enumerate() {
            let x = (w - 1).saturating_sub(x_offset as u32);
            if x <= max_x {
                break;
            }
            if px[3] >= 32 {
                max_x = x;
                break;
            }
        }
    }

    let (content_x, content_y, content_w, content_h) = if min_x <= max_x && min_y <= max_y {
        let cw = (max_x - min_x + 1).max(1);
        let ch = (max_y - min_y + 1).max(1);
        (min_x, min_y, cw, ch)
    } else {
        (0, 0, w, h)
    };

    // 2. Optionally remove standard OS window titlebar if requested
    if remove_titlebar {
        // Standard Windows/macOS titlebar height is ~32px scaled
        let titlebar_h = ((32.0 * scale_factor).round() as u32).min(content_h.saturating_sub(40));
        (
            content_x,
            content_y + titlebar_h,
            content_w,
            content_h.saturating_sub(titlebar_h).max(1),
        )
    } else {
        (content_x, content_y, content_w, content_h)
    }
}

/// Essential monitor geometry and DPI scale for multi-monitor window resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub is_primary: bool,
}

/// Determine the most appropriate monitor DPI scale factor for a window given its geometry.
///
/// In heterogeneous multi-monitor setups with mixed DPI (e.g. 1.0x 1080p and 2.0x 4K displays),
/// a window may reside entirely or primarily on a secondary monitor.
/// This algorithm:
/// 1. Converts each monitor's physical bounds to desktop virtual coordinates using its DPI scale.
/// 2. Calculates the 2D intersection area (overlap) between the window rectangle and each monitor.
/// 3. If overlapping, selects the monitor with the largest intersection area.
/// 4. If no monitor overlaps (e.g. 0-dimension window or offscreen coordinate), falls back to
///    checking point containment of the window center.
/// 5. If all geometric checks fail, falls back to the primary monitor, and finally to 1.0.
pub fn find_best_monitor_scale_factor(
    monitors: &[MonitorBounds],
    win_x: i32,
    win_y: i32,
    win_w: u32,
    win_h: u32,
) -> f32 {
    if monitors.is_empty() {
        return 1.0;
    }

    let mut best_overlap_area: u64 = 0;
    let mut best_overlap_scale = None;

    let win_r = win_x.saturating_add(win_w as i32);
    let win_b = win_y.saturating_add(win_h as i32);

    for m in monitors {
        let scale = if m.scale_factor.is_finite() && m.scale_factor > 0.0 {
            m.scale_factor as f64
        } else {
            1.0
        };

        // Convert monitor physical resolution to virtual desktop logical size
        let log_w = (m.width as f64 / scale).round().max(1.0) as i32;
        let log_h = (m.height as f64 / scale).round().max(1.0) as i32;

        let mon_r = m.x.saturating_add(log_w);
        let mon_b = m.y.saturating_add(log_h);

        let overlap_x0 = m.x.max(win_x);
        let overlap_y0 = m.y.max(win_y);
        let overlap_x1 = mon_r.min(win_r);
        let overlap_y1 = mon_b.min(win_b);

        if overlap_x1 > overlap_x0 && overlap_y1 > overlap_y0 {
            let area = (overlap_x1 - overlap_x0) as u64 * (overlap_y1 - overlap_y0) as u64;
            if area > best_overlap_area {
                best_overlap_area = area;
                best_overlap_scale = Some(m.scale_factor);
            }
        }
    }

    if let Some(scale) = best_overlap_scale {
        return if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
    }

    // Fallback 1: Test point containment of window center
    let center_x = win_x + (win_w as i32) / 2;
    let center_y = win_y + (win_h as i32) / 2;

    for m in monitors {
        let scale = if m.scale_factor.is_finite() && m.scale_factor > 0.0 {
            m.scale_factor as f64
        } else {
            1.0
        };
        let log_w = (m.width as f64 / scale).round().max(1.0) as i32;
        let log_h = (m.height as f64 / scale).round().max(1.0) as i32;

        if center_x >= m.x && center_x < m.x + log_w && center_y >= m.y && center_y < m.y + log_h {
            return if m.scale_factor.is_finite() && m.scale_factor > 0.0 {
                m.scale_factor
            } else {
                1.0
            };
        }
    }

    // Fallback 2: Primary monitor
    if let Some(primary) = monitors.iter().find(|m| m.is_primary) {
        if primary.scale_factor.is_finite() && primary.scale_factor > 0.0 {
            return primary.scale_factor;
        }
    }

    // Fallback 3: First monitor or 1.0
    monitors
        .first()
        .map(|m| {
            if m.scale_factor.is_finite() && m.scale_factor > 0.0 {
                m.scale_factor
            } else {
                1.0
            }
        })
        .unwrap_or(1.0)
}

/// Capture screenshot of an application window with optional region crop and client-only area trimming.
pub fn capture_window_with_options(
    query: &str,
    region: Option<&Region>,
    client_only: bool,
) -> Result<CapturedWindow, CaptureError> {
    let windows = get_windows()?;
    let target = find_window(&windows, query)?;

    let is_minimized = target.is_minimized().unwrap_or(false);
    let title = target.title().unwrap_or_else(|_| "Untitled".to_string());
    let id = target.id().unwrap_or(0);

    if is_minimized {
        return Err(CaptureError::WindowMinimized(id, title));
    }

    let win_x = target.x().unwrap_or(0);
    let win_y = target.y().unwrap_or(0);
    let win_w = target.width().unwrap_or(0);
    let win_h = target.height().unwrap_or(0);

    debug!(
        "Capturing window id={}, title={:?}, bounds=({}, {}) {}x{}",
        id, title, win_x, win_y, win_w, win_h
    );
    let raw = target
        .capture_image()
        .map_err(CaptureError::WindowCaptureFailed)?;
    let mut img = DynamicImage::ImageRgba8(raw);

    let scale_factor = get_monitors()
        .ok()
        .map(|monitors| {
            let bounds: Vec<MonitorBounds> = monitors
                .iter()
                .map(|m| MonitorBounds {
                    x: m.x().unwrap_or(0),
                    y: m.y().unwrap_or(0),
                    width: m.width().unwrap_or(0),
                    height: m.height().unwrap_or(0),
                    scale_factor: m.scale_factor().unwrap_or(1.0),
                    is_primary: m.is_primary().unwrap_or(false),
                })
                .collect();
            find_best_monitor_scale_factor(&bounds, win_x, win_y, win_w, win_h)
        })
        .unwrap_or(1.0);

    let mut base_crop_offset = (0u32, 0u32);

    // Apply client-only trimming if requested
    if client_only {
        let rgba = img.to_rgba8();
        let (cx, cy, cw, ch) = calculate_client_area_bounds(&rgba, scale_factor, true);
        if (cx, cy, cw, ch) != (0, 0, rgba.width(), rgba.height()) {
            debug!(
                "Applying client-only trim: x={}, y={}, w={}, h={}",
                cx, cy, cw, ch
            );
            let cropped = image::imageops::crop_imm(&img, cx, cy, cw, ch).to_image();
            img = DynamicImage::ImageRgba8(cropped);
            base_crop_offset = (cx, cy);
        }
    }

    if let Some(r) = region {
        let actual_w = img.width();
        let actual_h = img.height();
        let out_of_bounds = r.x < 0
            || r.y < 0
            || (r.x as u32) >= actual_w
            || (r.y as u32) >= actual_h
            || r.width > actual_w - (r.x as u32)
            || r.height > actual_h - (r.y as u32);
        if out_of_bounds {
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
        debug!(
            "Cropping window image to region: x={}, y={}, w={}, h={}",
            rx, ry, r.width, r.height
        );
        let cropped = image::imageops::crop_imm(&img, rx, ry, r.width, r.height).to_image();
        Ok(CapturedWindow {
            image: DynamicImage::ImageRgba8(cropped),
            scale_factor,
            crop_offset: (base_crop_offset.0 + rx, base_crop_offset.1 + ry),
            window_origin: (win_x, win_y),
        })
    } else {
        Ok(CapturedWindow {
            image: img,
            scale_factor,
            crop_offset: base_crop_offset,
            window_origin: (win_x, win_y),
        })
    }
}

/// Capture screenshot of an application window along with display metrics and crop offsets.
pub fn capture_window_with_details(
    query: &str,
    region: Option<&Region>,
) -> Result<CapturedWindow, CaptureError> {
    capture_window_with_options(query, region, false)
}

/// Wait until an application window matching `query` appears, is not minimized, and can be captured,
/// or until `timeout` expires.
pub fn wait_for_window_with_options(
    query: &str,
    region: Option<&Region>,
    client_only: bool,
    timeout: Duration,
    interval: Duration,
) -> Result<CapturedWindow, CaptureError> {
    let start_time = Instant::now();
    let poll_interval = interval.max(Duration::from_millis(50));

    loop {
        match capture_window_with_options(query, region, client_only) {
            Ok(captured) => return Ok(captured),
            Err(e) => {
                if start_time.elapsed() >= timeout {
                    return Err(e);
                }
            }
        }

        std::thread::sleep(poll_interval);
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
    fn test_wait_for_window_timeout_nonexistent() {
        let err = wait_for_window_with_options(
            "nonexistent_test_window_query_123456",
            None,
            false,
            Duration::from_millis(50),
            Duration::from_millis(10),
        )
        .unwrap_err();
        match err {
            CaptureError::WindowNotFound(_) | CaptureError::ListWindowsFailed(_) => {}
            other => panic!("Expected WindowNotFound or ListWindowsFailed, got: {other:?}"),
        }
    }

    #[test]
    fn test_calculate_client_area_bounds() {
        let mut rgba = image::RgbaImage::new(100, 100);
        // Put transparent outer shadow margin of 10px on all sides
        // Inner content from (10, 10) to (89, 89) -> 80x80
        for y in 10..90 {
            for x in 10..90 {
                rgba.put_pixel(x, y, image::Rgba([200, 200, 200, 255]));
            }
        }

        // Without titlebar removal: trims 10px shadows
        let (cx, cy, cw, ch) = calculate_client_area_bounds(&rgba, 1.0, false);
        assert_eq!((cx, cy, cw, ch), (10, 10, 80, 80));

        // With titlebar removal (32px * 1.0 = 32px):
        let (tx, ty, tw, th) = calculate_client_area_bounds(&rgba, 1.0, true);
        assert_eq!((tx, ty, tw, th), (10, 10 + 32, 80, 80 - 32));
    }

    #[test]
    fn test_find_best_monitor_scale_factor_multi_monitor() {
        // Monitor 1: Primary 1080p at (0, 0), physical 1920x1080, scale 1.0 (logical 1920x1080)
        let m1 = MonitorBounds {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
            is_primary: true,
        };
        // Monitor 2: Secondary 4K at (1920, 0), physical 3840x2160, scale 2.0 (logical 1920x1080: span 1920..3840)
        let m2 = MonitorBounds {
            x: 1920,
            y: 0,
            width: 3840,
            height: 2160,
            scale_factor: 2.0,
            is_primary: false,
        };
        let monitors = vec![m1, m2];

        // 1. Window entirely on Monitor 1
        assert_eq!(
            find_best_monitor_scale_factor(&monitors, 100, 100, 800, 600),
            1.0
        );

        // 2. Window entirely on Monitor 2 (x = 2200, width = 800)
        assert_eq!(
            find_best_monitor_scale_factor(&monitors, 2200, 100, 800, 600),
            2.0
        );

        // 3. Window spanning both monitors: mostly on Monitor 2
        // Window from x = 1800 to 2600 (width = 800):
        // Overlap with Monitor 1: 1800..1920 = 120px
        // Overlap with Monitor 2: 1920..2600 = 680px
        assert_eq!(
            find_best_monitor_scale_factor(&monitors, 1800, 100, 800, 600),
            2.0
        );

        // 4. Window spanning both monitors: mostly on Monitor 1
        // Window from x = 1300 to 2000 (width = 700):
        // Overlap with Monitor 1: 1300..1920 = 620px
        // Overlap with Monitor 2: 1920..2000 = 80px
        assert_eq!(
            find_best_monitor_scale_factor(&monitors, 1300, 100, 700, 600),
            1.0
        );

        // 5. Zero-dimension window at (2500, 500) (inside Monitor 2)
        assert_eq!(
            find_best_monitor_scale_factor(&monitors, 2500, 500, 0, 0),
            2.0
        );

        // 6. Window completely outside all monitors (off-screen) -> falls back to primary monitor
        assert_eq!(
            find_best_monitor_scale_factor(&monitors, -5000, -5000, 100, 100),
            1.0
        );

        // 7. Empty monitors list -> fallback to 1.0
        assert_eq!(find_best_monitor_scale_factor(&[], 100, 100, 800, 600), 1.0);
    }
}
