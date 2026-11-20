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
}
