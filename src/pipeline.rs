#![forbid(unsafe_code)]

//! Unified image processing and transformation pipeline for CLI and MCP servers.
//!
//! Centralizes all image transform stages into a single deterministic execution order:
//! 1. Cursor resolution and tracking (with optional cursor overlay)
//! 2. Privacy redaction (window/region masking on raw canvas)
//! 3. Downscaling / maxDimension resizing
//! 4. Visual diff comparison and optional diff-bounding-box cropping
//! 5. Diff change highlighting
//! 6. Visual grounding grid overlay
//! 7. Watermark and tag badge overlay
//! 8. Set-of-Mark (SoM) interactive UI element detection & overlay
//! 9. Action verification click markers and drag trajectories
//! 10. CoordinateTransform synthesis with bidirectional mapping across all metadata

use image::{DynamicImage, RgbaImage};

use crate::action::{self, ClickMarker, DragMarker};
use crate::api::{ApiError, CursorInfo, UiElement};
use crate::capture;
use crate::cli::{CoordMode, MaskRegion, MaskWindow};
use crate::coords::{CoordinateSpaceInfo, CoordinateTransform};
use crate::output;
use crate::som;
use crate::transform::{self, DiffResult};

/// Configuration options controlling all processing stages of the pipeline.
#[derive(Debug, Clone, Default)]
pub struct PipelineOptions {
    pub cursor: bool,
    pub highlight_cursor: bool,
    pub manual_masks: Vec<MaskRegion>,
    pub mask_windows: Vec<MaskWindow>,
    pub exclude_windows: Vec<String>,
    pub allow_missing_mask: bool,
    pub coords_mode: CoordMode,
    pub scale: Option<f64>,
    pub max_dimension: Option<u32>,
    pub diff_ref: Option<DynamicImage>,
    pub diff_threshold: u8,
    pub crop_changed: bool,
    pub highlight_diff: bool,
    pub grid: Option<u32>,
    pub timestamp: bool,
    pub tag: Option<String>,
    pub som: bool,
    pub som_threshold: u8,
    pub som_max_elements: usize,
    pub accessibility: bool,
    pub a11y_nodes_override: Option<Vec<crate::accessibility::AccessibilityNode>>,
    pub template_match: Option<crate::template::TemplateMatchOptions>,
    pub clicks: Vec<ClickMarker>,
    pub drags: Vec<DragMarker>,
    /// Optional cursor position override (useful for deterministic tests).
    pub global_cursor_override: Option<(i32, i32)>,
}

/// Output payload from the unified image processing pipeline.
#[derive(Debug)]
pub struct PipelineResult {
    pub image: DynamicImage,
    pub cursor: Option<CursorInfo>,
    pub diff: Option<DiffResult>,
    pub elements: Option<Vec<UiElement>>,
    pub matches: Option<Vec<crate::api::TemplateMatchResult>>,
    pub clicks: Option<Vec<ClickMarker>>,
    pub drags: Option<Vec<DragMarker>>,
    pub transform: CoordinateTransform,
    pub coordinates: CoordinateSpaceInfo,
}

/// Lightweight configuration for processing individual frames in continuous recordings.
#[derive(Debug, Clone, Default)]
pub struct RecordingFrameOptions {
    pub cursor: bool,
    pub highlight_cursor: bool,
    pub manual_masks: Vec<MaskRegion>,
    pub mask_windows: Vec<MaskWindow>,
    pub exclude_windows: Vec<String>,
    pub allow_missing_mask: bool,
    pub coords_mode: CoordMode,
    pub scale: Option<f64>,
    pub max_dimension: Option<u32>,
    pub grid: Option<u32>,
    pub timestamp: bool,
    pub tag: Option<String>,
    pub som: bool,
    pub som_threshold: u8,
    pub som_max_elements: usize,
}

/// Execute the unified capture pipeline on raw captured canvas.
pub fn execute_image_pipeline(
    raw_image: DynamicImage,
    scale_factor: f32,
    crop_offset: (u32, u32),
    monitor_origin: Option<(i32, i32)>,
    options: &PipelineOptions,
) -> Result<PipelineResult, ApiError> {
    let canvas_w = raw_image.width();
    let canvas_h = raw_image.height();
    let origin = monitor_origin.unwrap_or((0, 0));

    // Stage 1: Resolve mouse cursor on unscaled raw canvas
    let global_cursor = options
        .global_cursor_override
        .or_else(capture::get_global_cursor_position);

    let raw_canvas_cursor = capture::resolve_cursor_on_canvas(
        global_cursor,
        origin,
        scale_factor,
        crop_offset,
        (canvas_w, canvas_h),
    );

    let mut rgba = raw_image.into_rgba8();

    // Draw cursor overlay on raw canvas before scaling (ensures sharp cursor glyph)
    if (options.cursor || options.highlight_cursor) && raw_canvas_cursor.is_some() {
        if let Some(ref c) = raw_canvas_cursor {
            transform::draw_cursor_overlay_in_place(&mut rgba, c.x, c.y, options.highlight_cursor);
        }
    }

    // Stage 2: Privacy redaction masks (blackout, transparent, pixelate, blur, and excluded windows)
    let mut combined_mask_windows = options.mask_windows.clone();
    for query in &options.exclude_windows {
        combined_mask_windows.push(MaskWindow {
            query: query.clone(),
            effect: crate::cli::MaskEffect::Blackout,
        });
    }

    if !options.manual_masks.is_empty() || !combined_mask_windows.is_empty() {
        let windows = if !combined_mask_windows.is_empty() {
            capture::get_windows().ok()
        } else {
            None
        };
        let redactions = capture::build_redactions(
            rgba.width(),
            rgba.height(),
            scale_factor,
            crop_offset,
            monitor_origin,
            options.coords_mode,
            &options.manual_masks,
            &combined_mask_windows,
            windows.as_deref(),
            options.allow_missing_mask,
        )?;
        for &(x, y, w, h, effect) in &redactions {
            transform::apply_redaction(&mut rgba, x, y, w, h, effect);
        }
    }

    // Stage 3: Image scaling & max_dimension constraint
    let mut current_img = output::apply_image_transforms(
        DynamicImage::ImageRgba8(rgba),
        options.scale,
        options.max_dimension,
    );

    let actual_scale = if canvas_w > 0 {
        current_img.width() as f64 / canvas_w as f64
    } else {
        1.0
    };

    // Stage 4: Visual diff comparison against reference image
    let mut diff_crop_offset = (0u32, 0u32);
    let diff_result = if let Some(ref ref_image) = options.diff_ref {
        let diff = output::calculate_image_diff(&current_img, ref_image, options.diff_threshold);

        if options.crop_changed {
            if let Some(ref bbox) = diff.bbox {
                let padding = 8u32;
                diff_crop_offset = (
                    bbox.x.saturating_sub(padding),
                    bbox.y.saturating_sub(padding),
                );
                current_img = output::crop_to_diff_bbox(current_img, bbox, padding);
            }
        }

        Some(diff)
    } else {
        None
    };

    let mut rgba = current_img.into_rgba8();

    // Stage 5: Highlight diff bounding box
    if options.highlight_diff {
        if let Some(ref diff) = diff_result {
            if let Some(ref bbox) = diff.bbox {
                let local_bbox = transform::DiffBBox {
                    x: bbox.x.saturating_sub(diff_crop_offset.0),
                    y: bbox.y.saturating_sub(diff_crop_offset.1),
                    width: bbox.width,
                    height: bbox.height,
                };
                output::highlight_diff_bbox_in_place(&mut rgba, &local_bbox, 2, None);
            }
        }
    }

    // Stage 6: Visual grounding grid overlay
    if let Some(grid_step) = options.grid {
        output::apply_grid_overlay_in_place(&mut rgba, grid_step);
    }

    // Stage 7: Watermark and tag badge overlay
    if options.timestamp || options.tag.is_some() {
        let ts_str = if options.timestamp {
            Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
        } else {
            None
        };
        output::apply_watermark_and_tag_in_place(
            &mut rgba,
            ts_str.as_deref(),
            options.tag.as_deref(),
            output::WatermarkPosition::BottomRight,
        );
    }

    // Construct final CoordinateTransform
    let transform = CoordinateTransform::new(
        origin,
        scale_factor,
        crop_offset,
        actual_scale,
        diff_crop_offset,
        (canvas_w, canvas_h),
        (rgba.width(), rgba.height()),
    );

    // Stage 8: Set-of-Mark (SoM) UI element detection, accessibility fusion & overlay
    let detected_elements = if options.som {
        let mut elements = som::detect_ui_elements_from_rgba(
            &rgba,
            options.som_threshold,
            options.som_max_elements,
        );

        // Enrich each detected UI element with exact global OS desktop coordinates
        for elem in &mut elements {
            let (screen_cx, screen_cy) = transform.image_to_screen(elem.center_x, elem.center_y);
            let (screen_tl_x, screen_tl_y) = transform.image_to_screen(elem.x, elem.y);
            elem.screen_center_x = Some(screen_cx);
            elem.screen_center_y = Some(screen_cy);
            elem.screen_x = Some(screen_tl_x);
            elem.screen_y = Some(screen_tl_y);
        }

        // Stage 8b: Spatial fusion with OS accessibility semantics
        if options.accessibility {
            let a11y_nodes = if let Some(ref override_nodes) = options.a11y_nodes_override {
                override_nodes.clone()
            } else {
                let scale = if transform.screen_scale_factor.is_finite()
                    && transform.screen_scale_factor > 0.0
                {
                    transform.screen_scale_factor as f64
                } else {
                    1.0
                };
                let (screen_w, screen_h) = (
                    (canvas_w as f64 / scale).round() as u32,
                    (canvas_h as f64 / scale).round() as u32,
                );
                crate::accessibility::query_system_accessibility_nodes(Some((
                    origin.0, origin.1, screen_w, screen_h,
                )))
            };

            crate::accessibility::fuse_accessibility_and_vision(
                &mut elements,
                &a11y_nodes,
                &transform,
                true,
            );
        }

        som::draw_som_overlay_in_place(&mut rgba, &elements);
        Some(elements)
    } else {
        None
    };

    // Stage 8c: Visual template/icon matching
    let template_matches = if let Some(ref tpl_opts) = options.template_match {
        let mut matches = crate::template::find_template_matches(
            &rgba,
            &tpl_opts.template,
            tpl_opts.threshold,
            tpl_opts.max_matches,
        );
        crate::template::enrich_matches_with_coordinates(&mut matches, &transform);

        if tpl_opts.highlight {
            crate::template::draw_template_matches_in_place(&mut rgba, &matches);
        }

        Some(matches)
    } else {
        None
    };

    // Stage 9: Action markers (clicks and drags)
    let clicks = if !options.clicks.is_empty() {
        Some(options.clicks.clone())
    } else {
        None
    };

    let drags = if !options.drags.is_empty() {
        Some(options.drags.clone())
    } else {
        None
    };

    if !options.clicks.is_empty() || !options.drags.is_empty() {
        action::draw_action_overlays_in_place(&mut rgba, &options.clicks, &options.drags);
    }

    // Stage 10: Compute final CursorInfo mapped through CoordinateTransform
    let final_cursor = if let Some((gx, gy)) = global_cursor {
        let (img_x, img_y) = transform.screen_to_image(gx, gy);
        let visible = transform.is_image_coord_visible(img_x, img_y);
        Some(CursorInfo {
            x: img_x,
            y: img_y,
            visible,
            screen_x: Some(gx),
            screen_y: Some(gy),
            canvas_x: raw_canvas_cursor.as_ref().map(|c| c.x),
            canvas_y: raw_canvas_cursor.as_ref().map(|c| c.y),
        })
    } else {
        None
    };

    let coordinates_info = transform.to_info();

    Ok(PipelineResult {
        image: DynamicImage::ImageRgba8(rgba),
        cursor: final_cursor,
        diff: diff_result,
        elements: detected_elements,
        matches: template_matches,
        clicks,
        drags,
        transform,
        coordinates: coordinates_info,
    })
}

/// Process a single recording frame through cursor, redaction, scaling, grid, watermark, and SoM.
pub fn process_recording_frame(
    raw_rgba: &mut RgbaImage,
    scale_factor: f32,
    crop_offset: (u32, u32),
    monitor_origin: Option<(i32, i32)>,
    options: &RecordingFrameOptions,
) -> Result<DynamicImage, ApiError> {
    let canvas_w = raw_rgba.width();
    let canvas_h = raw_rgba.height();
    let origin = monitor_origin.unwrap_or((0, 0));

    // 1. Cursor overlay
    if options.cursor || options.highlight_cursor {
        let global_cursor = capture::get_global_cursor_position();
        if let Some(cursor_info) = capture::resolve_cursor_on_canvas(
            global_cursor,
            origin,
            scale_factor,
            crop_offset,
            (canvas_w, canvas_h),
        ) {
            transform::draw_cursor_overlay_in_place(
                raw_rgba,
                cursor_info.x,
                cursor_info.y,
                options.highlight_cursor,
            );
        }
    }

    // 2. Privacy redaction masks (blackout, transparent, pixelate, blur, and excluded windows)
    let mut combined_mask_windows = options.mask_windows.clone();
    for query in &options.exclude_windows {
        combined_mask_windows.push(MaskWindow {
            query: query.clone(),
            effect: crate::cli::MaskEffect::Blackout,
        });
    }

    if !options.manual_masks.is_empty() || !combined_mask_windows.is_empty() {
        let windows = if !combined_mask_windows.is_empty() {
            capture::get_windows().ok()
        } else {
            None
        };
        let redactions = capture::build_redactions(
            raw_rgba.width(),
            raw_rgba.height(),
            scale_factor,
            crop_offset,
            monitor_origin,
            options.coords_mode,
            &options.manual_masks,
            &combined_mask_windows,
            windows.as_deref(),
            options.allow_missing_mask,
        )?;
        for &(x, y, w, h, effect) in &redactions {
            transform::apply_redaction(raw_rgba, x, y, w, h, effect);
        }
    }

    // 3. Scaling / Max dimension (zero-copy buffer ownership transfer)
    let mut rgba = if options.scale.is_some() || options.max_dimension.is_some() {
        let frame_img = output::apply_image_transforms(
            DynamicImage::ImageRgba8(std::mem::take(raw_rgba)),
            options.scale,
            options.max_dimension,
        );
        frame_img.into_rgba8()
    } else {
        std::mem::take(raw_rgba)
    };

    // 4. Grid overlay
    if let Some(grid_step) = options.grid {
        output::apply_grid_overlay_in_place(&mut rgba, grid_step);
    }

    // 5. Watermark and tag badge
    if options.timestamp || options.tag.is_some() {
        let ts_str = if options.timestamp {
            Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
        } else {
            None
        };
        output::apply_watermark_and_tag_in_place(
            &mut rgba,
            ts_str.as_deref(),
            options.tag.as_deref(),
            output::WatermarkPosition::BottomRight,
        );
    }

    // 6. Set-of-Mark (SoM)
    if options.som {
        let elements = som::detect_ui_elements_from_rgba(
            &rgba,
            options.som_threshold,
            options.som_max_elements,
        );
        som::draw_som_overlay_in_place(&mut rgba, &elements);
    }

    Ok(DynamicImage::ImageRgba8(rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_image_pipeline_basic() {
        let raw = DynamicImage::new_rgba8(200, 200);
        let options = PipelineOptions::default();

        let result = execute_image_pipeline(raw, 1.0, (0, 0), Some((0, 0)), &options)
            .expect("Pipeline execution should succeed");

        assert_eq!(result.image.width(), 200);
        assert_eq!(result.image.height(), 200);
        assert_eq!(result.coordinates.canvas_width, 200);
        assert_eq!(result.coordinates.output_width, 200);
    }

    #[test]
    fn test_execute_image_pipeline_with_scaling_and_cursor() {
        let raw = DynamicImage::new_rgba8(400, 400);
        let options = PipelineOptions {
            scale: Some(0.5),
            global_cursor_override: Some((100, 100)),
            ..Default::default()
        };

        let result = execute_image_pipeline(raw, 1.0, (0, 0), Some((0, 0)), &options)
            .expect("Pipeline execution with scale should succeed");

        assert_eq!(result.image.width(), 200);
        assert_eq!(result.image.height(), 200);

        let cursor = result.cursor.expect("Cursor info should be present");
        assert_eq!(cursor.screen_x, Some(100));
        assert_eq!(cursor.screen_y, Some(100));
        assert_eq!(cursor.canvas_x, Some(100));
        assert_eq!(cursor.canvas_y, Some(100));
        // On 0.5 scaled image, (100, 100) becomes (50, 50)
        assert_eq!(cursor.x, 50);
        assert_eq!(cursor.y, 50);
        assert!(cursor.visible);
    }

    #[test]
    fn test_execute_image_pipeline_som_screen_coords() {
        let mut raw = DynamicImage::new_rgba8(300, 300).into_rgba8();
        // Draw a simulated button on raw canvas
        for y in 40..80 {
            for x in 40..120 {
                raw.put_pixel(x, y, image::Rgba([200, 200, 200, 255]));
            }
        }

        let options = PipelineOptions {
            som: true,
            som_threshold: 10,
            scale: Some(0.5),
            ..Default::default()
        };

        let result = execute_image_pipeline(
            DynamicImage::ImageRgba8(raw),
            1.0,
            (0, 0),
            Some((100, 200)),
            &options,
        )
        .expect("Pipeline with SoM should succeed");

        if let Some(elements) = result.elements {
            if let Some(first) = elements.first() {
                assert!(first.screen_center_x.is_some());
                assert!(first.screen_center_y.is_some());
                assert!(first.screen_x.is_some());
                assert!(first.screen_y.is_some());
            }
        }
    }

    #[test]
    fn test_execute_image_pipeline_diff_crop_and_markers() {
        let raw = DynamicImage::new_rgba8(200, 200);
        let mut ref_rgba = raw.to_rgba8();
        // Modify a 20x20 region at (80, 80)
        for y in 80..100 {
            for x in 80..100 {
                ref_rgba.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }

        let options = PipelineOptions {
            diff_ref: Some(DynamicImage::ImageRgba8(ref_rgba)),
            diff_threshold: 10,
            crop_changed: true,
            highlight_diff: true,
            clicks: vec![ClickMarker {
                x: 10,
                y: 10,
                label: Some("Test".to_string()),
            }],
            ..Default::default()
        };

        let result = execute_image_pipeline(raw, 1.0, (0, 0), Some((0, 0)), &options)
            .expect("Diff pipeline should succeed");

        assert!(result.diff.is_some());
        let diff = result.diff.unwrap();
        assert!(diff.changed);
        // Diff bbox was padded by 8, so (80 - 8, 80 - 8) = (72, 72)
        assert_eq!(result.coordinates.diff_crop_offset, (72, 72));
        assert!(result.clicks.is_some());
    }

    #[test]
    fn test_execute_image_pipeline_accessibility_fusion() {
        let raw = DynamicImage::new_rgba8(400, 300);
        let options = PipelineOptions {
            som: true,
            accessibility: true,
            a11y_nodes_override: Some(vec![crate::accessibility::AccessibilityNode::new(
                50,
                50,
                100,
                40,
                "Button",
                Some("OK".to_string()),
                true,
            )]),
            ..Default::default()
        };

        let result = execute_image_pipeline(raw, 1.0, (0, 0), Some((0, 0)), &options)
            .expect("Accessibility pipeline should succeed");

        assert!(result.elements.is_some());
        let elems = result.elements.unwrap();
        // Since the canvas is blank, vision finds nothing, but a11y injects the interactive Button!
        assert!(!elems.is_empty());
        let injected = elems
            .iter()
            .find(|e| e.role.as_deref() == Some("Button"))
            .unwrap();
        assert_eq!(injected.label.as_deref(), Some("OK"));
        assert_eq!(injected.source.as_deref(), Some("accessibility"));
    }

    #[test]
    fn test_execute_image_pipeline_template_matching() {
        let mut raw_rgba = image::RgbaImage::new(200, 200);
        let mut icon = image::RgbaImage::new(12, 12);
        for (x, y, p) in icon.enumerate_pixels_mut() {
            *p = image::Rgba([100, 150, (x * 20) as u8, 255]);
            raw_rgba.put_pixel(60 + x, 40 + y, *p);
        }

        let options = PipelineOptions {
            template_match: Some(crate::template::TemplateMatchOptions {
                template: DynamicImage::ImageRgba8(icon),
                threshold: 0.90,
                max_matches: 1,
                highlight: true,
            }),
            ..Default::default()
        };

        let result = execute_image_pipeline(
            DynamicImage::ImageRgba8(raw_rgba),
            1.0,
            (0, 0),
            Some((100, 200)),
            &options,
        )
        .expect("Template match pipeline should succeed");

        assert!(result.matches.is_some());
        let matches = result.matches.unwrap();
        assert_eq!(matches.len(), 1);
        let m = &matches[0];
        assert_eq!(m.x, 60);
        assert_eq!(m.y, 40);
        // Screen center: origin (100, 200) + center (60 + 6, 40 + 6) = (166, 246)
        assert_eq!(m.screen_center_x, Some(166));
        assert_eq!(m.screen_center_y, Some(246));
    }

    #[test]
    fn test_process_recording_frame_pipeline() {
        let mut raw_rgba = DynamicImage::new_rgba8(200, 200).into_rgba8();
        let options = RecordingFrameOptions {
            scale: Some(0.5),
            grid: Some(20),
            ..Default::default()
        };

        let processed = process_recording_frame(&mut raw_rgba, 1.0, (0, 0), None, &options)
            .expect("Frame processing should succeed");

        assert_eq!(processed.width(), 100);
        assert_eq!(processed.height(), 100);
    }
}
