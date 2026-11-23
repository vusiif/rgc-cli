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

    // 1. Detect non-transparent content boundary (ignores DWM translucent drop shadows)
    let mut min_x = w;
    let mut max_x = 0;
    let mut min_y = h;
    let mut max_y = 0;

    let raw = rgba.as_raw();
    for y in 0..h {
        let row_offset = (y as usize) * (w as usize) * 4;
        for x in 0..w {
            let a = raw[row_offset + (x as usize) * 4 + 3];
            // Alpha threshold 32 ignores semi-transparent blurred shadow falloff
            if a >= 32 {
                if x < min_x { min_x = x; }
                if x > max_x { max_x = x; }
                if y < min_y { min_y = y; }
                if y > max_y { max_y = y; }
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
        (content_x, content_y + titlebar_h, content_w, content_h.saturating_sub(titlebar_h).max(1))
    } else {
        (content_x, content_y, content_w, content_h)
    }
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

    let mut win_x = target.x().unwrap_or(0);
    let mut win_y = target.y().unwrap_or(0);

    debug!(
        "Capturing window id={}, title={:?}, origin=({}, {})",
        id, title, win_x, win_y
    );
    let raw = target
        .capture_image()
        .map_err(CaptureError::WindowCaptureFailed)?;
    let mut img = DynamicImage::ImageRgba8(raw);

    let scale_factor = get_monitors()
        .ok()
        .and_then(|m| m.first().and_then(|mon| mon.scale_factor().ok()))
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
            win_x += cx as i32;
            win_y += cy as i32;
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
}
