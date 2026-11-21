use std::path::Path;

use crate::capture;
use crate::cli::CoordMode;
use crate::mcp::base64::base64_encode;
use crate::mcp::types::{ToolCallResult, ToolContent, ToolDefinition};
use crate::output;
use crate::transform;

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
        Some(d) if d > 0.0 => d as f32,
        _ => {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: "Missing or invalid 'durationSecs' parameter: must be a positive number"
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
    let output_path = arguments.get("outputPath").and_then(|v| v.as_str());
    let return_base64 = arguments
        .get("returnBase64")
        .and_then(|v| v.as_bool())
        .unwrap_or(output_path.is_none());

    let interval = std::time::Duration::from_secs_f32(1.0 / fps as f32);
    let target_duration = std::time::Duration::from_secs_f32(duration_secs.clamp(0.1, 30.0));
    let start_time = std::time::Instant::now();

    let mut frames = Vec::new();

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

        let (mut frame_img, scale_factor, crop_offset, origin) = match captured {
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

        if cursor_flag || highlight_cursor_flag {
            let global_cursor = capture::get_global_cursor_position();
            if let Some(cursor_info) = capture::resolve_cursor_on_canvas(
                global_cursor,
                origin.unwrap_or((0, 0)),
                scale_factor,
                crop_offset,
                (frame_img.width(), frame_img.height()),
            ) {
                frame_img = transform::draw_cursor_overlay(
                    frame_img,
                    cursor_info.x,
                    cursor_info.y,
                    highlight_cursor_flag,
                );
            }
        }

        frame_img = output::apply_image_transforms(frame_img, scale, max_dimension);
        frames.push(frame_img);

        let elapsed = frame_start.elapsed();
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }
    }

    if frames.is_empty() {
        return ToolCallResult {
            content: vec![ToolContent::Text {
                text: "No frames captured during recording.".to_string(),
            }],
            is_error: true,
        };
    }

    let frame_count = frames.len();
    let width = frames[0].width();
    let height = frames[0].height();

    // Encode to animated GIF in memory
    let mut gif_bytes = Vec::new();
    {
        use image::codecs::gif::{GifEncoder, Repeat};
        use image::{Delay, Frame};

        let mut encoder = GifEncoder::new(&mut gif_bytes);
        let _ = encoder.set_repeat(Repeat::Infinite);
        let delay_ms = (1000 / fps.max(1)).max(10);
        let delay = Delay::from_numer_denom_ms(delay_ms, 1);

        for frame in &frames {
            let rgba = frame.to_rgba8();
            let gif_frame = Frame::from_parts(rgba, 0, 0, delay);
            if let Err(e) = encoder.encode_frame(gif_frame) {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Failed to encode GIF frame: {e}"),
                    }],
                    is_error: true,
                };
            }
        }
    }

    if let Some(path_str) = output_path {
        let p = Path::new(path_str);
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        if let Err(e) = std::fs::write(p, &gif_bytes) {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to write recorded GIF to '{path_str}': {e}"),
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
        .map(|p| format!(" saved to '{p}'"))
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
