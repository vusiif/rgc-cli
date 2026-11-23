use serde::{Deserialize, Serialize};

use crate::action::{ActionVerificationResult, ClickMarker, DragMarker};
use crate::cli::ImageFormat;
use crate::coords::CoordinateSpaceInfo;
use crate::transform::DiffResult;

/// Standardized error codes for machine-readable JSON responses and process exit codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    BadArgs,
    NoMonitor,
    MonitorNotFound,
    WindowNotFound,
    RegionOutOfBounds,
    PermissionDenied,
    CaptureFailed,
    ClipboardFailed,
    EncodeFailed,
    SaveFailed,
    FormatMismatch,
    IoError,
    Unknown,
}

impl ErrorCode {
    pub fn exit_code(&self) -> i32 {
        match self {
            ErrorCode::BadArgs => 1,
            ErrorCode::NoMonitor => 2,
            ErrorCode::MonitorNotFound => 3,
            ErrorCode::WindowNotFound => 12,
            ErrorCode::RegionOutOfBounds => 4,
            ErrorCode::PermissionDenied => 5,
            ErrorCode::CaptureFailed => 6,
            ErrorCode::ClipboardFailed => 7,
            ErrorCode::EncodeFailed => 8,
            ErrorCode::SaveFailed => 9,
            ErrorCode::FormatMismatch => 10,
            ErrorCode::IoError => 11,
            ErrorCode::Unknown => 127,
        }
    }
}

/// Detailed machine-readable error payload complying with the public schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub error_code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    pub exit_code: i32,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let exit_code = code.exit_code();
        Self {
            error_code: code,
            message: message.into(),
            stage: None,
            suggestion: None,
            exit_code,
        }
    }

    pub fn with_stage(mut self, stage: impl Into<String>) -> Self {
        self.stage = Some(stage.into());
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

/// Generic JSON envelope wrapper for stdout and MCP machine consumption.
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn failure(error: ApiError) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error),
        }
    }
}

/// Structured information about the mouse cursor location across output image, screen, and canvas spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorInfo {
    /// X coordinate on the final output image.
    pub x: i32,
    /// Y coordinate on the final output image.
    pub y: i32,
    /// Whether the cursor is visible inside the output image boundary.
    pub visible: bool,
    /// Global OS desktop X coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_x: Option<i32>,
    /// Global OS desktop Y coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_y: Option<i32>,
    /// Unscaled canvas X coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_x: Option<i32>,
    /// Unscaled canvas Y coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canvas_y: Option<i32>,
}

impl CursorInfo {
    pub fn new(x: i32, y: i32, visible: bool) -> Self {
        Self {
            x,
            y,
            visible,
            screen_x: None,
            screen_y: None,
            canvas_x: None,
            canvas_y: None,
        }
    }
}

/// Detected interactive UI element with bounding box and center click anchor across image and screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiElement {
    pub id: u32,
    /// Output image X coordinate (top-left).
    pub x: u32,
    /// Output image Y coordinate (top-left).
    pub y: u32,
    /// Output image element width.
    pub width: u32,
    /// Output image element height.
    pub height: u32,
    /// Output image center X coordinate.
    pub center_x: u32,
    /// Output image center Y coordinate.
    pub center_y: u32,
    /// Global OS desktop screen center X coordinate for direct automation clicks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_center_x: Option<i32>,
    /// Global OS desktop screen center Y coordinate for direct automation clicks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_center_y: Option<i32>,
    /// Global OS desktop screen top-left X coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_x: Option<i32>,
    /// Global OS desktop screen top-left Y coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_y: Option<i32>,
    /// Accessible role of the control (e.g., "button", "edit", "combobox", "window", "checkbox").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Human-readable label or text of the control (e.g., "Save", "Submit", "Search").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether the element is enabled for interaction according to accessibility tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Origin source: "vision" (SoM vision detection), "accessibility" (UIA/AX tree node), or "hybrid" (fused).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl UiElement {
    pub fn new(id: u32, x: u32, y: u32, width: u32, height: u32) -> Self {
        let center_x = x + width / 2;
        let center_y = y + height / 2;
        Self {
            id,
            x,
            y,
            width,
            height,
            center_x,
            center_y,
            screen_center_x: None,
            screen_center_y: None,
            screen_x: None,
            screen_y: None,
            role: None,
            label: None,
            enabled: None,
            source: Some("vision".to_string()),
        }
    }
}

/// Result of visual template/icon matching across image and screen coordinate spaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateMatchResult {
    /// 0-based image X coordinate (top-left).
    pub x: u32,
    /// 0-based image Y coordinate (top-left).
    pub y: u32,
    /// Match rectangle width in image pixels.
    pub width: u32,
    /// Match rectangle height in image pixels.
    pub height: u32,
    /// Center X coordinate on the image.
    pub center_x: u32,
    /// Center Y coordinate on the image.
    pub center_y: u32,
    /// Normalized cross-correlation confidence score (0.0 - 1.0).
    pub confidence: f32,
    /// Global OS desktop screen center X coordinate for direct automation clicks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_center_x: Option<i32>,
    /// Global OS desktop screen center Y coordinate for direct automation clicks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_center_y: Option<i32>,
    /// Global OS desktop screen top-left X coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_x: Option<i32>,
    /// Global OS desktop screen top-left Y coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_y: Option<i32>,
}

impl TemplateMatchResult {
    pub fn new(x: u32, y: u32, width: u32, height: u32, confidence: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            center_x: x + width / 2,
            center_y: y + height / 2,
            confidence,
            screen_center_x: None,
            screen_center_y: None,
            screen_x: None,
            screen_y: None,
        }
    }
}

/// Structured JSON representation of the capture operation result.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResultJson {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub clipboard: bool,
    #[serde(skip_serializing_if = "Option::is_none", alias = "size_bytes")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<DiffResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<CursorInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elements: Option<Vec<UiElement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_verification: Option<ActionVerificationResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clicks: Option<Vec<ClickMarker>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drags: Option<Vec<DragMarker>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<CoordinateSpaceInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<Vec<TemplateMatchResult>>,
}

/// Structured JSON representation of the continuous recording result.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordResultJson {
    pub status: String,
    pub path: String,
    pub frame_count: u32,
    pub duration_secs: f32,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}
