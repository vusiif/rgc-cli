#![forbid(unsafe_code)]

//! Unified coordinate transformation and bidirectional mapping across Screen, Canvas, and Image spaces.
//!
//! Provides deterministic coordinate conversions across:
//! - **ScreenSpace**: OS desktop virtual / monitor physical and logical coordinates.
//! - **CanvasSpace**: Unscaled captured raw pixels (accounts for monitor/window origin and region crop).
//! - **ImageSpace**: Final exported image pixels (accounts for downscaling/maxDimension and diff cropping).

use serde::{Deserialize, Serialize};

/// Detailed metadata describing the coordinate spaces and transformations applied to an image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoordinateSpaceInfo {
    /// Screen desktop origin (X, Y) in OS pixels (e.g. monitor or window top-left).
    pub screen_origin: (i32, i32),
    /// Display DPI scale factor (e.g. 1.0, 1.25, 1.5, 2.0).
    pub screen_scale_factor: f32,
    /// Physical crop offset applied during capture (X, Y) on the screen canvas.
    pub initial_crop_offset: (u32, u32),
    /// Downscale factor applied to image (e.g. 0.5 for half resolution).
    pub image_scale: f64,
    /// Additional crop offset applied after diff detection in scaled image coordinates.
    pub diff_crop_offset: (u32, u32),
    /// Dimensions of raw captured canvas (width, height).
    pub canvas_width: u32,
    pub canvas_height: u32,
    /// Dimensions of final output image (width, height).
    pub output_width: u32,
    pub output_height: u32,
}

/// Bidirectional coordinate transformer tracking coordinate transformations across the pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoordinateTransform {
    pub screen_origin: (i32, i32),
    pub screen_scale_factor: f32,
    pub crop_offset: (u32, u32),
    pub image_scale: f64,
    pub diff_crop_offset: (u32, u32),
    pub canvas_size: (u32, u32),
    pub output_size: (u32, u32),
}

impl CoordinateTransform {
    /// Create a new CoordinateTransform instance with parameter normalization.
    pub fn new(
        screen_origin: (i32, i32),
        screen_scale_factor: f32,
        crop_offset: (u32, u32),
        image_scale: f64,
        diff_crop_offset: (u32, u32),
        canvas_size: (u32, u32),
        output_size: (u32, u32),
    ) -> Self {
        Self {
            screen_origin,
            screen_scale_factor: if screen_scale_factor.is_finite() && screen_scale_factor > 0.0 {
                screen_scale_factor
            } else {
                1.0
            },
            crop_offset,
            image_scale: if image_scale.is_finite() && image_scale > 0.0 {
                image_scale
            } else {
                1.0
            },
            diff_crop_offset,
            canvas_size,
            output_size,
        }
    }

    /// Construct an identity coordinate transform for a canvas/output of given dimensions.
    pub fn identity(size: (u32, u32)) -> Self {
        Self::new((0, 0), 1.0, (0, 0), 1.0, (0, 0), size, size)
    }

    /// Map from OS screen desktop coordinates to unscaled canvas pixels.
    pub fn screen_to_canvas(&self, screen_x: i32, screen_y: i32) -> (i32, i32) {
        let rel_x = screen_x.saturating_sub(self.screen_origin.0);
        let rel_y = screen_y.saturating_sub(self.screen_origin.1);
        let scale = self.screen_scale_factor as f64;
        let cx = (rel_x as f64 * scale).round() as i32 - self.crop_offset.0 as i32;
        let cy = (rel_y as f64 * scale).round() as i32 - self.crop_offset.1 as i32;
        (cx, cy)
    }

    /// Map from unscaled canvas pixels back to OS screen desktop coordinates.
    pub fn canvas_to_screen(&self, canvas_x: i32, canvas_y: i32) -> (i32, i32) {
        let phys_x = canvas_x as f64 + self.crop_offset.0 as f64;
        let phys_y = canvas_y as f64 + self.crop_offset.1 as f64;
        let scale = self.screen_scale_factor as f64;
        let rel_x = (phys_x / scale).round() as i32;
        let rel_y = (phys_y / scale).round() as i32;
        (self.screen_origin.0 + rel_x, self.screen_origin.1 + rel_y)
    }

    /// Map from unscaled canvas pixels to final output image pixels.
    pub fn canvas_to_image(&self, canvas_x: i32, canvas_y: i32) -> (i32, i32) {
        let scaled_x = (canvas_x as f64 * self.image_scale).round() as i32;
        let scaled_y = (canvas_y as f64 * self.image_scale).round() as i32;
        (
            scaled_x - self.diff_crop_offset.0 as i32,
            scaled_y - self.diff_crop_offset.1 as i32,
        )
    }

    /// Map from final output image pixels back to unscaled canvas pixels.
    pub fn image_to_canvas(&self, img_x: u32, img_y: u32) -> (u32, u32) {
        let scaled_x = (img_x + self.diff_crop_offset.0) as f64;
        let scaled_y = (img_y + self.diff_crop_offset.1) as f64;
        let cx = (scaled_x / self.image_scale).round().max(0.0) as u32;
        let cy = (scaled_y / self.image_scale).round().max(0.0) as u32;
        (cx.min(self.canvas_size.0), cy.min(self.canvas_size.1))
    }

    /// Direct mapping from screen coordinates to final output image pixels.
    pub fn screen_to_image(&self, screen_x: i32, screen_y: i32) -> (i32, i32) {
        let (cx, cy) = self.screen_to_canvas(screen_x, screen_y);
        self.canvas_to_image(cx, cy)
    }

    /// Direct mapping from final output image pixels to OS screen desktop coordinates.
    pub fn image_to_screen(&self, img_x: u32, img_y: u32) -> (i32, i32) {
        let (cx, cy) = self.image_to_canvas(img_x, img_y);
        self.canvas_to_screen(cx as i32, cy as i32)
    }

    /// Check if an output image coordinate is within the visible image bounds.
    pub fn is_image_coord_visible(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < self.output_size.0 as i32 && y >= 0 && y < self.output_size.1 as i32
    }

    /// Generate serializable metadata snapshot.
    pub fn to_info(&self) -> CoordinateSpaceInfo {
        CoordinateSpaceInfo {
            screen_origin: self.screen_origin,
            screen_scale_factor: self.screen_scale_factor,
            initial_crop_offset: self.crop_offset,
            image_scale: self.image_scale,
            diff_crop_offset: self.diff_crop_offset,
            canvas_width: self.canvas_size.0,
            canvas_height: self.canvas_size.1,
            output_width: self.output_size.0,
            output_height: self.output_size.1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_coordinate_transform() {
        let transform =
            CoordinateTransform::new((0, 0), 1.0, (0, 0), 1.0, (0, 0), (1920, 1080), (1920, 1080));

        assert_eq!(transform.screen_to_image(500, 300), (500, 300));
        assert_eq!(transform.image_to_screen(500, 300), (500, 300));
        assert!(transform.is_image_coord_visible(500, 300));
        assert!(!transform.is_image_coord_visible(2000, 300));
    }

    #[test]
    fn test_scale_and_crop_transform() {
        // Monitor at (100, 200), DPI scale 1.0, region crop at (50, 50),
        // image scale 0.5, diff crop offset (10, 10)
        let transform = CoordinateTransform::new(
            (100, 200),
            1.0,
            (50, 50),
            0.5,
            (10, 10),
            (1000, 800),
            (490, 390),
        );

        // Screen coordinate (250, 350):
        // Canvas relative to monitor (100, 200) -> (150, 150)
        // Minus crop (50, 50) -> canvas (100, 100)
        assert_eq!(transform.screen_to_canvas(250, 350), (100, 100));

        // Canvas (100, 100) * 0.5 = (50, 50)
        // Minus diff crop (10, 10) -> image (40, 40)
        assert_eq!(transform.canvas_to_image(100, 100), (40, 40));
        assert_eq!(transform.screen_to_image(250, 350), (40, 40));

        // Roundtrip from image (40, 40) back to screen
        assert_eq!(transform.image_to_screen(40, 40), (250, 350));
    }

    #[test]
    fn test_dpi_scale_coordinate_transform() {
        // High-DPI display with scale factor 2.0
        let transform =
            CoordinateTransform::new((0, 0), 2.0, (0, 0), 1.0, (0, 0), (3840, 2160), (3840, 2160));

        // Screen point (100, 100) scaled by 2.0 -> (200, 200) on physical canvas
        assert_eq!(transform.screen_to_canvas(100, 100), (200, 200));
        assert_eq!(transform.screen_to_image(100, 100), (200, 200));
        assert_eq!(transform.image_to_screen(200, 200), (100, 100));
    }

    #[test]
    fn test_coordinate_transform_mixed_dpi_and_offset() {
        // Secondary monitor placed to the right at (1920, 0), DPI scale 1.25,
        // region crop offset (100, 50), image scale 0.8, no diff crop
        let transform = CoordinateTransform::new(
            (1920, 0),
            1.25,
            (100, 50),
            0.8,
            (0, 0),
            (1920, 1080),
            (1536, 864),
        );

        // Screen coordinate (2000, 100):
        // Relative to screen origin -> (80, 100)
        // Scaled by DPI 1.25 -> (100, 125)
        // Minus crop offset (100, 50) -> canvas (0, 75)
        assert_eq!(transform.screen_to_canvas(2000, 100), (0, 75));

        // Canvas (0, 75) * 0.8 = (0, 60)
        assert_eq!(transform.canvas_to_image(0, 75), (0, 60));
        assert_eq!(transform.screen_to_image(2000, 100), (0, 60));

        // Roundtrip from image (0, 60) back to screen
        assert_eq!(transform.image_to_screen(0, 60), (2000, 100));
    }

    #[test]
    fn test_client_only_coordinate_transform_roundtrip() {
        // Window on screen at (100, 100), scale factor 1.0
        // Client area trim has offset (10, 32)
        let transform = CoordinateTransform::new(
            (100, 100),
            1.0,
            (10, 32),
            1.0,
            (0, 0),
            (800, 600),
            (780, 558),
        );

        // Top-left of client area image (0, 0) maps to screen (110, 132)
        assert_eq!(transform.image_to_screen(0, 0), (110, 132));
        assert_eq!(transform.screen_to_image(110, 132), (0, 0));

        // Screen coordinate (200, 200) maps to image (90, 68)
        assert_eq!(transform.screen_to_image(200, 200), (90, 68));
        assert_eq!(transform.image_to_screen(90, 68), (200, 200));

        // Negative screen coordinates (multi-monitor setup with secondary monitor to the left)
        let neg_transform = CoordinateTransform::new(
            (-1920, 100),
            1.5,
            (12, 48), // client area offset in physical pixels (8px, 32px logical)
            1.0,
            (0, 0),
            (1920, 1080),
            (1896, 1020),
        );

        // Client area (0, 0) back to screen:
        // phys_x = 12 / 1.5 = 8 -> -1920 + 8 = -1912
        // phys_y = 48 / 1.5 = 32 -> 100 + 32 = 132
        assert_eq!(neg_transform.image_to_screen(0, 0), (-1912, 132));
        assert_eq!(neg_transform.screen_to_image(-1912, 132), (0, 0));
    }
}
