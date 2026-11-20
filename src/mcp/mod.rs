pub mod base64;
pub mod resources;
pub mod tools;
pub mod types;

pub use base64::*;
pub use resources::*;
pub use tools::*;
pub use types::*;

use log::{debug, error, info};
use std::io::{BufRead, Write};

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
            let tools = get_tool_definitions();
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

            if let Ok(monitors) = crate::capture::get_monitor_infos() {
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

            if let Ok(windows) = crate::capture::get_window_infos() {
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
    fn test_handle_mcp_tool_call_invalid_region() {
        let req = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"capture_screen","arguments":{"region":"bad_region"}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Invalid region"));
    }

    #[test]
    fn test_handle_mcp_tool_call_no_output_or_base64() {
        let req = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"capture_screen","arguments":{"returnBase64":false}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Invalid capture request"));
    }

    #[test]
    fn test_handle_mcp_tool_call_doctor() {
        let req = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"doctor"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains("monitorsDetected"));
        assert!(resp.contains("supportedFormats"));
    }

    #[test]
    fn test_handle_mcp_tool_call_list_windows() {
        let req =
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list_windows"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""id":6"#));
    }

    #[test]
    fn test_handle_mcp_tool_call_unknown() {
        let req = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"non_existent_tool"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Unknown tool name"));
    }

    #[test]
    fn test_handle_mcp_tool_call_invalid_monitor() {
        let req = r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"capture_screen","arguments":{"monitor":"not_a_number"}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Invalid monitor index"));
    }

    #[test]
    fn test_handle_mcp_tool_call_invalid_scale_quality() {
        let req1 = r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"capture_screen","arguments":{"scale":1.5}}}"#;
        let resp1 = handle_mcp_message(req1).expect("response expected");
        assert!(resp1.contains(r#""isError":true"#));
        assert!(resp1.contains("Invalid scale"));

        let req2 = r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"capture_screen","arguments":{"quality":0}}}"#;
        let resp2 = handle_mcp_message(req2).expect("response expected");
        assert!(resp2.contains(r#""isError":true"#));
        assert!(resp2.contains("Invalid quality"));

        let req3 = r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"capture_screen","arguments":{"maxDimension":0}}}"#;
        let resp3 = handle_mcp_message(req3).expect("response expected");
        assert!(resp3.contains(r#""isError":true"#));
        assert!(resp3.contains("Invalid maxDimension"));
    }

    #[test]
    fn test_handle_mcp_tool_call_capture_window_nonexistent() {
        let req = r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"capture_screen","arguments":{"window":"nonexistent_title_xyz_12345"}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Window capture failed") || resp.contains("No window found"));
    }

    #[test]
    fn test_handle_mcp_resources_list() {
        let req = r#"{"jsonrpc":"2.0","id":20,"method":"resources/list"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains("screen://current"));
        assert!(resp.contains("screen://diagnostics"));
    }

    #[test]
    fn test_handle_mcp_resources_templates_list() {
        let req = r#"{"jsonrpc":"2.0","id":21,"method":"resources/templates/list"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains("screen://monitor/{index}"));
    }

    #[test]
    fn test_handle_mcp_resources_read_not_found() {
        let req = r#"{"jsonrpc":"2.0","id":22,"method":"resources/read","params":{"uri":"screen://unknown"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""code":-32002"#));
    }

    #[test]
    fn test_handle_mcp_resources_read_missing_uri() {
        let req = r#"{"jsonrpc":"2.0","id":23,"method":"resources/read","params":{}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""code":-32602"#));
    }

    #[test]
    fn test_handle_mcp_resources_read_diagnostics() {
        let req = r#"{"jsonrpc":"2.0","id":24,"method":"resources/read","params":{"uri":"screen://diagnostics"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""mimeType":"application/json""#));
        assert!(resp.contains("monitorsDetected"));
    }

    #[test]
    fn test_handle_mcp_resources_read_invalid_monitor() {
        let req = r#"{"jsonrpc":"2.0","id":25,"method":"resources/read","params":{"uri":"screen://monitor/abc"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""code":-32602"#));
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
    fn test_handle_mcp_resources_list_has_all_monitors() {
        let req = r#"{"jsonrpc":"2.0","id":35,"method":"resources/list"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains("screen://all"));
    }

    #[test]
    fn test_handle_mcp_resources_read_all_monitors_safe() {
        let req = r#"{"jsonrpc":"2.0","id":36,"method":"resources/read","params":{"uri":"screen://all"}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
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
    fn test_handle_mcp_tool_call_invalid_diff_with() {
        let req = r#"{"jsonrpc":"2.0","id":37,"method":"tools/call","params":{"name":"capture_screen","arguments":{"diffWith":"non_existent_file_path_123456.png"}}}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""isError":true"#));
        assert!(resp.contains("Failed to open diff reference image"));
    }

    #[test]
    fn test_handle_mcp_invalid_version() {
        let req = r#"{"jsonrpc":"1.0","id":99,"method":"ping"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""code":-32600"#));
        assert!(resp.contains("Invalid Request: jsonrpc must be '2.0'"));
    }

    #[test]
    fn test_handle_mcp_method_not_found() {
        let req = r#"{"jsonrpc":"2.0","id":100,"method":"unknown_method"}"#;
        let resp = handle_mcp_message(req).expect("response expected");
        assert!(resp.contains(r#""code":-32601"#));
        assert!(resp.contains("Method not found"));
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
