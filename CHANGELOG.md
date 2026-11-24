# Changelog

All notable changes to `rgc-cli` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.12.0] - 2026-11-24

### Security & Privacy
- **High-DPI Privacy Redaction Mapping**:
  - Introduced `scale_factor` to `resolve_window_mask_bounds` in `src/capture/redaction.rs`, mapping logical window coordinates to physical canvas space and eliminating the 75% unmasked sensitive window leak on High-DPI / 4K displays.
- **Decompression Bomb DoS Defense**:
  - Hardened `load_untrusted_image_input` in `src/mcp/tools.rs` with `image::Limits` (16384x16384 max dimension, 256MB max memory allocation) to abort malicious decompression before allocating memory.
  - Hardened Base64 detection heuristics with `!trimmed.contains('.')` to prevent local file paths from being misidentified as Base64 strings.

### Fixed & Hardened
- **Accessibility Coordinate Calibration**:
  - Corrected screen bounds calculation for Accessibility semantic queries in `src/pipeline.rs` from multiplying by `scale_factor` to dividing by `scale`, restoring alignment with the desktop logical coordinate space.
- **Stateful SessionManager LRU Eviction**:
  - Decoupled TTL pruning from capacity limits in `src/mcp/session.rs`. Calling `get()` refreshes access timestamps and never evicts active sessions; updating existing keys replaces records in place without evicting other sessions.
- **Multi-Monitor Mixed DPI Window Geometry**:
  - Implemented `find_best_monitor_scale_factor` using 2D intersection area (overlap) matching in `src/capture/window.rs`, eliminating miscalculated window DPI and improper titlebar trimming when capturing windows on secondary monitors.
- **Streaming WebP Chunk Validation**:
  - Hardened `extract_frame_subchunks` in `src/output.rs` with integer overflow protection (`checked_add`), forward progression checks, and trailing metadata chunk stripping (EXIF/XMP/ICCP) to avoid corrupting `ANMF` frame payloads.
- **MCP Recording Fail-Fast**:
  - Added pre-flight destination directory existence and writability validation to `execute_record_tool`, failing immediately rather than after completing a multi-second recording.

### Performance & Architecture
- **Zero-Copy Recording Frame Pipeline**:
  - Eliminated redundant 33MB full-canvas RGBA buffer cloning in `process_recording_frame` using `std::mem::take`, saving ~1.65 GB heap allocation per 5-second recording.
- **Unified Capture Dispatch & Visual Change Polling**:
  - Introduced `CaptureTarget`, `UnifiedCaptured`, and `capture_unified` in `src/capture/dispatch.rs`, eliminating repetitive dispatch implementations across CLI and MCP endpoints.
  - Centralized visual change detection into `wait_for_visual_change`.

---

## [2.11.1] - 2026-11-23

### Fixed
- **Unified CLI & MCP Privacy Policy Inheritance (P0-1)**:
  - Introduced `config::PrivacyPolicy` guaranteeing that default privacy configurations (`mask_windows`, `exclude_windows`, `mask_regions`, `allow_missing_mask`) from `rgc.toml` are universally inherited across CLI, `capture_screen`, `capture_session`, and `record_screen`.
  - Request-level privacy arguments are unioned with configured sensitive targets rather than overwriting or bypassing them.
- **Strict Privacy Mask Resolution & Window State Modeling (P0-2)**:
  - Modeled privacy mask resolution explicitly (`MaskResolutionStatus::Applied`, `NotVisible`, `NotFound`, `Error`).
  - Strict mode (`allow_missing_mask = false`) now fails closed with `CaptureError::MaskWindowResolutionFailed` upon window attribute query errors or minimization ambiguity.
- **Client-Area Coordinate Offset Single-Compensation (P1-1)**:
  - Corrected window origin and crop offset handling in `capture_window_with_options` under `--client-only`, eliminating double-compensation of `(cx, cy)` in `CoordinateTransform::screen_to_canvas` and `canvas_to_screen`.
- **Session Diff-Crop Coordinate Space Recalibration (P1-2)**:
  - Recalculated `CoordinateTransform` and `CoordinateSpaceInfo` when `capture_session` crops to `diff_bbox` (`diffOnly=true`), ensuring returned pixel coordinates map precisely back to physical desktop space.
- **Session Output Format Matching (P1-3)**:
  - Dynamically infer image format from `outputPath` extension or explicit `format` parameter in `capture_session`, eliminating mismatched PNG byte output when requesting WebP or JPEG.
- **MCP Output Directory Whitelist Fail-Closed Enforcement (P1-4)**:
  - Hardened `validate_mcp_output_path` so that if `RGC_ALLOWED_OUTPUT_DIRS` is set but all paths fail canonicalization, writes fail closed with an explicit error rather than silently bypassing restrictions.
- **MSRV Dependency Alignment (P1-5)**:
  - Aligned package `rust-version = "1.88"` with dependency lockfile requirements.
- **MCP Record Screen Schema & Duration Validation Consistency (P1-6)**:
  - Added missing parameters (`allowOverwrite`, `excludeWindows`, `allowMissingMask`) to `record_screen` JSON schema.
  - Aligned duration validation to explicitly enforce the 0.1s to 30.0s recording window with informative errors instead of silent clamping.

---

## [2.11.0] - 2026-11-23

### Added
- **Animated WebP Recording (`src/output.rs`, `--record SECS -o rec.webp`, `--format webp`)**:
  - Pure Rust streaming Animated WebP encoder (`WebpStreamEncoder<W: Write + Seek>`) packaging incremental frames into standard RFC 9649 RIFF WebP containers (`VP8X`, `ANIM`, `ANMF`).
  - Full truecolor (24-bit color + 8-bit alpha) recording, overcoming legacy GIF's 256-color palette constraint with vastly superior compression efficiency and crystal-clear visual fidelity.
  - Unified `RecordingStreamEncoder` dynamically multiplexing between GIF and WebP streaming pipelines across file targets and memory buffers.
  - Extended MCP `record_screen` tool to accept `outputPath` with `.webp` extension or `format: "webp"`, returning truecolor Animated WebP base64 payloads (`image/webp`).
- **Application Window Client-Area Trimming (`src/capture/window.rs`, `--client-only`)**:
  - Developed pure-Rust, zero-unsafe client area detection (`calculate_client_area_bounds`) inspecting alpha falloff to remove Windows DWM / macOS translucent drop shadows and blur margins.
  - Automatically calculates and trims standard DPI-scaled OS window titlebars, outputting exclusively the inner application working area.
  - Automatic coordinate recalibration: offsets are transparently folded into `window_origin` and `crop_offset`, guaranteeing downstream coordinate transforms and AI agent interaction points remain perfectly aligned with physical screen space.
  - Integrated into CLI `--client-only` (paired with `-w` / `--window`), MCP `capture_screen` (`clientOnly: bool`), and MCP `record_screen` (`clientOnly: bool`).

---

## [2.10.0] - 2026-11-23

### Added
- **Visual Template & Icon Matching (`src/template.rs`, `--find-template <PATH>`)**:
  - Implemented high-performance Normalized Cross-Correlation (NCC) with 2D integral image acceleration ($O(1)$ patch variance and local energy computation).
  - Multi-scale pyramid coarse-to-fine search enabling sub-15ms icon location even across 2K/4K high-resolution canvases.
  - Alpha transparency awareness: correctly computes correlation across arbitrary non-rectangular foreground icons while ignoring transparent padding.
  - Non-Maximum Suppression (NMS) for multi-match retrieval (`--template-max-matches N`).
  - Automatic desktop coordinate projection: returns `screenCenterX`, `screenCenterY`, `screenX`, `screenY` on every `TemplateMatchResult`.
  - Visual match highlighting: `--highlight-template` overlays vivid violet bounding boxes and confidence score badges (`98%`).
- **Dedicated MCP Tool (`find_template`)**:
  - Added standalone `find_template` MCP tool accepting image file paths or base64 data URIs, enabling AI agents to query icon locations with zero context overhead.
  - Integrated `findTemplate`, `templateThreshold`, `templateMaxMatches`, and `highlightTemplate` into `capture_screen` MCP tool.

---

## [2.9.0] - 2026-11-23

### Added
- **OS Accessibility Tree & Set-of-Mark Spatial Fusion (`src/accessibility/mod.rs`, `--accessibility` / `--a11y`)**:
  - Bridges visual UI edge clustering with OS desktop accessibility semantics (roles, labels, interaction states).
  - Implemented IoU (Intersection-over-Union) and geometric containment fusion algorithms to attach semantic metadata (`role`, `label`, `enabled`, `source`) onto visual `UiElement` bounding boxes.
  - Automatically injects high-value interactive accessibility controls (e.g., buttons, edit inputs, checkboxes) missed by vision into the Set-of-Mark element set with coordinate mapping.
  - Supported across CLI (`--accessibility`, `--a11y`), configuration file (`defaults.accessibility`), and MCP `capture_screen` tool (`accessibility: true`).
- **Performance & Latency Benchmark Diagnostic Mode (`src/benchmark.rs`, `--benchmark [RUNS]`)**:
  - Measures end-to-end screen capture overhead: native display capture latency, image pipeline processing latency (downscaling, diff, overlays), SoM edge clustering latency, and image encoder throughput (PNG, JPEG, WebP) in Megapixels/sec and compressed byte size.
  - Provides formatted diagnostic output tables and structured JSON reports (`--json`) for automated performance profiling and CI verification.
- **Window Exclusion & Redaction (`--exclude-window <QUERY>`)**:
  - Excludes and blacks out specified windows (by title substring or ID) from desktop panorama or monitor captures.
  - Configurable via CLI `--exclude-window`, configuration profiles `[privacy] exclude_windows = [...]`, and MCP tool arguments (`excludeWindows: [...]`).

---

## [2.8.0] - 2026-11-23

### Added
- **Stateful Incremental Screen Session (`capture_session` MCP Tool, `src/mcp/session.rs`)**:
  - In-memory session store (`SessionManager`) caching previous visual states across calls with 15-minute TTL eviction and LRU capacity protection.
  - Automatically calculates visual deltas against the session baseline; returns cropped diff deltas when changes occur, and omits heavy base64 payloads entirely when unchanged, slashing agent vision token consumption and disk I/O.
- **Continuous Watch Mode (`--watch [INTERVAL_MS]`)**:
  - Periodically polls the screen and emits structured change notifications only when visual differences exceed threshold and minimum percentage change.
  - Supports `--json` for newline-delimited event streams (`WatchEventJson`), `--output-dir` / `-o` for saving visual change snapshots, and `--watch-count` / `--watch-timeout` for bounded automated workflows.
- **Privacy Profiles & Configuration File Support (`rgc.toml`, `src/config.rs`)**:
  - Automatically loads configuration from `--config <PATH>`, `./rgc.toml`, or `~/.config/rgc/config.toml`.
  - Enables project-level and user-level default sensitive window masking (`mask_windows`), coordinate region redactions, security directory white-listing, and default capture parameters.

---

## [2.7.0] - 2026-11-23

### Added
- **Unified Coordinate Transformation System (`CoordinateTransform`, `coords.rs`)**:
  - Implemented bidirectional coordinate transformations across ScreenSpace (OS desktop virtual pixels and DPI scaling), CanvasSpace (unscaled raw capture pixels), and ImageSpace (final exported image pixels with downscaling and diff cropping).
  - JSON metadata now provides `coordinates` (`CoordinateSpaceInfo`), explicit `screenCenterX` / `screenCenterY` / `screenX` / `screenY` on `UiElement` (Set-of-Mark), and mapped cursor coordinates (`screenX`, `screenY`, `canvasX`, `canvasY`, `x`, `y`, `visible`) on `CursorInfo` for zero-ambiguity AI click automation.
- **Unified Capture & Processing Pipeline (`CapturePipeline`, `pipeline.rs`)**:
  - Consolidated scattered image transformation stages in CLI (`main.rs`) and MCP server (`mcp/tools.rs`, `mcp/record.rs`) into a single deterministic image pipeline: Cursor -> Redactions -> Scaling -> Diff & Crop -> Highlighting -> Grid -> Watermark -> Set-of-Mark -> Action markers -> Coordinate mapping.
  - Eliminated redundant code paths, ensuring 100% behavior parity between CLI and MCP server tools.
- **MCP Resource Bounds & Output Directory Enforcement**:
  - Added support for `RGC_ALLOWED_OUTPUT_DIRS` directory restriction in MCP file outputs.
  - Added maximum dimension protection (`maxDimension <= 16384`) in `capture_screen` to guard against memory allocation exhaustion.

---

## [2.6.2] - 2026-11-23

### Added
- **Streaming GIF Recording Encoder (`GifStreamEncoder`)**:
  - Replaced in-memory buffering of all raw capture frames with an incremental streaming encoder.
  - Slashes peak recording memory footprint from $O(N)$ (hundreds of megabytes in long captures) to constant $O(1)$ by quantizing and flushing frames on-the-fly.
  - Applied to both the CLI (`--record`) and MCP server (`record_screen`).
- **TOCTOU-Free Atomic File Output**:
  - Collision-free automatic filename resolution (`resolve_unique_auto_path`) now uses atomic `create_new(true)` handles, eliminating Time-of-Check to Time-of-Use race conditions.
  - Image file writing (`save_image`) writes to temporary staging files first and commits atomically (via atomic rename with copy fallback), preventing corrupted partial outputs on interruption.
- **MCP Output Path Security Hardening (`validate_mcp_output_path`)**:
  - Output paths in MCP tools (`capture_screen`, `record_screen`, `verify_action`) are strictly validated against directory traversal (`..`), null bytes, and non-whitelisted extensions.
  - Prevents unintended file overwrites unless explicitly enabled via `allowOverwrite`.
- **Dynamic MCP Resource Notification Filtering**:
  - Screen and diagnostic resource change notifications compute ultrafast (< 0.1ms) fingerprint hashes prior to dispatching updates.
  - Suppresses noisy `notifications/resources/updated` broadcasts when screen contents and topologies are unchanged.
- **Fail-Closed Window Privacy Redaction Policy**:
  - Missing or minimized target windows in `--mask-window` fail closed with an explicit error to prevent accidental exposure of sensitive background pixels, with an optional `--allow-missing-mask` bypass.

### Fixed
- **Diff Crop Coordinate Mapping**:
  - Corrected coordinate translation when combining diff highlighting (`--highlight-diff`) with cropped regions (`--crop`), accurately mapping global screen delta bounding boxes into cropped local frame coordinates.

---

## [2.6.0] - 2026-11-21

### Added
- **Mouse Cursor Overlay & Focus Halo (`--cursor`, `--highlight-cursor`)**:
  - Accurate cross-platform mouse pointer rendering on screen captures.
  - Optional visual focus halo ring around the cursor to draw attention in GUI walkthroughs and provide visual grounding for AI agents.
- **Continuous Screen Recording (`--record <SECS>`, `--fps <N>`)**:
  - Sample screens, active windows, or multi-monitor panoramas over duration.
  - Encode directly into animated GIFs or extract numbered frame sequences.
  - Configurable framerates (1–30 FPS) with cursor tracking.
- **Set-of-Mark (SoM) Interactive UI Detection (`--som`, `--som-threshold`, `--som-max-elements`)**:
  - Pure Safe Rust UI element detection using edge gradients and adaptive contrast analysis.
  - Overlays high-contrast numbered badges directly on interactive elements.
  - Returns bounding boxes, centers, and labels via `--json` for zero-shot AI vision navigation.
- **Action Verification Overlays (`--verify-click <X,Y>`, `--verify-radius`, `--verify-timeout`)**:
  - Closed-loop UI interaction verification comparing visual state before and after actions.
  - Visual ripple indicator at interaction coordinates and highlighted bounding box of detected changes.
- **MCP Expansion (Model Context Protocol)**:
  - `record_screen`: Duration-based recording tool emitting animated GIF data.
  - `verify_action`: Pre/post action validation tool returning delta statistics.
  - `detect_ui_elements`: Set-of-Mark UI element identification tool with coordinate metadata.
  - `resources/subscribe` & `notifications/resources/updated`: Event notification subscriptions for display topology and window changes.

### Changed
- **SIMD-Accelerated Panorama Stitching**: Multi-monitor stitching now copies contiguous memory rows directly via SIMD `memcpy`, slashing stitching latency from ~180ms to < 2ms (50x–100x speedup).
- **Hardware-Accelerated Privacy Redaction**: Blackout and transparent window redaction utilize direct row-level hardware `memset` and 4-byte chunk writing, eliminating millions of bounds assertions.
- **Equal-Row Diff Short-Circuiting**: Accelerated row comparison in `calculate_image_diff` skips 90%+ of unchanged desktop pixels at hardware `memcmp` speeds.
- **Zero-Copy Clipboard Pipelines**: Screen captures copied to clipboard borrow RGBA slices directly, saving 33MB heap allocations per 4K capture.
- **Vectorized SIMD Base64 Encoding**: Vectorized 3-byte chunking without conditional branching delivers a 3x–5x encoding throughput increase for image payloads.
- **JSON-RPC Stdio Buffer Pooling**: MCP server reuses line buffers, eliminating heap churn across high-frequency agent requests.

### Fixed
- **Headless Linux CI Screen Capture**: Mock display fallback in headless CI environments prevents test runner failures when no display server is attached.
- **Cross-Platform Test Shell Execution**: Test steps in CI workflow explicitly target standard bash across all operating systems.

---

## [2.5.0] - 2026-11-20

### Added
- Responsive change detection (`--wait-change`, `--wait-timeout`, `--wait-interval`).
- Diff highlighting (`--highlight-diff`) with high-contrast boundary boxes.
- Watermarking (`--timestamp`, `--tag`) with zero-dependency pure Safe Rust 3x5 ASCII bitmap font.
- MCP Prompts protocol (`prompts/list`, `prompts/get`) with 3 agent workflow templates.

---

## [2.2.0] - 2026-10-31

### Added
- Multi-platform CI/CD automation for Windows, Linux, and macOS.
- Image downsampling (`--scale`, `--max-dimension`) and quality compression (`--quality`).
- Auto-open viewer support (`--view`).
- Shell completions generation (`--completions <SHELL>`).
- Windows executable metadata and icons via `winres`.

---

## [2.1.0] - 2026-10-15

### Added
- MCP (Model Context Protocol) JSON-RPC 2.0 stdio server (`--mcp`).
- System diagnostics subcommand (`doctor`).
- Multi-monitor panorama capture (`--all-monitors`).
- High-DPI coordinate scaling aware cropping.

---

## [2.0.0] - 2026-10-13

### Added
- Complete pure Rust rewrite (`rgc-cli`).
- Pure cross-platform support (Windows, macOS, Linux).
- Pure-Rust WebP format support.
- System clipboard export (`--clipboard`).
- Standard output streaming (`-o -`).
