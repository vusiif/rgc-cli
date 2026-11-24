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
                    "description": "Duration in seconds to record (0.1 to 30.0 seconds)."
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
                "clientOnly": {
                    "type": "boolean",
                    "description": "Optional flag when capturing a window to trim outer drop shadows and titlebars, returning only the application client/working area."
                },
                "allMonitors": {
                    "type": "boolean",
                    "description": "Optional flag to record multi-monitor panorama."
                },
                "format": {
                    "type": "string",
                    "enum": ["gif", "webp"],
                    "description": "Animation container format ('gif' or 'webp'). Defaults to 'gif', or inferred from outputPath."
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
                    "description": "Optional output path for animated GIF or WebP file."
                },
                "allowOverwrite": {
                    "type": "boolean",
                    "description": "Optional flag to permit overwriting an existing output file at outputPath."
                },
                "returnBase64": {
                    "type": "boolean",
                    "description": "Whether to return the animated GIF or WebP base64 data in the response. Defaults to true if outputPath is omitted."
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
                },
                "excludeWindows": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of window titles or patterns to exclude and blackout completely from recorded frames."
                },
                "allowMissingMask": {
                    "type": "boolean",
                    "description": "Optional flag to permit recording if a configured mask window is not found."
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
        Some(d) if d.is_finite() && (0.1..=30.0).contains(&d) => d as f32,
        _ => {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: "Missing or invalid 'durationSecs' parameter: must be a finite number between 0.1 and 30.0 seconds."
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
    let client_only = arguments
        .get("clientOnly")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let output_path = match arguments.get("outputPath").and_then(|v| v.as_str()) {
        Some(s) => {
            match crate::mcp::tools::validate_mcp_output_path(s, &["gif", "webp"], allow_overwrite)
            {
                Ok(pb) => Some(pb),
                Err(e) => {
                    return ToolCallResult {
                        content: vec![ToolContent::Text {
                            text: format!("Invalid outputPath: {e}"),
                        }],
                        is_error: true,
                    };
                }
            }
        }
        None => None,
    };
    let is_webp = if let Some(ref p) = output_path {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("webp"))
            .unwrap_or(false)
    } else {
        arguments
            .get("format")
            .and_then(|v| v.as_str())
            .map(|s| s.eq_ignore_ascii_case("webp"))
            .unwrap_or(false)
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

    let mut exclude_windows = Vec::new();
    if let Some(arr) = arguments.get("excludeWindows").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                exclude_windows.push(s.to_string());
            }
        }
    }

    let request_allow_missing = arguments.get("allowMissingMask").and_then(|v| v.as_bool());
    let cfg = crate::config::get_active_config();
    let privacy_policy = crate::config::PrivacyPolicy::from_config_and_request(
        &cfg.privacy,
        &window_masks,
        &exclude_windows,
        &manual_masks,
        request_allow_missing,
    );

    let interval = std::time::Duration::from_secs_f32(1.0 / fps as f32);
    let target_duration = std::time::Duration::from_secs_f32(duration_secs);
    let start_time = std::time::Instant::now();

    let mut anim_buffer = std::io::Cursor::new(Vec::new());
    let mut stream = if is_webp {
        match output::WebpStreamEncoder::new(&mut anim_buffer, fps) {
            Ok(s) => output::RecordingStreamEncoder::Webp(s),
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Failed to initialize streaming WebP encoder: {e}"),
                    }],
                    is_error: true,
                };
            }
        }
    } else {
        match output::GifStreamEncoder::new(&mut anim_buffer, fps) {
            Ok(s) => output::RecordingStreamEncoder::Gif(s),
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Failed to initialize streaming GIF encoder: {e}"),
                    }],
                    is_error: true,
                };
            }
        }
    };

    let frame_options = crate::pipeline::RecordingFrameOptions {
        cursor: cursor_flag,
        highlight_cursor: highlight_cursor_flag,
        manual_masks: privacy_policy.mask_regions,
        mask_windows: privacy_policy.mask_windows,
        exclude_windows: privacy_policy.exclude_windows,
        allow_missing_mask: privacy_policy.allow_missing_mask,
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
            capture::capture_window_with_options(win, None, client_only).map(|c| {
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
                    text: format!("Failed to encode streaming frame: {e}"),
                }],
                is_error: true,
            };
        }

        let elapsed = frame_start.elapsed();
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }

        // Cap maximum frame iterations to prevent runaway recording loops
        if stream.frame_count() >= 900 {
            log::warn!("Recording reached maximum frame limit (900); stopping capture.");
            break;
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
    if let Err(e) = stream.finish() {
        return ToolCallResult {
            content: vec![ToolContent::Text {
                text: format!("Failed to finalize animation container: {e}"),
            }],
            is_error: true,
        };
    }
    drop(stream);

    let anim_bytes = anim_buffer.into_inner();

    // Memory circuit breaker: prevent unbounded buffer payloads
    if anim_bytes.len() > 100 * 1024 * 1024 {
        return ToolCallResult {
            content: vec![ToolContent::Text {
                text: format!(
                    "Recorded animation size ({} MB) exceeds maximum allowed payload limit (100MB). Reduce duration or FPS.",
                    anim_bytes.len() / (1024 * 1024)
                ),
            }],
            is_error: true,
        };
    }

    if let Some(ref p) = output_path {
        if !allow_overwrite && p.exists() {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!(
                        "Target file already exists: '{}'. Set allowOverwrite=true to permit overwriting.",
                        p.display()
                    ),
                }],
                is_error: true,
            };
        }

        let parent = p.parent().unwrap_or_else(|| std::path::Path::new("."));
        if !parent.as_os_str().is_empty() && !parent.exists() {
            let _ = std::fs::create_dir_all(parent);
        }

        let temp_name = format!(
            ".tmp_rec_{}_{}.tmp",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        let temp_path = parent.join(temp_name);

        let write_res = std::fs::write(&temp_path, &anim_bytes);
        if let Err(e) = write_res {
            let _ = std::fs::remove_file(&temp_path);
            let label = if is_webp { "WebP" } else { "GIF" };
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to write recorded {label} to temporary file: {e}"),
                }],
                is_error: true,
            };
        }

        if let Err(_rename_err) = std::fs::rename(&temp_path, p) {
            if let Err(copy_err) = std::fs::copy(&temp_path, p) {
                let _ = std::fs::remove_file(&temp_path);
                let label = if is_webp { "WebP" } else { "GIF" };
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!(
                            "Failed to commit recorded {label} to '{}': {copy_err}",
                            p.display()
                        ),
                    }],
                    is_error: true,
                };
            }
            let _ = std::fs::remove_file(&temp_path);
        }
    }

    let mut content = Vec::new();
    if return_base64 {
        let b64 = base64_encode(&anim_bytes);
        content.push(ToolContent::Image {
            data: b64,
            mime_type: if is_webp {
                "image/webp".to_string()
            } else {
                "image/gif".to_string()
            },
        });
    }

    let save_msg = output_path
        .as_ref()
        .map(|p| format!(" saved to '{}'", p.display()))
        .unwrap_or_default();
    let format_label = if is_webp {
        "animated WebP"
    } else {
        "animated GIF"
    };
    content.push(ToolContent::Text {
        text: format!(
            "Recorded {} frames ({}x{}) at {} FPS over {:.2}s ({} bytes, format: {}){save_msg}",
            frame_count,
            width,
            height,
            fps,
            start_time.elapsed().as_secs_f32(),
            anim_bytes.len(),
            format_label,
        ),
    });

    ToolCallResult {
        content,
        is_error: false,
    }
}
