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
}
