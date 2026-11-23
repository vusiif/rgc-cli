#![forbid(unsafe_code)]

pub mod action;
pub mod api;
pub mod capture;
pub mod cli;
pub mod doctor;
pub mod mcp;
pub mod output;
pub mod som;
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
            "Initiating window capture (query={:?}, region={:?})",
            win_query, args.region
        );
        let captured = capture::capture_window_with_details(win_query, args.region.as_ref())?;
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

fn run(args: Cli) -> Result<(), ApiError> {
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

    // Infer image format and validate against file extension if explicit format provided
    let format = output::validate_and_infer_format(&destination, args.format)?;

    // 2.5 Handle continuous recording if requested
    if let Some(record_secs) = args.record {
        return run_recording(&args, record_secs, destination);
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

    let dynamic_image = raw_captured.image;
    let scale_factor = raw_captured.scale_factor;
    let crop_offset = raw_captured.crop_offset;
    let monitor_origin = raw_captured.monitor_origin;

    // 4.1 Apply mouse cursor overlay if requested or query cursor info
    let global_cursor = capture::get_global_cursor_position();
    let cursor_info = capture::resolve_cursor_on_canvas(
        global_cursor,
        monitor_origin.unwrap_or((0, 0)),
        scale_factor,
        crop_offset,
        (dynamic_image.width(), dynamic_image.height()),
    );

    let mut rgba = dynamic_image.into_rgba8();
    if (args.cursor || args.highlight_cursor) && cursor_info.is_some() {
        if let Some(ref c) = cursor_info {
            transform::draw_cursor_overlay_in_place(&mut rgba, c.x, c.y, args.highlight_cursor);
        }
    }

    // 4.2 Apply privacy redaction masks (transparent hole, blackout, pixelate, blur) before scaling
    if !args.mask.is_empty() || !args.mask_window.is_empty() {
        let windows = if !args.mask_window.is_empty() {
            capture::get_windows().ok()
        } else {
            None
        };
        let redactions = capture::build_redactions(
            rgba.width(),
            rgba.height(),
            scale_factor,
            crop_offset,
            monitor_origin,
            args.coords,
            &args.mask,
            &args.mask_window,
            windows.as_deref(),
            args.allow_missing_mask,
        )?;
        for &(x, y, w, h, effect) in &redactions {
            transform::apply_redaction(&mut rgba, x, y, w, h, effect);
        }
    }

    let mut dynamic_image = output::apply_image_transforms(
        DynamicImage::ImageRgba8(rgba),
        args.scale,
        args.max_dimension,
    );

    // 4.2 Handle diff calculation against reference image if requested
    let mut diff_crop_offset = (0u32, 0u32);
    let diff_result = if let Some(ref diff_path) = args.diff_with {
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

        let diff = output::calculate_image_diff(&dynamic_image, &ref_image, args.diff_threshold);

        if args.crop_changed {
            if let Some(ref bbox) = diff.bbox {
                let padding = 8u32;
                diff_crop_offset = (bbox.x.saturating_sub(padding), bbox.y.saturating_sub(padding));
                dynamic_image = output::crop_to_diff_bbox(dynamic_image, bbox, padding);
            }
        }

        Some(diff)
    } else {
        None
    };

    let mut rgba = dynamic_image.into_rgba8();

    if args.highlight_diff {
        if let Some(ref diff) = diff_result {
            if let Some(ref bbox) = diff.bbox {
                let local_bbox = transform::DiffBBox {
                    x: bbox.x.saturating_sub(diff_crop_offset.0),
                    y: bbox.y.saturating_sub(diff_crop_offset.1),
                    width: bbox.width,
                    height: bbox.height,
                };
                output::highlight_diff_bbox_in_place(&mut rgba, &local_bbox, 2, None);
            }
        }
    }

    // 4.3 Apply visual grounding grid overlay if requested
    if let Some(grid_step) = args.grid {
        output::apply_grid_overlay_in_place(&mut rgba, grid_step);
    }

    // 4.4 Apply timestamp and/or custom tag watermark badge if requested
    if args.timestamp || args.tag.is_some() {
        let ts_str = if args.timestamp {
            Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
        } else {
            None
        };
        output::apply_watermark_and_tag_in_place(
            &mut rgba,
            ts_str.as_deref(),
            args.tag.as_deref(),
            output::WatermarkPosition::BottomRight,
        );
    }

    // 4.5 Detect UI elements and apply Set-of-Mark (SoM) visual overlay if requested
    let detected_elements = if args.som {
        let elements =
            som::detect_ui_elements_from_rgba(&rgba, args.som_threshold, args.som_max_elements);
        som::draw_som_overlay_in_place(&mut rgba, &elements);
        Some(elements)
    } else {
        None
    };

    // 4.6 Parse and draw click markers and drag trajectories
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

    if !clicks.is_empty() || !drags.is_empty() {
        action::draw_action_overlays_in_place(&mut rgba, &clicks, &drags);
    }

    let dynamic_image = DynamicImage::ImageRgba8(rgba);

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
    // Verify destination format: only GIF file or directory destination is supported
    if let OutputDestination::File(ref path) = destination {
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("gif") {
            return Err(ApiError::new(
                ErrorCode::BadArgs,
                format!(
                    "Unsupported output format for screen recording: '{}'. Recording to a single file only supports animated GIF (.gif), or specify an output directory (--output-dir) to save an image sequence.",
                    path.display()
                ),
            )
            .with_stage("argument_validation")
            .with_suggestion("Change file extension to .gif (e.g. -o recording.gif) or use --output-dir ./frames."));
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

    let mut frames = Vec::new();
    while start_time.elapsed() < total_duration {
        let frame_start = Instant::now();
        let raw = capture_target(args)?;
        let frame_img = raw.image;

        let mut rgba = frame_img.into_rgba8();
        if args.cursor || args.highlight_cursor {
            let global_cursor = capture::get_global_cursor_position();
            let origin = raw.monitor_origin.unwrap_or((0, 0));
            if let Some(cursor_info) = capture::resolve_cursor_on_canvas(
                global_cursor,
                origin,
                raw.scale_factor,
                raw.crop_offset,
                (rgba.width(), rgba.height()),
            ) {
                transform::draw_cursor_overlay_in_place(
                    &mut rgba,
                    cursor_info.x,
                    cursor_info.y,
                    args.highlight_cursor,
                );
            }
        }

        // Apply privacy redaction masks to each recorded frame before scaling
        if !args.mask.is_empty() || !args.mask_window.is_empty() {
            let windows = if !args.mask_window.is_empty() {
                capture::get_windows().ok()
            } else {
                None
            };
            let redactions = capture::build_redactions(
                rgba.width(),
                rgba.height(),
                raw.scale_factor,
                raw.crop_offset,
                raw.monitor_origin,
                args.coords,
                &args.mask,
                &args.mask_window,
                windows.as_deref(),
                args.allow_missing_mask,
            )?;
            for &(x, y, w, h, effect) in &redactions {
                transform::apply_redaction(&mut rgba, x, y, w, h, effect);
            }
        }

        let frame_img = output::apply_image_transforms(
            DynamicImage::ImageRgba8(rgba),
            args.scale,
            args.max_dimension,
        );

        let mut rgba = frame_img.into_rgba8();
        if let Some(grid_step) = args.grid {
            output::apply_grid_overlay_in_place(&mut rgba, grid_step);
        }

        if args.timestamp || args.tag.is_some() {
            let ts_str = if args.timestamp {
                Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
            } else {
                None
            };
            output::apply_watermark_and_tag_in_place(
                &mut rgba,
                ts_str.as_deref(),
                args.tag.as_deref(),
                output::WatermarkPosition::BottomRight,
            );
        }

        if args.som {
            let elements =
                som::detect_ui_elements_from_rgba(&rgba, args.som_threshold, args.som_max_elements);
            som::draw_som_overlay_in_place(&mut rgba, &elements);
        }

        frames.push(DynamicImage::ImageRgba8(rgba));

        let elapsed = frame_start.elapsed();
        if elapsed < interval {
            sleep(interval - elapsed);
        }
    }

    if frames.is_empty() {
        return Err(ApiError::new(
            ErrorCode::CaptureFailed,
            "No frames captured during recording.",
        ));
    }

    let frame_count = frames.len() as u32;
    let (w, h) = (frames[0].width(), frames[0].height());
    let mut file_size_bytes = None;
    let saved_path_buf;

    match destination {
        OutputDestination::Directory(ref dir) => {
            if !dir.exists() {
                fs::create_dir_all(dir).map_err(|e| {
                    ApiError::new(
                        ErrorCode::SaveFailed,
                        format!("Failed to create output directory: {e}"),
                    )
                })?;
            }
            for (idx, frame) in frames.iter().enumerate() {
                let filename = format!("frame_{:04}.png", idx + 1);
                let frame_path = dir.join(filename);
                output::save_image(
                    frame,
                    &OutputDestination::File(frame_path),
                    ImageFormat::Png,
                    None,
                )?;
            }
            saved_path_buf = dir.clone();
            if !args.quiet && !args.json {
                println!(
                    "Recorded {} frames saved to directory: {}",
                    frame_count,
                    dir.display()
                );
            }
        }
        _ => {
            let gif_dest = match destination {
                OutputDestination::File(p) => p,
                _ => {
                    let filename = output::generate_default_filename(ImageFormat::Gif);
                    PathBuf::from(filename)
                }
            };
            output::save_animated_gif(&frames, fps, &gif_dest)?;
            if let Ok(meta) = fs::metadata(&gif_dest) {
                file_size_bytes = Some(meta.len());
            }
            if !args.quiet && !args.json {
                println!("Recording saved as animated GIF: {}", gif_dest.display());
            }
            saved_path_buf = gif_dest;
        }
    }

    if args.json {
        let rec_json = RecordResultJson {
            status: "success".to_string(),
            path: saved_path_buf.display().to_string(),
            frame_count,
            duration_secs: start_time.elapsed().as_secs_f32(),
            fps,
            width: w,
            height: h,
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
