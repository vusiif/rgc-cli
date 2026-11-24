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

enum RecordWriter {
    Memory(std::io::Cursor<Vec<u8>>),
    File {
        writer: std::io::BufWriter<std::fs::File>,
    },
}

impl std::io::Write for RecordWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            RecordWriter::Memory(c) => c.write(buf),
            RecordWriter::File { writer } => writer.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            RecordWriter::Memory(c) => c.flush(),
            RecordWriter::File { writer } => writer.flush(),
        }
    }
}

impl std::io::Seek for RecordWriter {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        match self {
            RecordWriter::Memory(c) => c.seek(pos),
            RecordWriter::File { writer } => writer.seek(pos),
        }
    }
}

fn cleanup_temp_file(path: Option<&std::path::Path>) {
    if let Some(p) = path {
        let _ = std::fs::remove_file(p);
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

    // Pre-flight fail-fast check: ensure output directory can be created before capturing
    if let Some(ref p) = output_path {
        let parent = p.parent().unwrap_or_else(|| std::path::Path::new("."));
        if !parent.as_os_str().is_empty() && !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!(
                            "Failed to create output directory '{}': {e}",
                            parent.display()
                        ),
                    }],
                    is_error: true,
                };
            }
        }
    }

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

    let temp_file_path = if let Some(ref p) = output_path {
        if !return_base64 {
            let parent = p.parent().unwrap_or_else(|| std::path::Path::new("."));
            let temp_name = format!(
                ".tmp_rec_{}_{}.tmp",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            );
            Some(parent.join(temp_name))
        } else {
            None
        }
    } else {
        None
    };

    let mut record_writer = if let Some(ref temp_path) = temp_file_path {
        match std::fs::File::create(temp_path) {
            Ok(file) => RecordWriter::File {
                writer: std::io::BufWriter::with_capacity(128 * 1024, file),
            },
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Failed to create temporary recording file: {e}"),
                    }],
                    is_error: true,
                };
            }
        }
    } else {
        RecordWriter::Memory(std::io::Cursor::new(Vec::new()))
    };

    let mut stream = if is_webp {
        match output::WebpStreamEncoder::new(&mut record_writer, fps) {
            Ok(s) => output::RecordingStreamEncoder::Webp(s),
            Err(e) => {
                cleanup_temp_file(temp_file_path.as_deref());
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Failed to initialize streaming WebP encoder: {e}"),
                    }],
                    is_error: true,
                };
            }
        }
    } else {
        match output::GifStreamEncoder::new(&mut record_writer, fps) {
            Ok(s) => output::RecordingStreamEncoder::Gif(s),
            Err(e) => {
                cleanup_temp_file(temp_file_path.as_deref());
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

    let target = if all_monitors {
        capture::CaptureTarget::AllMonitors
    } else if let Some(win) = window_query {
        capture::CaptureTarget::Window {
            query: win,
            client_only,
        }
    } else {
        capture::CaptureTarget::Monitor { index: monitor }
    };

    while start_time.elapsed() < target_duration {
        let frame_start = std::time::Instant::now();
        let captured = capture::capture_unified(target.clone(), None, CoordMode::Logical);

        let c = match captured {
            Ok(c) => c,
            Err(e) => {
                cleanup_temp_file(temp_file_path.as_deref());
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Recording frame capture failed: {e}"),
                    }],
                    is_error: true,
                };
            }
        };

        let frame_img = c.image;
        let scale_factor = c.scale_factor;
        let crop_offset = c.crop_offset;
        let origin = Some(c.origin);

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
                cleanup_temp_file(temp_file_path.as_deref());
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
            cleanup_temp_file(temp_file_path.as_deref());
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
        cleanup_temp_file(temp_file_path.as_deref());
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
        cleanup_temp_file(temp_file_path.as_deref());
        return ToolCallResult {
            content: vec![ToolContent::Text {
                text: format!("Failed to finalize animation container: {e}"),
            }],
            is_error: true,
        };
    }
    drop(stream);

    let (total_size, anim_bytes) = match record_writer {
        RecordWriter::File { mut writer } => {
            use std::io::Write;
            let temp_path = temp_file_path.as_deref().unwrap();
            if let Err(e) = writer.flush() {
                cleanup_temp_file(Some(temp_path));
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Failed to flush recorded temporary file: {e}"),
                    }],
                    is_error: true,
                };
            }
            drop(writer);

            let file_size = match std::fs::metadata(temp_path) {
                Ok(m) => m.len(),
                Err(e) => {
                    cleanup_temp_file(Some(temp_path));
                    return ToolCallResult {
                        content: vec![ToolContent::Text {
                            text: format!("Failed to read recorded temporary file metadata: {e}"),
                        }],
                        is_error: true,
                    };
                }
            };

            if file_size > 100 * 1024 * 1024 {
                cleanup_temp_file(Some(temp_path));
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!(
                            "Recorded animation size ({} MB) exceeds maximum allowed payload limit (100MB). Reduce duration or FPS.",
                            file_size / (1024 * 1024)
                        ),
                    }],
                    is_error: true,
                };
            }

            let p = output_path.as_ref().unwrap();
            let rename_res = std::fs::rename(temp_path, p).or_else(|_| {
                std::fs::copy(temp_path, p).map(|_| {
                    cleanup_temp_file(Some(temp_path));
                })
            });

            if let Err(e) = rename_res {
                cleanup_temp_file(Some(temp_path));
                let label = if is_webp { "WebP" } else { "GIF" };
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!(
                            "Failed to commit recorded {label} to '{}': {e}",
                            p.display()
                        ),
                    }],
                    is_error: true,
                };
            }

            (file_size as usize, None)
        }
        RecordWriter::Memory(cursor) => {
            let bytes = cursor.into_inner();
            if bytes.len() > 100 * 1024 * 1024 {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!(
                            "Recorded animation size ({} MB) exceeds maximum allowed payload limit (100MB). Reduce duration or FPS.",
                            bytes.len() / (1024 * 1024)
                        ),
                    }],
                    is_error: true,
                };
            }

            if let Some(ref p) = output_path {
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

                let write_res = std::fs::write(&temp_path, &bytes);
                if let Err(e) = write_res {
                    let _ = std::fs::remove_file(&temp_path);
                    let label = if is_webp { "WebP" } else { "GIF" };
                    return ToolCallResult {
                        content: vec![ToolContent::Text {
                            text: format!(
                                "Failed to write recorded {label} to temporary file: {e}"
                            ),
                        }],
                        is_error: true,
                    };
                }

                let rename_res = std::fs::rename(&temp_path, p).or_else(|_| {
                    std::fs::copy(&temp_path, p).map(|_| {
                        let _ = std::fs::remove_file(&temp_path);
                    })
                });

                if let Err(e) = rename_res {
                    let _ = std::fs::remove_file(&temp_path);
                    let label = if is_webp { "WebP" } else { "GIF" };
                    return ToolCallResult {
                        content: vec![ToolContent::Text {
                            text: format!(
                                "Failed to commit recorded {label} to '{}': {e}",
                                p.display()
                            ),
                        }],
                        is_error: true,
                    };
                }
            }

            let len = bytes.len();
            (len, Some(bytes))
        }
    };

    let mut content = Vec::new();
    if return_base64 {
        if let Some(ref bytes) = anim_bytes {
            let b64 = base64_encode(bytes);
            content.push(ToolContent::Image {
                data: b64,
                mime_type: if is_webp {
                    "image/webp".to_string()
                } else {
                    "image/gif".to_string()
                },
            });
        }
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
            total_size,
            format_label,
        ),
    });

    ToolCallResult {
        content,
        is_error: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_writer_file_and_memory_seek_write() {
        use std::io::{Read, Seek, Write};
        let mut mem = RecordWriter::Memory(std::io::Cursor::new(Vec::new()));
        mem.write_all(b"hello world").unwrap();
        mem.seek(std::io::SeekFrom::Start(0)).unwrap();
        let mut buf = Vec::new();
        if let RecordWriter::Memory(ref mut c) = mem {
            c.read_to_end(&mut buf).unwrap();
        }
        assert_eq!(&buf, b"hello world");

        let temp_path =
            std::env::temp_dir().join(format!("rgc_rec_writer_test_{}.tmp", std::process::id()));
        {
            let file = std::fs::File::create(&temp_path).unwrap();
            let mut writer = RecordWriter::File {
                writer: std::io::BufWriter::new(file),
            };
            writer.write_all(b"disk streaming data").unwrap();
            writer.flush().unwrap();
            writer.seek(std::io::SeekFrom::Start(5)).unwrap();
            writer.write_all(b"_INJECTED_").unwrap();
            writer.flush().unwrap();
        }
        let read_bytes = std::fs::read(&temp_path).unwrap();
        assert_eq!(&read_bytes, b"disk _INJECTED_data");
        let _ = std::fs::remove_file(&temp_path);
    }
}
