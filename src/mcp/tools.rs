use std::path::{Path, PathBuf};
use std::str::FromStr;

use image::DynamicImage;

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

fn parse_markers<T>(
    val: Option<&serde_json::Value>,
    parser: fn(&str) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    let mut out = Vec::new();
    if let Some(v) = val {
        if let Some(s) = v.as_str() {
            out.push(parser(s)?);
        } else if let Some(arr) = v.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    out.push(parser(s)?);
                }
            }
        }
    }
    Ok(out)
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
                    "activeWindow": {
                        "type": "boolean",
                        "description": "Optional flag to automatically capture the currently active/focused desktop window. Supports 'clientOnly' to trim titlebars and drop shadows."
                    },
                    "clientOnly": {
                        "type": "boolean",
                        "description": "Optional flag when capturing a window or activeWindow to trim outer drop shadows and titlebars, returning only the application client/working area."
                    },
                    "aroundCursor": {
                        "type": "integer",
                        "description": "Optional radius in pixels to dynamically crop a square bounding box centered around the mouse cursor (e.g. 200 for a 400x400 box)."
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
                    "allowOverwrite": {
                        "type": "boolean",
                        "description": "Whether to allow overwriting an existing file at outputPath. Defaults to false to prevent accidental data loss."
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
                    },
                    "highlightDiff": {
                        "type": "boolean",
                        "description": "Optional flag to highlight detected difference bounding box on the image."
                    },
                    "timestamp": {
                        "type": "boolean",
                        "description": "Optional flag to burn a timestamp watermark badge onto the output image."
                    },
                    "tag": {
                        "type": "string",
                        "description": "Optional custom text tag to burn onto the output image."
                    },
                    "waitChange": {
                        "type": "boolean",
                        "description": "Optional flag to wait for screen/window visual change before capturing (ideal for UI automation & click validation)."
                    },
                    "waitStableMs": {
                        "type": "integer",
                        "description": "Optional duration in milliseconds the screen must remain visually stable (no changes) before capturing (ideal for settling animations)."
                    },
                    "waitTimeoutMs": {
                        "type": "integer",
                        "description": "Optional timeout in milliseconds to wait for visual change before capturing (default: 5000)."
                    },
                    "waitIntervalMs": {
                        "type": "integer",
                        "description": "Optional polling interval in milliseconds between change detection checks (default: 100)."
                    },
                    "cursor": {
                        "type": "boolean",
                        "description": "Optional flag to capture and render mouse cursor arrow onto the screenshot."
                    },
                    "highlightCursor": {
                        "type": "boolean",
                        "description": "Optional flag to highlight mouse cursor with a vivid amber focus ring."
                    },
                    "som": {
                        "type": "boolean",
                        "description": "Optional flag to detect interactive UI elements and overlay Set-of-Mark (SoM) numbered badges."
                    },
                    "somThreshold": {
                        "type": "integer",
                        "description": "Optional edge sensitivity threshold (5-100, default: 25) for UI element detection."
                    },
                    "somMaxElements": {
                        "type": "integer",
                        "description": "Optional maximum number of UI elements to detect and mark (default: 50)."
                    },
                    "accessibility": {
                        "type": "boolean",
                        "description": "Optional flag to fuse OS accessibility tree semantics (role, label, state) with detected Set-of-Mark elements."
                    },
                    "excludeWindows": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of window titles or patterns to exclude and blackout completely from capture."
                    },
                    "findTemplate": {
                        "type": "string",
                        "description": "Optional file path or base64 data URI of a template/icon image to locate on the screenshot."
                    },
                    "templateThreshold": {
                        "type": "number",
                        "description": "Optional minimum confidence threshold (0.0-1.0, default: 0.85) for template matching."
                    },
                    "templateMaxMatches": {
                        "type": "integer",
                        "description": "Optional maximum number of matching occurrences to return (default: 1)."
                    },
                    "highlightTemplate": {
                        "type": "boolean",
                        "description": "Optional flag to highlight found template matches with bounding boxes and confidence score badges."
                    },
                    "clickAt": {
                        "description": "Optional click target coordinate(s) in 'X,Y' or 'X,Y,LABEL' format (string or array of strings) to overlay visual ripple and target indicators."
                    },
                    "drag": {
                        "description": "Optional drag trajectory coordinate(s) in 'X1,Y1,X2,Y2' or 'X1,Y1,X2,Y2,LABEL' format (string or array of strings) to overlay directional dashed trajectory arrows."
                    },
                    "allowMissingMask": {
                        "type": "boolean",
                        "description": "Optional flag to permit capture if a configured or requested mask window is missing or not visible."
                    }
                },
                "additionalProperties": false
            }),
        },
        crate::mcp::record::get_record_tool_definition(),
        crate::mcp::action::get_verify_action_tool_definition(),
        ToolDefinition {
            name: "doctor".to_string(),
            description: "Run hardware, display, and environment diagnostics to check screen capture availability.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "find_template".to_string(),
            description: "Locate a visual template or icon on the active screen or window and return its exact desktop coordinates and confidence score.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "template": {
                        "type": "string",
                        "description": "Required file path or base64 data URI of the target template/icon image to locate."
                    },
                    "threshold": {
                        "type": "number",
                        "description": "Optional minimum confidence score in range 0.0-1.0 (default: 0.85)."
                    },
                    "maxMatches": {
                        "type": "integer",
                        "description": "Optional maximum number of matching occurrences to return (default: 1)."
                    },
                    "monitor": {
                        "type": "integer",
                        "description": "Optional 0-indexed monitor index to search within."
                    },
                    "window": {
                        "type": "string",
                        "description": "Optional window title or ID to search within."
                    },
                    "region": {
                        "type": "string",
                        "description": "Optional region of interest (ROI) in 'X,Y,WIDTH,HEIGHT' format to accelerate matching and constrain search scope."
                    },
                    "waitTimeoutMs": {
                        "type": "integer",
                        "description": "Optional timeout in milliseconds to wait for template to appear before returning (default: 0 / immediate)."
                    }
                },
                "required": ["template"],
                "additionalProperties": false
            }),
        },
        crate::mcp::session::get_session_tool_definition(),
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
        "record_screen" => execute_record_tool(arguments),
        "verify_action" => execute_verify_action_tool(arguments),
        "capture_session" => execute_capture_session_tool(arguments),
        "find_template" => execute_find_template_tool(arguments),
        _ => ToolCallResult {
            content: vec![ToolContent::Text {
                text: format!("Unknown tool name: '{name}'"),
            }],
            is_error: true,
        },
    }
}

/// Validate and sanitize output path requested by an MCP client to prevent path traversal
/// and accidental data loss via unintended overwrites.
pub fn validate_mcp_output_path(
    path_str: &str,
    allowed_extensions: &[&str],
    allow_overwrite: bool,
) -> Result<PathBuf, String> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Err("Output path cannot be empty".to_string());
    }

    if trimmed.contains('\0') {
        return Err("Output path contains illegal null bytes".to_string());
    }

    let p = Path::new(trimmed);

    // Extension check
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if !allowed_extensions
        .iter()
        .any(|&allowed| allowed.eq_ignore_ascii_case(&ext))
    {
        return Err(format!(
            "Invalid file extension '.{ext}'. Allowed extensions: {}",
            allowed_extensions.join(", ")
        ));
    }

    // Path traversal check: components must not escape using ParentDir ("..")
    for comp in p.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err(format!(
                "Path traversal ('..') is not permitted in outputPath: '{trimmed}'"
            ));
        }
    }

    // Allowed directories check (from active config and RGC_ALLOWED_OUTPUT_DIRS environment variable)
    let active_cfg = crate::config::get_active_config();
    let mut configured_allowed: Vec<String> = active_cfg.security.allowed_output_dirs.clone();
    if let Ok(allowed_dirs_var) = std::env::var("RGC_ALLOWED_OUTPUT_DIRS") {
        for part in allowed_dirs_var.split([',', ';']) {
            let t = part.trim();
            if !t.is_empty() {
                configured_allowed.push(t.to_string());
            }
        }
    }

    if !configured_allowed.is_empty() {
        let allowed_dirs: Vec<PathBuf> = configured_allowed
            .iter()
            .map(PathBuf::from)
            .filter_map(|pb| pb.canonicalize().ok())
            .collect();

        if allowed_dirs.is_empty() {
            return Err(format!(
                "Allowed output directories configured ({configured_allowed:?}) but none are valid or accessible; refusing file write for security."
            ));
        }

        let abs_path = if p.is_relative() {
            std::env::current_dir()
                .map(|cwd| cwd.join(p))
                .unwrap_or_else(|_| p.to_path_buf())
        } else {
            p.to_path_buf()
        };

        let parent_to_check = abs_path
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .unwrap_or_else(|| abs_path.clone());

        let is_allowed = allowed_dirs
            .iter()
            .any(|allowed| parent_to_check.starts_with(allowed));

        if !is_allowed {
            return Err(format!(
                "Target outputPath '{}' is outside allowed directories: {:?}",
                trimmed, configured_allowed
            ));
        }
    }

    // Overwrite check
    if !allow_overwrite && p.exists() {
        return Err(format!(
            "Target file already exists: '{}'. Set allowOverwrite=true to permit overwriting.",
            p.display()
        ));
    }

    Ok(p.to_path_buf())
}

/// Validate and safely load an untrusted image input from a base64 string or filesystem path.
///
/// Security constraints:
/// - Base64 strings are capped at 32MB to prevent memory exhaustion DoS.
/// - File paths are strictly forbidden from path traversal (`..`) and Windows device namespaces (`\\.\`, `//./`).
/// - Decoded images are strictly protected against decompression bombs using reader limits
///   (max width/height 16384, max allocation 256MB).
pub fn load_untrusted_image_input(
    input_str: &str,
    param_name: &str,
) -> Result<DynamicImage, String> {
    let trimmed = input_str.trim();
    if trimmed.is_empty() {
        return Err(format!("Parameter '{param_name}' cannot be empty"));
    }

    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);

    let is_base64 = trimmed.starts_with("data:image/")
        || (!trimmed.contains('/')
            && !trimmed.contains('\\')
            && !trimmed.contains('.')
            && trimmed.len() > 100);

    let img = if is_base64 {
        let raw_b64 = if let Some((_, rest)) = trimmed.split_once(";base64,") {
            rest.trim()
        } else {
            trimmed
        };

        if raw_b64.len() > 32 * 1024 * 1024 {
            return Err(format!(
                "Base64 image payload in '{param_name}' exceeds maximum allowed size (32MB)"
            ));
        }

        let bytes =
            base64_decode(raw_b64).map_err(|e| format!("Invalid base64 in '{param_name}': {e}"))?;

        let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes));
        reader.limits(limits);
        let reader = reader
            .with_guessed_format()
            .map_err(|e| format!("Failed to guess image format in '{param_name}': {e}"))?;
        reader
            .decode()
            .map_err(|e| format!("Failed to decode image from memory in '{param_name}': {e}"))?
    } else {
        if trimmed.starts_with(r"\\.\") || trimmed.starts_with("//./") {
            return Err(format!(
                "Device paths are not permitted in '{param_name}': '{trimmed}'"
            ));
        }

        let p = Path::new(trimmed);
        for comp in p.components() {
            if matches!(comp, std::path::Component::ParentDir) {
                return Err(format!(
                    "Path traversal ('..') is not permitted in '{param_name}': '{trimmed}'"
                ));
            }
        }

        let mut reader = image::ImageReader::open(p)
            .map_err(|e| format!("Failed to open image from '{trimmed}' in '{param_name}': {e}"))?;
        reader.limits(limits);
        reader.decode().map_err(|e| {
            format!("Failed to decode image from '{trimmed}' in '{param_name}': {e}")
        })?
    };

    if img.width() > 16384 || img.height() > 16384 {
        return Err(format!(
            "Image dimensions ({}x{}) in '{param_name}' exceed maximum allowed size (16384x16384)",
            img.width(),
            img.height()
        ));
    }

    Ok(img)
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
        if d == 0 || d > 16384 {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Invalid maxDimension: must be between 1 and 16384 (got {d})"),
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

    let allow_overwrite = arguments
        .get("allowOverwrite")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let output_path = match arguments.get("outputPath").and_then(|v| v.as_str()) {
        Some(s) => {
            match validate_mcp_output_path(s, &["png", "jpg", "jpeg", "webp"], allow_overwrite) {
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
    let highlight_diff = arguments
        .get("highlightDiff")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let timestamp = arguments
        .get("timestamp")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let tag = arguments.get("tag").and_then(|v| v.as_str());
    let wait_change = arguments
        .get("waitChange")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let wait_timeout_ms = arguments
        .get("waitTimeoutMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(5000)
        .clamp(10, 60_000);
    let wait_interval_ms = arguments
        .get("waitIntervalMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(100)
        .clamp(10, 5000);
    let cursor_flag = arguments
        .get("cursor")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let highlight_cursor_flag = arguments
        .get("highlightCursor")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let som_flag = arguments
        .get("som")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let som_threshold = arguments
        .get("somThreshold")
        .and_then(|v| v.as_u64())
        .unwrap_or(25) as u8;
    let som_max_elements = arguments
        .get("somMaxElements")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;
    let accessibility_flag = arguments
        .get("accessibility")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut exclude_windows = Vec::new();
    if let Some(arr) = arguments.get("excludeWindows").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                exclude_windows.push(s.to_string());
            }
        }
    }
    let diff_with_str = arguments.get("diffWith").and_then(|v| v.as_str());

    // Validate and preload reference diff image early to fail fast before capture
    let preloaded_diff_ref = if let Some(diff_str) = diff_with_str {
        match load_untrusted_image_input(diff_str, "diffWith") {
            Ok(img) => Some(img),
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Failed to open diff reference image '{diff_str}': {e}"),
                    }],
                    is_error: true,
                };
            }
        }
    } else {
        None
    };

    let find_template_str = arguments.get("findTemplate").and_then(|v| v.as_str());
    let template_threshold = arguments
        .get("templateThreshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.85) as f32;
    let template_max_matches = arguments
        .get("templateMaxMatches")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as usize;
    let highlight_template = arguments
        .get("highlightTemplate")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let preloaded_template_match = if let Some(tpl_str) = find_template_str {
        let tpl_image = match load_untrusted_image_input(tpl_str, "findTemplate") {
            Ok(img) => img,
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Failed to open template image '{tpl_str}': {e}"),
                    }],
                    is_error: true,
                };
            }
        };

        Some(crate::template::TemplateMatchOptions {
            template: tpl_image,
            threshold: template_threshold,
            max_matches: template_max_matches,
            highlight: highlight_template,
        })
    } else {
        None
    };

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
    let active_window = arguments
        .get("activeWindow")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let client_only = arguments
        .get("clientOnly")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let target = if active_window {
        capture::CaptureTarget::ActiveWindow { client_only }
    } else if all_monitors {
        capture::CaptureTarget::AllMonitors
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
        capture::CaptureTarget::Window {
            query: win_query,
            client_only,
        }
    } else {
        capture::CaptureTarget::Monitor { index: monitor }
    };

    let err_prefix = match target {
        capture::CaptureTarget::ActiveWindow { .. } => "Active window capture failed",
        capture::CaptureTarget::AllMonitors => "Panorama capture failed",
        capture::CaptureTarget::Window { .. } => "Window capture failed",
        capture::CaptureTarget::Monitor { .. } => "Capture failed",
    };

    let mut capture_once = || {
        capture::capture_unified(target.clone(), region.as_ref(), coords)
            .map_err(|e| format!("{err_prefix}: {e}"))
    };

    let wait_stable_ms = arguments
        .get("waitStableMs")
        .and_then(|v| v.as_u64())
        .filter(|&ms| ms > 0);

    let captured = if let Some(stable_ms) = wait_stable_ms {
        match capture::wait_for_visual_stability(
            &mut capture_once,
            diff_threshold,
            std::time::Duration::from_millis(stable_ms),
            std::time::Duration::from_millis(wait_timeout_ms),
            std::time::Duration::from_millis(wait_interval_ms),
        ) {
            Ok((c, _)) => c,
            Err(err_msg) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text { text: err_msg }],
                    is_error: true,
                };
            }
        }
    } else if wait_change {
        match capture::wait_for_visual_change(
            &mut capture_once,
            diff_threshold,
            std::time::Duration::from_millis(wait_timeout_ms),
            std::time::Duration::from_millis(wait_interval_ms),
        ) {
            Ok((c, _)) => c,
            Err(err_msg) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text { text: err_msg }],
                    is_error: true,
                };
            }
        }
    } else {
        match capture_once() {
            Ok(c) => c,
            Err(err_msg) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text { text: err_msg }],
                    is_error: true,
                };
            }
        }
    };

    let image = captured.image;
    let scale_factor = captured.scale_factor;
    let crop_offset = captured.crop_offset;
    let monitor_origin = Some(captured.origin);

    // Action overlays: click ripples and drag trajectories
    let parsed_clicks =
        match parse_markers(arguments.get("clickAt"), crate::action::parse_click_marker) {
            Ok(c) => c,
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text { text: e }],
                    is_error: true,
                }
            }
        };
    let parsed_drags = match parse_markers(arguments.get("drag"), crate::action::parse_drag_marker)
    {
        Ok(d) => d,
        Err(e) => {
            return ToolCallResult {
                content: vec![ToolContent::Text { text: e }],
                is_error: true,
            }
        }
    };

    let request_allow_missing = arguments.get("allowMissingMask").and_then(|v| v.as_bool());
    let cfg = crate::config::get_active_config();
    let privacy_policy = crate::config::PrivacyPolicy::from_config_and_request(
        &cfg.privacy,
        &window_masks,
        &exclude_windows,
        &manual_masks,
        request_allow_missing,
    );

    let around_cursor = arguments
        .get("aroundCursor")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let pipeline_options = crate::pipeline::PipelineOptions {
        cursor: cursor_flag,
        highlight_cursor: highlight_cursor_flag,
        manual_masks: privacy_policy.mask_regions,
        mask_windows: privacy_policy.mask_windows,
        exclude_windows: privacy_policy.exclude_windows,
        allow_missing_mask: privacy_policy.allow_missing_mask,
        coords_mode: coords,
        scale,
        max_dimension,
        diff_ref: preloaded_diff_ref,
        diff_threshold,
        crop_changed,
        highlight_diff,
        grid,
        timestamp,
        tag: tag.map(str::to_string),
        som: som_flag,
        som_threshold,
        som_max_elements,
        accessibility: accessibility_flag,
        a11y_nodes_override: None,
        template_match: preloaded_template_match,
        clicks: parsed_clicks,
        drags: parsed_drags,
        global_cursor_override: None,
        around_cursor,
    };

    let pipeline_result = match crate::pipeline::execute_image_pipeline(
        image,
        scale_factor,
        crop_offset,
        monitor_origin,
        &pipeline_options,
    ) {
        Ok(r) => r,
        Err(e) => {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Privacy redaction failed: {}", e.message),
                }],
                is_error: true,
            };
        }
    };

    let image = pipeline_result.image;
    let cursor_info = pipeline_result.cursor;
    let detected_elements = pipeline_result.elements;
    let diff_summary = pipeline_result
        .diff
        .and_then(|d| serde_json::to_string_pretty(&d).ok());

    let mut content = Vec::new();

    if let Some(ref diff_json) = diff_summary {
        content.push(ToolContent::Text {
            text: format!("Diff Analysis:\n{diff_json}"),
        });
    }

    if let Some(ref elems) = detected_elements {
        if let Ok(elems_json) = serde_json::to_string_pretty(elems) {
            content.push(ToolContent::Text {
                text: format!("Detected UI Elements (Set-of-Mark):\n{elems_json}"),
            });
        }
    }

    if let Some(ref matches) = pipeline_result.matches {
        if let Ok(matches_json) = serde_json::to_string_pretty(matches) {
            content.push(ToolContent::Text {
                text: format!("Template Matches:\n{matches_json}"),
            });
        }
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
            ImageFormat::Gif => "image/gif",
        };

        content.push(ToolContent::Image {
            data: b64,
            mime_type: mime_type.to_string(),
        });
        let cursor_desc = if let Some(c) = cursor_info {
            format!(" [Cursor: x={}, y={}, visible={}]", c.x, c.y, c.visible)
        } else {
            String::new()
        };
        let som_desc = if let Some(ref elems) = detected_elements {
            format!(" [SoM: {} UI elements detected]", elems.len())
        } else {
            String::new()
        };

        content.push(ToolContent::Text {
            text: format!(
                "Captured {}x{} image ({:?}, {} bytes){cursor_desc}{som_desc}",
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

pub fn execute_find_template_tool(arguments: &serde_json::Value) -> ToolCallResult {
    let tpl_str = match arguments.get("template").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: "Missing or empty required parameter 'template'".to_string(),
                }],
                is_error: true,
            };
        }
    };

    let threshold = arguments
        .get("threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.85) as f32;
    let max_matches = arguments
        .get("maxMatches")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as usize;
    let monitor = arguments
        .get("monitor")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let window_query = arguments.get("window").and_then(|v| v.as_str());
    let wait_timeout_ms = arguments
        .get("waitTimeoutMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Load template image (base64 or file path) safely
    let tpl_img = match load_untrusted_image_input(tpl_str, "template") {
        Ok(img) => img,
        Err(e) => {
            return ToolCallResult {
                content: vec![ToolContent::Text { text: e }],
                is_error: true,
            };
        }
    };

    let roi = arguments
        .get("region")
        .or_else(|| arguments.get("roi"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<crate::cli::Region>().ok());

    let capture_and_search = || -> Result<Vec<crate::api::TemplateMatchResult>, String> {
        let captured = if let Some(win_q) = window_query {
            match capture::capture_window_with_details(win_q, None) {
                Ok(c) => (c.image, c.scale_factor, c.crop_offset, c.window_origin),
                Err(e) => return Err(format!("Failed to capture window '{win_q}': {e}")),
            }
        } else {
            match capture::capture_screen_with_details(monitor, None, CoordMode::Physical) {
                Ok(c) => (c.image, c.scale_factor, c.crop_offset, c.monitor_origin),
                Err(e) => return Err(format!("Failed to capture screen: {e}")),
            }
        };

        let (image, scale_factor, crop_offset, origin) = captured;
        let cfg = crate::config::get_active_config();
        let privacy_policy = crate::config::PrivacyPolicy::from_config(&cfg.privacy);
        let pipeline_opts = crate::pipeline::PipelineOptions {
            manual_masks: privacy_policy.mask_regions,
            mask_windows: privacy_policy.mask_windows,
            exclude_windows: privacy_policy.exclude_windows,
            allow_missing_mask: privacy_policy.allow_missing_mask,
            coords_mode: CoordMode::Physical,
            ..Default::default()
        };
        let sanitized_res = crate::pipeline::execute_image_pipeline(
            image,
            scale_factor,
            crop_offset,
            Some(origin),
            &pipeline_opts,
        )
        .map_err(|e| {
            format!(
                "Privacy redaction failed during template search: {}",
                e.message
            )
        })?;

        let rgba = sanitized_res.image.into_rgba8();
        let img_w = rgba.width();
        let img_h = rgba.height();

        let mut matches = crate::template::find_template_matches_in_roi(
            &rgba,
            &tpl_img,
            roi.as_ref(),
            threshold,
            max_matches,
        );
        let transform = crate::coords::CoordinateTransform::new(
            origin,
            scale_factor,
            crop_offset,
            1.0,
            (0, 0),
            (img_w, img_h),
            (img_w, img_h),
        );
        crate::template::enrich_matches_with_coordinates(&mut matches, &transform);
        Ok(matches)
    };

    let matches = if wait_timeout_ms > 0 {
        let start_time = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(wait_timeout_ms);
        let poll_interval = std::time::Duration::from_millis(50);
        loop {
            match capture_and_search() {
                Ok(m) => {
                    if !m.is_empty() || start_time.elapsed() >= timeout {
                        break m;
                    }
                }
                Err(e) => {
                    if start_time.elapsed() >= timeout {
                        return ToolCallResult {
                            content: vec![ToolContent::Text { text: e }],
                            is_error: true,
                        };
                    }
                }
            }
            std::thread::sleep(poll_interval);
        }
    } else {
        match capture_and_search() {
            Ok(m) => m,
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text { text: e }],
                    is_error: true,
                };
            }
        }
    };

    match serde_json::to_string_pretty(&matches) {
        Ok(json_str) => ToolCallResult {
            content: vec![ToolContent::Text {
                text: format!("Template Matches:\n{json_str}"),
            }],
            is_error: false,
        },
        Err(e) => ToolCallResult {
            content: vec![ToolContent::Text {
                text: format!("Failed to serialize match results: {e}"),
            }],
            is_error: true,
        },
    }
}

pub use crate::mcp::action::execute_verify_action_tool;
pub use crate::mcp::record::execute_record_tool;
pub use crate::mcp::session::execute_capture_session_tool;
