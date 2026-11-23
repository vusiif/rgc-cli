#![forbid(unsafe_code)]
#![recursion_limit = "256"]

pub mod accessibility;
pub mod action;
pub mod api;
pub mod benchmark;
pub mod capture;
pub mod cli;
pub mod config;
pub mod coords;
pub mod doctor;
pub mod mcp;
pub mod output;
pub mod pipeline;
pub mod som;
pub mod template;
pub mod transform;

use clap::Parser;
use cli::Cli;
use image::DynamicImage;
use log::debug;
use output::{
    ApiError, ApiResponse, CaptureResultJson, ErrorCode, ImageFormat, OutputDestination,
    RecordResultJson,
};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread::sleep;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let args = Cli::parse();

    let log_level = if args.verbose {
        "debug"
    } else if args.quiet {
        "error"
    } else {
        "warn"
    };

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level))
        .format_timestamp(None)
        .format_target(false)
        .init();

    debug!("Parsed CLI arguments: {:?}", args);

    let is_json = args.json;
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let exit_code = err.exit_code;
            if is_json {
                let response = ApiResponse::<()>::failure(err);
                if let Ok(json_str) = serde_json::to_string_pretty(&response) {
                    println!("{}", json_str);
                } else {
                    println!(
                        r#"{{"ok":false,"error":{{"errorCode":"UNKNOWN","message":"Failed to serialize error payload","exitCode":127}}}}"#
                    );
                }
            } else {
                eprintln!("Error: {}", err.message);
                if let Some(sug) = err.suggestion {
                    eprintln!("Suggestion: {}", sug);
                }
            }
            ExitCode::from(exit_code as u8)
        }
    }
}

struct RawCaptured {
    image: image::DynamicImage,
    scale_factor: f32,
    crop_offset: (u32, u32),
    monitor_origin: Option<(i32, i32)>,
}

fn capture_target(args: &Cli) -> Result<RawCaptured, ApiError> {
    if args.all_monitors {
        debug!(
            "Initiating all-monitors panorama capture (region={:?}, coords={:?})",
            args.region, args.coords
        );
        let captured =
            capture::capture_all_monitors_with_details(args.region.as_ref(), args.coords)?;
        Ok(RawCaptured {
            image: captured.image,
            scale_factor: captured.scale_factor,
            crop_offset: captured.crop_offset,
            monitor_origin: Some(captured.monitor_origin),
        })
    } else if let Some(ref win_query) = args.window {
        debug!(
            "Initiating window capture (query={:?}, region={:?}, client_only={})",
            win_query, args.region, args.client_only
        );
        let captured = capture::capture_window_with_options(
            win_query,
            args.region.as_ref(),
            args.client_only,
        )?;
        Ok(RawCaptured {
            image: captured.image,
            scale_factor: captured.scale_factor,
            crop_offset: captured.crop_offset,
            monitor_origin: Some(captured.window_origin),
        })
    } else {
        debug!(
            "Initiating monitor capture (monitor={:?}, region={:?}, coords={:?})",
            args.monitor, args.region, args.coords
        );
        let captured =
            capture::capture_screen_with_details(args.monitor, args.region.as_ref(), args.coords)?;
        Ok(RawCaptured {
            image: captured.image,
            scale_factor: captured.scale_factor,
            crop_offset: captured.crop_offset,
            monitor_origin: Some(captured.monitor_origin),
        })
    }
}

fn run(mut args: Cli) -> Result<(), ApiError> {
    // Load configuration file if present
    if let Some(cfg) = config::RgcConfig::load(args.config.as_deref()) {
        apply_config_to_cli(&mut args, &cfg);
    }

    // 0. Handle shell completions generation
    if let Some(shell) = args.completions {
        use clap::CommandFactory;
        let mut cmd = Cli::command();
        clap_complete::generate(shell, &mut cmd, "rgc-cli", &mut std::io::stdout());
        return Ok(());
    }

    // 0. Handle MCP stdio server mode
    if args.mcp {
        mcp::run_mcp_server()
            .map_err(|e| ApiError::new(ErrorCode::IoError, format!("MCP server error: {e}")))?;
        return Ok(());
    }

    // 0.5. Handle benchmark diagnostic mode
    if let Some(runs) = args.benchmark {
        benchmark::run_benchmark(runs, args.json)
            .map_err(|e| ApiError::new(ErrorCode::CaptureFailed, e))?;
        return Ok(());
    }

    // 1. Handle system diagnostics if requested
    if args.doctor {
        let report = doctor::run_doctor();
        if args.json {
            return print_json_success(report);
        }

        println!("[rgc-cli Diagnostics Report]");
        println!("{:-<65}", "");
        println!("Operating System:    {} ({})", report.os, report.arch);
        println!("Capture Backend:     {}", report.backend);
        println!("Monitors Detected:   {}", report.monitors_detected);
        if let Some(prim) = report.primary_monitor {
            println!("Primary Display:     {}", prim);
        }
        println!("Windows Detected:    {}", report.windows_detected);
        println!(
            "Clipboard Access:    {}",
            if report.clipboard_available {
                "Available"
            } else {
                "Unavailable"
            }
        );
        println!(
            "Supported Formats:   {}",
            report.supported_formats.join(", ")
        );
        println!("Subsystem Status:    {}", report.status);
        println!("{:-<65}", "");
        return Ok(());
    }

    // 2. Handle monitor listing if requested
    if args.list_monitors {
        let monitors = capture::get_monitor_infos()?;

        if args.json {
            return print_json_success(monitors);
        }

        if monitors.is_empty() {
            println!("No monitors detected.");
            return Ok(());
        }

        println!("Available Monitors (total: {}):", monitors.len());
        println!("{:-<60}", "");
        for m in monitors {
            let primary_tag = if m.is_primary { " [Primary]" } else { "" };
            println!("[{}] {}{}", m.index, m.name, primary_tag);
            println!("    Resolution:   {} x {}", m.width, m.height);
            println!("    Scale Factor: {:.2}x", m.scale_factor);
            println!("{:-<60}", "");
        }
        return Ok(());
    }

    // 2.1 Handle window listing if requested
    if args.list_windows {
        let windows = capture::get_window_infos()?;

        if args.json {
            return print_json_success(windows);
        }

        if windows.is_empty() {
            println!("No active application windows detected.");
            return Ok(());
        }

        println!("Available Windows (total: {}):", windows.len());
        println!("{:-<75}", "");
        for w in windows {
            let status = if w.is_minimized {
                "Minimized"
            } else {
                "Visible"
            };
            let app = if w.app_name.is_empty() {
                "Unknown"
            } else {
                &w.app_name
            };
            println!("[ID: {}] {} ({})", w.id, w.title, app);
            println!(
                "    Position:     {}, {} ({} x {})",
                w.x, w.y, w.width, w.height
            );
            println!("    State:        {}", status);
            println!("{:-<75}", "");
        }
        return Ok(());
    }

    // 3. Determine output destination and validate arguments early
    let destination = output::determine_output_destination(
        args.output.as_deref(),
        args.output_file.as_deref(),
        args.output_dir.as_deref(),
    )?;

    let is_stdout = destination == OutputDestination::Stdout;

    if args.open && is_stdout {
        return Err(ApiError::new(
            ErrorCode::BadArgs,
            "Cannot use '--open' when streaming screenshot to stdout ('-o -').",
        )
        .with_stage("argument_validation")
        .with_suggestion("Omit '-o -' or specify a file path instead of stdout to use --open."));
    }

    if args.open
        && args.clipboard
        && destination == OutputDestination::Auto
        && args.output.is_none()
        && args.output_file.is_none()
        && args.output_dir.is_none()
    {
        return Err(ApiError::new(
            ErrorCode::BadArgs,
            "Cannot use '--open' in clipboard-only mode: no file was saved to disk.",
        )
        .with_stage("argument_validation")
        .with_suggestion(
            "Specify an output file destination with -o or --output-file when using --open.",
        ));
    }

    // R-01: Prohibit combining --json with -o - (stdout) to prevent stream corruption
    if is_stdout && args.json {
        return Err(ApiError::new(
            ErrorCode::BadArgs,
            "Cannot combine '--json' with standard output streaming ('-o -'). \
             Binary image stream and structured JSON text cannot both occupy stdout.",
        )
        .with_stage("argument_validation")
        .with_suggestion(
            "To retrieve structured JSON metadata, save to a file or directory instead of stdout.",
        ));
    }

    if is_stdout && args.record.is_some() {
        return Err(ApiError::new(
            ErrorCode::BadArgs,
            "Cannot stream continuous recording to stdout ('-o -'). \
             Specify an output file destination (.gif) or directory with -o, --output-file, or --output-dir.",
        )
        .with_stage("argument_validation")
        .with_suggestion("Provide a target output path (e.g. -o recording.gif or --output-dir ./frames)."));
    }

    if is_stdout && args.watch.is_some() && !args.json {
        return Err(ApiError::new(
            ErrorCode::BadArgs,
            "Cannot stream raw binary images to stdout in '--watch' mode. \
             Use '--json' to stream newline-delimited event JSON, or specify a directory via '--output-dir'.",
        )
        .with_stage("argument_validation")
        .with_suggestion("Add '--json' or '--output-dir <DIR>' when using '--watch'."));
    }

    // Infer image format and validate against file extension if explicit format provided
    let format = output::validate_and_infer_format(&destination, args.format)?;

    // 2.5 Handle continuous recording if requested
    if let Some(record_secs) = args.record {
        return run_recording(&args, record_secs, destination);
    }

    // 2.6 Handle watch mode if requested
    if let Some(interval_ms) = args.watch {
        return run_watch_mode(&args, interval_ms, destination);
    }

    // 3. Handle delayed capture if requested
    if let Some(delay_ms) = args.delay {
        if delay_ms > 0 {
            if !args.quiet && !args.json {
                if is_stdout {
                    eprintln!("Waiting {} ms before capturing...", delay_ms);
                } else {
                    println!("Waiting {} ms before capturing...", delay_ms);
                }
            }
            sleep(Duration::from_millis(delay_ms));
        }
    }

    // 4. Perform capture (with optional verify-click or wait-change detection loop)
    let (raw_captured, action_verification) = if let Some(ref verify_str) = args.verify_click {
        let click_marker = action::parse_click_marker(verify_str).map_err(|e| {
            ApiError::new(ErrorCode::BadArgs, e)
                .with_stage("verify_click_parsing")
                .with_suggestion("Ensure --verify-click is passed in 'X,Y' or 'X,Y,LABEL' format.")
        })?;

        if !args.quiet && !args.json {
            if is_stdout {
                eprintln!(
                    "Verifying action at ({}, {}) within radius {}px (timeout: {} ms)...",
                    click_marker.x, click_marker.y, args.verify_radius, args.verify_timeout
                );
            } else {
                println!(
                    "Verifying action at ({}, {}) within radius {}px (timeout: {} ms)...",
                    click_marker.x, click_marker.y, args.verify_radius, args.verify_timeout
                );
            }
        }

        let baseline = capture_target(&args)?;
        sleep(Duration::from_millis(args.verify_timeout));
        let follow_up = capture_target(&args)?;

        let verification = action::verify_action_change(
            &baseline.image,
            &follow_up.image,
            click_marker.x,
            click_marker.y,
            args.verify_radius,
            args.verify_threshold,
        );

        (follow_up, Some((click_marker, verification)))
    } else if args.wait_change {
        let mut baseline = capture_target(&args)?;
        let start_time = std::time::Instant::now();
        let timeout = Duration::from_millis(args.wait_timeout);
        let interval = Duration::from_millis(args.wait_interval.max(10));
        let mut detected_change = false;

        if !args.quiet && !args.json {
            if is_stdout {
                eprintln!(
                    "Waiting for screen visual change (timeout: {} ms)...",
                    args.wait_timeout
                );
            } else {
                println!(
                    "Waiting for screen visual change (timeout: {} ms)...",
                    args.wait_timeout
                );
            }
        }

        while start_time.elapsed() < timeout {
            sleep(interval);
            let next_captured = capture_target(&args)?;
            let diff = output::calculate_image_diff(
                &next_captured.image,
                &baseline.image,
                args.diff_threshold,
            );
            if diff.changed && (diff.changed_pixels >= 50 || diff.changed_percent >= 0.05) {
                debug!(
                    "Visual change detected after {:?}: {} pixels changed ({:.2}%)",
                    start_time.elapsed(),
                    diff.changed_pixels,
                    diff.changed_percent
                );
                baseline = next_captured;
                detected_change = true;
                break;
            }
            baseline = next_captured;
        }

        if !detected_change {
            debug!(
                "No visual change detected before timeout ({:?}), proceeding with latest frame",
                timeout
            );
        }

        (baseline, None)
    } else {
        (capture_target(&args)?, None)
    };

    // Parse click markers and drag trajectories
    let mut clicks = Vec::new();
    for c_str in &args.click_at {
        let c = action::parse_click_marker(c_str).map_err(|e| {
            ApiError::new(ErrorCode::BadArgs, e)
                .with_stage("click_at_parsing")
                .with_suggestion("Ensure --click-at is formatted as 'X,Y' or 'X,Y,LABEL'.")
        })?;
        clicks.push(c);
    }

    let mut drags = Vec::new();
    for d_str in &args.drag {
        let d = action::parse_drag_marker(d_str).map_err(|e| {
            ApiError::new(ErrorCode::BadArgs, e)
                .with_stage("drag_parsing")
                .with_suggestion(
                    "Ensure --drag is formatted as 'X1,Y1,X2,Y2' or 'X1,Y1,X2,Y2,LABEL'.",
                )
        })?;
        drags.push(d);
    }

    let action_verification_result =
        if let Some((verify_click_marker, verification)) = action_verification {
            if !clicks
                .iter()
                .any(|c| c.x == verify_click_marker.x && c.y == verify_click_marker.y)
            {
                clicks.push(action::ClickMarker {
                    x: verify_click_marker.x,
                    y: verify_click_marker.y,
                    label: Some(
                        if verification.verified {
                            "ACTION OK"
                        } else {
                            "NO CHANGE"
                        }
                        .to_string(),
                    ),
                });
            }
            Some(verification)
        } else {
            None
        };

    let diff_ref = if let Some(ref diff_path) = args.diff_with {
        let ref_image = image::open(diff_path).map_err(|e| {
            ApiError::new(
                ErrorCode::BadArgs,
                format!(
                    "Failed to open diff reference image '{}': {e}",
                    diff_path.display()
                ),
            )
            .with_stage("diff_comparison")
            .with_suggestion("Check that the reference path points to a valid readable image.")
        })?;
        Some(ref_image)
    } else {
        None
    };

    let template_match = if let Some(ref tpl_path) = args.find_template {
        let tpl_img = image::open(tpl_path).map_err(|e| {
            ApiError::new(
                ErrorCode::BadArgs,
                format!(
                    "Failed to open template image '{}': {e}",
                    tpl_path.display()
                ),
            )
            .with_stage("template_matching")
            .with_suggestion("Ensure the path points to a valid image file (PNG, JPG, WebP).")
        })?;
        Some(template::TemplateMatchOptions {
            template: tpl_img,
            threshold: args.template_threshold,
            max_matches: args.template_max_matches,
            highlight: args.highlight_template,
        })
    } else {
        None
    };

    let pipeline_options = pipeline::PipelineOptions {
        cursor: args.cursor,
        highlight_cursor: args.highlight_cursor,
        manual_masks: args.mask.clone(),
        mask_windows: args.mask_window.clone(),
        exclude_windows: args.exclude_window.clone(),
        allow_missing_mask: args.allow_missing_mask,
        coords_mode: args.coords,
        scale: args.scale,
        max_dimension: args.max_dimension,
        diff_ref,
        diff_threshold: args.diff_threshold,
        crop_changed: args.crop_changed,
        highlight_diff: args.highlight_diff,
        grid: args.grid,
        timestamp: args.timestamp,
        tag: args.tag.clone(),
        som: args.som,
        som_threshold: args.som_threshold,
        som_max_elements: args.som_max_elements,
        accessibility: args.accessibility,
        a11y_nodes_override: None,
        template_match,
        clicks,
        drags,
        global_cursor_override: None,
    };

    let pipeline_result = pipeline::execute_image_pipeline(
        raw_captured.image,
        raw_captured.scale_factor,
        raw_captured.crop_offset,
        raw_captured.monitor_origin,
        &pipeline_options,
    )?;

    let dynamic_image = pipeline_result.image;
    let cursor_info = pipeline_result.cursor;
    let diff_result = pipeline_result.diff;
    let detected_elements = pipeline_result.elements;
    let template_matches = pipeline_result.matches;
    let clicks = pipeline_result.clicks.unwrap_or_default();
    let drags = pipeline_result.drags.unwrap_or_default();
    let coordinates_info = Some(pipeline_result.coordinates);

    let img_width = dynamic_image.width();
    let img_height = dynamic_image.height();

    // 5. Handle clipboard export if requested
    if args.clipboard {
        output::copy_to_clipboard(&dynamic_image)?;
        if !args.quiet && !args.json {
            if is_stdout {
                eprintln!("Screenshot successfully copied to clipboard.");
            } else {
                println!("Screenshot successfully copied to clipboard.");
            }
        }
    }

    // 6. Output handling:
    let mut saved_path_buf = None;
    let mut file_size_bytes = None;

    if is_stdout {
        output::write_to_stdout(&dynamic_image, format, args.quality)?;
    } else if destination != OutputDestination::Auto || !args.clipboard {
        let saved_path = output::save_image(&dynamic_image, &destination, format, args.quality)?;

        if let Ok(meta) = fs::metadata(&saved_path) {
            file_size_bytes = Some(meta.len());
        }

        if !args.quiet && !args.json {
            println!("Screenshot successfully saved to: {}", saved_path.display());
            if let Some(ref diff) = diff_result {
                println!(
                    "Diff Analysis: changed={}, changedPixels={} ({:.2}%), bbox={:?}",
                    diff.changed, diff.changed_pixels, diff.changed_percent, diff.bbox
                );
            }
            if let Some(ref elems) = detected_elements {
                println!(
                    "Set-of-Mark (SoM): detected {} interactive UI elements",
                    elems.len()
                );
            }
            if let Some(ref matches) = template_matches {
                if !matches.is_empty() {
                    println!("Template Matches: found {} occurrences", matches.len());
                    for (idx, m) in matches.iter().enumerate() {
                        let scx = m.screen_center_x.map(|v| v.to_string()).unwrap_or_else(|| "N/A".to_string());
                        let scy = m.screen_center_y.map(|v| v.to_string()).unwrap_or_else(|| "N/A".to_string());
                        println!(
                            "  [{}] ({}, {}) {}x{} (conf: {:.1}%) -> Screen Click: ({}, {})",
                            idx + 1, m.x, m.y, m.width, m.height, m.confidence * 100.0,
                            scx, scy
                        );
                    }
                } else {
                    println!("Template Matches: no match found above threshold {:.2}", args.template_threshold);
                }
            }
            if let Some(ref ver) = action_verification_result {
                println!(
                    "Action Verification: verified={} ({} pixels changed in ROI, {:.2}%), bbox={:?}",
                    ver.verified, ver.changed_pixels, ver.change_percent, ver.bbox
                );
            }
        }

        if args.open {
            output::open_file_in_viewer(&saved_path)?;
        }

        saved_path_buf = Some(saved_path);
    }

    // 7. Handle JSON output mode
    if args.json {
        let result_payload = CaptureResultJson {
            status: "success".to_string(),
            path: saved_path_buf.map(|p| p.display().to_string()),
            width: img_width,
            height: img_height,
            format,
            clipboard: args.clipboard,
            size_bytes: file_size_bytes,
            diff: diff_result,
            cursor: cursor_info,
            elements: detected_elements,
            action_verification: action_verification_result,
            clicks: if clicks.is_empty() {
                None
            } else {
                Some(clicks)
            },
            drags: if drags.is_empty() { None } else { Some(drags) },
            coordinates: coordinates_info,
            matches: template_matches,
        };
        print_json_success(result_payload)?;
    }

    Ok(())
}

fn run_recording(
    args: &Cli,
    record_secs: f32,
    destination: OutputDestination,
) -> Result<(), ApiError> {
    // Verify destination format: only GIF or WebP file or directory destination is supported
    if let OutputDestination::File(ref path) = destination {
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("gif") && !ext.eq_ignore_ascii_case("webp") {
            return Err(ApiError::new(
                ErrorCode::BadArgs,
                format!(
                    "Unsupported output format for screen recording: '{}'. Recording to a single file supports animated WebP (.webp) or animated GIF (.gif), or specify an output directory (--output-dir) to save an image sequence.",
                    path.display()
                ),
            )
            .with_stage("argument_validation")
            .with_suggestion("Change file extension to .webp or .gif (e.g. -o recording.webp) or use --output-dir ./frames."));
        }
    }

    if let Some(delay_ms) = args.delay {
        if delay_ms > 0 {
            if !args.quiet && !args.json {
                println!("Waiting {} ms before recording...", delay_ms);
            }
            sleep(Duration::from_millis(delay_ms));
        }
    }

    let fps = args.fps.clamp(1, 30);
    let interval = Duration::from_secs_f32(1.0 / fps as f32);
    let total_duration = Duration::from_secs_f32(record_secs.max(0.1));
    let start_time = Instant::now();

    if !args.quiet && !args.json {
        println!(
            "Recording screen for {:.1}s at {} FPS (sampling every {} ms)...",
            record_secs,
            fps,
            interval.as_millis()
        );
    }

    let (_is_dir_output, dir_target, anim_target, is_webp) = match destination {
        OutputDestination::Directory(ref dir) => {
            if !dir.exists() {
                fs::create_dir_all(dir).map_err(|e| {
                    ApiError::new(
                        ErrorCode::SaveFailed,
                        format!("Failed to create output directory: {e}"),
                    )
                })?;
            }
            (true, Some(dir.clone()), None, false)
        }
        OutputDestination::File(p) => {
            let is_webp = p
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("webp"))
                .unwrap_or(args.format == Some(ImageFormat::Webp));
            (false, None, Some(p), is_webp)
        }
        _ => {
            let format = if args.format == Some(ImageFormat::Webp) {
                ImageFormat::Webp
            } else {
                ImageFormat::Gif
            };
            let filename = output::generate_default_filename(format);
            (
                false,
                None,
                Some(PathBuf::from(filename)),
                format == ImageFormat::Webp,
            )
        }
    };

    let mut anim_stream = if let Some(ref path) = anim_target {
        Some(
            output::RecordingStreamEncoder::create_file(path, fps, is_webp).map_err(|e| {
                ApiError::new(
                    ErrorCode::SaveFailed,
                    format!("Failed to initialize streaming recording encoder: {e}"),
                )
            })?,
        )
    } else {
        None
    };

    let frame_options = pipeline::RecordingFrameOptions {
        cursor: args.cursor,
        highlight_cursor: args.highlight_cursor,
        manual_masks: args.mask.clone(),
        mask_windows: args.mask_window.clone(),
        exclude_windows: args.exclude_window.clone(),
        allow_missing_mask: args.allow_missing_mask,
        coords_mode: args.coords,
        scale: args.scale,
        max_dimension: args.max_dimension,
        grid: args.grid,
        timestamp: args.timestamp,
        tag: args.tag.clone(),
        som: args.som,
        som_threshold: args.som_threshold,
        som_max_elements: args.som_max_elements,
    };

    let mut frame_count = 0u32;
    let mut stream_dims = (0u32, 0u32);

    while start_time.elapsed() < total_duration {
        let frame_start = Instant::now();
        let raw = capture_target(args)?;

        let mut rgba = raw.image.into_rgba8();
        let frame_img = pipeline::process_recording_frame(
            &mut rgba,
            raw.scale_factor,
            raw.crop_offset,
            raw.monitor_origin,
            &frame_options,
        )?;
        let rgba = frame_img.into_rgba8();

        stream_dims = (rgba.width(), rgba.height());
        frame_count += 1;

        if let Some(ref mut stream) = anim_stream {
            stream.encode_frame(&rgba).map_err(|e| {
                ApiError::new(
                    ErrorCode::SaveFailed,
                    format!("Failed to encode streaming recording frame #{frame_count}: {e}"),
                )
            })?;
        } else if let Some(ref dir) = dir_target {
            let filename = format!("frame_{:04}.png", frame_count);
            let frame_path = dir.join(filename);
            output::save_image(
                &DynamicImage::ImageRgba8(rgba),
                &OutputDestination::File(frame_path),
                ImageFormat::Png,
                None,
            )?;
        }

        let elapsed = frame_start.elapsed();
        if elapsed < interval {
            sleep(interval - elapsed);
        }
    }

    if frame_count == 0 {
        return Err(ApiError::new(
            ErrorCode::CaptureFailed,
            "No frames captured during recording.",
        ));
    }

    let (saved_path_buf, file_size_bytes) = if let Some(path) = anim_target {
        if let Some(ref mut stream) = anim_stream {
            stream.finish().map_err(|e| {
                ApiError::new(
                    ErrorCode::SaveFailed,
                    format!("Failed to finalize recording stream: {e}"),
                )
            })?;
        }
        drop(anim_stream);
        let size = fs::metadata(&path).ok().map(|m| m.len());
        if !args.quiet && !args.json {
            let label = if is_webp {
                "animated WebP"
            } else {
                "animated GIF"
            };
            println!("Recording saved as {label}: {}", path.display());
        }
        (path, size)
    } else {
        let dir = dir_target.unwrap();
        if !args.quiet && !args.json {
            println!(
                "Recorded {} frames saved to directory: {}",
                frame_count,
                dir.display()
            );
        }
        (dir, None)
    };

    if args.json {
        let rec_json = RecordResultJson {
            status: "success".to_string(),
            path: saved_path_buf.display().to_string(),
            frame_count,
            duration_secs: start_time.elapsed().as_secs_f32(),
            fps,
            width: stream_dims.0,
            height: stream_dims.1,
            size_bytes: file_size_bytes,
        };
        print_json_success(rec_json)?;
    }

    Ok(())
}

fn print_json_success<T: serde::Serialize>(data: T) -> Result<(), ApiError> {
    let response = ApiResponse::success(data);
    let json_str = serde_json::to_string_pretty(&response).map_err(|e| {
        ApiError::new(
            ErrorCode::EncodeFailed,
            format!("JSON serialization failed: {e}"),
        )
    })?;
    println!("{}", json_str);
    Ok(())
}

fn apply_config_to_cli(args: &mut Cli, cfg: &config::RgcConfig) {
    if args.mask_window.is_empty() {
        for w in &cfg.privacy.mask_windows {
            if let Ok(mw) = w.parse::<cli::MaskWindow>() {
                args.mask_window.push(mw);
            }
        }
    }

    if args.exclude_window.is_empty() {
        for w in &cfg.privacy.exclude_windows {
            args.exclude_window.push(w.clone());
        }
    }

    if !args.accessibility {
        if let Some(a) = cfg.defaults.accessibility {
            args.accessibility = a;
        }
    }

    if !args.som {
        if let Some(s) = cfg.defaults.som {
            args.som = s;
        }
    }

    if args.mask.is_empty() {
        for r in &cfg.privacy.mask_regions {
            if let Ok(mr) = r.parse::<cli::MaskRegion>() {
                args.mask.push(mr);
            }
        }
    }

    if !args.allow_missing_mask {
        if let Some(amm) = cfg.privacy.allow_missing_mask {
            args.allow_missing_mask = amm;
        }
    }

    if args.scale.is_none() {
        args.scale = cfg.defaults.scale;
    }

    if args.max_dimension.is_none() {
        args.max_dimension = cfg.security.max_dimension;
    }

    if !args.cursor {
        if let Some(c) = cfg.defaults.cursor {
            args.cursor = c;
        }
    }

    if !cfg.security.allowed_output_dirs.is_empty()
        && std::env::var("RGC_ALLOWED_OUTPUT_DIRS").is_err()
    {
        std::env::set_var(
            "RGC_ALLOWED_OUTPUT_DIRS",
            cfg.security.allowed_output_dirs.join(","),
        );
    }
}

/// Structured JSON representation of a visual change event in watch mode.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchEventJson {
    pub event: &'static str,
    pub index: u32,
    pub changed_pixels: u64,
    pub changed_percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<transform::DiffBBox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub timestamp: String,
}

fn run_watch_mode(
    args: &Cli,
    interval_ms: u64,
    destination: OutputDestination,
) -> Result<(), ApiError> {
    let interval = Duration::from_millis(interval_ms.max(20));
    let start_time = Instant::now();
    let timeout = args.watch_timeout.map(|s| Duration::from_secs_f32(s.max(0.1)));

    if !args.quiet && !args.json {
        println!(
            "Starting watch mode (interval: {} ms, threshold: {}, min_change: {:.2}%)...",
            interval_ms, args.watch_threshold, args.watch_min_percent
        );
    }

    let mut baseline = capture_target(args)?;
    let mut event_count = 0u32;
    let format = output::validate_and_infer_format(&destination, args.format)?;

    loop {
        if let Some(t) = timeout {
            if start_time.elapsed() >= t {
                if !args.quiet && !args.json {
                    println!("Watch mode timed out after {:?}", t);
                }
                break;
            }
        }

        sleep(interval);

        let current = capture_target(args)?;
        let diff = output::calculate_image_diff(&current.image, &baseline.image, args.watch_threshold);

        if diff.changed && diff.changed_percent >= args.watch_min_percent {
            event_count += 1;
            let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();

            let saved_path_str = match &destination {
                OutputDestination::Directory(dir) => {
                    let filename = format!(
                        "change_{:04}_{}.{}",
                        event_count,
                        chrono::Local::now().format("%H%M%S"),
                        format.extension()
                    );
                    let out_path = dir.join(filename);
                    let dest = OutputDestination::File(out_path.clone());
                    output::save_image(&current.image, &dest, format, args.quality)?;
                    Some(out_path.display().to_string())
                }
                OutputDestination::File(f) => {
                    output::save_image(&current.image, &destination, format, args.quality)?;
                    Some(f.display().to_string())
                }
                _ => None,
            };

            if args.json {
                let event_json = WatchEventJson {
                    event: "change",
                    index: event_count,
                    changed_pixels: diff.changed_pixels,
                    changed_percent: diff.changed_percent,
                    bbox: diff.bbox,
                    path: saved_path_str,
                    timestamp: now_str,
                };
                if let Ok(json_line) = serde_json::to_string(&event_json) {
                    println!("{}", json_line);
                }
            } else if !args.quiet {
                println!(
                    "[{}] Change #{} detected: {:.2}% pixels changed, bbox: {:?}",
                    now_str, event_count, diff.changed_percent, diff.bbox
                );
            }

            baseline = current;

            if let Some(limit) = args.watch_count {
                if event_count >= limit {
                    if !args.quiet && !args.json {
                        println!("Reached target change count ({}), exiting watch mode.", limit);
                    }
                    break;
                }
            }
        }
    }

    Ok(())
}
