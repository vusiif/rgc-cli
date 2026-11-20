use log::debug;

use crate::capture::types::CaptureError;
use crate::cli::{CoordMode, Region};

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

#[cfg(test)]
mod tests {
    use super::*;

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
