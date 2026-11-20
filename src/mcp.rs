use crate::capture;
use crate::cli::{CoordMode, ImageFormat, Region};
use crate::doctor;
use crate::output::{self, OutputDestination};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::str::FromStr;

const BASE64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Fast, dependency-free standard base64 encoding (RFC 4648).
pub fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i];
        let b1 = if i + 1 < input.len() { input[i + 1] } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] } else { 0 };

        out.push(BASE64_CHARS[(b0 >> 2) as usize] as char);
        out.push(BASE64_CHARS[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);

        if i + 1 < input.len() {
            out.push(BASE64_CHARS[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }

        if i + 2 < input.len() {
            out.push(BASE64_CHARS[(b2 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }

        i += 3;
    }
    out
}

/// Fast, dependency-free standard base64 decoding (RFC 4648).
pub fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;

    for &b in input.as_bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            break;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(format!("Invalid base64 character: {}", b as char)),
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse<T> {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub result: T,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcErrorResponse {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub error: JsonRpcError,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub tools: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct ToolListResult {
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ResourceListResult {
    pub resources: Vec<ResourceDefinition>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDefinition {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceTemplatesListResult {
    pub resource_templates: Vec<ResourceTemplateDefinition>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceTemplateDefinition {
    pub uri_template: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResourceReadResult {
    pub contents: Vec<ResourceContent>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ResourceContent {
    #[serde(rename_all = "camelCase")]
    Text {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Blob {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        blob: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResult {
    pub content: Vec<ToolContent>,
    pub is_error: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

/// Handle a single incoming JSON-RPC line and return serialized response line if applicable.
pub fn handle_mcp_message(line: &str) -> Option<String> {
    let req: JsonRpcRequest = match serde_json::from_str(line) {
        Ok(req) => req,
        Err(e) => {
            debug!("Failed to parse JSON-RPC request: {e}");
            let err = JsonRpcErrorResponse {
                jsonrpc: "2.0",
                id: serde_json::Value::Null,
                error: JsonRpcError {
                    code: -32700,
                    message: format!("Parse error: {e}"),
                    data: None,
                },
            };
            return serde_json::to_string(&err).ok();
        }
    };

    let id = req.id?;

    if req.jsonrpc != "2.0" {
        let err = JsonRpcErrorResponse {
            jsonrpc: "2.0",
            id,
            error: JsonRpcError {
                code: -32600,
                message: format!(
                    "Invalid Request: jsonrpc must be '2.0', got '{}'",
                    req.jsonrpc
                ),
                data: None,
            },
        };
        return serde_json::to_string(&err).ok();
    }

    match req.method.as_str() {
        "initialize" => {
            let res = JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: InitializeResult {
                    protocol_version: "2024-11-05".to_string(),
                    capabilities: ServerCapabilities {
                        tools: serde_json::json!({}),
                        resources: Some(serde_json::json!({
                            "subscribe": false,
                            "listChanged": false,
                        })),
                    },
                    server_info: ServerInfo {
                        name: "rgc-cli".to_string(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                    },
                },
            };
            serde_json::to_string(&res).ok()
        }
        "ping" => {
            let res = JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: serde_json::json!({}),
            };
            serde_json::to_string(&res).ok()
        }
        "tools/list" => {
            let tools = vec![
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
            ];

            let res = JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: ToolListResult { tools },
            };
            serde_json::to_string(&res).ok()
        }
        "tools/call" => {
            let tool_name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = req
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let call_result = execute_tool_call(tool_name, &arguments);

            let res = JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: call_result,
            };
            serde_json::to_string(&res).ok()
        }
        "resources/list" => {
            let mut resources = vec![ResourceDefinition {
                uri: "screen://current".to_string(),
                name: "Current Primary Screen".to_string(),
                description: Some(
                    "Live snapshot of the current primary display in PNG format".to_string(),
                ),
                mime_type: Some("image/png".to_string()),
            }];

            if let Ok(monitors) = capture::get_monitor_infos() {
                for m in monitors {
                    resources.push(ResourceDefinition {
                        uri: format!("screen://monitor/{}", m.index),
                        name: format!("Monitor #{} ({})", m.index, m.name),
                        description: Some(format!(
                            "Live snapshot of monitor #{} ({}x{} @ {:.2}x{})",
                            m.index,
                            m.width,
                            m.height,
                            m.scale_factor,
                            if m.is_primary { " [Primary]" } else { "" }
                        )),
                        mime_type: Some("image/png".to_string()),
                    });
                }
            }

            if let Ok(windows) = capture::get_window_infos() {
                for w in windows.iter().take(10) {
                    if !w.is_minimized && !w.title.is_empty() {
                        resources.push(ResourceDefinition {
                            uri: format!("screen://window/{}", w.id),
                            name: format!("Window #{} ({})", w.id, w.title),
                            description: Some(format!(
                                "Live snapshot of application window '{}' ({}x{} @ {},{})",
                                w.app_name, w.width, w.height, w.x, w.y
                            )),
                            mime_type: Some("image/png".to_string()),
                        });
                    }
                }
            }

            resources.push(ResourceDefinition {
                uri: "screen://all".to_string(),
                name: "All Monitors Desktop Panorama".to_string(),
                description: Some(
                    "Capture a live stitched panorama of all connected displays into a single virtual desktop canvas".to_string(),
                ),
                mime_type: Some("image/png".to_string()),
            });

            resources.push(ResourceDefinition {
                uri: "screen://diagnostics".to_string(),
                name: "System Diagnostics Report".to_string(),
                description: Some(
                    "Live display and graphics capture subsystem diagnostic report in JSON format"
                        .to_string(),
                ),
                mime_type: Some("application/json".to_string()),
            });

            let res = JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: ResourceListResult { resources },
            };
            serde_json::to_string(&res).ok()
        }
        "resources/templates/list" => {
            let templates = vec![
                ResourceTemplateDefinition {
                    uri_template: "screen://monitor/{index}".to_string(),
                    name: "Monitor Display Snapshot".to_string(),
                    description: Some(
                        "Capture a live snapshot of a specific monitor by zero-based numeric index (e.g. screen://monitor/0)".to_string(),
                    ),
                    mime_type: Some("image/png".to_string()),
                },
                ResourceTemplateDefinition {
                    uri_template: "screen://window/{id}".to_string(),
                    name: "Application Window Snapshot".to_string(),
                    description: Some(
                        "Capture a live snapshot of an application window by unique numeric ID (e.g. screen://window/1234)".to_string(),
                    ),
                    mime_type: Some("image/png".to_string()),
                },
            ];

            let res = JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: ResourceTemplatesListResult {
                    resource_templates: templates,
                },
            };
            serde_json::to_string(&res).ok()
        }
        "resources/read" => {
            let uri = req.params.get("uri").and_then(|v| v.as_str());
            let uri = match uri {
                Some(u) => u,
                None => {
                    let err = JsonRpcErrorResponse {
                        jsonrpc: "2.0",
                        id,
                        error: JsonRpcError {
                            code: -32602,
                            message: "Missing 'uri' parameter in resources/read request"
                                .to_string(),
                            data: None,
                        },
                    };
                    return serde_json::to_string(&err).ok();
                }
            };

            match read_resource(uri) {
                Ok(read_result) => {
                    let res = JsonRpcResponse {
                        jsonrpc: "2.0",
                        id,
                        result: read_result,
                    };
                    serde_json::to_string(&res).ok()
                }
                Err(err) => {
                    let err_res = JsonRpcErrorResponse {
                        jsonrpc: "2.0",
                        id,
                        error: err,
                    };
                    serde_json::to_string(&err_res).ok()
                }
            }
        }
        _ => {
            let err = JsonRpcErrorResponse {
                jsonrpc: "2.0",
                id,
                error: JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", req.method),
                    data: None,
                },
            };
            serde_json::to_string(&err).ok()
        }
    }
}

fn read_resource(uri: &str) -> Result<ResourceReadResult, JsonRpcError> {
    if uri == "screen://current" || uri == "screen://primary" {
        let image =
            capture::capture_screen(None, None, CoordMode::Logical).map_err(|e| JsonRpcError {
                code: -32000,
                message: format!("Failed to capture current screen for '{uri}': {e}"),
                data: None,
            })?;
        let bytes = output::encode_image_bytes(&image, ImageFormat::Png, None).map_err(|e| {
            JsonRpcError {
                code: -32000,
                message: format!("Failed to encode image to PNG: {e}"),
                data: None,
            }
        })?;
        let blob = base64_encode(&bytes);
        Ok(ResourceReadResult {
            contents: vec![ResourceContent::Blob {
                uri: uri.to_string(),
                mime_type: Some("image/png".to_string()),
                blob,
            }],
        })
    } else if uri == "screen://all" || uri == "screen://panorama" {
        let image =
            capture::capture_all_monitors(None, CoordMode::Logical).map_err(|e| JsonRpcError {
                code: -32000,
                message: format!("Failed to capture all monitors panorama for '{uri}': {e}"),
                data: None,
            })?;
        let bytes = output::encode_image_bytes(&image, ImageFormat::Png, None).map_err(|e| {
            JsonRpcError {
                code: -32000,
                message: format!("Failed to encode image to PNG: {e}"),
                data: None,
            }
        })?;
        let blob = base64_encode(&bytes);
        Ok(ResourceReadResult {
            contents: vec![ResourceContent::Blob {
                uri: uri.to_string(),
                mime_type: Some("image/png".to_string()),
                blob,
            }],
        })
    } else if let Some(idx_str) = uri.strip_prefix("screen://monitor/") {
        let monitor_idx: usize = idx_str.trim().parse().map_err(|_| JsonRpcError {
            code: -32602,
            message: format!("Invalid monitor index '{idx_str}' in URI '{uri}'"),
            data: None,
        })?;
        let image =
            capture::capture_screen(Some(monitor_idx), None, CoordMode::Logical).map_err(|e| {
                JsonRpcError {
                    code: -32000,
                    message: format!("Failed to capture monitor #{monitor_idx} for '{uri}': {e}"),
                    data: None,
                }
            })?;
        let bytes = output::encode_image_bytes(&image, ImageFormat::Png, None).map_err(|e| {
            JsonRpcError {
                code: -32000,
                message: format!("Failed to encode image to PNG: {e}"),
                data: None,
            }
        })?;
        let blob = base64_encode(&bytes);
        Ok(ResourceReadResult {
            contents: vec![ResourceContent::Blob {
                uri: uri.to_string(),
                mime_type: Some("image/png".to_string()),
                blob,
            }],
        })
    } else if let Some(win_id_str) = uri.strip_prefix("screen://window/") {
        let image = capture::capture_window(win_id_str).map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Failed to capture window for '{uri}': {e}"),
            data: None,
        })?;
        let bytes = output::encode_image_bytes(&image, ImageFormat::Png, None).map_err(|e| {
            JsonRpcError {
                code: -32000,
                message: format!("Failed to encode image to PNG: {e}"),
                data: None,
            }
        })?;
        let blob = base64_encode(&bytes);
        Ok(ResourceReadResult {
            contents: vec![ResourceContent::Blob {
                uri: uri.to_string(),
                mime_type: Some("image/png".to_string()),
                blob,
            }],
        })
    } else if uri == "screen://diagnostics" || uri == "screen://doctor" {
        let report = doctor::run_doctor();
        let text = serde_json::to_string_pretty(&report).map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Failed to serialize diagnostics report: {e}"),
            data: None,
        })?;
        Ok(ResourceReadResult {
            contents: vec![ResourceContent::Text {
                uri: uri.to_string(),
                mime_type: Some("application/json".to_string()),
                text,
            }],
        })
    } else {
        Err(JsonRpcError {
            code: -32002,
            message: format!("Resource not found: '{uri}'"),
            data: None,
        })
    }
}

fn execute_tool_call(name: &str, arguments: &serde_json::Value) -> ToolCallResult {
    match name {
        "list_monitors" => match capture::get_monitor_infos() {
            Ok(monitors) => match serde_json::to_string_pretty(&monitors) {
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
            },
            Err(e) => ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to list monitors: {e}"),
                }],
                is_error: true,
            },
        },
        "list_windows" => match capture::get_window_infos() {
            Ok(windows) => match serde_json::to_string_pretty(&windows) {
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
            },
            Err(e) => ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to list windows: {e}"),
                }],
                is_error: true,
            },
        },
        "doctor" => {
            let report = doctor::run_doctor();
            match serde_json::to_string_pretty(&report) {
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
        "capture_screen" => execute_capture_tool(arguments),
        _ => ToolCallResult {
            content: vec![ToolContent::Text {
                text: format!("Unknown tool name: '{name}'"),
            }],
            is_error: true,
        },
    }
}

fn execute_capture_tool(arguments: &serde_json::Value) -> ToolCallResult {
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
                    let reg = match reg_str.parse::<Region>() {
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
                    manual_masks.push(crate::cli::MaskRegion {
                        region: reg,
                        effect,
                    });
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
                        text: format!("All-monitors capture failed: {e}"),
                    }],
                    is_error: true,
                };
            }
        }
    } else if let Some(win_query) = arguments.get("window").and_then(|v| v.as_str()) {
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
            match image::open(trimmed) {
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

/// Run Model Context Protocol (MCP) server loop over standard I/O streams.
pub fn run_mcp_loop(reader: impl BufRead, mut writer: impl Write) -> std::io::Result<()> {
    info!("Starting MCP stdio server loop");
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::UnexpectedEof
                    || e.kind() == std::io::ErrorKind::BrokenPipe
                {
                    break;
                }
                error!("Error reading line from MCP stdin: {e}");
                return Err(e);
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        if let Some(response_json) = handle_mcp_message(&line) {
            if let Err(e) = writeln!(writer, "{response_json}") {
                if e.kind() == std::io::ErrorKind::BrokenPipe {
                    debug!("Downstream MCP client closed pipe (BrokenPipe)");
                    break;
                }
                return Err(e);
            }
            if let Err(e) = writer.flush() {
                if e.kind() == std::io::ErrorKind::BrokenPipe {
                    debug!("Downstream MCP client closed pipe during flush (BrokenPipe)");
                    break;
                }
                return Err(e);
            }
        }
    }
    info!("Exiting MCP stdio server loop");
    Ok(())
}

/// Run standard Model Context Protocol (MCP) server over stdin and stdout.
pub fn run_mcp_server() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    run_mcp_loop(stdin.lock(), stdout.lock())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_encode_rfc_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn test_base64_decode_rfc_vectors() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYg==").unwrap(), b"foob");
        assert_eq!(base64_decode("Zm9vYmE=").unwrap(), b"fooba");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        assert!(base64_decode("!@#$%").is_err());
    }

    #[test]
    fn test_handle_mcp_initialize() {
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""protocolVersion":"2024-11-05""#));
        assert!(resp.contains(r#""name":"rgc-cli""#));
    }

    #[test]
    fn test_handle_mcp_ping() {
        let req = r#"{"jsonrpc":"2.0","id":"ping-1","method":"ping"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""id":"ping-1""#));
        assert!(resp.contains(r#""result":{}"#));
    }

    #[test]
    fn test_handle_mcp_tools_list() {
        let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains("list_monitors"));
        assert!(resp.contains("capture_screen"));
        assert!(resp.contains("doctor"));
    }

    #[test]
    fn test_handle_mcp_tool_call_doctor() {
        let req = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"doctor","arguments":{}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":false"#));
        assert!(
            resp.contains("Operating System")
                || resp.contains("supportedFormats")
                || resp.contains("backend")
        );
    }

    #[test]
    fn test_handle_mcp_tool_call_unknown() {
        let req = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"non_existent_tool","arguments":{}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Unknown tool name"));
    }

    #[test]
    fn test_handle_mcp_method_not_found() {
        let req = r#"{"jsonrpc":"2.0","id":5,"method":"unknown_method"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""code":-32601"#));
        assert!(resp.contains("Method not found"));
    }

    #[test]
    fn test_handle_mcp_invalid_version() {
        let req = r#"{"jsonrpc":"1.0","id":6,"method":"ping"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""code":-32600"#));
        assert!(resp.contains("jsonrpc must be '2.0'"));
    }

    #[test]
    fn test_handle_mcp_tool_call_invalid_region() {
        let req = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"capture_screen","arguments":{"region":"invalid"}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Invalid region"));

        let req_obj = r#"{"jsonrpc":"2.0","id":71,"method":"tools/call","params":{"name":"capture_screen","arguments":{"region":{"x":0,"y":0}}}}"#;
        let resp_obj = handle_mcp_message(req_obj).expect("response expected");
        assert!(resp_obj.contains(r#""isError":true"#));
        assert!(resp_obj.contains("Invalid region parameter"));
    }

    #[test]
    fn test_handle_mcp_tool_call_invalid_monitor() {
        let req = r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"capture_screen","arguments":{"monitor":99999}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Capture failed"));

        // Monitor as numeric string is parsed
        let req_str = r#"{"jsonrpc":"2.0","id":81,"method":"tools/call","params":{"name":"capture_screen","arguments":{"monitor":"99999"}}}"#;
        let resp_str = handle_mcp_message(req_str).expect("response expected");
        assert!(resp_str.contains(r#""isError":true"#));
        assert!(resp_str.contains("Capture failed"));

        // Non-numeric monitor string returns validation error
        let req_bad_str = r#"{"jsonrpc":"2.0","id":82,"method":"tools/call","params":{"name":"capture_screen","arguments":{"monitor":"abc"}}}"#;
        let resp_bad_str = handle_mcp_message(req_bad_str).expect("response expected");
        assert!(resp_bad_str.contains(r#""isError":true"#));
        assert!(resp_bad_str.contains("expected a non-negative integer"));
    }

    #[test]
    fn test_handle_mcp_tool_call_no_output_or_base64() {
        let req = r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"capture_screen","arguments":{"returnBase64":false}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(
            resp.contains("either 'outputPath' must be provided or 'returnBase64' must be true")
        );
    }

    #[test]
    fn test_handle_mcp_tool_call_invalid_scale_quality() {
        let req_scale = r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"capture_screen","arguments":{"scale":1.5}}}"#;
        let resp_scale = handle_mcp_message(req_scale).expect("response expected");
        assert!(resp_scale.contains(r#""isError":true"#));
        assert!(resp_scale.contains("Invalid scale"));

        let req_dim = r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"capture_screen","arguments":{"maxDimension":0}}}"#;
        let resp_dim = handle_mcp_message(req_dim).expect("response expected");
        assert!(resp_dim.contains(r#""isError":true"#));
        assert!(resp_dim.contains("Invalid maxDimension"));

        let req_quality = r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"capture_screen","arguments":{"quality":105}}}"#;
        let resp_quality = handle_mcp_message(req_quality).expect("response expected");
        assert!(resp_quality.contains(r#""isError":true"#));
        assert!(resp_quality.contains("Invalid quality"));
    }

    #[test]
    fn test_handle_mcp_resources_list() {
        let req = r#"{"jsonrpc":"2.0","id":20,"method":"resources/list"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains("screen://current"));
        assert!(resp.contains("screen://diagnostics"));
        assert!(resp.contains("image/png"));
        assert!(resp.contains("application/json"));
    }

    #[test]
    fn test_handle_mcp_resources_templates_list() {
        let req = r#"{"jsonrpc":"2.0","id":21,"method":"resources/templates/list"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains("screen://monitor/{index}"));
        assert!(resp.contains("image/png"));
    }

    #[test]
    fn test_handle_mcp_resources_read_diagnostics() {
        let req = r#"{"jsonrpc":"2.0","id":22,"method":"resources/read","params":{"uri":"screen://diagnostics"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""uri":"screen://diagnostics""#));
        assert!(resp.contains(r#""mimeType":"application/json""#));
        assert!(
            resp.contains("supportedFormats")
                || resp.contains("operatingSystem")
                || resp.contains("backend")
        );
    }

    #[test]
    fn test_handle_mcp_resources_read_missing_uri() {
        let req = r#"{"jsonrpc":"2.0","id":23,"method":"resources/read","params":{}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""code":-32602"#));
        assert!(resp.contains("Missing 'uri' parameter"));
    }

    #[test]
    fn test_handle_mcp_resources_read_not_found() {
        let req = r#"{"jsonrpc":"2.0","id":24,"method":"resources/read","params":{"uri":"unknown://resource"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""code":-32002"#));
        assert!(resp.contains("Resource not found"));
    }

    #[test]
    fn test_handle_mcp_resources_read_invalid_monitor() {
        let req = r#"{"jsonrpc":"2.0","id":25,"method":"resources/read","params":{"uri":"screen://monitor/not_a_number"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""code":-32602"#));
        assert!(resp.contains("Invalid monitor index"));
    }

    #[test]
    fn test_handle_mcp_tools_list_has_windows() {
        let req = r#"{"jsonrpc":"2.0","id":26,"method":"tools/list"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains("list_windows"));
        assert!(resp.contains("window"));
    }

    #[test]
    fn test_handle_mcp_tool_call_list_windows() {
        let req = r#"{"jsonrpc":"2.0","id":27,"method":"tools/call","params":{"name":"list_windows","arguments":{}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""id":27"#));
    }

    #[test]
    fn test_handle_mcp_tool_call_capture_window_nonexistent() {
        let req = r#"{"jsonrpc":"2.0","id":28,"method":"tools/call","params":{"name":"capture_screen","arguments":{"window":"nonexistent_window_123456"}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Window capture failed") || resp.contains("No window found"));
    }

    #[test]
    fn test_handle_mcp_resources_templates_list_has_window() {
        let req = r#"{"jsonrpc":"2.0","id":29,"method":"resources/templates/list"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains("screen://window/{id}"));
    }

    #[test]
    fn test_handle_mcp_resources_read_window_nonexistent() {
        let req = r#"{"jsonrpc":"2.0","id":30,"method":"resources/read","params":{"uri":"screen://window/9999999"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""code":-32000"#));
        assert!(resp.contains("Failed to capture window"));
    }

    #[test]
    fn test_handle_mcp_tools_list_has_redactions_and_masks() {
        let req = r#"{"jsonrpc":"2.0","id":31,"method":"tools/list"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains("redactions"));
        assert!(resp.contains("maskWindows"));
        assert!(resp.contains("allMonitors"));
        assert!(resp.contains("grid"));
        assert!(resp.contains("diffWith"));
        assert!(resp.contains("diffThreshold"));
        assert!(resp.contains("cropChanged"));
    }

    #[test]
    fn test_handle_mcp_tool_call_invalid_diff_with() {
        let req = r#"{"jsonrpc":"2.0","id":37,"method":"tools/call","params":{"name":"capture_screen","arguments":{"diffWith":"non_existent_file_path_123456.png"}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Failed to open diff reference image"));
    }

    #[test]
    fn test_handle_mcp_resources_list_has_all_monitors() {
        let req = r#"{"jsonrpc":"2.0","id":35,"method":"resources/list"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains("screen://all"));
    }

    #[test]
    fn test_handle_mcp_resources_read_all_monitors_safe() {
        let req = r#"{"jsonrpc":"2.0","id":36,"method":"resources/read","params":{"uri":"screen://all"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        // In headless CI it may return code -32000 (No monitors found), in desktop session it returns blob
        assert!(resp.contains(r#""id":36"#));
    }

    #[test]
    fn test_handle_mcp_tool_call_invalid_redaction() {
        let req = r#"{"jsonrpc":"2.0","id":32,"method":"tools/call","params":{"name":"capture_screen","arguments":{"redactions":["invalid_region_str"]}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Invalid redaction"));

        let req_obj = r#"{"jsonrpc":"2.0","id":33,"method":"tools/call","params":{"name":"capture_screen","arguments":{"redactions":[{"effect":"blur"}]}}}"#;
        let resp_obj = handle_mcp_message(req_obj).expect("response expected");
        assert!(resp_obj.contains(r#""isError":true"#));
        assert!(resp_obj.contains("missing 'region' field"));
    }

    #[test]
    fn test_handle_mcp_tool_call_invalid_mask_windows() {
        let req = r#"{"jsonrpc":"2.0","id":34,"method":"tools/call","params":{"name":"capture_screen","arguments":{"maskWindows":["   "]}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Invalid maskWindows entry"));
    }

    #[test]
    fn test_mcp_loop_execution() {
        let input = "{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"ping\"}\n";
        let mut output = Vec::new();
        run_mcp_loop(input.as_bytes(), &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains(r#""id":10"#));
    }
}
