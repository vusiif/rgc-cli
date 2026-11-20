use chrono::Local;
use image::DynamicImage;
use log::debug;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use thiserror::Error;

use crate::cli::ImageFormat;

/// Atomic sequence counter for collision-free auto-generated filenames within the process.
static SEQUENCE_COUNTER: AtomicU32 = AtomicU32::new(0);

#[derive(Error, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum OutputError {
    #[error("Failed to access system clipboard: {0}")]
    ClipboardFailed(String),

    #[error("Failed to write image to stdout: {0}")]
    StdoutFailed(#[source] image::ImageError),

    #[error("Failed to write bytes to stdout: {0}")]
    StdoutIoFailed(#[source] std::io::Error),

    #[error("Failed to create destination directory '{path}': {source}")]
    CreateDirFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to save image to '{path}': {source}")]
    SaveFailed {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },

    #[error("Explicit format '{explicit:?}' conflicts with file extension of '{path}' (which indicates '{inferred:?}')")]
    FormatMismatch {
        explicit: ImageFormat,
        inferred: ImageFormat,
        path: PathBuf,
    },

    #[error("Invalid destination path: {0}")]
    InvalidPath(String),

    #[error("Failed to open default image viewer: {0}")]
    OpenViewerFailed(String),
}

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

impl From<OutputError> for ApiError {
    fn from(err: OutputError) -> Self {
        match err {
            OutputError::ClipboardFailed(msg) => ApiError::new(ErrorCode::ClipboardFailed, msg)
                .with_stage("clipboard")
                .with_suggestion("Ensure clipboard is accessible and not held exclusively by another process."),
            OutputError::StdoutFailed(e) => ApiError::new(ErrorCode::EncodeFailed, e.to_string())
                .with_stage("stdout_encode")
                .with_suggestion("Ensure the chosen format supports the image dimensions."),
            OutputError::StdoutIoFailed(e) => ApiError::new(ErrorCode::IoError, e.to_string())
                .with_stage("stdout_stream")
                .with_suggestion("Check if downstream pipe consumer is accepting data."),
            OutputError::CreateDirFailed { path, source } => ApiError::new(
                ErrorCode::SaveFailed,
                format!("Failed to create destination directory '{}': {}", path.display(), source),
            )
            .with_stage("directory_creation")
            .with_suggestion("Verify parent directory permissions and disk space."),
            OutputError::SaveFailed { path, source } => ApiError::new(
                ErrorCode::SaveFailed,
                format!("Failed to save image to '{}': {}", path.display(), source),
            )
            .with_stage("save_file")
            .with_suggestion("Verify write permissions and disk space."),
            OutputError::FormatMismatch { explicit, inferred, path } => ApiError::new(
                ErrorCode::FormatMismatch,
                format!(
                    "Explicit format '{:?}' conflicts with file extension of '{}' (which implies '{:?}')",
                    explicit, path.display(), inferred
                ),
            )
            .with_stage("argument_validation")
            .with_suggestion("Specify matching file extension or omit explicit --format."),
            OutputError::InvalidPath(msg) => ApiError::new(ErrorCode::BadArgs, msg)
                .with_stage("argument_validation")
                .with_suggestion("Provide a valid file or directory destination."),
            OutputError::OpenViewerFailed(msg) => ApiError::new(ErrorCode::IoError, msg)
                .with_stage("viewer_launch")
                .with_suggestion("Ensure a default image viewer is configured on your system."),
        }
    }
}

/// Unified API response wrapper for JSON output mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// Bounding box of detected visual differences.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffBBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Structured summary of visual diff comparison against a reference image.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub changed: bool,
    pub changed_pixels: u64,
    pub total_pixels: u64,
    pub changed_percent: f64,
    pub threshold: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<DiffBBox>,
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
}

/// Destination target for screenshot output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputDestination {
    Stdout,
    Auto,
    Directory(PathBuf),
    File(PathBuf),
}

/// Determines whether a path is intended as a directory destination.
pub fn is_directory_target(path: &Path) -> bool {
    if path.is_dir() {
        return true;
    }

    let path_str = path.to_string_lossy();
    if path_str.ends_with('/') || path_str.ends_with('\\') {
        return true;
    }

    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name == "." || name == ".." {
            return true;
        }
    }

    false
}

/// Resolves user CLI arguments into an unambiguous `OutputDestination`.
pub fn determine_output_destination(
    output: Option<&Path>,
    output_file: Option<&Path>,
    output_dir: Option<&Path>,
) -> Result<OutputDestination, OutputError> {
    if let Some(dir) = output_dir {
        if dir.is_file() {
            return Err(OutputError::InvalidPath(format!(
                "Path '{}' is an existing file, but was specified as --output-dir",
                dir.display()
            )));
        }
        return Ok(OutputDestination::Directory(dir.to_path_buf()));
    }

    if let Some(file) = output_file {
        if is_directory_target(file) {
            return Err(OutputError::InvalidPath(format!(
                "Path '{}' is a directory, but was specified as --output-file",
                file.display()
            )));
        }
        return Ok(OutputDestination::File(file.to_path_buf()));
    }

    if let Some(path) = output {
        if path.as_os_str() == "-" {
            return Ok(OutputDestination::Stdout);
        }
        if is_directory_target(path) {
            return Ok(OutputDestination::Directory(path.to_path_buf()));
        }
        return Ok(OutputDestination::File(path.to_path_buf()));
    }

    Ok(OutputDestination::Auto)
}

/// Validates explicit format against destination file extension and infers image format.
pub fn validate_and_infer_format(
    destination: &OutputDestination,
    explicit_format: Option<ImageFormat>,
) -> Result<ImageFormat, OutputError> {
    if let OutputDestination::File(path) = destination {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let inferred = match ext.to_ascii_lowercase().as_str() {
                "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
                "webp" => Some(ImageFormat::Webp),
                "png" => Some(ImageFormat::Png),
                _ => None,
            };

            if let Some(inf) = inferred {
                if let Some(exp) = explicit_format {
                    if exp != inf {
                        return Err(OutputError::FormatMismatch {
                            explicit: exp,
                            inferred: inf,
                            path: path.clone(),
                        });
                    }
                }
                return Ok(inf);
            }
        }
    }

    Ok(explicit_format.unwrap_or(ImageFormat::Png))
}

/// Automatically infer the image format (fallback for backwards compatibility).
#[allow(dead_code)]
pub fn infer_format(path: Option<&Path>, explicit_format: Option<ImageFormat>) -> ImageFormat {
    if let Some(fmt) = explicit_format {
        return fmt;
    }

    if let Some(p) = path {
        if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            match ext.to_ascii_lowercase().as_str() {
                "jpg" | "jpeg" => return ImageFormat::Jpeg,
                "webp" => return ImageFormat::Webp,
                "png" => return ImageFormat::Png,
                _ => {}
            }
        }
    }

    ImageFormat::Png
}

/// Copy the captured image directly into the system clipboard.
pub fn copy_to_clipboard(image: &DynamicImage) -> Result<(), OutputError> {
    debug!("Converting image to RGBA8 for system clipboard");
    let rgba = image.to_rgba8();
    let width = rgba.width() as usize;
    let height = rgba.height() as usize;
    let raw_bytes = rgba.into_raw();

    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| OutputError::ClipboardFailed(e.to_string()))?;

    let img_data = arboard::ImageData {
        width,
        height,
        bytes: raw_bytes.into(),
    };

    clipboard
        .set_image(img_data)
        .map_err(|e| OutputError::ClipboardFailed(e.to_string()))?;

    debug!("Image successfully written to clipboard buffer");
    Ok(())
}

/// Applies scaling factor or max dimension constraint to captured image.
pub fn apply_image_transforms(
    image: DynamicImage,
    scale: Option<f64>,
    max_dimension: Option<u32>,
) -> DynamicImage {
    if let Some(factor) = scale {
        let new_w = ((image.width() as f64) * factor).round().max(1.0) as u32;
        let new_h = ((image.height() as f64) * factor).round().max(1.0) as u32;
        debug!(
            "Resizing image from {}x{} to {}x{} (scale: {:.2})",
            image.width(),
            image.height(),
            new_w,
            new_h,
            factor
        );
        return image.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3);
    }
    if let Some(max_dim) = max_dimension {
        if image.width() > max_dim || image.height() > max_dim {
            debug!(
                "Constraining image to max dimension {}px from {}x{}",
                max_dim,
                image.width(),
                image.height()
            );
            return image.resize(max_dim, max_dim, image::imageops::FilterType::Lanczos3);
        }
    }
    image
}

/// Encodes DynamicImage into byte buffer using specified format and optional JPEG quality.
pub fn encode_image_bytes(
    image: &DynamicImage,
    format: ImageFormat,
    quality: Option<u8>,
) -> Result<Vec<u8>, OutputError> {
    use std::io::Cursor;

    let initial_capacity = (image.width() as usize * image.height() as usize).min(1024 * 1024);
    let mut buffer = Cursor::new(Vec::with_capacity(initial_capacity));

    match format {
        ImageFormat::Png => {
            image
                .write_to(&mut buffer, image::ImageFormat::Png)
                .map_err(OutputError::StdoutFailed)?;
        }
        ImageFormat::Webp => {
            image
                .write_to(&mut buffer, image::ImageFormat::WebP)
                .map_err(OutputError::StdoutFailed)?;
        }
        ImageFormat::Jpeg => {
            let q = quality.unwrap_or(85);
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, q);
            encoder
                .encode_image(image)
                .map_err(OutputError::StdoutFailed)?;
        }
    }

    Ok(buffer.into_inner())
}

/// Write encoded image bytes directly to standard output (stdout stream).
pub fn write_to_stdout(
    image: &DynamicImage,
    format: ImageFormat,
    quality: Option<u8>,
) -> Result<(), OutputError> {
    use std::io::Write;

    debug!(
        "Streaming encoded image ({:?}, quality={:?}) to stdout",
        format, quality
    );
    let mut out = stdout().lock();
    let bytes = encode_image_bytes(image, format, quality)?;

    if let Err(e) = out.write_all(&bytes) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            debug!("Downstream pipe closed by consumer (BrokenPipe)");
            return Ok(());
        }
        return Err(OutputError::StdoutIoFailed(e));
    }

    if let Err(e) = out.flush() {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            debug!("Downstream pipe closed by consumer during flush (BrokenPipe)");
            return Ok(());
        }
        return Err(OutputError::StdoutIoFailed(e));
    }

    Ok(())
}

/// Generates a default collision-resistant timestamped filename, e.g. `screenshot_2026-10-15_23-55-12.123_0001.png`.
pub fn generate_default_filename(format: ImageFormat) -> String {
    let now = Local::now();
    let seq = SEQUENCE_COUNTER.fetch_add(1, Ordering::Relaxed) % 10000;
    format!(
        "screenshot_{}_{:04}.{}",
        now.format("%Y-%m-%d_%H-%M-%S%.3f"),
        seq,
        format.extension()
    )
}

/// Resolves a non-colliding path inside the given directory for auto-generated files.
pub fn resolve_unique_auto_path(dir: &Path, format: ImageFormat) -> Result<PathBuf, OutputError> {
    for _ in 0..100 {
        let candidate = dir.join(generate_default_filename(format));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    // High-entropy fallback if 100 fast attempts collided
    let now = Local::now();
    let nanos = now.timestamp_subsec_nanos();
    let candidate = dir.join(format!(
        "screenshot_{}_{}_{:09}.{}",
        now.format("%Y-%m-%d_%H-%M-%S"),
        std::process::id(),
        nanos,
        format.extension()
    ));
    Ok(candidate)
}

/// Resolves the final destination path for saving an image.
pub fn resolve_output_path(
    destination: &OutputDestination,
    format: ImageFormat,
) -> Result<PathBuf, OutputError> {
    match destination {
        OutputDestination::Stdout => Err(OutputError::InvalidPath(
            "Cannot resolve filesystem path for stdout destination".to_string(),
        )),
        OutputDestination::Auto => {
            let cwd = Path::new(".");
            resolve_unique_auto_path(cwd, format)
        }
        OutputDestination::Directory(dir) => {
            if !dir.exists() {
                debug!("Creating destination directory: {:?}", dir);
                fs::create_dir_all(dir).map_err(|e| OutputError::CreateDirFailed {
                    path: dir.to_path_buf(),
                    source: e,
                })?;
            }
            resolve_unique_auto_path(dir, format)
        }
        OutputDestination::File(file) => {
            if let Some(parent) = file.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    debug!("Creating parent directory: {:?}", parent);
                    fs::create_dir_all(parent).map_err(|e| OutputError::CreateDirFailed {
                        path: parent.to_path_buf(),
                        source: e,
                    })?;
                }
            }
            // User-specified file paths preserve intentional overwrite semantics
            Ok(file.clone())
        }
    }
}

pub use crate::cli::MaskEffect;

/// Apply a single redaction mask to an RGBA image in-place.
pub fn apply_redaction(
    img: &mut image::RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    effect: MaskEffect,
) {
    let img_w = img.width();
    let img_h = img.height();

    // Defensive clipping to image boundaries
    let x0 = x.min(img_w);
    let y0 = y.min(img_h);
    let x1 = x.saturating_add(width).min(img_w);
    let y1 = y.saturating_add(height).min(img_h);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    match effect {
        MaskEffect::Transparent => {
            // Set Alpha to 0 (completely transparent hole)
            for py in y0..y1 {
                for px in x0..x1 {
                    img.put_pixel(px, py, image::Rgba([0, 0, 0, 0]));
                }
            }
        }
        MaskEffect::Blackout => {
            // Set RGB to 0, Alpha to 255 (solid black box)
            for py in y0..y1 {
                for px in x0..x1 {
                    img.put_pixel(px, py, image::Rgba([0, 0, 0, 255]));
                }
            }
        }
        MaskEffect::Pixelate => {
            let block_size: u32 = 12;
            let mut by = y0;
            while by < y1 {
                let mut bx = x0;
                let b_h = (by + block_size).min(y1) - by;
                while bx < x1 {
                    let b_w = (bx + block_size).min(x1) - bx;
                    let count = (b_w * b_h) as u64;

                    // Compute average RGBA across block
                    let mut r_sum = 0u64;
                    let mut g_sum = 0u64;
                    let mut b_sum = 0u64;
                    let mut a_sum = 0u64;

                    for py in by..(by + b_h) {
                        for px in bx..(bx + b_w) {
                            let p = img.get_pixel(px, py);
                            r_sum += p[0] as u64;
                            g_sum += p[1] as u64;
                            b_sum += p[2] as u64;
                            a_sum += p[3] as u64;
                        }
                    }

                    let avg_pixel = image::Rgba([
                        (r_sum / count) as u8,
                        (g_sum / count) as u8,
                        (b_sum / count) as u8,
                        (a_sum / count) as u8,
                    ]);

                    for py in by..(by + b_h) {
                        for px in bx..(bx + b_w) {
                            img.put_pixel(px, py, avg_pixel);
                        }
                    }

                    bx += block_size;
                }
                by += block_size;
            }
        }
        MaskEffect::Blur => {
            let rw = x1 - x0;
            let rh = y1 - y0;
            let sub = image::imageops::crop_imm(img, x0, y0, rw, rh).to_image();
            let down_w = (rw / 8).max(1);
            let down_h = (rh / 8).max(1);
            let downscaled = image::imageops::resize(
                &sub,
                down_w,
                down_h,
                image::imageops::FilterType::Triangle,
            );
            let blurred =
                image::imageops::resize(&downscaled, rw, rh, image::imageops::FilterType::Triangle);
            image::imageops::overlay(img, &blurred, x0 as i64, y0 as i64);
        }
    }
}

/// Apply a list of redaction masks to a DynamicImage.
pub fn apply_redactions(
    image: DynamicImage,
    masks: &[(u32, u32, u32, u32, MaskEffect)],
) -> DynamicImage {
    if masks.is_empty() {
        return image;
    }

    let mut rgba = image.into_rgba8();
    for &(x, y, w, h, effect) in masks {
        debug!(
            "Applying redaction: x={}, y={}, w={}, h={}, effect={:?}",
            x, y, w, h, effect
        );
        apply_redaction(&mut rgba, x, y, w, h, effect);
    }
    DynamicImage::ImageRgba8(rgba)
}

// 3x5 bitmap font for '0'..='9', ',', and '-' (used for visual grounding grid coordinates)
const FONT_3X5_DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b010, 0b010, 0b010], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
];
const FONT_3X5_COMMA: [u8; 5] = [0b000, 0b000, 0b000, 0b010, 0b100];
const FONT_3X5_MINUS: [u8; 5] = [0b000, 0b000, 0b111, 0b000, 0b000];

/// Alpha-blends a source pixel over the destination image at (x, y).
#[inline]
fn blend_pixel(img: &mut image::RgbaImage, x: u32, y: u32, src: image::Rgba<u8>) {
    if x >= img.width() || y >= img.height() {
        return;
    }
    if src[3] == 255 {
        img.put_pixel(x, y, src);
        return;
    }
    if src[3] == 0 {
        return;
    }

    let dst = img.get_pixel(x, y);
    let sa = src[3] as u32;
    let da = dst[3] as u32;
    let inv_sa = 255 - sa;

    let out_a = sa + (da * inv_sa) / 255;
    if out_a == 0 {
        img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
        return;
    }

    let out_r = ((src[0] as u32 * sa + dst[0] as u32 * da * inv_sa / 255) / out_a) as u8;
    let out_g = ((src[1] as u32 * sa + dst[1] as u32 * da * inv_sa / 255) / out_a) as u8;
    let out_b = ((src[2] as u32 * sa + dst[2] as u32 * da * inv_sa / 255) / out_a) as u8;

    img.put_pixel(x, y, image::Rgba([out_r, out_g, out_b, out_a as u8]));
}

/// Draws coordinate label using 3x5 font with background padding for high contrast.
fn draw_label_3x5(
    img: &mut image::RgbaImage,
    start_x: u32,
    start_y: u32,
    text: &str,
    fg: image::Rgba<u8>,
    bg: image::Rgba<u8>,
) {
    let img_w = img.width();
    let img_h = img.height();

    let text_len = text.len() as u32;
    if text_len == 0 {
        return;
    }
    let total_w = text_len * 4;
    let total_h = 7;

    let bx0 = start_x.min(img_w);
    let by0 = start_y.min(img_h);
    let bx1 = (start_x + total_w + 1).min(img_w);
    let by1 = (start_y + total_h).min(img_h);

    for py in by0..by1 {
        for px in bx0..bx1 {
            blend_pixel(img, px, py, bg);
        }
    }

    let mut cur_x = start_x + 1;
    let cur_y = start_y + 1;

    for ch in text.chars() {
        let glyph = match ch {
            '0'..='9' => Some(&FONT_3X5_DIGITS[(ch as usize) - ('0' as usize)]),
            ',' => Some(&FONT_3X5_COMMA),
            '-' => Some(&FONT_3X5_MINUS),
            _ => None,
        };

        if let Some(g) = glyph {
            for (row, &bits) in g.iter().enumerate() {
                let py = cur_y + row as u32;
                if py >= img_h {
                    continue;
                }
                for col in 0..3 {
                    if (bits & (1 << (2 - col))) != 0 {
                        let px = cur_x + col;
                        if px < img_w {
                            img.put_pixel(px, py, fg);
                        }
                    }
                }
            }
        }
        cur_x += 4;
    }
}

/// Overlay visual grounding grid lines and coordinate anchors for AI agent localization.
pub fn apply_grid_overlay(image: DynamicImage, step: u32) -> DynamicImage {
    if step < 10 {
        return image;
    }

    let mut rgba = image.into_rgba8();
    let w = rgba.width();
    let h = rgba.height();

    let grid_color = image::Rgba([0, 170, 255, 90]);
    let cross_color = image::Rgba([255, 70, 70, 220]);
    let fg_text = image::Rgba([255, 255, 255, 255]);
    let bg_text = image::Rgba([0, 0, 0, 190]);

    // Vertical grid lines
    let mut x = step;
    while x < w {
        for y in 0..h {
            blend_pixel(&mut rgba, x, y, grid_color);
        }
        x += step;
    }

    // Horizontal grid lines
    let mut y = step;
    while y < h {
        for x_coord in 0..w {
            blend_pixel(&mut rgba, x_coord, y, grid_color);
        }
        y += step;
    }

    // Intersection anchors and coordinate labels
    let mut iy = step;
    while iy < h {
        let mut ix = step;
        while ix < w {
            for dx in ix.saturating_sub(2)..=(ix + 2).min(w.saturating_sub(1)) {
                rgba.put_pixel(dx, iy, cross_color);
            }
            for dy in iy.saturating_sub(2)..=(iy + 2).min(h.saturating_sub(1)) {
                rgba.put_pixel(ix, dy, cross_color);
            }

            let label = format!("{ix},{iy}");
            draw_label_3x5(&mut rgba, ix + 2, iy + 2, &label, fg_text, bg_text);

            ix += step;
        }
        iy += step;
    }

    DynamicImage::ImageRgba8(rgba)
}

/// Compare current image against a reference image and calculate bounding box of differences.
pub fn calculate_image_diff(
    current: &DynamicImage,
    reference: &DynamicImage,
    threshold: u8,
) -> DiffResult {
    let curr_rgba = current.to_rgba8();
    let ref_rgba = reference.to_rgba8();

    let cw = curr_rgba.width();
    let ch = curr_rgba.height();
    let rw = ref_rgba.width();
    let rh = ref_rgba.height();

    let total_pixels = (cw as u64) * (ch as u64);
    if total_pixels == 0 {
        return DiffResult {
            changed: false,
            changed_pixels: 0,
            total_pixels: 0,
            changed_percent: 0.0,
            threshold,
            bbox: None,
        };
    }

    let mut changed_pixels = 0u64;
    let mut min_x = u32::MAX;
    let mut max_x = 0u32;
    let mut min_y = u32::MAX;
    let mut max_y = 0u32;

    for y in 0..ch {
        for x in 0..cw {
            let is_diff = if x < rw && y < rh {
                let cp = curr_rgba.get_pixel(x, y);
                let rp = ref_rgba.get_pixel(x, y);
                cp[0].abs_diff(rp[0]) > threshold
                    || cp[1].abs_diff(rp[1]) > threshold
                    || cp[2].abs_diff(rp[2]) > threshold
                    || cp[3].abs_diff(rp[3]) > threshold
            } else {
                true
            };

            if is_diff {
                changed_pixels += 1;
                if x < min_x {
                    min_x = x;
                }
                if x > max_x {
                    max_x = x;
                }
                if y < min_y {
                    min_y = y;
                }
                if y > max_y {
                    max_y = y;
                }
            }
        }
    }

    if changed_pixels > 0 && min_x <= max_x && min_y <= max_y {
        let width = max_x - min_x + 1;
        let height = max_y - min_y + 1;
        let percent =
            ((changed_pixels as f64) / (total_pixels as f64) * 100.0 * 1000.0).round() / 1000.0;
        DiffResult {
            changed: true,
            changed_pixels,
            total_pixels,
            changed_percent: percent,
            threshold,
            bbox: Some(DiffBBox {
                x: min_x,
                y: min_y,
                width,
                height,
            }),
        }
    } else {
        DiffResult {
            changed: false,
            changed_pixels: 0,
            total_pixels,
            changed_percent: 0.0,
            threshold,
            bbox: None,
        }
    }
}

/// Crop image to the difference bounding box with optional padding margin.
pub fn crop_to_diff_bbox(image: DynamicImage, bbox: &DiffBBox, padding: u32) -> DynamicImage {
    let w = image.width();
    let h = image.height();

    let px = bbox.x.saturating_sub(padding);
    let py = bbox.y.saturating_sub(padding);
    let pw = (bbox.width + padding * 2).min(w.saturating_sub(px));
    let ph = (bbox.height + padding * 2).min(h.saturating_sub(py));

    if pw == 0 || ph == 0 {
        return image;
    }

    image.crop_imm(px, py, pw, ph)
}

/// Save a `DynamicImage` to the filesystem destination with optional JPEG quality.
pub fn save_image(
    image: &DynamicImage,
    destination: &OutputDestination,
    format: ImageFormat,
    quality: Option<u8>,
) -> Result<PathBuf, OutputError> {
    let file_path = resolve_output_path(destination, format)?;
    debug!(
        "Saving image ({:?}, quality={:?}) to: {:?}",
        format, quality, file_path
    );

    if format == ImageFormat::Jpeg && quality.is_some() {
        let bytes = encode_image_bytes(image, format, quality)?;
        fs::write(&file_path, bytes).map_err(|e| OutputError::SaveFailed {
            path: file_path.clone(),
            source: image::ImageError::IoError(e),
        })?;
    } else {
        let image_format = match format {
            ImageFormat::Png => image::ImageFormat::Png,
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
            ImageFormat::Webp => image::ImageFormat::WebP,
        };

        image
            .save_with_format(&file_path, image_format)
            .map_err(|e| OutputError::SaveFailed {
                path: file_path.clone(),
                source: e,
            })?;
    }

    Ok(file_path)
}

/// Opens a saved image file using the system's default image viewer.
pub fn open_file_in_viewer(path: &Path) -> Result<(), OutputError> {
    use std::process::Command;

    // Ensure relative paths are resolved to absolute coordinates so system viewers (especially Windows Explorer)
    // resolve the file against current working directory rather than the system or user directory.
    let target_path = if path.is_relative() {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    };

    debug!("Opening file in default viewer: {:?}", target_path);

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(&target_path)
            .spawn()
            .map_err(|e| OutputError::OpenViewerFailed(e.to_string()))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&target_path)
            .spawn()
            .map_err(|e| OutputError::OpenViewerFailed(e.to_string()))?;
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(&target_path)
            .spawn()
            .map_err(|e| OutputError::OpenViewerFailed(e.to_string()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn test_infer_format() {
        assert_eq!(
            infer_format(Some(Path::new("test.jpg")), Some(ImageFormat::Png)),
            ImageFormat::Png
        );
        assert_eq!(
            infer_format(Some(Path::new("photo.jpg")), None),
            ImageFormat::Jpeg
        );
        assert_eq!(
            infer_format(Some(Path::new("photo.jpeg")), None),
            ImageFormat::Jpeg
        );
        assert_eq!(
            infer_format(Some(Path::new("photo.webp")), None),
            ImageFormat::Webp
        );
        assert_eq!(
            infer_format(Some(Path::new("pic.png")), None),
            ImageFormat::Png
        );
        assert_eq!(infer_format(None, None), ImageFormat::Png);
        assert_eq!(
            infer_format(Some(Path::new("unknown.xyz")), None),
            ImageFormat::Png
        );
    }

    #[test]
    fn test_generate_default_filename_uniqueness() {
        let name1 = generate_default_filename(ImageFormat::Png);
        let name2 = generate_default_filename(ImageFormat::Png);
        assert_ne!(
            name1, name2,
            "Consecutive generated filenames must be unique"
        );
        assert!(name1.starts_with("screenshot_"));
        assert!(name1.ends_with(".png"));
    }

    #[test]
    fn test_is_directory_target() {
        assert!(is_directory_target(Path::new("foo/bar/")));
        assert!(is_directory_target(Path::new("foo\\bar\\")));
        assert!(is_directory_target(Path::new(".")));
        assert!(is_directory_target(Path::new("..")));
        assert!(!is_directory_target(Path::new("foo/bar/pic.png")));
    }

    #[test]
    fn test_determine_output_destination() {
        assert_eq!(
            determine_output_destination(None, None, None).unwrap(),
            OutputDestination::Auto
        );
        assert_eq!(
            determine_output_destination(Some(Path::new("-")), None, None).unwrap(),
            OutputDestination::Stdout
        );
        assert_eq!(
            determine_output_destination(Some(Path::new("dir/")), None, None).unwrap(),
            OutputDestination::Directory(PathBuf::from("dir/"))
        );
        assert_eq!(
            determine_output_destination(Some(Path::new("out.png")), None, None).unwrap(),
            OutputDestination::File(PathBuf::from("out.png"))
        );
        assert_eq!(
            determine_output_destination(None, Some(Path::new("custom.png")), None).unwrap(),
            OutputDestination::File(PathBuf::from("custom.png"))
        );
        assert_eq!(
            determine_output_destination(None, None, Some(Path::new("screenshots"))).unwrap(),
            OutputDestination::Directory(PathBuf::from("screenshots"))
        );
    }

    #[test]
    fn test_validate_and_infer_format_mismatch() {
        let dest = OutputDestination::File(PathBuf::from("shot.jpg"));
        let err = validate_and_infer_format(&dest, Some(ImageFormat::Png)).unwrap_err();
        match err {
            OutputError::FormatMismatch {
                explicit, inferred, ..
            } => {
                assert_eq!(explicit, ImageFormat::Png);
                assert_eq!(inferred, ImageFormat::Jpeg);
            }
            _ => panic!("Expected FormatMismatch error"),
        }

        // Matching format succeeds
        let ok_fmt = validate_and_infer_format(&dest, Some(ImageFormat::Jpeg)).unwrap();
        assert_eq!(ok_fmt, ImageFormat::Jpeg);

        // Inferred when omitted
        let inf_fmt = validate_and_infer_format(&dest, None).unwrap();
        assert_eq!(inf_fmt, ImageFormat::Jpeg);
    }

    #[test]
    fn test_resolve_output_path() {
        let dest = OutputDestination::Auto;
        let path = resolve_output_path(&dest, ImageFormat::Png).unwrap();
        assert!(path.to_str().unwrap().ends_with(".png"));

        let custom = OutputDestination::File(PathBuf::from("target/test_output_dir/my_pic.webp"));
        let resolved = resolve_output_path(&custom, ImageFormat::Webp).unwrap();
        assert_eq!(
            resolved,
            PathBuf::from("target/test_output_dir/my_pic.webp")
        );
        assert!(Path::new("target/test_output_dir").exists());

        // Clean up test dir
        let _ = fs::remove_dir_all("target/test_output_dir");
    }

    #[test]
    fn test_save_image_png_and_webp() {
        let mut img = RgbaImage::new(2, 2);
        img.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        let dynamic = DynamicImage::ImageRgba8(img);

        let test_file_png = OutputDestination::File(PathBuf::from("target/test_scratch/test.png"));
        let saved_png = save_image(&dynamic, &test_file_png, ImageFormat::Png, None).unwrap();
        assert_eq!(saved_png, PathBuf::from("target/test_scratch/test.png"));
        assert!(saved_png.exists());

        let test_file_webp =
            OutputDestination::File(PathBuf::from("target/test_scratch/test.webp"));
        let saved_webp = save_image(&dynamic, &test_file_webp, ImageFormat::Webp, None).unwrap();
        assert_eq!(saved_webp, PathBuf::from("target/test_scratch/test.webp"));
        assert!(saved_webp.exists());

        let test_file_jpg = OutputDestination::File(PathBuf::from("target/test_scratch/test.jpg"));
        let saved_jpg = save_image(&dynamic, &test_file_jpg, ImageFormat::Jpeg, Some(75)).unwrap();
        assert_eq!(saved_jpg, PathBuf::from("target/test_scratch/test.jpg"));
        assert!(saved_jpg.exists());

        // Clean up
        let _ = fs::remove_dir_all("target/test_scratch");
    }

    #[test]
    fn test_apply_image_transforms() {
        let img = DynamicImage::ImageRgba8(RgbaImage::new(100, 50));

        // Scale 0.5x
        let scaled = apply_image_transforms(img.clone(), Some(0.5), None);
        assert_eq!(scaled.width(), 50);
        assert_eq!(scaled.height(), 25);

        // Max dimension 40px
        let constrained = apply_image_transforms(img.clone(), None, Some(40));
        assert_eq!(constrained.width(), 40);
        assert_eq!(constrained.height(), 20);

        // No transforms
        let unchanged = apply_image_transforms(img, None, None);
        assert_eq!(unchanged.width(), 100);
        assert_eq!(unchanged.height(), 50);
    }

    #[test]
    fn test_apply_redaction_transparent() {
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([255, 0, 0, 255]));
        apply_redaction(&mut img, 10, 10, 20, 20, MaskEffect::Transparent);

        // Inside masked hole: alpha should be 0
        assert_eq!(img.get_pixel(10, 10), &Rgba([0, 0, 0, 0]));
        assert_eq!(img.get_pixel(29, 29), &Rgba([0, 0, 0, 0]));

        // Outside masked hole: should still be original red
        assert_eq!(img.get_pixel(9, 9), &Rgba([255, 0, 0, 255]));
        assert_eq!(img.get_pixel(30, 30), &Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn test_apply_redaction_blackout() {
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        apply_redaction(&mut img, 5, 5, 10, 10, MaskEffect::Blackout);

        // Inside masked hole: solid black
        assert_eq!(img.get_pixel(5, 5), &Rgba([0, 0, 0, 255]));
        assert_eq!(img.get_pixel(14, 14), &Rgba([0, 0, 0, 255]));

        // Outside masked hole: white
        assert_eq!(img.get_pixel(4, 4), &Rgba([255, 255, 255, 255]));
        assert_eq!(img.get_pixel(15, 15), &Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn test_apply_redaction_pixelate_and_blur() {
        let mut img = RgbaImage::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                img.put_pixel(x, y, Rgba([(x * 2) as u8, (y * 2) as u8, 128, 255]));
            }
        }

        // Test pixelate
        apply_redaction(&mut img, 10, 10, 30, 30, MaskEffect::Pixelate);
        // Pixels within the same 12x12 block should have identical color
        let p1 = *img.get_pixel(10, 10);
        let p2 = *img.get_pixel(15, 15);
        assert_eq!(p1, p2);

        // Test blur
        apply_redaction(&mut img, 50, 50, 40, 40, MaskEffect::Blur);

        // Test out of bounds / zero dimension defense
        apply_redaction(&mut img, 200, 200, 50, 50, MaskEffect::Transparent);
        apply_redaction(&mut img, 10, 10, 0, 0, MaskEffect::Transparent);
    }

    #[test]
    fn test_apply_redactions_wrapper() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            50,
            50,
            Rgba([100, 100, 100, 255]),
        ));
        let masks = vec![
            (0, 0, 10, 10, MaskEffect::Transparent),
            (20, 20, 10, 10, MaskEffect::Blackout),
        ];
        let redacted = apply_redactions(img, &masks);
        let rgba = redacted.into_rgba8();

        assert_eq!(rgba.get_pixel(0, 0), &Rgba([0, 0, 0, 0]));
        assert_eq!(rgba.get_pixel(20, 20), &Rgba([0, 0, 0, 255]));
        assert_eq!(rgba.get_pixel(40, 40), &Rgba([100, 100, 100, 255]));
    }

    #[test]
    fn test_apply_grid_overlay() {
        let img =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 200, Rgba([0, 0, 0, 255])));
        let with_grid = apply_grid_overlay(img, 50);
        let rgba = with_grid.into_rgba8();

        // Pixel at (50, 50) is crosshair center
        assert_eq!(rgba.get_pixel(50, 50), &Rgba([255, 70, 70, 220]));
        // Label starts at (52, 52), background is dark alpha blend, character pixels are white
        assert_ne!(rgba.get_pixel(53, 53), &Rgba([0, 0, 0, 255]));
        // Step below 10 is ignored
        let orig =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(50, 50, Rgba([1, 2, 3, 255])));
        let unchanged = apply_grid_overlay(orig.clone(), 5);
        assert_eq!(orig.into_rgba8(), unchanged.into_rgba8());
    }

    #[test]
    fn test_calculate_image_diff_identical() {
        let img1 = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            100,
            100,
            Rgba([128, 128, 128, 255]),
        ));
        let img2 = img1.clone();
        let diff = calculate_image_diff(&img1, &img2, 16);
        assert!(!diff.changed);
        assert_eq!(diff.changed_pixels, 0);
        assert_eq!(diff.total_pixels, 10000);
        assert_eq!(diff.changed_percent, 0.0);
        assert_eq!(diff.bbox, None);
    }

    #[test]
    fn test_calculate_image_diff_modified_and_crop() {
        let img1 = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            100,
            100,
            Rgba([128, 128, 128, 255]),
        ));
        let mut img2_rgba = RgbaImage::from_pixel(100, 100, Rgba([128, 128, 128, 255]));

        // Modify a 20x10 rectangle from (30, 40) to (49, 49)
        for y in 40..50 {
            for x in 30..50 {
                img2_rgba.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        let img2 = DynamicImage::ImageRgba8(img2_rgba);

        let diff = calculate_image_diff(&img2, &img1, 16);
        assert!(diff.changed);
        assert_eq!(diff.changed_pixels, 200);
        assert_eq!(diff.total_pixels, 10000);
        assert_eq!(diff.changed_percent, 2.0);
        assert_eq!(
            diff.bbox,
            Some(DiffBBox {
                x: 30,
                y: 40,
                width: 20,
                height: 10
            })
        );

        // Test crop to diff bbox with padding=5
        let cropped = crop_to_diff_bbox(img2, diff.bbox.as_ref().unwrap(), 5);
        assert_eq!(cropped.width(), 30); // 20 + 5*2
        assert_eq!(cropped.height(), 20); // 10 + 5*2
    }

    #[test]
    #[ignore = "requires system clipboard access"]
    fn test_clipboard_copy() {
        let mut img = RgbaImage::new(4, 4);
        img.put_pixel(0, 0, Rgba([0, 255, 0, 255]));
        let dynamic = DynamicImage::ImageRgba8(img);

        let result = copy_to_clipboard(&dynamic);
        assert!(
            result.is_ok(),
            "Clipboard copy should succeed: {:?}",
            result.err()
        );
    }
}
