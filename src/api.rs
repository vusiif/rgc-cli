use serde::{Deserialize, Serialize};

use crate::action::{ActionVerificationResult, ClickMarker, DragMarker};
use crate::cli::ImageFormat;
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

/// Structured information about the mouse cursor location on the captured image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorInfo {
    pub x: i32,
    pub y: i32,
    pub visible: bool,
}

/// Detected interactive UI element with bounding box and center click anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiElement {
    pub id: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub center_x: u32,
    pub center_y: u32,
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
