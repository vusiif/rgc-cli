use chrono::Local;
use image::DynamicImage;
use log::debug;
use std::fs;
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use thiserror::Error;

pub use crate::cli::ImageFormat;

// Re-export API and Transform modules so all existing output::* call sites remain 100% compatible
pub use crate::api::*;
pub use crate::transform::*;

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

impl From<OutputError> for ApiError {
    fn from(err: OutputError) -> Self {
        match err {
            OutputError::ClipboardFailed(msg) => {
                ApiError::new(ErrorCode::ClipboardFailed, msg).with_stage("clipboard")
            }
            OutputError::StdoutFailed(e) => ApiError::new(
                ErrorCode::EncodeFailed,
                format!("Failed to encode image for stdout stream: {e}"),
            )
            .with_stage("stdout_encode"),
            OutputError::StdoutIoFailed(e) => ApiError::new(
                ErrorCode::IoError,
                format!("Failed to stream image bytes to stdout: {e}"),
            )
            .with_stage("stdout_stream"),
            OutputError::CreateDirFailed { path, source } => ApiError::new(
                ErrorCode::SaveFailed,
                format!(
                    "Failed to create output directory '{}': {source}",
                    path.display()
                ),
            )
            .with_stage("filesystem_mkdir")
            .with_suggestion("Ensure you have write permissions for this directory path."),
            OutputError::SaveFailed { path, source } => ApiError::new(
                ErrorCode::SaveFailed,
                format!(
                    "Failed to save screenshot to '{}': {source}",
                    path.display()
                ),
            )
            .with_stage("filesystem_write")
            .with_suggestion("Check disk space and write permissions for the destination."),
            OutputError::FormatMismatch {
                explicit,
                inferred,
                path,
            } => ApiError::new(
                ErrorCode::FormatMismatch,
                format!(
                    "Conflicting image formats: --format was set to '{explicit:?}', \
                     but output path '{}' specifies a '{inferred:?}' extension.",
                    path.display()
                ),
            )
            .with_stage("format_validation")
            .with_suggestion("Align the --format flag with your file extension or omit --format."),
            OutputError::InvalidPath(msg) => ApiError::new(ErrorCode::BadArgs, msg)
                .with_stage("argument_validation")
                .with_suggestion(
                    "Check output path arguments: directories must not conflict with file targets.",
                ),
            OutputError::OpenViewerFailed(msg) => ApiError::new(
                ErrorCode::Unknown,
                format!("Failed to launch system image viewer: {msg}"),
            )
            .with_stage("system_viewer")
            .with_suggestion("Ensure a default image viewer is configured on your system."),
        }
    }
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
                "gif" => Some(ImageFormat::Gif),
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
                "gif" => return ImageFormat::Gif,
                _ => {}
            }
        }
    }

    ImageFormat::Png
}

/// Copy the captured image directly into the system clipboard with zero-copy RGBA borrowing.
pub fn copy_to_clipboard(image: &DynamicImage) -> Result<(), OutputError> {
    debug!("Borrowing or converting image to RGBA8 for system clipboard");
    let cow = borrow_or_convert_rgba8(image);
    let width = cow.width() as usize;
    let height = cow.height() as usize;
    let bytes: std::borrow::Cow<[u8]> = match &cow {
        std::borrow::Cow::Borrowed(rgba) => std::borrow::Cow::Borrowed(rgba.as_raw().as_slice()),
        std::borrow::Cow::Owned(rgba) => std::borrow::Cow::Owned(rgba.clone().into_raw()),
    };

    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| OutputError::ClipboardFailed(e.to_string()))?;

    let img_data = arboard::ImageData {
        width,
        height,
        bytes,
    };

    clipboard
        .set_image(img_data)
        .map_err(|e| OutputError::ClipboardFailed(e.to_string()))?;

    debug!("Image successfully written to clipboard buffer");
    Ok(())
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
        ImageFormat::Gif => {
            image
                .write_to(&mut buffer, image::ImageFormat::Gif)
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

/// Writes image bytes directly to standard output for shell pipeline workflows.
pub fn write_to_stdout(
    image: &DynamicImage,
    format: ImageFormat,
    quality: Option<u8>,
) -> Result<(), OutputError> {
    use std::io::ErrorKind;

    debug!("Streaming image bytes to stdout in format: {:?}", format);
    let bytes = encode_image_bytes(image, format, quality)?;

    let mut out = stdout().lock();
    use std::io::Write;
    if let Err(e) = out.write_all(&bytes) {
        if e.kind() == ErrorKind::BrokenPipe {
            debug!("Downstream pipe closed early (BrokenPipe)");
            return Ok(());
        }
        return Err(OutputError::StdoutIoFailed(e));
    }
    if let Err(e) = out.flush() {
        if e.kind() == ErrorKind::BrokenPipe {
            debug!("Downstream pipe closed early during flush (BrokenPipe)");
            return Ok(());
        }
        return Err(OutputError::StdoutIoFailed(e));
    }
    Ok(())
}

/// Generates a millisecond-precision, collision-resistant screenshot filename.
pub fn generate_default_filename(format: ImageFormat) -> String {
    let now = Local::now();
    let seq = SEQUENCE_COUNTER.fetch_add(1, Ordering::Relaxed) % 10000;
    format!(
        "screenshot_{}_{:04}.{}",
        now.format("%Y%m%d_%H%M%S_%3f"),
        seq,
        format.extension()
    )
}

/// Resolves an automated unique filename inside the given directory target.
/// Uses atomic creation (`create_new(true)`) to eliminate TOCTOU competition windows.
pub fn resolve_unique_auto_path(dir: &Path, format: ImageFormat) -> Result<PathBuf, OutputError> {
    if !dir.exists() {
        debug!("Creating target directory: {:?}", dir);
        fs::create_dir_all(dir).map_err(|e| OutputError::CreateDirFailed {
            path: dir.to_path_buf(),
            source: e,
        })?;
    }

    for _ in 0..100 {
        let candidate = dir.join(generate_default_filename(format));
        // Atomic exclusivity check: create_new(true) prevents multiple concurrent processes
        // from claiming the exact same candidate filename.
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(OutputError::SaveFailed {
                    path: candidate,
                    source: image::ImageError::IoError(e),
                });
            }
        }
    }

    Ok(dir.join(generate_default_filename(format)))
}

/// Resolves an `OutputDestination` into a concrete, canonical `PathBuf` on the filesystem.
pub fn resolve_output_path(
    destination: &OutputDestination,
    format: ImageFormat,
) -> Result<PathBuf, OutputError> {
    match destination {
        OutputDestination::Stdout => Err(OutputError::InvalidPath(
            "Cannot resolve filesystem path for stdout destination".to_string(),
        )),
        OutputDestination::Auto => {
            let cwd = std::env::current_dir().map_err(|e| OutputError::CreateDirFailed {
                path: PathBuf::from("."),
                source: e,
            })?;
            resolve_unique_auto_path(&cwd, format)
        }
        OutputDestination::Directory(dir) => resolve_unique_auto_path(dir, format),
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
            Ok(file.clone())
        }
    }
}

/// Save a `DynamicImage` to the filesystem destination with optional JPEG quality.
/// Uses a temporary file write and atomic replace strategy to prevent partial writes.
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

    let parent = file_path.parent().unwrap_or_else(|| Path::new("."));
    let temp_name = format!(
        ".tmp_{}_{}_{}.tmp",
        std::process::id(),
        SEQUENCE_COUNTER.fetch_add(1, Ordering::Relaxed),
        format.extension()
    );
    let temp_path = parent.join(temp_name);

    let write_res = (|| -> Result<(), OutputError> {
        if format == ImageFormat::Jpeg && quality.is_some() {
            let bytes = encode_image_bytes(image, format, quality)?;
            fs::write(&temp_path, bytes).map_err(|e| OutputError::SaveFailed {
                path: file_path.clone(),
                source: image::ImageError::IoError(e),
            })?;
        } else {
            let image_format = match format {
                ImageFormat::Png => image::ImageFormat::Png,
                ImageFormat::Jpeg => image::ImageFormat::Jpeg,
                ImageFormat::Webp => image::ImageFormat::WebP,
                ImageFormat::Gif => image::ImageFormat::Gif,
            };

            image
                .save_with_format(&temp_path, image_format)
                .map_err(|e| OutputError::SaveFailed {
                    path: file_path.clone(),
                    source: e,
                })?;
        }
        Ok(())
    })();

    if let Err(e) = write_res {
        let _ = fs::remove_file(&temp_path);
        return Err(e);
    }

    // Atomic replace / commit
    if let Err(_rename_err) = fs::rename(&temp_path, &file_path) {
        // Fallback for cross-device filesystems or Windows replace restrictions
        if let Err(copy_err) = fs::copy(&temp_path, &file_path) {
            let _ = fs::remove_file(&temp_path);
            return Err(OutputError::SaveFailed {
                path: file_path,
                source: image::ImageError::IoError(copy_err),
            });
        }
        let _ = fs::remove_file(&temp_path);
    }

    Ok(file_path)
}

/// Opens a saved image file using the system's default image viewer.
pub fn open_file_in_viewer(path: &Path) -> Result<(), OutputError> {
    use std::process::Command;

    let target_path = if path.is_relative() {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    };

    let target_str = target_path.to_string_lossy();
    debug!("Opening image viewer for file: {}", target_str);

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(&*target_str)
            .spawn()
            .map_err(|e| OutputError::OpenViewerFailed(e.to_string()))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&*target_str)
            .spawn()
            .map_err(|e| OutputError::OpenViewerFailed(e.to_string()))?;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(&*target_str)
            .spawn()
            .map_err(|e| OutputError::OpenViewerFailed(e.to_string()))?;
    }

    Ok(())
}

/// A streaming GIF encoder that encodes and flushes frames incrementally to an underlying writer.
/// Drastically reduces memory consumption by avoiding buffering all uncompressed frames in memory.
pub struct GifStreamEncoder<W: std::io::Write> {
    encoder: image::codecs::gif::GifEncoder<W>,
    delay: image::Delay,
    frame_count: u32,
    dimensions: Option<(u32, u32)>,
}

impl<W: std::io::Write> GifStreamEncoder<W> {
    /// Create a new streaming GIF encoder wrapping an arbitrary `std::io::Write` destination.
    pub fn new(writer: W, fps: u32) -> Result<Self, OutputError> {
        use image::codecs::gif::{GifEncoder, Repeat};
        use image::Delay;

        let mut encoder = GifEncoder::new(writer);
        let _ = encoder.set_repeat(Repeat::Infinite);

        let delay_ms = (1000 / fps.clamp(1, 30)).max(10);
        let delay = Delay::from_numer_denom_ms(delay_ms, 1);

        Ok(Self {
            encoder,
            delay,
            frame_count: 0,
            dimensions: None,
        })
    }

    /// Encode a single RGBA frame immediately into the streaming GIF output.
    pub fn encode_frame(&mut self, rgba: &image::RgbaImage) -> Result<(), OutputError> {
        use image::Frame;

        let (w, h) = (rgba.width(), rgba.height());
        if let Some((first_w, first_h)) = self.dimensions {
            if (w, h) != (first_w, first_h) {
                return Err(OutputError::SaveFailed {
                    path: PathBuf::from("stream.gif"),
                    source: image::ImageError::Parameter(image::error::ParameterError::from_kind(
                        image::error::ParameterErrorKind::DimensionMismatch,
                    )),
                });
            }
        } else {
            self.dimensions = Some((w, h));
        }

        let frame = Frame::from_parts(rgba.clone(), 0, 0, self.delay);
        self.encoder
            .encode_frame(frame)
            .map_err(|e| OutputError::SaveFailed {
                path: PathBuf::from("stream.gif"),
                source: e,
            })?;

        self.frame_count += 1;
        Ok(())
    }

    /// Returns the total number of frames encoded so far.
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    /// Returns the resolution (width, height) of the stream if at least one frame was encoded.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        self.dimensions
    }
}

impl GifStreamEncoder<std::io::BufWriter<std::fs::File>> {
    /// Create a new streaming GIF encoder that writes directly to a file on disk.
    pub fn create_file(path: &Path, fps: u32) -> Result<Self, OutputError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| OutputError::CreateDirFailed {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
        }

        let file = fs::File::create(path).map_err(|e| OutputError::SaveFailed {
            path: path.to_path_buf(),
            source: image::ImageError::IoError(e),
        })?;

        let writer = std::io::BufWriter::with_capacity(128 * 1024, file);
        Self::new(writer, fps)
    }
}

/// Save a sequence of image frames into an animated GIF file.
pub fn save_animated_gif(
    frames: &[DynamicImage],
    fps: u32,
    path: &Path,
) -> Result<(), OutputError> {
    if frames.is_empty() {
        return Err(OutputError::InvalidPath(
            "Cannot encode animated GIF: frame buffer is empty".to_string(),
        ));
    }

    let mut stream = GifStreamEncoder::create_file(path, fps)?;
    for frame in frames {
        stream.encode_frame(&frame.to_rgba8())?;
    }

    Ok(())
}

/// Helper to extract valid frame bitstream subchunks (ALPH, VP8, VP8L) from a single encoded WebP image.
pub fn extract_frame_subchunks(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let mut offset = 12;
    let mut start_chunk = None;
    while offset + 8 <= bytes.len() {
        let fourcc = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        let padded_size = size + (size % 2);
        let next_offset = offset + 8 + padded_size;

        if (fourcc == b"VP8L" || fourcc == b"VP8 " || fourcc == b"ALPH") && start_chunk.is_none() {
            start_chunk = Some(offset);
        }
        offset = next_offset;
    }

    start_chunk.map(|start| &bytes[start..])
}

/// A streaming WebP animation encoder that encodes and flushes frames incrementally using RIFF WebP chunks.
/// Delivers full truecolor (24-bit + 8-bit alpha) recording with dramatically better compression and quality than GIF.
pub struct WebpStreamEncoder<W: std::io::Write + std::io::Seek> {
    writer: W,
    fps: u32,
    frame_count: u32,
    dimensions: Option<(u32, u32)>,
    total_bytes: u64,
    finished: bool,
}

impl<W: std::io::Write + std::io::Seek> WebpStreamEncoder<W> {
    /// Create a new streaming WebP encoder wrapping an arbitrary `std::io::Write + std::io::Seek` destination.
    pub fn new(mut writer: W, fps: u32) -> Result<Self, OutputError> {
        // Write placeholder RIFF header: 12 bytes
        writer
            .write_all(b"RIFF\0\0\0\0WEBP")
            .map_err(OutputError::StdoutIoFailed)?;

        Ok(Self {
            writer,
            fps: fps.clamp(1, 60),
            frame_count: 0,
            dimensions: None,
            total_bytes: 12,
            finished: false,
        })
    }

    /// Encode a single RGBA frame immediately into the streaming WebP animation container.
    pub fn encode_frame(&mut self, rgba: &image::RgbaImage) -> Result<(), OutputError> {
        let (w, h) = (rgba.width(), rgba.height());
        if w == 0 || h == 0 {
            return Err(OutputError::InvalidPath(
                "Frame dimensions cannot be 0".to_string(),
            ));
        }

        if let Some((first_w, first_h)) = self.dimensions {
            if (w, h) != (first_w, first_h) {
                return Err(OutputError::SaveFailed {
                    path: PathBuf::from("stream.webp"),
                    source: image::ImageError::Parameter(image::error::ParameterError::from_kind(
                        image::error::ParameterErrorKind::DimensionMismatch,
                    )),
                });
            }
        } else {
            self.dimensions = Some((w, h));

            // 1. Write VP8X chunk (18 bytes total)
            let mut vp8x = Vec::with_capacity(18);
            vp8x.extend_from_slice(b"VP8X");
            vp8x.extend_from_slice(&10u32.to_le_bytes());
            vp8x.extend_from_slice(&[0x12, 0x00, 0x00, 0x00]); // Animation flag (0x02) | Alpha flag (0x10)
            let w_minus_1 = w.saturating_sub(1);
            let h_minus_1 = h.saturating_sub(1);
            vp8x.push((w_minus_1 & 0xFF) as u8);
            vp8x.push(((w_minus_1 >> 8) & 0xFF) as u8);
            vp8x.push(((w_minus_1 >> 16) & 0xFF) as u8);
            vp8x.push((h_minus_1 & 0xFF) as u8);
            vp8x.push(((h_minus_1 >> 8) & 0xFF) as u8);
            vp8x.push(((h_minus_1 >> 16) & 0xFF) as u8);

            self.writer
                .write_all(&vp8x)
                .map_err(OutputError::StdoutIoFailed)?;
            self.total_bytes += vp8x.len() as u64;

            // 2. Write ANIM chunk (14 bytes total)
            let mut anim = Vec::with_capacity(14);
            anim.extend_from_slice(b"ANIM");
            anim.extend_from_slice(&6u32.to_le_bytes());
            anim.extend_from_slice(&[0, 0, 0, 0]); // Background color BGRA: black transparent
            anim.extend_from_slice(&0u16.to_le_bytes()); // Loop count: 0 = infinite

            self.writer
                .write_all(&anim)
                .map_err(OutputError::StdoutIoFailed)?;
            self.total_bytes += anim.len() as u64;
        }

        // 3. Encode single frame RGBA into temporary WebP buffer
        let mut single_buf = std::io::Cursor::new(Vec::new());
        let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut single_buf);
        encoder
            .encode(rgba.as_raw(), w, h, image::ExtendedColorType::Rgba8)
            .map_err(|e| OutputError::SaveFailed {
                path: PathBuf::from("frame.webp"),
                source: e,
            })?;

        let raw_bytes = single_buf.into_inner();
        let subchunks =
            extract_frame_subchunks(&raw_bytes).ok_or_else(|| OutputError::SaveFailed {
                path: PathBuf::from("frame.webp"),
                source: image::ImageError::Decoding(image::error::DecodingError::from_format_hint(
                    image::error::ImageFormatHint::Exact(image::ImageFormat::WebP),
                )),
            })?;

        // 4. Wrap into ANMF chunk
        let payload_size = 16 + subchunks.len();
        let mut anmf = Vec::with_capacity(8 + payload_size + 1);
        anmf.extend_from_slice(b"ANMF");
        anmf.extend_from_slice(&(payload_size as u32).to_le_bytes());

        // Frame X (3 bytes, 24-bit little endian, X / 2) -> 0
        anmf.extend_from_slice(&[0, 0, 0]);
        // Frame Y (3 bytes, 24-bit little endian, Y / 2) -> 0
        anmf.extend_from_slice(&[0, 0, 0]);

        // Frame Width - 1 (3 bytes)
        let w_minus_1 = w.saturating_sub(1);
        anmf.push((w_minus_1 & 0xFF) as u8);
        anmf.push(((w_minus_1 >> 8) & 0xFF) as u8);
        anmf.push(((w_minus_1 >> 16) & 0xFF) as u8);

        // Frame Height - 1 (3 bytes)
        let h_minus_1 = h.saturating_sub(1);
        anmf.push((h_minus_1 & 0xFF) as u8);
        anmf.push(((h_minus_1 >> 8) & 0xFF) as u8);
        anmf.push(((h_minus_1 >> 16) & 0xFF) as u8);

        // Frame Duration in ms (3 bytes, uint24 little endian)
        let delay_ms = (1000 / self.fps).max(10);
        anmf.push((delay_ms & 0xFF) as u8);
        anmf.push(((delay_ms >> 8) & 0xFF) as u8);
        anmf.push(((delay_ms >> 16) & 0xFF) as u8);

        // Flags (1 byte): bit 1 = Blending (1 = Overwrite / Do not blend), bit 0 = Disposal (0 = Do not dispose)
        anmf.push(0x02);

        // Append subchunks payload
        anmf.extend_from_slice(subchunks);

        // RIFF 2-byte alignment padding
        if payload_size % 2 != 0 {
            anmf.push(0);
        }

        self.writer
            .write_all(&anmf)
            .map_err(OutputError::StdoutIoFailed)?;
        self.total_bytes += anmf.len() as u64;
        self.frame_count += 1;
        Ok(())
    }

    /// Finalizes the WebP animation by writing the total RIFF file size to the container header.
    pub fn finish(&mut self) -> Result<(), OutputError> {
        if self.finished || self.frame_count == 0 {
            return Ok(());
        }
        let riff_size = (self.total_bytes.saturating_sub(8)) as u32;
        self.writer
            .seek(std::io::SeekFrom::Start(4))
            .map_err(OutputError::StdoutIoFailed)?;
        self.writer
            .write_all(&riff_size.to_le_bytes())
            .map_err(OutputError::StdoutIoFailed)?;
        self.writer
            .seek(std::io::SeekFrom::End(0))
            .map_err(OutputError::StdoutIoFailed)?;
        self.writer.flush().map_err(OutputError::StdoutIoFailed)?;
        self.finished = true;
        Ok(())
    }

    /// Returns the total number of frames encoded so far.
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    /// Returns the resolution (width, height) of the stream if at least one frame was encoded.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        self.dimensions
    }
}

impl<W: std::io::Write + std::io::Seek> Drop for WebpStreamEncoder<W> {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

impl WebpStreamEncoder<std::io::BufWriter<std::fs::File>> {
    /// Create a new streaming WebP encoder that writes directly to a file on disk.
    pub fn create_file(path: &Path, fps: u32) -> Result<Self, OutputError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| OutputError::CreateDirFailed {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
        }

        let file = fs::File::create(path).map_err(|e| OutputError::SaveFailed {
            path: path.to_path_buf(),
            source: image::ImageError::IoError(e),
        })?;

        let writer = std::io::BufWriter::with_capacity(128 * 1024, file);
        Self::new(writer, fps)
    }
}

/// Save a sequence of image frames into an animated WebP file.
pub fn save_animated_webp(
    frames: &[DynamicImage],
    fps: u32,
    path: &Path,
) -> Result<(), OutputError> {
    if frames.is_empty() {
        return Err(OutputError::InvalidPath(
            "Cannot encode animated WebP: frame buffer is empty".to_string(),
        ));
    }

    let mut stream = WebpStreamEncoder::create_file(path, fps)?;
    for frame in frames {
        stream.encode_frame(&frame.to_rgba8())?;
    }
    stream.finish()?;
    Ok(())
}

/// Unified streaming recording encoder supporting both GIF and truecolor Animated WebP.
pub enum RecordingStreamEncoder<W: std::io::Write + std::io::Seek> {
    Gif(GifStreamEncoder<W>),
    Webp(WebpStreamEncoder<W>),
}

impl<W: std::io::Write + std::io::Seek> RecordingStreamEncoder<W> {
    pub fn encode_frame(&mut self, rgba: &image::RgbaImage) -> Result<(), OutputError> {
        match self {
            RecordingStreamEncoder::Gif(s) => s.encode_frame(rgba),
            RecordingStreamEncoder::Webp(s) => s.encode_frame(rgba),
        }
    }

    pub fn frame_count(&self) -> u32 {
        match self {
            RecordingStreamEncoder::Gif(s) => s.frame_count(),
            RecordingStreamEncoder::Webp(s) => s.frame_count(),
        }
    }

    pub fn dimensions(&self) -> Option<(u32, u32)> {
        match self {
            RecordingStreamEncoder::Gif(s) => s.dimensions(),
            RecordingStreamEncoder::Webp(s) => s.dimensions(),
        }
    }

    pub fn finish(&mut self) -> Result<(), OutputError> {
        match self {
            RecordingStreamEncoder::Gif(_) => Ok(()),
            RecordingStreamEncoder::Webp(s) => s.finish(),
        }
    }
}

impl RecordingStreamEncoder<std::io::BufWriter<std::fs::File>> {
    pub fn create_file(path: &Path, fps: u32, is_webp: bool) -> Result<Self, OutputError> {
        if is_webp {
            Ok(RecordingStreamEncoder::Webp(
                WebpStreamEncoder::create_file(path, fps)?,
            ))
        } else {
            Ok(RecordingStreamEncoder::Gif(GifStreamEncoder::create_file(
                path, fps,
            )?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn test_determine_output_destination() {
        assert_eq!(
            determine_output_destination(Some(Path::new("-")), None, None).unwrap(),
            OutputDestination::Stdout
        );

        let file_path = PathBuf::from("test.png");
        assert_eq!(
            determine_output_destination(Some(&file_path), None, None).unwrap(),
            OutputDestination::File(file_path.clone())
        );

        let explicit_file = PathBuf::from("explicit.jpg");
        assert_eq!(
            determine_output_destination(None, Some(&explicit_file), None).unwrap(),
            OutputDestination::File(explicit_file)
        );

        let dir_path = PathBuf::from("./test_dir/");
        assert_eq!(
            determine_output_destination(Some(&dir_path), None, None).unwrap(),
            OutputDestination::Directory(dir_path.clone())
        );

        assert_eq!(
            determine_output_destination(None, None, None).unwrap(),
            OutputDestination::Auto
        );
    }

    #[test]
    fn test_is_directory_target() {
        assert!(is_directory_target(Path::new("some_dir/")));
        assert!(is_directory_target(Path::new("some_dir\\")));
        assert!(is_directory_target(Path::new(".")));
        assert!(is_directory_target(Path::new("..")));
        assert!(!is_directory_target(Path::new("file.png")));
    }

    #[test]
    fn test_validate_and_infer_format_mismatch() {
        let dest = OutputDestination::File(PathBuf::from("image.jpg"));
        let err = validate_and_infer_format(&dest, Some(ImageFormat::Png));
        assert!(matches!(err, Err(OutputError::FormatMismatch { .. })));

        let ok_dest = OutputDestination::File(PathBuf::from("image.webp"));
        assert_eq!(
            validate_and_infer_format(&ok_dest, Some(ImageFormat::Webp)).unwrap(),
            ImageFormat::Webp
        );
    }

    #[test]
    fn test_infer_format() {
        assert_eq!(
            infer_format(Some(Path::new("test.jpg")), None),
            ImageFormat::Jpeg
        );
        assert_eq!(
            infer_format(Some(Path::new("test.webp")), None),
            ImageFormat::Webp
        );
        assert_eq!(
            infer_format(Some(Path::new("test.png")), None),
            ImageFormat::Png
        );
        assert_eq!(
            infer_format(Some(Path::new("test.unknown")), None),
            ImageFormat::Png
        );
        assert_eq!(infer_format(None, None), ImageFormat::Png);
    }

    #[test]
    fn test_generate_default_filename_uniqueness() {
        let name1 = generate_default_filename(ImageFormat::Png);
        let name2 = generate_default_filename(ImageFormat::Png);
        assert!(name1.ends_with(".png"));
        assert_ne!(name1, name2);
    }

    #[test]
    fn test_resolve_output_path() {
        let dir = PathBuf::from("test_resolve_dir");
        let path =
            resolve_output_path(&OutputDestination::Directory(dir.clone()), ImageFormat::Png)
                .unwrap();
        assert!(path.starts_with(&dir));
        assert!(path.extension().unwrap() == "png");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_save_image_png_and_webp() {
        let img = DynamicImage::ImageRgba8(RgbaImage::new(10, 10));
        let temp_png = PathBuf::from("temp_test.png");
        let temp_webp = PathBuf::from("temp_test.webp");

        save_image(
            &img,
            &OutputDestination::File(temp_png.clone()),
            ImageFormat::Png,
            None,
        )
        .unwrap();
        assert!(temp_png.exists());
        let _ = fs::remove_file(temp_png);

        save_image(
            &img,
            &OutputDestination::File(temp_webp.clone()),
            ImageFormat::Webp,
            None,
        )
        .unwrap();
        assert!(temp_webp.exists());
        let _ = fs::remove_file(temp_webp);
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

    #[test]
    fn test_save_animated_gif() {
        let f1 = DynamicImage::ImageRgba8(RgbaImage::from_pixel(16, 16, Rgba([255, 0, 0, 255])));
        let f2 = DynamicImage::ImageRgba8(RgbaImage::from_pixel(16, 16, Rgba([0, 255, 0, 255])));
        let target = PathBuf::from("temp_anim_test.gif");
        let res = save_animated_gif(&[f1, f2], 5, &target);
        assert!(res.is_ok());
        assert!(target.exists());
        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_gif_stream_encoder_in_memory_and_file() {
        use std::io::Cursor;
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut stream = GifStreamEncoder::new(&mut buffer, 10).unwrap();

            let frame1 = RgbaImage::from_pixel(32, 32, Rgba([255, 100, 0, 255]));
            let frame2 = RgbaImage::from_pixel(32, 32, Rgba([0, 100, 255, 255]));

            stream.encode_frame(&frame1).unwrap();
            stream.encode_frame(&frame2).unwrap();

            assert_eq!(stream.frame_count(), 2);
            assert_eq!(stream.dimensions(), Some((32, 32)));
        }

        let encoded_bytes = buffer.into_inner();
        assert!(!encoded_bytes.is_empty());
        assert_eq!(&encoded_bytes[0..3], b"GIF");
    }

    #[test]
    fn test_webp_stream_encoder_in_memory_and_file() {
        use std::io::Cursor;
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut stream = WebpStreamEncoder::new(&mut buffer, 10).unwrap();

            let frame1 = RgbaImage::from_pixel(32, 32, Rgba([255, 100, 0, 255]));
            let frame2 = RgbaImage::from_pixel(32, 32, Rgba([0, 100, 255, 255]));

            stream.encode_frame(&frame1).unwrap();
            stream.encode_frame(&frame2).unwrap();

            assert_eq!(stream.frame_count(), 2);
            assert_eq!(stream.dimensions(), Some((32, 32)));
            stream.finish().unwrap();
        }

        let encoded_bytes = buffer.into_inner();
        assert!(encoded_bytes.len() > 30);
        assert_eq!(&encoded_bytes[0..4], b"RIFF");
        assert_eq!(&encoded_bytes[8..12], b"WEBP");
        assert_eq!(&encoded_bytes[12..16], b"VP8X");

        // Verify RIFF size matches total bytes - 8
        let riff_size = u32::from_le_bytes(encoded_bytes[4..8].try_into().unwrap());
        assert_eq!(riff_size as usize, encoded_bytes.len() - 8);

        // Test file creation
        let temp_webp = PathBuf::from("temp_stream_test.webp");
        {
            let mut file_stream = WebpStreamEncoder::create_file(&temp_webp, 10).unwrap();
            let frame = RgbaImage::from_pixel(16, 16, Rgba([120, 200, 50, 255]));
            file_stream.encode_frame(&frame).unwrap();
            file_stream.finish().unwrap();
        }
        assert!(temp_webp.exists());
        let meta = fs::metadata(&temp_webp).unwrap();
        assert!(meta.len() > 20);
        let _ = fs::remove_file(temp_webp);
    }

    #[test]
    fn test_save_animated_webp() {
        let f1 = DynamicImage::ImageRgba8(RgbaImage::from_pixel(16, 16, Rgba([255, 0, 0, 255])));
        let f2 = DynamicImage::ImageRgba8(RgbaImage::from_pixel(16, 16, Rgba([0, 255, 0, 255])));
        let target = PathBuf::from("temp_anim_test.webp");
        let res = save_animated_webp(&[f1, f2], 5, &target);
        assert!(res.is_ok());
        assert!(target.exists());
        let _ = fs::remove_file(target);
    }

    #[test]
    fn test_recording_stream_encoder_unified() {
        use std::io::Cursor;
        let mut gif_buf = Cursor::new(Vec::new());
        let mut gif_stream =
            RecordingStreamEncoder::Gif(GifStreamEncoder::new(&mut gif_buf, 10).unwrap());
        let frame = RgbaImage::from_pixel(10, 10, Rgba([255, 255, 255, 255]));
        gif_stream.encode_frame(&frame).unwrap();
        assert_eq!(gif_stream.frame_count(), 1);

        let mut webp_buf = Cursor::new(Vec::new());
        {
            let mut webp_stream =
                RecordingStreamEncoder::Webp(WebpStreamEncoder::new(&mut webp_buf, 10).unwrap());
            webp_stream.encode_frame(&frame).unwrap();
            webp_stream.finish().unwrap();
            assert_eq!(webp_stream.frame_count(), 1);
        }
        let webp_bytes = webp_buf.into_inner();
        assert_eq!(&webp_bytes[0..4], b"RIFF");
    }
}
