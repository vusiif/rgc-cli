use std::collections::HashSet;
use std::sync::Mutex;

use crate::capture;
use crate::cli::{CoordMode, ImageFormat};
use crate::doctor;
use crate::mcp::base64::base64_encode;
use crate::mcp::types::{JsonRpcError, JsonRpcNotification, ResourceContent, ResourceReadResult};
use crate::output;

static SUBSCRIPTIONS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Retrieve all currently subscribed resource URIs.
pub fn get_subscriptions() -> Vec<String> {
    let mut guard = SUBSCRIPTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let set = guard.get_or_insert_with(HashSet::new);
    set.iter().cloned().collect()
}

/// Subscribe a resource URI for live change notifications.
pub fn subscribe_resource(uri: &str) -> bool {
    let mut guard = SUBSCRIPTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let set = guard.get_or_insert_with(HashSet::new);
    set.insert(uri.to_string())
}

/// Unsubscribe a resource URI from live change notifications.
pub fn unsubscribe_resource(uri: &str) -> bool {
    let mut guard = SUBSCRIPTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let set = guard.get_or_insert_with(HashSet::new);
    set.remove(uri)
}

/// Clear all active resource subscriptions.
pub fn clear_subscriptions() {
    let mut guard = SUBSCRIPTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let set = guard.get_or_insert_with(HashSet::new);
    set.clear();
}

/// Create a JSON-RPC notification for an updated resource.
pub fn create_resource_updated_notification(uri: &str) -> String {
    let notif = JsonRpcNotification {
        jsonrpc: "2.0",
        method: "notifications/resources/updated".to_string(),
        params: Some(serde_json::json!({ "uri": uri })),
    };
    serde_json::to_string(&notif).unwrap_or_default()
}

/// Create a JSON-RPC notification when the list of available resources has changed.
pub fn create_resource_list_changed_notification() -> String {
    let notif = JsonRpcNotification {
        jsonrpc: "2.0",
        method: "notifications/resources/list_changed".to_string(),
        params: None::<serde_json::Value>,
    };
    serde_json::to_string(&notif).unwrap_or_default()
}

pub fn image_to_blob_resource(
    uri: &str,
    image: &image::DynamicImage,
) -> Result<ResourceReadResult, JsonRpcError> {
    let bytes =
        output::encode_image_bytes(image, ImageFormat::Png, None).map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Failed to encode image to PNG: {e}"),
            data: None,
        })?;
    let blob = base64_encode(&bytes);
    Ok(ResourceReadResult {
        contents: vec![ResourceContent::Blob {
            uri: uri.to_string(),
            mime_type: Some("image/png".to_string()),
            blob,
        }],
    })
}

pub fn read_resource(uri: &str) -> Result<ResourceReadResult, JsonRpcError> {
    if uri == "screen://current" || uri == "screen://primary" {
        let image =
            capture::capture_screen(None, None, CoordMode::Logical).map_err(|e| JsonRpcError {
                code: -32000,
                message: format!("Failed to capture current screen for '{uri}': {e}"),
                data: None,
            })?;
        image_to_blob_resource(uri, &image)
    } else if uri == "screen://all" || uri == "screen://panorama" {
        let image =
            capture::capture_all_monitors(None, CoordMode::Logical).map_err(|e| JsonRpcError {
                code: -32000,
                message: format!("Failed to capture all monitors panorama for '{uri}': {e}"),
                data: None,
            })?;
        image_to_blob_resource(uri, &image)
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
        image_to_blob_resource(uri, &image)
    } else if let Some(win_id_str) = uri.strip_prefix("screen://window/") {
        let image = capture::capture_window(win_id_str).map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Failed to capture window for '{uri}': {e}"),
            data: None,
        })?;
        image_to_blob_resource(uri, &image)
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
