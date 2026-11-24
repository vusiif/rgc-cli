use std::time::{Duration, Instant};

use image::DynamicImage;

use crate::capture::types::{CaptureError, CapturedScreen, CapturedWindow};
use crate::capture::{
    capture_all_monitors_with_details, capture_screen_with_details, capture_window_with_options,
};
use crate::cli::{CoordMode, Region};

/// Unified capture target specification for CLI, MCP tools, and recordings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget<'a> {
    AllMonitors,
    Window { query: &'a str, client_only: bool },
    Monitor { index: Option<usize> },
}

/// Unified raw capture result returning image and spatial calibration metrics.
#[derive(Debug, Clone)]
pub struct UnifiedCaptured {
    pub image: DynamicImage,
    pub scale_factor: f32,
    pub crop_offset: (u32, u32),
    pub origin: (i32, i32),
}

impl From<CapturedScreen> for UnifiedCaptured {
    fn from(c: CapturedScreen) -> Self {
        Self {
            image: c.image,
            scale_factor: c.scale_factor,
            crop_offset: c.crop_offset,
            origin: c.monitor_origin,
        }
    }
}

impl From<CapturedWindow> for UnifiedCaptured {
    fn from(c: CapturedWindow) -> Self {
        Self {
            image: c.image,
            scale_factor: c.scale_factor,
            crop_offset: c.crop_offset,
            origin: c.window_origin,
        }
    }
}

/// Dispatch capture across all monitors, specific window, or single monitor.
pub fn capture_unified(
    target: CaptureTarget<'_>,
    region: Option<&Region>,
    coords: CoordMode,
) -> Result<UnifiedCaptured, CaptureError> {
    match target {
        CaptureTarget::AllMonitors => {
            capture_all_monitors_with_details(region, coords).map(Into::into)
        }
        CaptureTarget::Window { query, client_only } => {
            capture_window_with_options(query, region, client_only).map(Into::into)
        }
        CaptureTarget::Monitor { index } => {
            capture_screen_with_details(index, region, coords).map(Into::into)
        }
    }
}

/// Wait until a target window appears and can be captured, returning unified coordinates.
pub fn wait_for_window_capture(
    query: &str,
    region: Option<&Region>,
    client_only: bool,
    timeout: Duration,
    interval: Duration,
) -> Result<UnifiedCaptured, CaptureError> {
    crate::capture::window::wait_for_window_with_options(
        query,
        region,
        client_only,
        timeout,
        interval,
    )
    .map(Into::into)
}

/// Wait for a visual change between successive capture frames or until timeout.
///
/// Returns the latest captured frame and whether a significant visual change was detected.
pub fn wait_for_visual_change<F, E>(
    mut capture_fn: F,
    diff_threshold: u8,
    timeout: Duration,
    interval: Duration,
) -> Result<(UnifiedCaptured, bool), E>
where
    F: FnMut() -> Result<UnifiedCaptured, E>,
{
    let mut baseline = capture_fn()?;
    let start_time = Instant::now();
    let poll_interval = interval.max(Duration::from_millis(10));
    let mut detected_change = false;

    while start_time.elapsed() < timeout {
        std::thread::sleep(poll_interval);
        let next_captured = capture_fn()?;
        let diff = crate::transform::calculate_image_diff(
            &next_captured.image,
            &baseline.image,
            diff_threshold,
        );
        if diff.changed && (diff.changed_pixels >= 50 || diff.changed_percent >= 0.05) {
            baseline = next_captured;
            detected_change = true;
            break;
        }
        baseline = next_captured;
    }

    Ok((baseline, detected_change))
}

/// Wait until successive capture frames remain visually stable (no significant changes)
/// for at least `stable_duration`, or until `timeout` expires.
///
/// Returns the latest captured frame and whether stability was successfully achieved.
pub fn wait_for_visual_stability<F, E>(
    mut capture_fn: F,
    diff_threshold: u8,
    stable_duration: Duration,
    timeout: Duration,
    interval: Duration,
) -> Result<(UnifiedCaptured, bool), E>
where
    F: FnMut() -> Result<UnifiedCaptured, E>,
{
    let mut baseline = capture_fn()?;
    let start_time = Instant::now();
    let poll_interval = interval.max(Duration::from_millis(10));
    let mut last_change_time = Instant::now();

    while start_time.elapsed() < timeout {
        std::thread::sleep(poll_interval);
        let next_captured = capture_fn()?;
        let diff = crate::transform::calculate_image_diff(
            &next_captured.image,
            &baseline.image,
            diff_threshold,
        );
        if diff.changed && (diff.changed_pixels >= 50 || diff.changed_percent >= 0.05) {
            last_change_time = Instant::now();
        } else if last_change_time.elapsed() >= stable_duration {
            return Ok((next_captured, true));
        }
        baseline = next_captured;
    }

    Ok((baseline, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    #[test]
    fn test_wait_for_visual_change_detects_diff() {
        let mut count = 0;
        let capture_mock = || {
            count += 1;
            let img = if count == 1 {
                DynamicImage::ImageRgba8(RgbaImage::new(100, 100))
            } else {
                let mut rgba = RgbaImage::new(100, 100);
                for y in 0..10 {
                    for x in 0..10 {
                        rgba.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
                    }
                }
                DynamicImage::ImageRgba8(rgba)
            };
            Ok::<_, String>(UnifiedCaptured {
                image: img,
                scale_factor: 1.0,
                crop_offset: (0, 0),
                origin: (0, 0),
            })
        };

        let (_captured, changed) = wait_for_visual_change(
            capture_mock,
            16,
            Duration::from_millis(500),
            Duration::from_millis(10),
        )
        .unwrap();

        assert!(changed);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_wait_for_visual_change_timeout_unchanged() {
        let capture_mock = || {
            Ok::<_, String>(UnifiedCaptured {
                image: DynamicImage::ImageRgba8(RgbaImage::new(50, 50)),
                scale_factor: 1.0,
                crop_offset: (0, 0),
                origin: (0, 0),
            })
        };

        let (_captured, changed) = wait_for_visual_change(
            capture_mock,
            16,
            Duration::from_millis(50),
            Duration::from_millis(10),
        )
        .unwrap();

        assert!(!changed);
    }

    #[test]
    fn test_wait_for_visual_stability_achieved() {
        let capture_mock = || {
            Ok::<_, String>(UnifiedCaptured {
                image: DynamicImage::ImageRgba8(RgbaImage::new(40, 40)),
                scale_factor: 1.0,
                crop_offset: (0, 0),
                origin: (0, 0),
            })
        };

        let (_captured, stable) = wait_for_visual_stability(
            capture_mock,
            16,
            Duration::from_millis(30),
            Duration::from_millis(200),
            Duration::from_millis(10),
        )
        .unwrap();

        assert!(stable);
    }

    #[test]
    fn test_wait_for_visual_stability_timeout_on_constant_change() {
        let mut frame_id = 0;
        let capture_mock = move || {
            frame_id += 1;
            let mut rgba = RgbaImage::new(40, 40);
            // Constantly change pixels every frame with high delta (0 vs 255)
            let color = if frame_id % 2 == 0 { 255 } else { 0 };
            for y in 0..10 {
                for x in 0..10 {
                    rgba.put_pixel(x, y, image::Rgba([color, color, color, 255]));
                }
            }
            Ok::<_, String>(UnifiedCaptured {
                image: DynamicImage::ImageRgba8(rgba),
                scale_factor: 1.0,
                crop_offset: (0, 0),
                origin: (0, 0),
            })
        };

        let (_captured, stable) = wait_for_visual_stability(
            capture_mock,
            10,
            Duration::from_millis(50),
            Duration::from_millis(70),
            Duration::from_millis(10),
        )
        .unwrap();

        assert!(!stable);
    }
}
