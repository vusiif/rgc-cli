use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::capture;
use crate::cli::{CoordMode, ImageFormat, Region};
use crate::doctor;
use crate::mcp::base64::{base64_decode, base64_encode};
use crate::mcp::types::{ToolCallResult, ToolContent, ToolDefinition};
use crate::output::{self, OutputDestination};

pub fn json_tool_result<T: serde::Serialize>(val: &T) -> ToolCallResult {
    match serde_json::to_string_pretty(val) {
        Ok(text) => ToolCallResult {
            content: vec![ToolContent::Text { text }],
            is_error: false,
        },
        Err(e) => ToolCallResult {
            content: vec![ToolContent::Text {
                text: format!("Serialization failed: {e}"),
            }],
            is_error: true,
        },
    }
}

pub fn get_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "list_monitors".to_string(),
            description: "List all active physical displays/monitors with index, dimensions, DPI scale factor, and primary display flag.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "list_windows".to_string(),
            description: "List all active application windows with window ID, title, application/process name, coordinates, dimensions, and minimized state.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "capture_screen".to_string(),
            description: "Capture screen image from a monitor, application window, or rectangular region. Can return image base64 data directly or save to a file path.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "monitor": {
                        "type": "integer",
                        "description": "0-indexed monitor index. If omitted and window is omitted, captures primary display."
                    },
                    "window": {
                        "type": "string",
                        "description": "Optional window ID (e.g. '1234') or window title/app name to capture a specific application window instead of a monitor."
                    },
                    "allMonitors": {
                        "type": "boolean",
                        "description": "Optional flag to capture all connected displays stitched into a unified virtual desktop panorama. Conflicts with 'monitor' and 'window'."
                    },
                    "region": {
                        "type": "string",
                        "description": "Cropping region in 'x,y,w,h' format (e.g. '100,100,800,600')."
                    },
                    "coords": {
                        "type": "string",
                        "enum": ["logical", "physical"],
                        "description": "Coordinate system for cropping region: 'logical' (scaled by display DPI) or 'physical' (raw pixels). Defaults to 'logical'."
                    },
                    "format": {
                        "type": "string",
                        "enum": ["png", "jpeg", "webp"],
                        "description": "Image format ('png', 'jpeg', or 'webp'). Defaults to 'png'."
                    },
                    "scale": {
                        "type": "number",
                        "description": "Optional downscale factor between 0.01 and 1.0 (e.g. 0.5 for half resolution to reduce bandwidth/tokens)."
                    },
                    "maxDimension": {
                        "type": "integer",
                        "description": "Optional maximum width or height in pixels (preserves aspect ratio)."
                    },
                    "quality": {
                        "type": "integer",
                        "description": "Optional JPEG compression quality (1-100, defaults to 85)."
                    },
                    "outputPath": {
                        "type": "string",
                        "description": "Optional file path to write image to disk. If omitted, image is returned as base64."
                    },
                    "returnBase64": {
                        "type": "boolean",
                        "description": "Whether to return image base64 data in the response. Defaults to true if outputPath is omitted."
                    },
                    "redactions": {
                        "type": "array",
                        "description": "Optional list of regions to redact for privacy. Each entry can be an object with 'region' (e.g. '100,100,200,200') and optional 'effect' ('transparent', 'blackout', 'pixelate', 'blur'; default: 'transparent'), or a string in 'x,y,w,h[:effect]' format.",
                        "items": {
                            "oneOf": [
                                { "type": "string" },
                                {
                                    "type": "object",
                                    "properties": {
                                        "region": { "type": "string" },
                                        "effect": { "type": "string", "enum": ["transparent", "blackout", "pixelate", "blur"] }
                                    },
                                    "required": ["region"]
                                }
                            ]
                        }
                    },
                    "maskWindows": {
                        "type": "array",
                        "description": "Optional list of window titles or IDs to redact from screen capture. Each entry can be 'ID|TITLE[:effect]' (e.g. '1Password', 'Terminal:blur', '1234:blackout').",
                        "items": {
                            "type": "string"
                        }
                    },
                    "grid": {
                        "type": "integer",
                        "description": "Optional grid step in pixels (e.g. 100 or 50) to overlay visual grounding grid lines and coordinate anchors for AI agent visual localization."
                    },
                    "diffWith": {
                        "type": "string",
                        "description": "Optional previous image file path or Base64 data to compute visual differences and changed bounding box."
                    },
                    "diffThreshold": {
                        "type": "integer",
                        "description": "Optional pixel difference threshold (0-255, default: 16) for diff calculation to ignore minor noise."
                    },
                    "cropChanged": {
                        "type": "boolean",
                        "description": "Optional flag to crop output image to only the bounding box of changed pixels compared to 'diffWith'."
                    }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "doctor".to_string(),
            description: "Run hardware, display, and environment diagnostics to check screen capture availability.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
    ]
}

pub fn execute_tool_call(name: &str, arguments: &serde_json::Value) -> ToolCallResult {
    match name {
        "list_monitors" => match capture::get_monitor_infos() {
            Ok(monitors) => json_tool_result(&monitors),
            Err(e) => ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to list monitors: {e}"),
                }],
                is_error: true,
            },
        },
        "list_windows" => match capture::get_window_infos() {
            Ok(windows) => json_tool_result(&windows),
            Err(e) => ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to list windows: {e}"),
                }],
                is_error: true,
            },
        },
        "doctor" => {
            let report = doctor::run_doctor();
            json_tool_result(&report)
        }
        "capture_screen" => execute_capture_tool(arguments),
        _ => ToolCallResult {
            content: vec![ToolContent::Text {
                text: format!("Unknown tool name: '{name}'"),
            }],
            is_error: true,
        },
    }
}

pub fn execute_capture_tool(arguments: &serde_json::Value) -> ToolCallResult {
    let monitor = if let Some(m_val) = arguments.get("monitor") {
        if let Some(idx) = m_val.as_u64() {
            Some(idx as usize)
        } else if let Some(s) = m_val.as_str() {
            match s.trim().parse::<usize>() {
                Ok(idx) => Some(idx),
                Err(_) => {
                    return ToolCallResult {
                        content: vec![ToolContent::Text {
                            text: format!(
                                "Invalid monitor index '{s}': expected a non-negative integer"
                            ),
                        }],
                        is_error: true,
                    };
                }
            }
        } else if !m_val.is_null() {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Invalid monitor argument '{m_val}': expected integer index"),
                }],
                is_error: true,
            };
        } else {
            None
        }
    } else {
        None
    };

    let region = if let Some(reg_val) = arguments.get("region") {
        if let Some(reg_str) = reg_val.as_str() {
            match Region::from_str(reg_str) {
                Ok(r) => Some(r),
                Err(e) => {
                    return ToolCallResult {
                        content: vec![ToolContent::Text {
                            text: format!("Invalid region '{reg_str}': {e}"),
                        }],
                        is_error: true,
                    };
                }
            }
        } else if !reg_val.is_null() {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!(
                        "Invalid region parameter '{reg_val}': expected a string in 'x,y,w,h' format (e.g. '100,100,800,600')"
                    ),
                }],
                is_error: true,
            };
        } else {
            None
        }
    } else {
        None
    };

    let coords = match arguments.get("coords").and_then(|v| v.as_str()) {
        Some("physical") => CoordMode::Physical,
        _ => CoordMode::Logical,
    };

    let format = match arguments.get("format").and_then(|v| v.as_str()) {
        Some("jpeg") | Some("jpg") => ImageFormat::Jpeg,
        Some("webp") => ImageFormat::Webp,
        _ => ImageFormat::Png,
    };

    let scale = arguments.get("scale").and_then(|v| v.as_f64());
    if let Some(s) = scale {
        if !s.is_finite() || s <= 0.0 || s > 1.0 {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!(
                        "Invalid scale '{s}': expected a positive number between 0.01 and 1.0"
                    ),
                }],
                is_error: true,
            };
        }
    }

    let max_dimension = arguments
        .get("maxDimension")
        .and_then(|v| v.as_u64())
        .map(|d| d as u32);
    if let Some(d) = max_dimension {
        if d == 0 {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: "Invalid maxDimension: must be greater than 0".to_string(),
                }],
                is_error: true,
            };
        }
    }

    let quality = arguments
        .get("quality")
        .and_then(|v| v.as_u64())
        .map(|q| q as u8);
    if let Some(q) = quality {
        if q == 0 || q > 100 {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: "Invalid quality: must be between 1 and 100".to_string(),
                }],
                is_error: true,
            };
        }
    }

    let output_path = arguments
        .get("outputPath")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);

    let return_base64 = arguments
        .get("returnBase64")
        .and_then(|v| v.as_bool())
        .unwrap_or(output_path.is_none());

    if output_path.is_none() && !return_base64 {
        return ToolCallResult {
            content: vec![ToolContent::Text {
                text: "Invalid capture request: either 'outputPath' must be provided or 'returnBase64' must be true.".to_string(),
            }],
            is_error: true,
        };
    }

    let grid = arguments
        .get("grid")
        .and_then(|v| v.as_u64())
        .map(|g| g as u32);
    let diff_threshold = arguments
        .get("diffThreshold")
        .and_then(|v| v.as_u64())
        .map(|t| t as u8)
        .unwrap_or(16);
    let crop_changed = arguments
        .get("cropChanged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let diff_with_str = arguments.get("diffWith").and_then(|v| v.as_str());

    let mut manual_masks = Vec::new();
    if let Some(redacts_val) = arguments.get("redactions") {
        if let Some(arr) = redacts_val.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    match s.parse::<crate::cli::MaskRegion>() {
                        Ok(mr) => manual_masks.push(mr),
                        Err(e) => {
                            return ToolCallResult {
                                content: vec![ToolContent::Text {
                                    text: format!("Invalid redaction '{s}': {e}"),
                                }],
                                is_error: true,
                            };
                        }
                    }
                } else if let Some(obj) = item.as_object() {
                    let reg_str = match obj.get("region").and_then(|v| v.as_str()) {
                        Some(s) => s,
                        None => {
                            return ToolCallResult {
                                content: vec![ToolContent::Text {
                                    text: "Invalid redaction object: missing 'region' field"
                                        .to_string(),
                                }],
                                is_error: true,
                            };
                        }
                    };
                    let effect = if let Some(eff_str) = obj.get("effect").and_then(|v| v.as_str()) {
                        match eff_str.parse::<crate::cli::MaskEffect>() {
                            Ok(eff) => eff,
                            Err(e) => {
                                return ToolCallResult {
                                    content: vec![ToolContent::Text {
                                        text: format!("Invalid redaction effect '{eff_str}': {e}"),
                                    }],
                                    is_error: true,
                                };
                            }
                        }
                    } else {
                        crate::cli::MaskEffect::Transparent
                    };
                    let region = match Region::from_str(reg_str) {
                        Ok(r) => r,
                        Err(e) => {
                            return ToolCallResult {
                                content: vec![ToolContent::Text {
                                    text: format!("Invalid redaction region '{reg_str}': {e}"),
                                }],
                                is_error: true,
                            };
                        }
                    };
                    manual_masks.push(crate::cli::MaskRegion { region, effect });
                } else {
                    return ToolCallResult {
                        content: vec![ToolContent::Text {
                            text: format!(
                                "Invalid redaction entry '{item}': expected string or object"
                            ),
                        }],
                        is_error: true,
                    };
                }
            }
        }
    }

    let mut window_masks = Vec::new();
    if let Some(win_masks_val) = arguments.get("maskWindows") {
        if let Some(arr) = win_masks_val.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    match s.parse::<crate::cli::MaskWindow>() {
                        Ok(mw) => window_masks.push(mw),
                        Err(e) => {
                            return ToolCallResult {
                                content: vec![ToolContent::Text {
                                    text: format!("Invalid maskWindows entry '{s}': {e}"),
                                }],
                                is_error: true,
                            };
                        }
                    }
                } else if let Some(obj) = item.as_object() {
                    let win_str = match obj.get("window").and_then(|v| v.as_str()) {
                        Some(s) => s,
                        None => {
                            return ToolCallResult {
                                content: vec![ToolContent::Text {
                                    text: "Invalid maskWindows object: missing 'window' field"
                                        .to_string(),
                                }],
                                is_error: true,
                            };
                        }
                    };
                    let effect = if let Some(eff_str) = obj.get("effect").and_then(|v| v.as_str()) {
                        match eff_str.parse::<crate::cli::MaskEffect>() {
                            Ok(eff) => eff,
                            Err(e) => {
                                return ToolCallResult {
                                    content: vec![ToolContent::Text {
                                        text: format!(
                                            "Invalid maskWindows effect '{eff_str}': {e}"
                                        ),
                                    }],
                                    is_error: true,
                                };
                            }
                        }
                    } else {
                        crate::cli::MaskEffect::Transparent
                    };
                    window_masks.push(crate::cli::MaskWindow {
                        query: win_str.to_string(),
                        effect,
                    });
                }
            }
        }
    }

    let all_monitors = arguments
        .get("allMonitors")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let (image, scale_factor, crop_offset, monitor_origin) = if all_monitors {
        match capture::capture_all_monitors_with_details(region.as_ref(), coords) {
            Ok(c) => (
                c.image,
                c.scale_factor,
                c.crop_offset,
                Some(c.monitor_origin),
            ),
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Panorama capture failed: {e}"),
                    }],
                    is_error: true,
                };
            }
        }
    } else if let Some(win_val) = arguments.get("window") {
        let win_query = match win_val.as_str() {
            Some(s) => s,
            None => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: "Invalid 'window' parameter: expected a string window ID or title"
                            .to_string(),
                    }],
                    is_error: true,
                };
            }
        };
        match capture::capture_window_with_details(win_query, region.as_ref()) {
            Ok(c) => (c.image, c.scale_factor, c.crop_offset, None),
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Window capture failed: {e}"),
                    }],
                    is_error: true,
                };
            }
        }
    } else {
        match capture::capture_screen_with_details(monitor, region.as_ref(), coords) {
            Ok(c) => (
                c.image,
                c.scale_factor,
                c.crop_offset,
                Some(c.monitor_origin),
            ),
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Capture failed: {e}"),
                    }],
                    is_error: true,
                };
            }
        }
    };

    let image = if !manual_masks.is_empty() || !window_masks.is_empty() {
        let windows = if !window_masks.is_empty() {
            capture::get_windows().ok()
        } else {
            None
        };
        let redactions = capture::build_redactions(
            image.width(),
            image.height(),
            scale_factor,
            crop_offset,
            monitor_origin,
            coords,
            &manual_masks,
            &window_masks,
            windows.as_deref(),
        );
        output::apply_redactions(image, &redactions)
    } else {
        image
    };

    let mut image = output::apply_image_transforms(image, scale, max_dimension);

    let mut diff_summary = None;
    if let Some(diff_str) = diff_with_str {
        let trimmed = diff_str.trim();
        let ref_image = if trimmed.starts_with("data:image/")
            || (!trimmed.contains('/') && !trimmed.contains('\\') && trimmed.len() > 100)
        {
            let raw_b64 = if let Some((_, rest)) = trimmed.split_once(";base64,") {
                rest
            } else {
                trimmed
            };
            match base64_decode(raw_b64) {
                Ok(bytes) => match image::load_from_memory(&bytes) {
                    Ok(img) => img,
                    Err(e) => {
                        return ToolCallResult {
                            content: vec![ToolContent::Text {
                                text: format!(
                                    "Failed to decode diff reference image from memory: {e}"
                                ),
                            }],
                            is_error: true,
                        };
                    }
                },
                Err(e) => {
                    return ToolCallResult {
                        content: vec![ToolContent::Text {
                            text: format!("Invalid base64 in 'diffWith': {e}"),
                        }],
                        is_error: true,
                    };
                }
            }
        } else {
            match image::open(Path::new(trimmed)) {
                Ok(img) => img,
                Err(e) => {
                    return ToolCallResult {
                        content: vec![ToolContent::Text {
                            text: format!(
                                "Failed to open diff reference image from '{trimmed}': {e}"
                            ),
                        }],
                        is_error: true,
                    };
                }
            }
        };

        let diff = output::calculate_image_diff(&image, &ref_image, diff_threshold);
        if crop_changed {
            if let Some(ref bbox) = diff.bbox {
                image = output::crop_to_diff_bbox(image, bbox, 8);
            }
        }
        diff_summary = serde_json::to_string_pretty(&diff).ok();
    }

    if let Some(grid_step) = grid {
        image = output::apply_grid_overlay(image, grid_step);
    }

    let mut content = Vec::new();

    if let Some(ref diff_json) = diff_summary {
        content.push(ToolContent::Text {
            text: format!("Diff Analysis:\n{diff_json}"),
        });
    }

    if let Some(ref path) = output_path {
        let destination = OutputDestination::File(path.clone());
        let saved_path = match output::save_image(&image, &destination, format, quality) {
            Ok(p) => p,
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Failed to save image to '{}': {e}", path.display()),
                    }],
                    is_error: true,
                };
            }
        };
        content.push(ToolContent::Text {
            text: format!("Image successfully saved to: {}", saved_path.display()),
        });
    }

    if return_base64 {
        let bytes = match output::encode_image_bytes(&image, format, quality) {
            Ok(b) => b,
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Failed to encode image as {format:?}: {e}"),
                    }],
                    is_error: true,
                };
            }
        };

        let b64 = base64_encode(&bytes);
        let mime_type = match format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Webp => "image/webp",
        };

        content.push(ToolContent::Image {
            data: b64,
            mime_type: mime_type.to_string(),
        });
        content.push(ToolContent::Text {
            text: format!(
                "Captured {}x{} image ({:?}, {} bytes)",
                image.width(),
                image.height(),
                format,
                bytes.len()
            ),
        });
    }

    ToolCallResult {
        content,
        is_error: false,
    }
}
