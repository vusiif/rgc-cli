use image::DynamicImage;
use thiserror::Error;

use crate::api::{ApiError, ErrorCode};

#[derive(Error, Debug)]
pub enum CaptureError {
    #[error("No monitors found on system")]
    NoMonitorsFound,

    #[error("Monitor index {0} is out of bounds (found {1} monitors)")]
    MonitorNotFound(usize, usize),

    #[error("Failed to list monitors: {0}")]
    ListMonitorsFailed(#[source] xcap::XCapError),

    #[error("Failed to query monitor properties: {0}")]
    MonitorPropertyFailed(#[source] xcap::XCapError),

    #[error("Failed to capture monitor: {0}")]
    CaptureFailed(#[source] xcap::XCapError),

    #[error("Failed to list windows: {0}")]
    ListWindowsFailed(#[source] xcap::XCapError),

    #[error("No window found matching '{0}'")]
    WindowNotFound(String),

    #[error("Mask window target '{0}' was not found on desktop")]
    MaskWindowNotFound(String),

    #[error("Failed to resolve privacy mask for window '{query}': {reason}")]
    MaskWindowResolutionFailed { query: String, reason: String },

    #[error("Cannot capture window #{0} ('{1}') because it is minimized")]
    WindowMinimized(u32, String),

    #[error("Failed to capture window: {0}")]
    WindowCaptureFailed(#[source] xcap::XCapError),

    #[error("Invalid scale factor {0}: scale factor must be a finite, positive number")]
    InvalidScaleFactor(f32),

    #[error(
        "Specified crop region (x={x}, y={y}, w={width}, h={height}) is out of monitor bounds ({monitor_w}x{monitor_h})"
    )]
    RegionOutOfBounds {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        monitor_w: u32,
        monitor_h: u32,
    },

    #[error("Canvas dimensions ({width}x{height}) exceed maximum allowed allocation limit")]
    CanvasTooLarge { width: u32, height: u32 },

    #[error("No active focused window found on desktop")]
    ActiveWindowNotFound,

    #[error("Mouse cursor position is unavailable or outside the captured bounds")]
    CursorPositionUnavailable,
}

impl From<CaptureError> for ApiError {
    fn from(err: CaptureError) -> Self {
        match err {
            CaptureError::NoMonitorsFound => {
                ApiError::new(ErrorCode::NoMonitor, "No monitors found on system.")
                    .with_stage("monitor_selection")
                    .with_suggestion("Ensure at least one active display is connected and powered on.")
            }
            CaptureError::MonitorNotFound(req, avail) => ApiError::new(
                ErrorCode::MonitorNotFound,
                format!("Monitor index {req} is out of bounds (found {avail} monitors)."),
            )
            .with_stage("monitor_selection")
            .with_suggestion("Run 'rgc-cli --list-monitors' to see all valid monitor indices."),
            CaptureError::ListMonitorsFailed(e) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Failed to list monitors: {e}"),
            )
            .with_stage("monitor_enumeration")
            .with_suggestion("Verify display server permissions or graphics drivers."),
            CaptureError::MonitorPropertyFailed(e) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Failed to query monitor properties: {e}"),
            )
            .with_stage("monitor_properties")
            .with_suggestion("Verify display driver stability."),
            CaptureError::CaptureFailed(e) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Failed to capture monitor: {e}"),
            )
            .with_stage("screen_capture")
            .with_suggestion("Check display capture permissions in OS privacy settings."),
            CaptureError::ListWindowsFailed(e) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Failed to list windows: {e}"),
            )
            .with_stage("window_enumeration")
            .with_suggestion("Ensure application window server permissions are granted."),
            CaptureError::WindowNotFound(pattern) => ApiError::new(
                ErrorCode::WindowNotFound,
                format!("No window found matching '{pattern}'."),
            )
            .with_stage("window_selection")
            .with_suggestion("Run 'rgc-cli --list-windows' to inspect all active window titles and IDs."),
            CaptureError::MaskWindowNotFound(pattern) => ApiError::new(
                ErrorCode::BadArgs,
                format!(
                    "Mask window target '{pattern}' was not found on desktop. Aborting export to prevent sensitive information leakage."
                ),
            )
            .with_stage("mask_window_lookup")
            .with_suggestion(
                "Verify active windows with --list-windows, or pass --allow-missing-mask to ignore nonexistent mask windows.",
            ),
            CaptureError::MaskWindowResolutionFailed { query, reason } => ApiError::new(
                ErrorCode::BadArgs,
                format!(
                    "Failed to resolve privacy mask for window '{query}': {reason}. Aborting export to prevent sensitive information leakage."
                ),
            )
            .with_stage("mask_window_lookup")
            .with_suggestion(
                "Check window state or pass --allow-missing-mask to permit capturing if privacy mask cannot be resolved.",
            ),
            CaptureError::WindowMinimized(id, title) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Cannot capture window #{id} ('{title}') because it is minimized."),
            )
            .with_stage("window_capture")
            .with_suggestion("Restore or un-minimize the window before attempting capture."),
            CaptureError::WindowCaptureFailed(e) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Failed to capture window: {e}"),
            )
            .with_stage("window_capture")
            .with_suggestion("Check OS screen/window capture privacy permissions."),
            CaptureError::InvalidScaleFactor(factor) => ApiError::new(
                ErrorCode::CaptureFailed,
                format!("Invalid scale factor {factor}: must be a finite, positive number."),
            )
            .with_stage("dpi_scaling")
            .with_suggestion("Check OS display scaling settings."),
            CaptureError::RegionOutOfBounds {
                x,
                y,
                width,
                height,
                monitor_w,
                monitor_h,
            } => ApiError::new(
                ErrorCode::RegionOutOfBounds,
                format!(
                    "Specified crop region (x={x}, y={y}, w={width}, h={height}) is out of monitor bounds ({monitor_w}x{monitor_h})."
                ),
            )
            .with_stage("region_validation")
            .with_suggestion("Adjust region coordinates to fit within the selected monitor boundaries."),
            CaptureError::CanvasTooLarge { width, height } => ApiError::new(
                ErrorCode::RegionOutOfBounds,
                format!("Canvas dimensions ({width}x{height}) exceed maximum allowed allocation limit."),
            )
            .with_stage("canvas_allocation")
            .with_suggestion("Reduce capture region or number of monitors."),
            CaptureError::ActiveWindowNotFound => ApiError::new(
                ErrorCode::WindowNotFound,
                "No active focused window found on desktop.",
            )
            .with_stage("active_window_selection")
            .with_suggestion("Ensure an application window is in the foreground and has focus."),
            CaptureError::CursorPositionUnavailable => ApiError::new(
                ErrorCode::BadArgs,
                "Mouse cursor position is unavailable or outside the captured bounds.",
            )
            .with_stage("around_cursor_cropping")
            .with_suggestion("Ensure the mouse cursor is located within the target display or window bounds."),
        }
    }
}

/// Information about a connected display/monitor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MonitorInfo {
    pub index: usize,
    pub name: String,
    pub is_primary: bool,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
}

/// Information about an active application window.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_minimized: bool,
}

/// Screen capture output along with display metrics and crop offsets for downstream processing.
#[derive(Debug)]
pub struct CapturedScreen {
    pub image: DynamicImage,
    pub scale_factor: f32,
    pub crop_offset: (u32, u32),
    pub monitor_origin: (i32, i32),
}

/// Window capture output along with display metrics and crop offsets for downstream processing.
#[derive(Debug)]
pub struct CapturedWindow {
    pub image: DynamicImage,
    pub scale_factor: f32,
    pub crop_offset: (u32, u32),
    pub window_origin: (i32, i32),
}
