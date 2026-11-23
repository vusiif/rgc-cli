#![forbid(unsafe_code)]

//! MCP tool implementation for localized action verification and feedback.

use std::thread::sleep;
use std::time::Duration;

use crate::action::{self, ClickMarker};
use crate::capture;
use crate::cli::CoordMode;
use crate::mcp::base64::base64_encode;
use crate::mcp::types::{ToolCallResult, ToolContent, ToolDefinition};
use crate::output::{self, ImageFormat};

/// Returns the MCP tool definition for `verify_action`.
pub fn get_verify_action_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "verify_action".to_string(),
        description: "Capture screen state before and after an interaction, verifying whether a localized visual UI change occurred within a radius around (x, y). Returns verification status, changed pixel stats, bounding box, and an annotated screenshot with click ripple feedback.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "x": {
                    "type": "integer",
                    "description": "Horizontal target coordinate (X) of the interaction."
                },
                "y": {
                    "type": "integer",
                    "description": "Vertical target coordinate (Y) of the interaction."
                },
                "radius": {
                    "type": "integer",
                    "description": "Search radius in pixels around (x, y) to check for changes (default: 100, 0 for full screen)."
                },
                "timeoutMs": {
                    "type": "integer",
                    "description": "Wait duration in milliseconds before sampling follow-up state (default: 500 ms)."
                },
                "threshold": {
                    "type": "integer",
                    "description": "Pixel difference threshold (1-255, default: 10)."
                },
                "monitor": {
                    "type": "integer",
                    "description": "0-indexed monitor index. If omitted and window is omitted, captures primary display."
                },
                "window": {
                    "type": "string",
                    "description": "Optional window ID or title substring to capture."
                },
                "allMonitors": {
                    "type": "boolean",
                    "description": "Optional flag to capture multi-monitor desktop panorama."
                },
                "outputPath": {
                    "type": "string",
                    "description": "Optional file path to save the annotated verification screenshot."
                },
                "returnBase64": {
                    "type": "boolean",
                    "description": "Whether to return the annotated screenshot as base64 image data in the response. Defaults to true if outputPath is omitted."
                }
            },
            "required": ["x", "y"],
            "additionalProperties": false
        }),
    }
}

fn capture_single_target(
    monitor: Option<usize>,
    window_query: Option<&str>,
    all_monitors: bool,
) -> Result<image::DynamicImage, String> {
    if all_monitors {
        capture::capture_all_monitors_with_details(None, CoordMode::Logical)
            .map(|c| c.image)
            .map_err(|e| format!("Panorama capture failed: {e}"))
    } else if let Some(win) = window_query {
        capture::capture_window_with_details(win, None)
            .map(|c| c.image)
            .map_err(|e| format!("Window capture failed: {e}"))
    } else {
        capture::capture_screen_with_details(monitor, None, CoordMode::Logical)
            .map(|c| c.image)
            .map_err(|e| format!("Screen capture failed: {e}"))
    }
}

/// Execute the `verify_action` MCP tool.
pub fn execute_verify_action_tool(arguments: &serde_json::Value) -> ToolCallResult {
    let target_x = match arguments.get("x").and_then(|v| v.as_u64()) {
        Some(x) => x as u32,
        None => {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: "Missing or invalid 'x' coordinate parameter".to_string(),
                }],
                is_error: true,
            };
        }
    };

    let target_y = match arguments.get("y").and_then(|v| v.as_u64()) {
        Some(y) => y as u32,
        None => {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: "Missing or invalid 'y' coordinate parameter".to_string(),
                }],
                is_error: true,
            };
        }
    };

    let radius = arguments
        .get("radius")
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as u32;

    let timeout_ms = arguments
        .get("timeoutMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(500)
        .clamp(10, 10000);

    let threshold = arguments
        .get("threshold")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
        .clamp(1, 255) as u8;

    let monitor = arguments
        .get("monitor")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let all_monitors = arguments
        .get("allMonitors")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let window_query = arguments.get("window").and_then(|v| v.as_str());

    let allow_overwrite = arguments
        .get("allowOverwrite")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let output_path = match arguments.get("outputPath").and_then(|v| v.as_str()) {
        Some(s) => match crate::mcp::tools::validate_mcp_output_path(s, &["png", "jpg", "jpeg", "webp"], allow_overwrite) {
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

    // 1. Capture baseline frame
    let before_img = match capture_single_target(monitor, window_query, all_monitors) {
        Ok(img) => img,
        Err(e) => {
            return ToolCallResult {
                content: vec![ToolContent::Text { text: e }],
                is_error: true,
            };
        }
    };

    // 2. Wait for UI interaction effect to take place
    sleep(Duration::from_millis(timeout_ms));

    // 3. Capture follow-up frame
    let after_img = match capture_single_target(monitor, window_query, all_monitors) {
        Ok(img) => img,
        Err(e) => {
            return ToolCallResult {
                content: vec![ToolContent::Text { text: e }],
                is_error: true,
            };
        }
    };

    // 4. Verify action changes around target coordinate
    let verification = action::verify_action_change(
        &before_img,
        &after_img,
        target_x,
        target_y,
        radius,
        threshold,
    );

    // 5. Draw visual click marker at target on the follow-up image
    let click_marker = ClickMarker {
        x: target_x,
        y: target_y,
        label: Some(
            if verification.verified {
                "ACTION OK"
            } else {
                "NO CHANGE"
            }
            .to_string(),
        ),
    };

    let annotated_img = action::draw_action_overlays(after_img, &[click_marker], &[]);

    // 6. Handle output file saving if requested
    if let Some(ref out_p) = output_path {
        let dest = output::OutputDestination::File(out_p.clone());
        if let Err(e) = output::save_image(&annotated_img, &dest, ImageFormat::Png, None) {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to save annotated screenshot to '{}': {e}", out_p.display()),
                }],
                is_error: true,
            };
        }
    }

    let mut content = Vec::new();

    // 7. Embed Base64 image if requested
    if return_base64 {
        let mut bytes = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut bytes);
        if let Err(e) = annotated_img.write_to(&mut cursor, image::ImageFormat::Png) {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to encode image to PNG: {e}"),
                }],
                is_error: true,
            };
        }

        let b64 = base64_encode(&bytes);
        content.push(ToolContent::Image {
            data: b64,
            mime_type: "image/png".to_string(),
        });
    }

    // 8. Add structured verification summary
    let summary_json = match serde_json::to_string_pretty(&verification) {
        Ok(j) => j,
        Err(_) => format!("{:?}", verification),
    };

    let status_str = if verification.verified {
        "SUCCESS: Visual change detected"
    } else {
        "FAILURE: No visual change detected"
    };

    let human_text = format!(
        "Action Verification Result: {status_str} at ({target_x}, {target_y}) within radius {radius}px\n\
         - Changed pixels: {} ({:.2}% of ROI)\n\
         - Bounding box: {:?}\n\n\
         JSON Result:\n{summary_json}",
        verification.changed_pixels, verification.change_percent, verification.bbox
    );

    content.push(ToolContent::Text { text: human_text });

    ToolCallResult {
        content,
        is_error: false,
    }
}
