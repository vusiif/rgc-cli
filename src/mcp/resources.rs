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

fn capture_and_sanitize_resource(
    target: capture::CaptureTarget,
    context: &str,
) -> Result<image::DynamicImage, JsonRpcError> {
    let captured =
        capture::capture_unified(target, None, CoordMode::Logical).map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Failed to capture {context}: {e}"),
            data: None,
        })?;

    let cfg = crate::config::get_active_config();
    let privacy_policy = crate::config::PrivacyPolicy::from_config(&cfg.privacy);

    let pipeline_opts = crate::pipeline::PipelineOptions {
        manual_masks: privacy_policy.mask_regions,
        mask_windows: privacy_policy.mask_windows,
        exclude_windows: privacy_policy.exclude_windows,
        allow_missing_mask: privacy_policy.allow_missing_mask,
        coords_mode: CoordMode::Logical,
        ..Default::default()
    };

    let res = crate::pipeline::execute_image_pipeline(
        captured.image,
        captured.scale_factor,
        captured.crop_offset,
        Some(captured.origin),
        &pipeline_opts,
    )
    .map_err(|e| JsonRpcError {
        code: -32000,
        message: format!(
            "Privacy redaction failed on resource '{context}': {}",
            e.message
        ),
        data: None,
    })?;

    Ok(res.image)
}

pub fn read_resource(uri: &str) -> Result<ResourceReadResult, JsonRpcError> {
    if uri == "screen://current" || uri == "screen://primary" {
        let image = capture_and_sanitize_resource(
            capture::CaptureTarget::Monitor { index: None },
            &format!("current screen for '{uri}'"),
        )?;
        image_to_blob_resource(uri, &image)
    } else if uri == "screen://all" || uri == "screen://panorama" {
        let image = capture_and_sanitize_resource(
            capture::CaptureTarget::AllMonitors,
            &format!("all monitors panorama for '{uri}'"),
        )?;
        image_to_blob_resource(uri, &image)
    } else if let Some(idx_str) = uri.strip_prefix("screen://monitor/") {
        let monitor_idx: usize = idx_str.trim().parse().map_err(|_| JsonRpcError {
            code: -32602,
            message: format!("Invalid monitor index '{idx_str}' in URI '{uri}'"),
            data: None,
        })?;
        let image = capture_and_sanitize_resource(
            capture::CaptureTarget::Monitor {
                index: Some(monitor_idx),
            },
            &format!("monitor #{monitor_idx} for '{uri}'"),
        )?;
        image_to_blob_resource(uri, &image)
    } else if let Some(win_id_str) = uri.strip_prefix("screen://window/") {
        let image = capture_and_sanitize_resource(
            capture::CaptureTarget::Window {
                query: win_id_str,
                client_only: false,
            },
            &format!("window for '{uri}'"),
        )?;
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

/// Compute a lightweight state fingerprint for a resource URI to determine whether its content has changed.
pub fn compute_resource_fingerprint(uri: &str) -> Option<u64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    if uri == "screen://diagnostics" || uri == "screen://doctor" {
        let report = doctor::run_doctor();
        if let Ok(json) = serde_json::to_string(&report) {
            let mut hasher = DefaultHasher::new();
            json.hash(&mut hasher);
            return Some(hasher.finish());
        }
        return None;
    }

    if uri == "screen://current" || uri == "screen://primary" {
        if let Ok(img) =
            capture_and_sanitize_resource(capture::CaptureTarget::Monitor { index: None }, uri)
        {
            return Some(hash_image_sample(&img));
        }
    } else if uri == "screen://all" || uri == "screen://panorama" {
        if let Ok(img) = capture_and_sanitize_resource(capture::CaptureTarget::AllMonitors, uri) {
            return Some(hash_image_sample(&img));
        }
    } else if let Some(idx_str) = uri.strip_prefix("screen://monitor/") {
        if let Ok(idx) = idx_str.trim().parse::<usize>() {
            if let Ok(img) = capture_and_sanitize_resource(
                capture::CaptureTarget::Monitor { index: Some(idx) },
                uri,
            ) {
                return Some(hash_image_sample(&img));
            }
        }
    } else if let Some(win_id_str) = uri.strip_prefix("screen://window/") {
        if let Ok(img) = capture_and_sanitize_resource(
            capture::CaptureTarget::Window {
                query: win_id_str,
                client_only: false,
            },
            uri,
        ) {
            return Some(hash_image_sample(&img));
        }
    }

    None
}

/// Compute a fast sample hash across the image buffer (dimensions + sampled pixels).
fn hash_image_sample(img: &image::DynamicImage) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    let (w, h) = (img.width(), img.height());
    w.hash(&mut hasher);
    h.hash(&mut hasher);

    let raw = img.as_bytes();
    let step = (raw.len() / 512).max(1);
    for chunk in raw.chunks(step) {
        if let Some(&byte) = chunk.first() {
            byte.hash(&mut hasher);
        }
    }

    hasher.finish()
}
