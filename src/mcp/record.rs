use crate::capture;
use crate::cli::{CoordMode, MaskRegion, MaskWindow};
use crate::mcp::base64::base64_encode;
use crate::mcp::types::{ToolCallResult, ToolContent, ToolDefinition};
use crate::output;

/// Returns the MCP tool definition for `record_screen`.
pub fn get_record_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "record_screen".to_string(),
        description: "Continuously record screen frames over a specified duration and encode them into an animated GIF or sequence of images.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "durationSecs": {
                    "type": "number",
                    "description": "Duration in seconds to record (e.g. 1.0 to 30.0)."
                },
                "fps": {
                    "type": "integer",
                    "description": "Sampling framerate (1 to 30 FPS, default: 5)."
                },
                "monitor": {
                    "type": "integer",
                    "description": "0-indexed monitor index. If omitted and window is omitted, records primary display."
                },
                "window": {
                    "type": "string",
                    "description": "Optional window ID or title substring to record."
                },
                "allMonitors": {
                    "type": "boolean",
                    "description": "Optional flag to record multi-monitor panorama."
                },
                "cursor": {
                    "type": "boolean",
                    "description": "Optional flag to capture and render mouse cursor."
                },
                "highlightCursor": {
                    "type": "boolean",
                    "description": "Optional flag to highlight mouse cursor with amber focus ring."
                },
                "outputPath": {
                    "type": "string",
                    "description": "Optional output path for animated GIF file."
                },
                "returnBase64": {
                    "type": "boolean",
                    "description": "Whether to return the animated GIF base64 data in the response. Defaults to true if outputPath is omitted."
                },
                "scale": {
                    "type": "number",
                    "description": "Optional downscale factor between 0.01 and 1.0."
                },
                "maxDimension": {
                    "type": "integer",
                    "description": "Optional maximum dimension in pixels."
                },
                "redactions": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional privacy redaction regions: 'x,y,w,h[:effect]' (effect: transparent, blackout, pixelate, blur)."
                },
                "maskWindows": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional window titles or IDs to redact from recorded frames."
                }
            },
            "required": ["durationSecs"],
            "additionalProperties": false
        }),
    }
}

/// Execute the `record_screen` MCP tool, sampling consecutive frames and encoding them as an animated GIF.
pub fn execute_record_tool(arguments: &serde_json::Value) -> ToolCallResult {
    let duration_secs = match arguments.get("durationSecs").and_then(|v| v.as_f64()) {
        Some(d) if d.is_finite() && d > 0.0 && d <= 3600.0 => d as f32,
        _ => {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: "Missing or invalid 'durationSecs' parameter: must be a finite positive number <= 3600.0"
                        .to_string(),
                }],
                is_error: true,
            };
        }
    };

    let fps = arguments
        .get("fps")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 30) as u32;
    let monitor = arguments
        .get("monitor")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let all_monitors = arguments
        .get("allMonitors")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let window_query = arguments.get("window").and_then(|v| v.as_str());
    let cursor_flag = arguments
        .get("cursor")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let highlight_cursor_flag = arguments
        .get("highlightCursor")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let scale = arguments.get("scale").and_then(|v| v.as_f64());
    let max_dimension = arguments
        .get("maxDimension")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let allow_overwrite = arguments
        .get("allowOverwrite")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let output_path = match arguments.get("outputPath").and_then(|v| v.as_str()) {
        Some(s) => match crate::mcp::tools::validate_mcp_output_path(s, &["gif"], allow_overwrite) {
            Ok(pb) => Some(pb),
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Invalid outputPath: {e}"),
                    }],
                    is_error: true,
                };
            }
        },
        None => None,
    };
    let return_base64 = arguments
        .get("returnBase64")
        .and_then(|v| v.as_bool())
        .unwrap_or(output_path.is_none());

    let mut manual_masks = Vec::new();
    if let Some(redacts_val) = arguments.get("redactions") {
        if let Some(arr) = redacts_val.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    match s.parse::<MaskRegion>() {
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
                }
            }
        }
    }

    let mut window_masks = Vec::new();
    if let Some(win_masks_val) = arguments.get("maskWindows") {
        if let Some(arr) = win_masks_val.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    match s.parse::<MaskWindow>() {
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
                }
            }
        }
    }

    let interval = std::time::Duration::from_secs_f32(1.0 / fps as f32);
    let target_duration = std::time::Duration::from_secs_f32(duration_secs.clamp(0.1, 30.0));
    let start_time = std::time::Instant::now();

    let mut gif_bytes = Vec::new();
    let mut stream = match output::GifStreamEncoder::new(&mut gif_bytes, fps) {
        Ok(s) => s,
        Err(e) => {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to initialize streaming GIF encoder: {e}"),
                }],
                is_error: true,
            };
        }
    };

    let frame_options = crate::pipeline::RecordingFrameOptions {
        cursor: cursor_flag,
        highlight_cursor: highlight_cursor_flag,
        manual_masks,
        mask_windows: window_masks,
        allow_missing_mask: false,
        coords_mode: CoordMode::Logical,
        scale,
        max_dimension,
        grid: None,
        timestamp: false,
        tag: None,
        som: false,
        som_threshold: 25,
        som_max_elements: 50,
    };

    while start_time.elapsed() < target_duration {
        let frame_start = std::time::Instant::now();
        let captured = if all_monitors {
            capture::capture_all_monitors_with_details(None, CoordMode::Logical).map(|c| {
                (
                    c.image,
                    c.scale_factor,
                    c.crop_offset,
                    Some(c.monitor_origin),
                )
            })
        } else if let Some(win) = window_query {
            capture::capture_window_with_details(win, None).map(|c| {
                (
                    c.image,
                    c.scale_factor,
                    c.crop_offset,
                    Some(c.window_origin),
                )
            })
        } else {
            capture::capture_screen_with_details(monitor, None, CoordMode::Logical).map(|c| {
                (
                    c.image,
                    c.scale_factor,
                    c.crop_offset,
                    Some(c.monitor_origin),
                )
            })
        };

        let (frame_img, scale_factor, crop_offset, origin) = match captured {
            Ok(c) => c,
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Recording frame capture failed: {e}"),
                    }],
                    is_error: true,
                };
            }
        };

        let mut rgba = frame_img.into_rgba8();
        let frame_img = match crate::pipeline::process_recording_frame(
            &mut rgba,
            scale_factor,
            crop_offset,
            origin,
            &frame_options,
        ) {
            Ok(img) => img,
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Privacy redaction failed in recording: {}", e.message),
                    }],
                    is_error: true,
                };
            }
        };
        let rgba = frame_img.into_rgba8();

        if let Err(e) = stream.encode_frame(&rgba) {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to encode streaming GIF frame: {e}"),
                }],
                is_error: true,
            };
        }

        let elapsed = frame_start.elapsed();
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }
    }

    if stream.frame_count() == 0 {
        return ToolCallResult {
            content: vec![ToolContent::Text {
                text: "No frames captured during recording.".to_string(),
            }],
            is_error: true,
        };
    }

    let frame_count = stream.frame_count() as usize;
    let (width, height) = stream.dimensions().unwrap_or((0, 0));
    drop(stream);

    if let Some(ref p) = output_path {
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        if let Err(e) = std::fs::write(p, &gif_bytes) {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to write recorded GIF to '{}': {e}", p.display()),
                }],
                is_error: true,
            };
        }
    }

    let mut content = Vec::new();
    if return_base64 {
        let b64 = base64_encode(&gif_bytes);
        content.push(ToolContent::Image {
            data: b64,
            mime_type: "image/gif".to_string(),
        });
    }

    let save_msg = output_path
        .as_ref()
        .map(|p| format!(" saved to '{}'", p.display()))
        .unwrap_or_default();
    content.push(ToolContent::Text {
        text: format!(
            "Recorded {} frames ({}x{}) at {} FPS over {:.2}s ({} bytes){save_msg}",
            frame_count,
            width,
            height,
            fps,
            start_time.elapsed().as_secs_f32(),
            gif_bytes.len(),
        ),
    });

    ToolCallResult {
        content,
        is_error: false,
    }
}
