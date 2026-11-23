use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

/// Representation of a rectangular screen region (logical coordinates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl FromStr for Region {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
        if parts.len() != 4 {
            return Err("Region must be formatted as 'x,y,w,h' (e.g. 100,200,800,600)".to_string());
        }

        let x = parts[0]
            .parse::<i32>()
            .map_err(|e| format!("Invalid x coordinate: {e}"))?;
        let y = parts[1]
            .parse::<i32>()
            .map_err(|e| format!("Invalid y coordinate: {e}"))?;
        let width = parts[2]
            .parse::<u32>()
            .map_err(|e| format!("Invalid width: {e}"))?;
        let height = parts[3]
            .parse::<u32>()
            .map_err(|e| format!("Invalid height: {e}"))?;

        if width == 0 || height == 0 {
            return Err("Width and height of region must be greater than 0".to_string());
        }

        Ok(Region {
            x,
            y,
            width,
            height,
        })
    }
}

/// Coordinate space for region cropping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoordMode {
    /// Logical coordinates scaled by monitor DPI (default)
    #[default]
    Logical,
    /// Exact physical pixel coordinates (1:1 without DPI scaling)
    Physical,
}

/// Redaction and masking effects for privacy-sensitive regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskEffect {
    #[default]
    Transparent,
    Blackout,
    Pixelate,
    Blur,
}

impl FromStr for MaskEffect {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "transparent" | "clear" | "alpha" => Ok(MaskEffect::Transparent),
            "blackout" | "black" | "solid" => Ok(MaskEffect::Blackout),
            "pixelate" | "mosaic" => Ok(MaskEffect::Pixelate),
            "blur" | "gaussian" => Ok(MaskEffect::Blur),
            other => Err(format!(
                "Invalid mask effect '{other}': expected 'transparent', 'blackout', 'pixelate', or 'blur'"
            )),
        }
    }
}

/// A rectangular region with an associated redaction effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskRegion {
    pub region: Region,
    pub effect: MaskEffect,
}

impl FromStr for MaskRegion {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if let Some((reg_str, effect_str)) = trimmed.rsplit_once(':') {
            let effect = MaskEffect::from_str(effect_str)?;
            let region = Region::from_str(reg_str)?;
            Ok(MaskRegion { region, effect })
        } else {
            let region = Region::from_str(trimmed)?;
            Ok(MaskRegion {
                region,
                effect: MaskEffect::Transparent,
            })
        }
    }
}

/// A window target with an associated redaction effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskWindow {
    pub query: String,
    pub effect: MaskEffect,
}

impl FromStr for MaskWindow {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("Mask window target cannot be empty".to_string());
        }

        if let Some((left, right)) = trimmed.rsplit_once(':') {
            if let Ok(effect) = MaskEffect::from_str(right) {
                let query = left.trim().to_string();
                if query.is_empty() {
                    return Err("Mask window target query cannot be empty".to_string());
                }
                return Ok(MaskWindow { query, effect });
            }
        }

        Ok(MaskWindow {
            query: trimmed.to_string(),
            effect: MaskEffect::Transparent,
        })
    }
}

/// Supported output image formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default, Serialize, Deserialize)]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
    Webp,
    Gif,
}

impl ImageFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
            ImageFormat::Gif => "gif",
        }
    }
}

impl std::fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.extension())
    }
}

/// Parses and validates a downscaling factor between 0.01 and 1.0.
pub fn parse_scale_factor(s: &str) -> Result<f64, String> {
    let factor: f64 = s
        .trim()
        .parse()
        .map_err(|_| format!("Invalid scale '{s}': expected a floating-point number"))?;
    if !factor.is_finite() || factor <= 0.0 {
        return Err(format!(
            "Scale factor must be positive and non-zero (got {s})"
        ));
    }
    if factor > 1.0 {
        return Err(format!(
            "Scale factor cannot exceed 1.0 (downscaling only, got {s})"
        ));
    }
    Ok(factor)
}

/// Parses and validates a recording duration in seconds (must be finite positive <= 3600s).
pub fn parse_record_duration(s: &str) -> Result<f32, String> {
    let secs: f32 = s
        .trim()
        .parse()
        .map_err(|_| format!("Invalid record duration '{s}': expected a positive number of seconds"))?;
    if !secs.is_finite() || secs <= 0.0 {
        return Err(format!(
            "Recording duration must be a finite positive number of seconds (got '{s}')"
        ));
    }
    if secs > 3600.0 {
        return Err(format!(
            "Recording duration cannot exceed 3600.0 seconds (got '{s}')"
        ));
    }
    Ok(secs)
}

/// Cross-platform command-line screen capture utility.
#[derive(Parser, Debug)]
#[command(
    name = "rgc-cli",
    version,
    about = "Cross-platform screen capture CLI utility written in Rust",
    long_about = "Fast, lightweight, and cross-platform screen capture utility supporting multi-monitor selection, high-DPI scaling, clipboard export, and stdout streaming."
)]
pub struct Cli {
    /// Run diagnostic environment check and inspect capture capabilities
    #[arg(
        long = "doctor",
        conflicts_with_all = [
            "mcp", "list_monitors", "list_windows", "completions", "benchmark", "monitor", "window", "all_monitors", "region", "output",
            "output_file", "output_dir", "clipboard", "delay", "coords",
            "scale", "max_dimension", "quality", "open", "mask", "mask_window", "exclude_window",
            "grid", "diff_with", "diff_threshold", "crop_changed", "highlight_diff",
            "timestamp", "tag", "wait_change", "wait_timeout", "wait_interval",
            "cursor", "highlight_cursor", "record", "fps",
            "som", "som_threshold", "som_max_elements", "accessibility",
            "click_at", "drag", "verify_click", "verify_radius", "verify_timeout", "verify_threshold"
        ]
    )]
    pub doctor: bool,

    /// Start Model Context Protocol (MCP) stdio server for AI agent automation
    #[arg(
        long = "mcp",
        conflicts_with_all = [
            "doctor", "list_monitors", "list_windows", "completions", "benchmark", "monitor", "window", "all_monitors", "region", "output",
            "output_file", "output_dir", "clipboard", "delay", "format",
            "coords", "json", "scale", "max_dimension", "quality", "open", "mask", "mask_window", "exclude_window",
            "grid", "diff_with", "diff_threshold", "crop_changed", "highlight_diff",
            "timestamp", "tag", "wait_change", "wait_timeout", "wait_interval",
            "cursor", "highlight_cursor", "record", "fps",
            "som", "som_threshold", "som_max_elements", "accessibility",
            "click_at", "drag", "verify_click", "verify_radius", "verify_timeout", "verify_threshold"
        ]
    )]
    pub mcp: bool,

    /// List all available monitors and exit
    #[arg(
        short = 'l',
        long = "list-monitors",
        conflicts_with_all = [
            "doctor", "mcp", "list_windows", "completions", "benchmark", "monitor", "window", "all_monitors", "region", "output",
            "output_file", "output_dir", "clipboard", "delay", "coords",
            "scale", "max_dimension", "quality", "open", "mask", "mask_window", "exclude_window",
            "grid", "diff_with", "diff_threshold", "crop_changed", "highlight_diff",
            "timestamp", "tag", "wait_change", "wait_timeout", "wait_interval",
            "click_at", "drag", "verify_click", "verify_radius", "verify_timeout", "verify_threshold"
        ]
    )]
    pub list_monitors: bool,

    /// List all active application windows and exit
    #[arg(
        long = "list-windows",
        conflicts_with_all = [
            "doctor", "mcp", "list_monitors", "completions", "benchmark", "monitor", "window", "all_monitors", "region", "output",
            "output_file", "output_dir", "clipboard", "delay", "coords",
            "scale", "max_dimension", "quality", "open", "mask", "mask_window", "exclude_window",
            "grid", "diff_with", "diff_threshold", "crop_changed", "highlight_diff",
            "timestamp", "tag", "wait_change", "wait_timeout", "wait_interval",
            "click_at", "drag", "verify_click", "verify_radius", "verify_timeout", "verify_threshold"
        ]
    )]
    pub list_windows: bool,

    /// Generate shell completion script to stdout (bash, zsh, fish, powershell, elvish)
    #[arg(
        long = "completions",
        value_name = "SHELL",
        value_enum,
        conflicts_with_all = [
            "doctor", "mcp", "list_monitors", "list_windows", "benchmark", "monitor", "window", "all_monitors", "region", "output",
            "output_file", "output_dir", "clipboard", "delay", "coords",
            "scale", "max_dimension", "quality", "format", "json", "open", "mask", "mask_window", "exclude_window",
            "grid", "diff_with", "diff_threshold", "crop_changed", "highlight_diff",
            "timestamp", "tag", "wait_change", "wait_timeout", "wait_interval",
            "click_at", "drag", "verify_click", "verify_radius", "verify_timeout", "verify_threshold"
        ]
    )]
    pub completions: Option<clap_complete::Shell>,

    /// Monitor index to capture (0-indexed). Defaults to primary monitor
    #[arg(
        short = 'm',
        long = "monitor",
        value_name = "INDEX",
        conflicts_with_all = ["window", "all_monitors"]
    )]
    pub monitor: Option<usize>,

    /// Window to capture by ID or title substring (e.g. -w 1234 or -w "Code")
    #[arg(
        short = 'w',
        long = "window",
        value_name = "ID|TITLE",
        conflicts_with_all = ["monitor", "all_monitors"]
    )]
    pub window: Option<String>,

    /// Capture all connected monitors stitched into a unified virtual desktop panorama
    #[arg(
        short = 'a',
        long = "all-monitors",
        conflicts_with_all = ["monitor", "window"]
    )]
    pub all_monitors: bool,

    /// Region to capture in coordinates: 'x,y,w,h' (e.g. 100,100,800,600)
    #[arg(short = 'r', long = "region", value_name = "x,y,w,h")]
    pub region: Option<Region>,

    /// Coordinate system for region cropping: 'logical' (scaled by monitor DPI) or 'physical' (exact pixels)
    #[arg(long = "coords", value_enum, default_value_t = CoordMode::Logical)]
    pub coords: CoordMode,

    /// Redact rectangular region(s) on captured image: 'x,y,w,h[:effect]' (effect: transparent, blackout, pixelate, blur; default: transparent)
    #[arg(
        long = "mask",
        value_name = "x,y,w,h[:effect]",
        action = clap::ArgAction::Append
    )]
    pub mask: Vec<MaskRegion>,

    /// Redact active window(s) on captured screen: 'ID|TITLE[:effect]' (effect: transparent, blackout, pixelate, blur; default: transparent)
    #[arg(
        long = "mask-window",
        value_name = "ID|TITLE[:effect]",
        conflicts_with = "window",
        action = clap::ArgAction::Append
    )]
    pub mask_window: Vec<MaskWindow>,

    /// Exclude active window(s) completely from captured screen by title substring or ID
    #[arg(
        long = "exclude-window",
        value_name = "ID|TITLE",
        conflicts_with = "window",
        action = clap::ArgAction::Append
    )]
    pub exclude_window: Vec<String>,

    /// Allow export to proceed if specified --mask-window target is not found on desktop (default: fail-closed to protect privacy)
    #[arg(long = "allow-missing-mask")]
    pub allow_missing_mask: bool,

    /// Output file path or directory, or '-' for stdout stream. Defaults to timestamped file in CWD
    #[arg(
        short = 'o',
        long = "output",
        value_name = "PATH",
        conflicts_with_all = ["output_file", "output_dir"]
    )]
    pub output: Option<PathBuf>,

    /// Explicit output file destination (creates parent directories if missing)
    #[arg(
        long = "output-file",
        value_name = "FILE",
        conflicts_with_all = ["output", "output_dir"]
    )]
    pub output_file: Option<PathBuf>,

    /// Explicit output directory destination (timestamped filename generated inside)
    #[arg(
        long = "output-dir",
        value_name = "DIR",
        conflicts_with_all = ["output", "output_file"]
    )]
    pub output_dir: Option<PathBuf>,

    /// Copy screenshot directly to system clipboard
    #[arg(short = 'c', long = "clipboard")]
    pub clipboard: bool,

    /// Delay before capturing in milliseconds (e.g. 1000 for 1 second)
    #[arg(short = 'd', long = "delay", value_name = "MS")]
    pub delay: Option<u64>,

    /// Output image format. If omitted, automatically inferred from file extension or defaults to PNG
    #[arg(short = 'f', long = "format", value_enum)]
    pub format: Option<ImageFormat>,

    /// Downscale image by a factor between 0.01 and 1.0 (e.g. 0.5 for half resolution)
    #[arg(
        long = "scale",
        value_name = "FACTOR",
        conflicts_with = "max_dimension",
        value_parser = parse_scale_factor
    )]
    pub scale: Option<f64>,

    /// Constrain maximum image dimension (width or height) to PX pixels, preserving aspect ratio
    #[arg(
        long = "max-dimension",
        value_name = "PX",
        conflicts_with = "scale",
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub max_dimension: Option<u32>,

    /// Compression quality for JPEG output (1-100). Defaults to 85
    #[arg(
        long = "quality",
        value_name = "1-100",
        value_parser = clap::value_parser!(u8).range(1..=100)
    )]
    pub quality: Option<u8>,

    /// Output result as structured JSON (ideal for automation & AI agents)
    #[arg(long = "json")]
    pub json: bool,

    /// Overlay visual grounding grid lines and coordinate anchors for AI agent visual localization (default step: 100px)
    #[arg(
        long = "grid",
        value_name = "STEP",
        num_args = 0..=1,
        default_missing_value = "100",
        value_parser = clap::value_parser!(u32).range(10..)
    )]
    pub grid: Option<u32>,

    /// Compare captured screen with a previous reference image file and compute difference bounding box
    #[arg(long = "diff-with", value_name = "IMAGE_PATH")]
    pub diff_with: Option<PathBuf>,

    /// Pixel difference threshold (0-255, default: 16) for diff calculation to ignore minor noise
    #[arg(long = "diff-threshold", value_name = "0-255", default_value_t = 16)]
    pub diff_threshold: u8,

    /// Crop captured screenshot to only the bounding box of changed pixels compared to --diff-with
    #[arg(long = "crop-changed", requires = "diff_with")]
    pub crop_changed: bool,

    /// Highlight detected differences by drawing a high-contrast boundary box on the screenshot
    #[arg(long = "highlight-diff", requires = "diff_with")]
    pub highlight_diff: bool,

    /// Burn a discrete UTC/local timestamp watermark badge onto the output screenshot
    #[arg(long = "timestamp")]
    pub timestamp: bool,

    /// Burn a custom text tag (e.g. step name, session ID) onto the output screenshot
    #[arg(long = "tag", value_name = "TEXT")]
    pub tag: Option<String>,

    /// Wait for screen/window visual change before capturing (ideal for UI automation & click validation)
    #[arg(long = "wait-change")]
    pub wait_change: bool,

    /// Maximum timeout in milliseconds to wait for a visual change before capturing (default: 5000 ms)
    #[arg(long = "wait-timeout", value_name = "MS", default_value_t = 5000)]
    pub wait_timeout: u64,

    /// Polling interval in milliseconds between change detection frames (default: 100 ms)
    #[arg(long = "wait-interval", value_name = "MS", default_value_t = 100)]
    pub wait_interval: u64,

    /// Capture and render mouse cursor on the screenshot
    #[arg(long = "cursor")]
    pub cursor: bool,

    /// Render a prominent visual highlight ring around the mouse cursor
    #[arg(long = "highlight-cursor")]
    pub highlight_cursor: bool,

    /// Continuously sample and record screen over SECONDS duration (saved as animated GIF or image sequence)
    #[arg(
        long = "record",
        value_name = "SECS",
        conflicts_with_all = ["diff_with", "crop_changed", "highlight_diff", "verify_click"],
        value_parser = parse_record_duration
    )]
    pub record: Option<f32>,

    /// Sampling framerate (FPS, 1-30) for continuous recording (default: 5)
    #[arg(
        long = "fps",
        value_name = "1-30",
        default_value_t = 5,
        value_parser = clap::value_parser!(u32).range(1..=30)
    )]
    pub fps: u32,

    /// Detect interactive UI elements and overlay Set-of-Mark (SoM) numbered badges for AI navigation
    #[arg(long = "som")]
    pub som: bool,

    /// Sensitivity / edge threshold (5-100, default: 25) for UI element detection
    #[arg(long = "som-threshold", value_name = "1-100", default_value_t = 25)]
    pub som_threshold: u8,

    /// Maximum number of UI elements to detect and mark (default: 50)
    #[arg(long = "som-max-elements", value_name = "COUNT", default_value_t = 50)]
    pub som_max_elements: usize,

    /// Fuse UI automation accessibility tree semantics (role, label, state) with Set-of-Mark detections
    #[arg(long = "accessibility", visible_alias = "a11y")]
    pub accessibility: bool,

    /// Overlay a visual click ripple and target indicator at coordinate X,Y (e.g. 500,300 or 500,300,submit)
    #[arg(long = "click-at", value_name = "X,Y[,LABEL]", action = clap::ArgAction::Append)]
    pub click_at: Vec<String>,

    /// Overlay a visual drag trajectory arrow between coordinates (e.g. 100,200,400,500 or 100,200,400,500,swipe)
    #[arg(long = "drag", value_name = "X1,Y1,X2,Y2[,LABEL]", action = clap::ArgAction::Append)]
    pub drag: Vec<String>,

    /// Verify whether a visual change occurred within a radius around X,Y after interaction
    #[arg(long = "verify-click", value_name = "X,Y")]
    pub verify_click: Option<String>,

    /// Search radius in pixels around interaction coordinate for change verification (default: 100, 0 for full screen)
    #[arg(long = "verify-radius", value_name = "PIXELS", default_value_t = 100)]
    pub verify_radius: u32,

    /// Wait duration in milliseconds before sampling follow-up state for verification (default: 500 ms)
    #[arg(long = "verify-timeout", value_name = "MS", default_value_t = 500)]
    pub verify_timeout: u64,

    /// Color difference threshold (1-255, default: 10) for action change verification
    #[arg(long = "verify-threshold", value_name = "1-255", default_value_t = 10)]
    pub verify_threshold: u8,

    /// Optional path to configuration file (defaults to ./rgc.toml or ~/.config/rgc/config.toml)
    #[arg(long = "config", value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,

    /// Continuous watch mode: poll screen periodically and emit events/files on visual change.
    /// Optional value specifies check interval in milliseconds (default: 500 ms).
    #[arg(
        long = "watch",
        value_name = "INTERVAL_MS",
        num_args = 0..=1,
        default_missing_value = "500",
        conflicts_with_all = ["record", "completions", "doctor", "mcp"]
    )]
    pub watch: Option<u64>,

    /// Sensitivity threshold (0-255) for detecting visual changes in --watch mode (default: 16)
    #[arg(long = "watch-threshold", value_name = "N", default_value_t = 16)]
    pub watch_threshold: u8,

    /// Minimum percent of changed pixels (0.01 - 100.0) required to trigger a --watch event (default: 0.05%)
    #[arg(long = "watch-min-percent", value_name = "PERCENT", default_value_t = 0.05)]
    pub watch_min_percent: f64,

    /// Maximum number of change events to trigger before terminating --watch mode (default: infinite)
    #[arg(long = "watch-count", value_name = "COUNT")]
    pub watch_count: Option<u32>,

    /// Timeout in seconds before automatically terminating --watch mode
    #[arg(long = "watch-timeout", value_name = "SECS")]
    pub watch_timeout: Option<f32>,

    /// Run comprehensive performance and latency benchmarks (capture, processing, encoders)
    #[arg(
        long = "benchmark",
        value_name = "RUNS",
        num_args = 0..=1,
        default_missing_value = "5",
        conflicts_with_all = [
            "record", "completions", "doctor", "mcp", "list_monitors", "list_windows", "watch"
        ]
    )]
    pub benchmark: Option<u32>,

    /// Open captured screenshot file with system default viewer
    #[arg(
        long = "open",
        conflicts_with_all = ["doctor", "mcp", "list_monitors", "list_windows", "completions"]
    )]
    pub open: bool,

    /// Enable verbose / debug logging
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Suppress non-error output
    #[arg(short = 'q', long = "quiet")]
    pub quiet: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_region_valid() {
        let region = "100,200,800,600".parse::<Region>().unwrap();
        assert_eq!(
            region,
            Region {
                x: 100,
                y: 200,
                width: 800,
                height: 600
            }
        );
    }

    #[test]
    fn test_parse_region_with_spaces() {
        let region = " 50 , -10 , 1920 , 1080 ".parse::<Region>().unwrap();
        assert_eq!(
            region,
            Region {
                x: 50,
                y: -10,
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn test_parse_region_invalid_zero_dimension() {
        assert!("100,200,0,600".parse::<Region>().is_err());
        assert!("100,200,800,0".parse::<Region>().is_err());
    }

    #[test]
    fn test_parse_region_invalid_format() {
        assert!("100,200,800".parse::<Region>().is_err());
        assert!("abc,200,800,600".parse::<Region>().is_err());
    }

    #[test]
    fn test_cli_parsing_defaults() {
        let cli = Cli::try_parse_from(["rgc-cli"]).unwrap();
        assert_eq!(cli.monitor, None);
        assert_eq!(cli.region, None);
        assert_eq!(cli.output, None);
        assert!(!cli.clipboard);
        assert_eq!(cli.delay, None);
        assert_eq!(cli.format, None);
        assert!(!cli.json);
        assert!(!cli.list_monitors);
    }

    #[test]
    fn test_cli_parsing_options() {
        let cli = Cli::try_parse_from([
            "rgc-cli",
            "--monitor",
            "1",
            "--region",
            "10,20,300,400",
            "--output",
            "screenshot.webp",
            "-c",
            "--delay",
            "500",
            "--format",
            "webp",
            "--json",
            "-v",
        ])
        .unwrap();

        assert_eq!(cli.monitor, Some(1));
        assert_eq!(
            cli.region,
            Some(Region {
                x: 10,
                y: 20,
                width: 300,
                height: 400
            })
        );
        assert_eq!(cli.output.unwrap(), PathBuf::from("screenshot.webp"));
        assert!(cli.clipboard);
        assert_eq!(cli.delay, Some(500));
        assert_eq!(cli.format, Some(ImageFormat::Webp));
        assert!(cli.json);
        assert!(cli.verbose);
    }

    #[test]
    fn test_cli_parsing_output_file_and_dir() {
        let cli_file = Cli::try_parse_from(["rgc-cli", "--output-file", "dest.png"]).unwrap();
        assert_eq!(cli_file.output_file, Some(PathBuf::from("dest.png")));
        assert_eq!(cli_file.output, None);
        assert_eq!(cli_file.output_dir, None);

        let cli_dir = Cli::try_parse_from(["rgc-cli", "--output-dir", "captures"]).unwrap();
        assert_eq!(cli_dir.output_dir, Some(PathBuf::from("captures")));
        assert_eq!(cli_dir.output, None);
        assert_eq!(cli_dir.output_file, None);

        // Conflict: --output and --output-file
        assert!(
            Cli::try_parse_from(["rgc-cli", "-o", "out.png", "--output-file", "file.png"]).is_err()
        );

        // Conflict: --output and --output-dir
        assert!(Cli::try_parse_from(["rgc-cli", "-o", "out.png", "--output-dir", "dir"]).is_err());

        // Conflict: --output-file and --output-dir
        assert!(Cli::try_parse_from([
            "rgc-cli",
            "--output-file",
            "out.png",
            "--output-dir",
            "dir"
        ])
        .is_err());
    }

    #[test]
    fn test_cli_parsing_coords() {
        let cli_default = Cli::try_parse_from(["rgc-cli"]).unwrap();
        assert_eq!(cli_default.coords, CoordMode::Logical);

        let cli_physical = Cli::try_parse_from(["rgc-cli", "--coords", "physical"]).unwrap();
        assert_eq!(cli_physical.coords, CoordMode::Physical);

        let cli_logical = Cli::try_parse_from(["rgc-cli", "--coords", "logical"]).unwrap();
        assert_eq!(cli_logical.coords, CoordMode::Logical);

        assert!(Cli::try_parse_from(["rgc-cli", "--coords", "invalid"]).is_err());
    }

    #[test]
    fn test_cli_parsing_mode_conflicts() {
        // list_monitors conflicts with capture options
        assert!(Cli::try_parse_from(["rgc-cli", "-l", "-o", "shot.png"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "-l", "-c"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "-l", "-m", "0"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "-l", "-r", "0,0,10,10"]).is_err());

        // doctor conflicts with capture options
        assert!(Cli::try_parse_from(["rgc-cli", "--doctor", "-o", "shot.png"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "--doctor", "-c"]).is_err());

        // mcp conflicts with CLI formatting/output options
        assert!(Cli::try_parse_from(["rgc-cli", "--mcp", "--json"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "--mcp", "-d", "1000"]).is_err());
    }

    #[test]
    fn test_cli_parsing_window_options() {
        let cli_win = Cli::try_parse_from(["rgc-cli", "-w", "12345"]).unwrap();
        assert_eq!(cli_win.window, Some("12345".to_string()));
        assert_eq!(cli_win.monitor, None);

        let cli_win_title =
            Cli::try_parse_from(["rgc-cli", "--window", "Code", "-o", "out.png"]).unwrap();
        assert_eq!(cli_win_title.window, Some("Code".to_string()));

        // Conflict: --window and --monitor
        assert!(Cli::try_parse_from(["rgc-cli", "-w", "123", "-m", "0"]).is_err());

        // Conflict: --list-windows and capture options
        assert!(Cli::try_parse_from(["rgc-cli", "--list-windows", "-o", "out.png"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "--list-windows", "-w", "123"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "--list-windows", "-l"]).is_err());
    }

    #[test]
    fn test_cli_parsing_scale_and_quality() {
        let cli = Cli::try_parse_from(["rgc-cli", "--scale", "0.5", "--quality", "80"]).unwrap();
        assert_eq!(cli.scale, Some(0.5));
        assert_eq!(cli.quality, Some(80));

        let cli_dim = Cli::try_parse_from(["rgc-cli", "--max-dimension", "1920"]).unwrap();
        assert_eq!(cli_dim.max_dimension, Some(1920));

        // Conflict: --scale and --max-dimension
        assert!(
            Cli::try_parse_from(["rgc-cli", "--scale", "0.5", "--max-dimension", "1920"]).is_err()
        );

        // Invalid scale > 1.0 or <= 0.0
        assert!(Cli::try_parse_from(["rgc-cli", "--scale", "1.5"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "--scale", "0.0"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "--scale", "-0.5"]).is_err());

        // Invalid quality > 100 or 0
        assert!(Cli::try_parse_from(["rgc-cli", "--quality", "101"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "--quality", "0"]).is_err());

        // Completions flag parsing and conflicts
        let cli_comp = Cli::try_parse_from(["rgc-cli", "--completions", "bash"]).unwrap();
        assert_eq!(cli_comp.completions, Some(clap_complete::Shell::Bash));
        assert!(
            Cli::try_parse_from(["rgc-cli", "--completions", "bash", "-o", "out.png"]).is_err()
        );
        assert!(Cli::try_parse_from(["rgc-cli", "--completions", "bash", "--doctor"]).is_err());

        // Open flag parsing
        let cli_open = Cli::try_parse_from(["rgc-cli", "--open"]).unwrap();
        assert!(cli_open.open);
        assert!(Cli::try_parse_from(["rgc-cli", "--open", "--doctor"]).is_err());
    }

    #[test]
    fn test_mask_effect_parsing() {
        assert_eq!(
            "transparent".parse::<MaskEffect>().unwrap(),
            MaskEffect::Transparent
        );
        assert_eq!(
            "clear".parse::<MaskEffect>().unwrap(),
            MaskEffect::Transparent
        );
        assert_eq!(
            "alpha".parse::<MaskEffect>().unwrap(),
            MaskEffect::Transparent
        );
        assert_eq!(
            "blackout".parse::<MaskEffect>().unwrap(),
            MaskEffect::Blackout
        );
        assert_eq!("black".parse::<MaskEffect>().unwrap(), MaskEffect::Blackout);
        assert_eq!("solid".parse::<MaskEffect>().unwrap(), MaskEffect::Blackout);
        assert_eq!(
            "pixelate".parse::<MaskEffect>().unwrap(),
            MaskEffect::Pixelate
        );
        assert_eq!(
            "mosaic".parse::<MaskEffect>().unwrap(),
            MaskEffect::Pixelate
        );
        assert_eq!("blur".parse::<MaskEffect>().unwrap(), MaskEffect::Blur);
        assert_eq!("gaussian".parse::<MaskEffect>().unwrap(), MaskEffect::Blur);
        assert!("unknown".parse::<MaskEffect>().is_err());
    }

    #[test]
    fn test_mask_region_parsing() {
        let mr1 = "100,200,300,400".parse::<MaskRegion>().unwrap();
        assert_eq!(mr1.region.x, 100);
        assert_eq!(mr1.region.y, 200);
        assert_eq!(mr1.region.width, 300);
        assert_eq!(mr1.region.height, 400);
        assert_eq!(mr1.effect, MaskEffect::Transparent);

        let mr2 = "50,60,70,80:blur".parse::<MaskRegion>().unwrap();
        assert_eq!(mr2.region.x, 50);
        assert_eq!(mr2.region.y, 60);
        assert_eq!(mr2.region.width, 70);
        assert_eq!(mr2.region.height, 80);
        assert_eq!(mr2.effect, MaskEffect::Blur);

        let mr3 = "0,0,10,20:blackout".parse::<MaskRegion>().unwrap();
        assert_eq!(mr3.effect, MaskEffect::Blackout);

        let mr4 = "0,0,10,20:pixelate".parse::<MaskRegion>().unwrap();
        assert_eq!(mr4.effect, MaskEffect::Pixelate);

        assert!("invalid,region".parse::<MaskRegion>().is_err());
        assert!("0,0,0,0".parse::<MaskRegion>().is_err());
        assert!("0,0,10,20:invalid_effect".parse::<MaskRegion>().is_err());
    }

    #[test]
    fn test_mask_window_parsing() {
        let mw1 = "1Password".parse::<MaskWindow>().unwrap();
        assert_eq!(mw1.query, "1Password");
        assert_eq!(mw1.effect, MaskEffect::Transparent);

        let mw2 = "1Password:blur".parse::<MaskWindow>().unwrap();
        assert_eq!(mw2.query, "1Password");
        assert_eq!(mw2.effect, MaskEffect::Blur);

        let mw3 = "Terminal: Cat:blackout".parse::<MaskWindow>().unwrap();
        assert_eq!(mw3.query, "Terminal: Cat");
        assert_eq!(mw3.effect, MaskEffect::Blackout);

        let mw4 = "Video: Animals".parse::<MaskWindow>().unwrap();
        assert_eq!(mw4.query, "Video: Animals");
        assert_eq!(mw4.effect, MaskEffect::Transparent);

        assert!("".parse::<MaskWindow>().is_err());
        assert!("   ".parse::<MaskWindow>().is_err());
    }

    #[test]
    fn test_cli_parsing_mask_options() {
        let cli = Cli::try_parse_from([
            "rgc-cli",
            "--mask",
            "100,100,200,200",
            "--mask",
            "300,300,50,50:blur",
            "--mask-window",
            "1Password:blackout",
            "--mask-window",
            "WeChat",
        ])
        .unwrap();

        assert_eq!(cli.mask.len(), 2);
        assert_eq!(cli.mask[0].region.x, 100);
        assert_eq!(cli.mask[0].effect, MaskEffect::Transparent);
        assert_eq!(cli.mask[1].effect, MaskEffect::Blur);

        assert_eq!(cli.mask_window.len(), 2);
        assert_eq!(cli.mask_window[0].query, "1Password");
        assert_eq!(cli.mask_window[0].effect, MaskEffect::Blackout);
        assert_eq!(cli.mask_window[1].query, "WeChat");
        assert_eq!(cli.mask_window[1].effect, MaskEffect::Transparent);

        // Conflict: --mask-window and --window
        assert!(Cli::try_parse_from(["rgc-cli", "-w", "Code", "--mask-window", "Other"]).is_err());

        // Conflict: --mask and --doctor
        assert!(Cli::try_parse_from(["rgc-cli", "--doctor", "--mask", "0,0,10,10"]).is_err());
    }

    #[test]
    fn test_cli_parsing_v290_options() {
        let cli = Cli::try_parse_from([
            "rgc-cli",
            "--exclude-window",
            "Terminal",
            "--exclude-window",
            "rgc-cli",
            "--a11y",
            "--som",
            "--benchmark",
            "10",
        ])
        .unwrap();

        assert_eq!(cli.exclude_window, vec!["Terminal", "rgc-cli"]);
        assert!(cli.accessibility);
        assert!(cli.som);
        assert_eq!(cli.benchmark, Some(10));

        let cli_default_bench = Cli::try_parse_from(["rgc-cli", "--benchmark"]).unwrap();
        assert_eq!(cli_default_bench.benchmark, Some(5));

        // Conflict: --benchmark and --record
        assert!(Cli::try_parse_from(["rgc-cli", "--benchmark", "--record", "2"]).is_err());
        // Conflict: --exclude-window and --window
        assert!(Cli::try_parse_from(["rgc-cli", "-w", "Code", "--exclude-window", "Terminal"]).is_err());
    }

    #[test]
    fn test_cli_parsing_all_monitors_options() {
        let cli = Cli::try_parse_from(["rgc-cli", "--all-monitors"]).unwrap();
        assert!(cli.all_monitors);
        assert_eq!(cli.monitor, None);
        assert_eq!(cli.window, None);

        let cli_short = Cli::try_parse_from(["rgc-cli", "-a", "-o", "pano.png"]).unwrap();
        assert!(cli_short.all_monitors);

        // Conflict: --all-monitors and --monitor
        assert!(Cli::try_parse_from(["rgc-cli", "-a", "-m", "0"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "--all-monitors", "--monitor", "1"]).is_err());

        // Conflict: --all-monitors and --window
        assert!(Cli::try_parse_from(["rgc-cli", "-a", "-w", "Code"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "--all-monitors", "--window", "123"]).is_err());

        // Conflict: --all-monitors and --doctor / --list-monitors
        assert!(Cli::try_parse_from(["rgc-cli", "-a", "--doctor"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "-a", "-l"]).is_err());
    }

    #[test]
    fn test_cli_parsing_grid_and_diff_options() {
        // Grid default value (100)
        let cli_grid_default = Cli::try_parse_from(["rgc-cli", "--grid"]).unwrap();
        assert_eq!(cli_grid_default.grid, Some(100));

        // Grid explicit value
        let cli_grid_explicit = Cli::try_parse_from(["rgc-cli", "--grid", "50"]).unwrap();
        assert_eq!(cli_grid_explicit.grid, Some(50));

        // Diff options
        let cli_diff = Cli::try_parse_from([
            "rgc-cli",
            "--diff-with",
            "prev.png",
            "--diff-threshold",
            "30",
            "--crop-changed",
        ])
        .unwrap();
        assert_eq!(cli_diff.diff_with, Some(PathBuf::from("prev.png")));
        assert_eq!(cli_diff.diff_threshold, 30);
        assert!(cli_diff.crop_changed);

        // crop-changed requires diff-with
        assert!(Cli::try_parse_from(["rgc-cli", "--crop-changed"]).is_err());

        // Grid conflict with doctor
        assert!(Cli::try_parse_from(["rgc-cli", "--doctor", "--grid"]).is_err());
    }

    #[test]
    fn test_cli_parsing_phase9_options() {
        // Highlight diff requires diff-with
        assert!(Cli::try_parse_from(["rgc-cli", "--highlight-diff"]).is_err());

        let cli_highlight = Cli::try_parse_from([
            "rgc-cli",
            "--diff-with",
            "base.png",
            "--highlight-diff",
            "--timestamp",
            "--tag",
            "LOGIN-FLOW",
        ])
        .unwrap();
        assert!(cli_highlight.highlight_diff);
        assert!(cli_highlight.timestamp);
        assert_eq!(cli_highlight.tag.as_deref(), Some("LOGIN-FLOW"));

        // Wait change options
        let cli_wait = Cli::try_parse_from([
            "rgc-cli",
            "--wait-change",
            "--wait-timeout",
            "3000",
            "--wait-interval",
            "150",
        ])
        .unwrap();
        assert!(cli_wait.wait_change);
        assert_eq!(cli_wait.wait_timeout, 3000);
        assert_eq!(cli_wait.wait_interval, 150);

        // Wait change conflict with doctor
        assert!(Cli::try_parse_from(["rgc-cli", "--doctor", "--wait-change"]).is_err());

        // Phase 10: Cursor and Recording options
        let cli_phase10 = Cli::try_parse_from([
            "rgc-cli",
            "--cursor",
            "--highlight-cursor",
            "--record",
            "3.5",
            "--fps",
            "10",
        ])
        .unwrap();
        assert!(cli_phase10.cursor);
        assert!(cli_phase10.highlight_cursor);
        assert_eq!(cli_phase10.record, Some(3.5));
        assert_eq!(cli_phase10.fps, 10);

        // Record conflict with doctor
        assert!(Cli::try_parse_from(["rgc-cli", "--doctor", "--record", "2.0"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "--doctor", "--cursor"]).is_err());

        // Phase 11: Set-of-Mark (SoM) options
        let cli_som = Cli::try_parse_from([
            "rgc-cli",
            "--som",
            "--som-threshold",
            "30",
            "--som-max-elements",
            "20",
        ])
        .unwrap();
        assert!(cli_som.som);
        assert_eq!(cli_som.som_threshold, 30);
        assert_eq!(cli_som.som_max_elements, 20);

        // SoM conflict with doctor
        assert!(Cli::try_parse_from(["rgc-cli", "--doctor", "--som"]).is_err());

        // Phase 11.2: Action overlays & Verification options
        let cli_action = Cli::try_parse_from([
            "rgc-cli",
            "--click-at",
            "500,300",
            "--click-at",
            "600,400,btn",
            "--drag",
            "100,200,300,400,drag_label",
            "--verify-click",
            "500,300",
            "--verify-radius",
            "150",
            "--verify-timeout",
            "800",
            "--verify-threshold",
            "15",
        ])
        .unwrap();
        assert_eq!(cli_action.click_at.len(), 2);
        assert_eq!(cli_action.drag.len(), 1);
        assert_eq!(cli_action.verify_click, Some("500,300".to_string()));
        assert_eq!(cli_action.verify_radius, 150);
        assert_eq!(cli_action.verify_timeout, 800);
        assert_eq!(cli_action.verify_threshold, 15);

        // Action conflict with doctor
        assert!(Cli::try_parse_from(["rgc-cli", "--doctor", "--click-at", "100,100"]).is_err());
        assert!(Cli::try_parse_from(["rgc-cli", "--doctor", "--verify-click", "100,100"]).is_err());
    }

    #[test]
    fn test_parse_record_duration_validation() {
        assert_eq!(parse_record_duration("5").unwrap(), 5.0);
        assert_eq!(parse_record_duration("2.5").unwrap(), 2.5);

        // Disallow negative, zero, NaN, inf, and over 3600
        assert!(parse_record_duration("0").is_err());
        assert!(parse_record_duration("-1").is_err());
        assert!(parse_record_duration("nan").is_err());
        assert!(parse_record_duration("inf").is_err());
        assert!(parse_record_duration("-inf").is_err());
        assert!(parse_record_duration("3601").is_err());

        // CLI conflict with diff-with
        assert!(Cli::try_parse_from(["rgc-cli", "--record", "5", "--diff-with", "ref.png"]).is_err());
    }

    #[test]
    fn test_cli_parsing_watch_and_config() {
        // Watch defaults
        let cli_watch_default = Cli::try_parse_from(["rgc-cli", "--watch"]).unwrap();
        assert_eq!(cli_watch_default.watch, Some(500));
        assert_eq!(cli_watch_default.watch_threshold, 16);
        assert_eq!(cli_watch_default.watch_min_percent, 0.05);

        // Custom watch parameters and config path
        let cli_watch = Cli::try_parse_from([
            "rgc-cli",
            "--config",
            "custom_rgc.toml",
            "--watch",
            "200",
            "--watch-threshold",
            "25",
            "--watch-min-percent",
            "0.1",
            "--watch-count",
            "5",
            "--watch-timeout",
            "10.0",
        ])
        .unwrap();

        assert_eq!(cli_watch.config, Some(std::path::PathBuf::from("custom_rgc.toml")));
        assert_eq!(cli_watch.watch, Some(200));
        assert_eq!(cli_watch.watch_threshold, 25);
        assert_eq!(cli_watch.watch_min_percent, 0.1);
        assert_eq!(cli_watch.watch_count, Some(5));
        assert_eq!(cli_watch.watch_timeout, Some(10.0));

        // Conflict: --watch with --record
        assert!(Cli::try_parse_from(["rgc-cli", "--watch", "--record", "5.0"]).is_err());
        // Conflict: --watch with --doctor
        assert!(Cli::try_parse_from(["rgc-cli", "--watch", "--doctor"]).is_err());
    }
}
