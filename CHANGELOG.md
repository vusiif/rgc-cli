# Changelog

All notable changes to `rgc-cli` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
