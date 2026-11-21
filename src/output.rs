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
        if !candidate.exists() {
            return Ok(candidate);
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
            ImageFormat::Gif => image::ImageFormat::Gif,
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

/// Save a sequence of image frames into an animated GIF file.
pub fn save_animated_gif(
    frames: &[DynamicImage],
    fps: u32,
    path: &Path,
) -> Result<(), OutputError> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame};

    if frames.is_empty() {
        return Err(OutputError::InvalidPath(
            "Cannot encode animated GIF: frame buffer is empty".to_string(),
        ));
    }

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

    let mut encoder = GifEncoder::new(file);
    let _ = encoder.set_repeat(Repeat::Infinite);

    let delay_ms = (1000 / fps.max(1)).max(10);
    let delay = Delay::from_numer_denom_ms(delay_ms, 1);

    for frame in frames {
        let rgba = frame.to_rgba8();
        let gif_frame = Frame::from_parts(rgba, 0, 0, delay);
        encoder
            .encode_frame(gif_frame)
            .map_err(|e| OutputError::SaveFailed {
                path: path.to_path_buf(),
                source: e,
            })?;
    }

    Ok(())
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
}
