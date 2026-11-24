#![forbid(unsafe_code)]

//! Stateful screen session management for AI Vision Agents.
//!
//! Maintains in-memory frame baselines across calls with automatic TTL eviction,
//! eliminating redundant image disk roundtrips and drastically reducing multimodal token consumption.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use image::DynamicImage;
use serde::{Deserialize, Serialize};

use crate::capture;
use crate::cli::CoordMode;
use crate::coords::CoordinateSpaceInfo;
use crate::mcp::base64::base64_encode;
use crate::mcp::tools::validate_mcp_output_path;
use crate::mcp::types::{ToolCallResult, ToolContent, ToolDefinition};
use crate::output::{self, OutputDestination};
use crate::pipeline::{execute_image_pipeline, PipelineOptions};
use crate::transform::DiffBBox;

/// Maximum duration an idle session remains valid in memory (15 minutes).
pub const SESSION_TTL: Duration = Duration::from_secs(15 * 60);

/// Maximum concurrent active visual sessions retained in memory.
pub const MAX_CONCURRENT_SESSIONS: usize = 16;

/// Maximum aggregate uncompressed cache size across all active visual sessions (256 MiB).
pub const MAX_SESSION_CACHE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub last_image: Arc<DynamicImage>,
    pub frame_index: u64,
    pub last_accessed: Instant,
    pub target_fingerprint: String,
}

#[derive(Clone, Default)]
pub struct SessionManager {
    sessions: Arc<Mutex<HashMap<String, SessionRecord>>>,
}

static GLOBAL_SESSIONS: OnceLock<SessionManager> = OnceLock::new();

pub fn get_session_manager() -> &'static SessionManager {
    GLOBAL_SESSIONS.get_or_init(SessionManager::new)
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn prune_expired(map: &mut HashMap<String, SessionRecord>) {
        let now = Instant::now();
        map.retain(|_, record| now.saturating_duration_since(record.last_accessed) < SESSION_TTL);
    }

    fn total_cache_bytes(map: &HashMap<String, SessionRecord>) -> usize {
        map.values().map(|r| r.last_image.as_bytes().len()).sum()
    }

    /// Prune expired sessions older than TTL and evict oldest if capacity or memory budget exceeded.
    pub fn cleanup(&self) {
        if let Ok(mut map) = self.sessions.lock() {
            Self::prune_expired(&mut map);
            while map.len() > MAX_CONCURRENT_SESSIONS
                || Self::total_cache_bytes(&map) > MAX_SESSION_CACHE_BYTES
            {
                if let Some(oldest_key) = map
                    .iter()
                    .min_by_key(|(_, r)| r.last_accessed)
                    .map(|(k, _)| k.clone())
                {
                    map.remove(&oldest_key);
                } else {
                    break;
                }
            }
        }
    }

    pub fn get(&self, session_id: &str) -> Option<SessionRecord> {
        let mut map = self.sessions.lock().ok()?;
        Self::prune_expired(&mut map);
        if let Some(record) = map.get_mut(session_id) {
            record.last_accessed = Instant::now();
            Some(record.clone())
        } else {
            None
        }
    }

    /// Update session baseline with standard `DynamicImage` (backward compatibility).
    pub fn update(&self, session_id: String, image: DynamicImage, frame_index: u64) {
        self.update_with_target(session_id, Arc::new(image), frame_index, String::new());
    }

    /// Update session baseline with `Arc<DynamicImage>` and explicit capture target fingerprint.
    pub fn update_with_target(
        &self,
        session_id: String,
        image: Arc<DynamicImage>,
        frame_index: u64,
        target_fingerprint: String,
    ) {
        if let Ok(mut map) = self.sessions.lock() {
            Self::prune_expired(&mut map);
            let img_bytes = image.as_bytes().len();
            // Evict if capacity or memory limit is exceeded
            while (!map.contains_key(&session_id) && map.len() >= MAX_CONCURRENT_SESSIONS)
                || (Self::total_cache_bytes(&map) + img_bytes > MAX_SESSION_CACHE_BYTES
                    && !map.is_empty())
            {
                if let Some(oldest_key) = map
                    .iter()
                    .min_by_key(|(_, r)| r.last_accessed)
                    .map(|(k, _)| k.clone())
                {
                    map.remove(&oldest_key);
                } else {
                    break;
                }
            }
            map.insert(
                session_id,
                SessionRecord {
                    last_image: image,
                    frame_index,
                    last_accessed: Instant::now(),
                    target_fingerprint,
                },
            );
        }
    }

    pub fn reset(&self, session_id: &str) {
        if let Ok(mut map) = self.sessions.lock() {
            map.remove(session_id);
        }
    }
}

/// Structured JSON summary returned by `capture_session`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponseJson {
    pub session_id: String,
    pub frame_index: u64,
    pub is_first_frame: bool,
    pub changed: bool,
    pub changed_pixels: u64,
    pub changed_percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<DiffBBox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tiles: Option<Vec<crate::transform::DiffTile>>,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<CoordinateSpaceInfo>,
}

/// Returns the MCP tool definition for `capture_session`.
pub fn get_session_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "capture_session".to_string(),
        description: "Stateful incremental screen capture session for AI Agents. Caches prior visual state in memory to detect changes and return cropped deltas, eliminating redundant disk I/O and saving vision tokens.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "sessionId": {
                    "type": "string",
                    "description": "Unique session identifier for the agent (e.g. 'agent-01'). Defaults to 'default'."
                },
                "reset": {
                    "type": "boolean",
                    "description": "Reset session history and establish a new baseline from current screen state. Defaults to false."
                },
                "diffOnly": {
                    "type": "boolean",
                    "description": "When true and visual change occurs, return cropped diff image delta instead of full screen to save vision tokens. If unchanged, omits heavy base64 data. Defaults to true."
                },
                "threshold": {
                    "type": "integer",
                    "description": "Per-pixel color delta threshold (0-255, default: 16) for change detection."
                },
                "monitor": {
                    "type": "integer",
                    "description": "0-indexed monitor index. Captures primary display if omitted."
                },
                "window": {
                    "type": "string",
                    "description": "Window title substring or ID to target."
                },
                "scale": {
                    "type": "number",
                    "description": "Downscale factor between 0.01 and 1.0 (e.g. 0.5)."
                },
                "maxDimension": {
                    "type": "integer",
                    "description": "Maximum width/height in pixels."
                },
                "cursor": {
                    "type": "boolean",
                    "description": "Render mouse cursor overlay on image."
                },
                "highlightCursor": {
                    "type": "boolean",
                    "description": "Render prominent cursor halo ring."
                },
                "som": {
                    "type": "boolean",
                    "description": "Detect interactive UI elements and overlay Set-of-Mark numbered badges."
                },
                "outputPath": {
                    "type": "string",
                    "description": "Optional file path to save output."
                },
                "allowOverwrite": {
                    "type": "boolean",
                    "description": "Allow overwriting existing file at outputPath."
                },
                "format": {
                    "type": "string",
                    "enum": ["png", "webp", "jpeg", "jpg"],
                    "description": "Image format ('png', 'webp', or 'jpeg'). Inferred from outputPath if omitted, defaulting to 'png'."
                },
                "redactions": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional privacy redaction regions: 'x,y,w,h[:effect]'."
                },
                "maskWindows": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional window titles or IDs to redact from frames."
                },
                "excludeWindows": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of window titles to blackout completely."
                },
                "allowMissingMask": {
                    "type": "boolean",
                    "description": "Optional flag to permit capture if a configured mask window is not found."
                },
                "tiles": {
                    "type": "boolean",
                    "description": "Optional flag to decompose visual change detection into spatial grid tiles."
                },
                "tileSize": {
                    "type": "integer",
                    "description": "Spatial grid tile size in pixels (16-512, default: 64) when 'tiles' is enabled."
                },
                "waitStableMs": {
                    "type": "integer",
                    "description": "Optional duration in milliseconds the screen must remain visually stable (no changes) before capturing."
                },
                "waitTimeoutMs": {
                    "type": "integer",
                    "description": "Optional timeout in milliseconds for visual stability waiting (default: 5000)."
                }
            },
            "additionalProperties": false
        }),
    }
}

/// Execute the `capture_session` MCP tool.
pub fn execute_capture_session_tool(arguments: &serde_json::Value) -> ToolCallResult {
    let session_id = {
        let raw = arguments
            .get("sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .trim();
        if raw.is_empty() {
            "default".to_string()
        } else {
            raw.chars().take(128).collect()
        }
    };

    let reset_flag = arguments
        .get("reset")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let diff_only = arguments
        .get("diffOnly")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let threshold = arguments
        .get("threshold")
        .and_then(|v| v.as_u64())
        .unwrap_or(16)
        .clamp(1, 255) as u8;

    let tiles_flag = arguments
        .get("tiles")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let tile_size = arguments
        .get("tileSize")
        .and_then(|v| v.as_u64())
        .unwrap_or(64)
        .clamp(16, 512) as u32;

    let monitor = arguments
        .get("monitor")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let window_query = arguments.get("window").and_then(|v| v.as_str());

    let scale = arguments.get("scale").and_then(|v| v.as_f64());
    if let Some(s) = scale {
        if !s.is_finite() || s <= 0.0 || s > 1.0 {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Invalid scale '{s}': expected a number between 0.01 and 1.0"),
                }],
                is_error: true,
            };
        }
    }

    let max_dimension = arguments
        .get("maxDimension")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
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

    let explicit_format = match arguments.get("format").and_then(|v| v.as_str()) {
        Some("webp") => Some(crate::cli::ImageFormat::Webp),
        Some("jpeg") | Some("jpg") => Some(crate::cli::ImageFormat::Jpeg),
        Some("png") => Some(crate::cli::ImageFormat::Png),
        _ => None,
    };

    let mut manual_masks = Vec::new();
    if let Some(arr) = arguments.get("redactions").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                if let Ok(mr) = s.parse::<crate::cli::MaskRegion>() {
                    manual_masks.push(mr);
                }
            }
        }
    }

    let mut window_masks = Vec::new();
    if let Some(arr) = arguments.get("maskWindows").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                if let Ok(mw) = s.parse::<crate::cli::MaskWindow>() {
                    window_masks.push(mw);
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

    let allow_overwrite = arguments
        .get("allowOverwrite")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let output_path = match arguments.get("outputPath").and_then(|v| v.as_str()) {
        Some(s) => {
            match validate_mcp_output_path(s, &["png", "webp", "jpeg", "jpg"], allow_overwrite) {
                Ok(p) => Some(p),
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

    // Step 1: Capture current screen target
    let target = if let Some(win) = window_query {
        capture::CaptureTarget::Window {
            query: win,
            client_only: false,
        }
    } else {
        capture::CaptureTarget::Monitor { index: monitor }
    };

    let wait_stable_ms = arguments
        .get("waitStableMs")
        .and_then(|v| v.as_u64())
        .filter(|&ms| ms > 0);
    let wait_timeout_ms = arguments
        .get("waitTimeoutMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(5000);

    let mut capture_once = || {
        capture::capture_unified(target.clone(), None, CoordMode::Logical)
            .map_err(|e| format!("Session capture failed: {e}"))
    };

    let c = if let Some(stable_ms) = wait_stable_ms {
        match capture::wait_for_visual_stability(
            &mut capture_once,
            16,
            std::time::Duration::from_millis(stable_ms),
            std::time::Duration::from_millis(wait_timeout_ms),
            std::time::Duration::from_millis(100),
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

    let raw_image = c.image;
    let scale_factor = c.scale_factor;
    let crop_offset = c.crop_offset;
    let monitor_origin = Some(c.origin);

    // Step 2: Execute base image pipeline (scale, cursor, privacy masks)
    let pipeline_opts = PipelineOptions {
        cursor: cursor_flag,
        highlight_cursor: highlight_cursor_flag,
        scale,
        max_dimension,
        som: som_flag,
        manual_masks: privacy_policy.mask_regions,
        mask_windows: privacy_policy.mask_windows,
        exclude_windows: privacy_policy.exclude_windows,
        allow_missing_mask: privacy_policy.allow_missing_mask,
        ..Default::default()
    };

    let pipeline_res = match execute_image_pipeline(
        raw_image,
        scale_factor,
        crop_offset,
        monitor_origin,
        &pipeline_opts,
    ) {
        Ok(r) => r,
        Err(e) => {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Pipeline execution error: {}", e.message),
                }],
                is_error: true,
            };
        }
    };

    let current_img = pipeline_res.image;
    let manager = get_session_manager();

    if reset_flag {
        manager.reset(&session_id);
    }

    let target_fingerprint = format!("{:?}:scale={:?}:max_dim={:?}", target, scale, max_dimension);

    let prior_session = manager.get(&session_id).and_then(|rec| {
        if !rec.target_fingerprint.is_empty() && rec.target_fingerprint != target_fingerprint {
            log::debug!(
                "Session '{}' target context changed from '{}' to '{}'. Resetting baseline.",
                session_id,
                rec.target_fingerprint,
                target_fingerprint
            );
            None
        } else {
            Some(rec)
        }
    });

    let (
        is_first_frame,
        frame_index,
        changed,
        changed_pixels,
        changed_percent,
        bbox,
        detected_tiles,
        output_img,
        diff_crop_offset,
        is_diff_cropped,
        pending_baseline_image,
    ) = match prior_session {
        None => (
            true,
            1u64,
            false,
            0u64,
            0.0f64,
            None,
            None,
            current_img.clone(),
            (0u32, 0u32),
            false,
            Arc::new(current_img),
        ),
        Some(record) => {
            let diff = output::calculate_image_diff(&current_img, &record.last_image, threshold);
            let next_frame_idx = record.frame_index + 1;

            if diff.changed && (diff.changed_pixels >= 50 || diff.changed_percent >= 0.05) {
                let tiles = if tiles_flag {
                    let tile_res = output::calculate_tile_diff(
                        &current_img,
                        &record.last_image,
                        threshold,
                        tile_size,
                    );
                    Some(tile_res.tiles)
                } else {
                    None
                };

                let (out_img, crop_off, is_cropped) = if diff_only {
                    if let Some(ref b) = diff.bbox {
                        let padding = 8u32;
                        let px = b.x.saturating_sub(padding);
                        let py = b.y.saturating_sub(padding);
                        (
                            output::crop_to_diff_bbox(current_img.clone(), b, padding),
                            (px, py),
                            true,
                        )
                    } else {
                        (current_img.clone(), (0, 0), false)
                    }
                } else {
                    (current_img.clone(), (0, 0), false)
                };

                (
                    false,
                    next_frame_idx,
                    true,
                    diff.changed_pixels,
                    diff.changed_percent,
                    diff.bbox,
                    tiles,
                    out_img,
                    crop_off,
                    is_cropped,
                    Arc::new(current_img),
                )
            } else {
                // No visual change: update access time, return unchanged notice without heavy image
                (
                    false,
                    next_frame_idx,
                    false,
                    diff.changed_pixels,
                    diff.changed_percent,
                    None,
                    None,
                    current_img,
                    (0u32, 0u32),
                    false,
                    Arc::clone(&record.last_image),
                )
            }
        }
    };

    let final_transform = if is_diff_cropped {
        crate::coords::CoordinateTransform::new(
            pipeline_res.transform.screen_origin,
            pipeline_res.transform.screen_scale_factor,
            pipeline_res.transform.crop_offset,
            pipeline_res.transform.image_scale,
            (
                pipeline_res.transform.diff_crop_offset.0 + diff_crop_offset.0,
                pipeline_res.transform.diff_crop_offset.1 + diff_crop_offset.1,
            ),
            pipeline_res.transform.canvas_size,
            (output_img.width(), output_img.height()),
        )
    } else {
        pipeline_res.transform
    };

    let mut content = Vec::new();

    let response_json = SessionResponseJson {
        session_id: session_id.clone(),
        frame_index,
        is_first_frame,
        changed,
        changed_pixels,
        changed_percent,
        bbox,
        tiles: detected_tiles,
        width: output_img.width(),
        height: output_img.height(),
        coordinates: Some(final_transform.to_info()),
    };

    if let Ok(json_text) = serde_json::to_string_pretty(&response_json) {
        content.push(ToolContent::Text {
            text: format!("Session State:\n{json_text}"),
        });
    }

    let remapped_elements: Option<Vec<crate::api::UiElement>> =
        pipeline_res.elements.map(|elems| {
            if is_diff_cropped {
                let crop_x = diff_crop_offset.0;
                let crop_y = diff_crop_offset.1;
                let crop_w = output_img.width();
                let crop_h = output_img.height();

                elems
                    .into_iter()
                    .filter_map(|mut el| {
                        let ex2 = el.x.saturating_add(el.width);
                        let ey2 = el.y.saturating_add(el.height);
                        let cx2 = crop_x.saturating_add(crop_w);
                        let cy2 = crop_y.saturating_add(crop_h);

                        // Exclude elements completely outside the cropped bounding box
                        if ex2 <= crop_x || el.x >= cx2 || ey2 <= crop_y || el.y >= cy2 {
                            return None;
                        }

                        let new_x1 = el.x.max(crop_x);
                        let new_y1 = el.y.max(crop_y);
                        let new_x2 = ex2.min(cx2);
                        let new_y2 = ey2.min(cy2);

                        el.x = new_x1.saturating_sub(crop_x);
                        el.y = new_y1.saturating_sub(crop_y);
                        el.width = new_x2.saturating_sub(new_x1);
                        el.height = new_y2.saturating_sub(new_y1);
                        el.center_x = el.x + el.width / 2;
                        el.center_y = el.y + el.height / 2;
                        Some(el)
                    })
                    .collect()
            } else {
                elems
            }
        });

    if let Some(ref elems) = remapped_elements {
        if let Ok(elems_json) = serde_json::to_string_pretty(elems) {
            content.push(ToolContent::Text {
                text: format!("Detected UI Elements:\n{elems_json}"),
            });
        }
    }

    let format = match &output_path {
        Some(path) => output::validate_and_infer_format(
            &OutputDestination::File(path.clone()),
            explicit_format,
        )
        .unwrap_or(explicit_format.unwrap_or(crate::cli::ImageFormat::Png)),
        None => explicit_format.unwrap_or(crate::cli::ImageFormat::Png),
    };

    // Save to disk if requested
    if let Some(ref path) = output_path {
        let destination = OutputDestination::File(path.clone());
        if let Err(e) = output::save_image(&output_img, &destination, format, None) {
            return ToolCallResult {
                content: vec![ToolContent::Text {
                    text: format!("Failed to save session image: {e}"),
                }],
                is_error: true,
            };
        }
    }

    // Attach base64 image: on first frame, or on visual changes, or when diffOnly is false
    if is_first_frame || changed || !diff_only {
        match output::encode_image_bytes(&output_img, format, None) {
            Ok(bytes) => {
                let b64 = base64_encode(&bytes);
                let mime_type = match format {
                    crate::cli::ImageFormat::Webp => "image/webp",
                    crate::cli::ImageFormat::Jpeg => "image/jpeg",
                    _ => "image/png",
                };
                content.push(ToolContent::Image {
                    data: b64,
                    mime_type: mime_type.to_string(),
                });
            }
            Err(e) => {
                return ToolCallResult {
                    content: vec![ToolContent::Text {
                        text: format!("Failed to encode session image: {e}"),
                    }],
                    is_error: true,
                };
            }
        }
    }

    // COMMIT TRANSACTION: Both disk save and image encode succeeded, commit baseline update to session manager
    manager.update_with_target(
        session_id.clone(),
        pending_baseline_image,
        frame_index,
        target_fingerprint,
    );

    ToolCallResult {
        content,
        is_error: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_manager_lifecycle() {
        let manager = SessionManager::new();
        let img1 = DynamicImage::new_rgba8(100, 100);

        assert!(manager.get("test-01").is_none());

        manager.update("test-01".to_string(), img1.clone(), 1);
        let rec = manager.get("test-01").expect("Session should exist");
        assert_eq!(rec.frame_index, 1);

        manager.update("test-01".to_string(), img1, 2);
        let rec2 = manager.get("test-01").expect("Session should exist");
        assert_eq!(rec2.frame_index, 2);

        manager.reset("test-01");
        assert!(manager.get("test-01").is_none());
    }

    #[test]
    fn test_session_manager_lru_capacity_and_retention() {
        let manager = SessionManager::new();
        let dummy_img = DynamicImage::new_rgba8(10, 10);

        // Fill up to maximum concurrent capacity (16 sessions)
        for i in 0..MAX_CONCURRENT_SESSIONS {
            let id = format!("session-{i:02}");
            manager.update(id, dummy_img.clone(), i as u64);
            // Tiny sleep or monotonic clock tick ensures unique last_accessed timestamps
            std::thread::sleep(Duration::from_millis(2));
        }

        // Verify all 16 sessions are retained and get() does NOT evict any session
        for i in 0..MAX_CONCURRENT_SESSIONS {
            let id = format!("session-{i:02}");
            let rec = manager
                .get(&id)
                .unwrap_or_else(|| panic!("Session {id} should exist"));
            assert_eq!(rec.frame_index, i as u64);
        }

        // Access session-00 again so it is the most recently accessed
        std::thread::sleep(Duration::from_millis(2));
        assert!(manager.get("session-00").is_some());

        // Update session-01 in place; should NOT evict any session
        std::thread::sleep(Duration::from_millis(2));
        manager.update("session-01".to_string(), dummy_img.clone(), 999);
        assert_eq!(manager.get("session-01").unwrap().frame_index, 999);
        assert_eq!(
            manager.sessions.lock().unwrap().len(),
            MAX_CONCURRENT_SESSIONS
        );

        // Now insert session-16 (17th unique session)
        // Since session-02 was accessed earliest among the remaining sessions, it should be the LRU victim
        std::thread::sleep(Duration::from_millis(2));
        manager.update("session-16".to_string(), dummy_img.clone(), 16);

        // Check map size remains capped at 16
        assert_eq!(
            manager.sessions.lock().unwrap().len(),
            MAX_CONCURRENT_SESSIONS
        );
        // session-00, session-01, and session-16 must be present
        assert!(manager.get("session-00").is_some());
        assert!(manager.get("session-01").is_some());
        assert!(manager.get("session-16").is_some());
        // session-02 was the LRU victim
        assert!(manager.get("session-02").is_none());
    }

    #[test]
    fn test_session_tool_definition() {
        let def = get_session_tool_definition();
        assert_eq!(def.name, "capture_session");
        assert!(def.description.contains("Stateful"));

        let props = def
            .input_schema
            .get("properties")
            .expect("Properties exist");
        assert!(props.get("format").is_some());
        assert!(props.get("redactions").is_some());
        assert!(props.get("maskWindows").is_some());
        assert!(props.get("excludeWindows").is_some());
        assert!(props.get("allowMissingMask").is_some());
        assert!(props.get("tiles").is_some());
        assert!(props.get("tileSize").is_some());
    }

    #[test]
    fn test_session_diff_crop_coordinate_transform_mapping() {
        use crate::coords::CoordinateTransform;

        // Base uncropped pipeline transform: canvas 1920x1080 at screen (0, 0)
        let base_transform =
            CoordinateTransform::new((0, 0), 1.0, (0, 0), 1.0, (0, 0), (1920, 1080), (1920, 1080));

        // Diff detected changes with bbox: x=500, y=300, w=100, h=50
        // Padding is 8: px = 492, py = 292, pw = 116, ph = 66
        let diff_crop_offset = (492u32, 292u32);
        let output_size = (116u32, 66u32);

        let final_transform = CoordinateTransform::new(
            base_transform.screen_origin,
            base_transform.screen_scale_factor,
            base_transform.crop_offset,
            base_transform.image_scale,
            diff_crop_offset,
            base_transform.canvas_size,
            output_size,
        );

        let info = final_transform.to_info();
        assert_eq!(info.diff_crop_offset, (492, 292));
        assert_eq!(info.output_width, 116);
        assert_eq!(info.output_height, 66);

        // Top-left of the cropped diff image (0, 0) must map directly to desktop (492, 292)
        assert_eq!(final_transform.image_to_screen(0, 0), (492, 292));
        // Screen point (500, 300) must map to (8, 8) in the cropped image
        assert_eq!(final_transform.screen_to_image(500, 300), (8, 8));
        assert_eq!(final_transform.image_to_screen(8, 8), (500, 300));
    }

    #[test]
    fn test_session_target_fingerprint_reset() {
        let manager = SessionManager::new();
        let img = Arc::new(DynamicImage::new_rgba8(50, 50));

        manager.update_with_target(
            "session-t1".to_string(),
            img.clone(),
            1,
            "Target::Window('Calc'):scale=None:max_dim=None".to_string(),
        );

        let rec = manager.get("session-t1").expect("Session exists");
        assert_eq!(rec.frame_index, 1);
        assert_eq!(
            rec.target_fingerprint,
            "Target::Window('Calc'):scale=None:max_dim=None"
        );
    }
}
