# rgc-cli (Rust Graphics Capture)

[![CI](https://github.com/vusiif/wgc-cli-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/vusiif/wgc-cli-rust/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Language: Rust](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/vusiif/wgc-cli-rust)
[![MCP Compatible](https://img.shields.io/badge/MCP-Compatible-brightgreen.svg)](https://modelcontextprotocol.io)

**[English](README.md)** | **[简体中文](README_zh.md)**

Fast, lightweight, and cross-platform command-line screen capture utility and Model Context Protocol (MCP) server written purely in Rust.

> [!NOTE]
> `rgc-cli` is the cross-platform, pure-Rust evolution of the original `wgc-cli-rust` project. It eliminates all platform-dependent C++ glue and runs natively on Windows, macOS, and Linux with zero external native build dependencies.

---

## 🌟 Features

- **Pure Cross-Platform**: Native support for Windows, macOS, and Linux (X11 & Wayland) with 100% safe Rust code (`#![forbid(unsafe_code)]`).
- **Model Context Protocol (MCP)**: Built-in `--mcp` stdio server implementing the official MCP JSON-RPC 2.0 specification for seamless AI agent integration (Claude Desktop, Cursor, Cline, Roo Code).
- **Modern Formats (PNG, JPEG, WebP)**: Native WebP encoding support, typically delivering ~20-35% file size reduction compared to PNG with lossless or near-lossless clarity.
- **Structured JSON Mode**: `--json` flag formats monitor lists, captured metadata, and errors into machine-readable JSON envelopes, ideal for scripting and automation.
- **System Diagnostics**: Built-in `--doctor` flag to inspect platform display subsystems, DPI scales, capture backends, and clipboard readiness.
- **Direct Clipboard Export**: Copy screenshots directly to the system clipboard via `--clipboard` / `-c` without touching the filesystem.
- **Stdout Streaming**: Stream raw encoded image bytes to stdout (`-o -`) with graceful `BrokenPipe` handling for seamless UNIX piping (`wl-copy`, `xclip`, ImageMagick, ffmpeg).
- **High-DPI Awareness**: Accurate translation between user logical coordinates and physical screen pixels with display `scale_factor` (e.g. 1.25x, 1.50x, 2.0x), adapting seamlessly to HiDPI displays.
- **Privacy Redaction & Transparent Hole Punching**: Tailored for AI Agent vision workflows with `--mask <x,y,w,h[:effect]>` and `--mask-window <ID|TITLE[:effect]>`. Supports 4 redaction effects: `transparent` (alpha=0 hole punching), `blackout` (solid black), `pixelate` (mosaic), and `blur` (downsampled bilinear blur). Executed before downsampling to prevent interpolation leakage.
- **Visual Grounding Grid & Anchors**: Overlay subtle visual grid lines and coordinate anchors (`--grid [STEP]`) with a built-in 3x5 bitmap font in pure safe Rust, dramatically boosting AI Agent click grounding and GUI navigation accuracy.
- **Screen Diff & Incremental Token Cropping**: Dynamic visual difference calculation (`--diff-with <IMAGE>`, `--diff-threshold <0-255>`, `--crop-changed`) to compare against reference screenshots, reporting changed pixel counts, bounding box (`changed_bbox`), and automatically cropping to only changed areas to save 80%+ multimodal tokens.
- **Multi-Monitor Management & Panorama Stitching**: Enumerate all connected displays and capture any screen by index (`--list-monitors`, `--monitor <INDEX>`), or seamlessly stitch dual/triple/multi-monitor virtual desktops into a single unified panorama with `-a, --all-monitors`, adapting to negative virtual desktop coordinates and heterogeneous display layouts.
- **Sub-Image Region Cropping**: Flexible rectangle cropping (`--region x,y,w,h`) with fast bounds pre-validation.
- **Collision-Resistant Naming**: Millisecond-accurate timestamping with atomic monotonic counter prevents filename collisions in rapid capture loops.
- **Delayed Capture**: Built-in timer delay before taking screenshot (`--delay <MS>`).

---

## 📦 Installation & Build

Ensure you have the Rust toolchain installed (1.75+ recommended):

```bash
cargo build --release
```

The compiled release binary is located at `target/release/rgc-cli` (or `rgc-cli.exe` on Windows), typically weighing in at approximately **~2 to 3 MB** thanks to LTO and binary stripping.

---

## 🚀 Usage Guide

```text
Usage: rgc-cli [OPTIONS]

Options:
      --doctor                Run diagnostic environment check and inspect capture capabilities
      --mcp                   Start Model Context Protocol (MCP) stdio server for AI agent automation
  -l, --list-monitors         List all available monitors and exit
      --list-windows          List all active application windows and exit
  -m, --monitor <INDEX>       Monitor index to capture (0-indexed). Defaults to primary monitor
  -w, --window <ID|TITLE>     Capture a specific window by ID or title substring (e.g. -w 1234 or -w "Code")
  -a, --all-monitors          Capture all connected monitors stitched into a virtual desktop panorama (conflicts with -m, -w)
  -r, --region <x,y,w,h>      Region to capture in coordinates: 'x,y,w,h' (e.g. 100,100,800,600)
      --coords <MODE>         Coordinate system for region: 'logical' (default, scaled by DPI) or 'physical' (raw pixels)
      --mask <x,y,w,h[:effect]> Redact rectangular region(s): 'x,y,w,h[:effect]' (effect: transparent, blackout, pixelate, blur; default: transparent)
      --mask-window <TARGET>  Auto-detect window bounds & redact: 'ID|TITLE[:effect]' (effect: transparent, blackout, pixelate, blur; default: transparent)
  -o, --output <PATH>         Output file path, directory, or '-' for stdout stream. Defaults to timestamped file in CWD
      --output-file <FILE>    Explicit output file destination (creates parent directories if missing)
      --output-dir <DIR>      Explicit output directory destination (timestamped filename generated inside)
  -c, --clipboard             Copy screenshot directly to system clipboard
  -d, --delay <MS>            Delay before capturing in milliseconds (e.g. 1000 for 1 second)
  -f, --format <FORMAT>       Output image format. If omitted, inferred from file extension [possible values: png, jpeg, webp, gif]
      --scale <FACTOR>        Downscale image by a factor between 0.01 and 1.0 (e.g. 0.5 for half resolution)
      --max-dimension <PX>    Constrain maximum width or height to PX pixels, preserving aspect ratio
      --quality <1-100>       Compression quality for JPEG output (1-100, defaults to 85)
      --grid [STEP]           Overlay visual grounding grid lines and coordinate anchors (default step: 100px)
      --diff-with <IMAGE>     Compare with reference screenshot and compute difference bounding box
      --diff-threshold <0-255> Pixel difference tolerance threshold (default: 16)
      --crop-changed          Crop output to only the bounding box of changed pixels (requires --diff-with)
      --highlight-diff        Draw a prominent high-contrast boundary box on the image around differences (requires --diff-with)
      --timestamp             Burn a UTC/local timestamp watermark badge onto the output screenshot
      --tag <TEXT>            Burn a custom text tag (e.g. step name, session ID) onto the output screenshot
      --wait-change           Wait for screen/window visual change before capturing (ideal for UI automation)
      --wait-timeout <MS>     Maximum timeout in ms to wait for a visual change before capturing (default: 5000)
      --wait-interval <MS>    Polling interval in ms between change detection checks (default: 100)
      --cursor                Capture and render mouse cursor on the screenshot
      --highlight-cursor      Render a prominent visual highlight ring around the mouse cursor
      --record <SECS>         Continuously sample and record screen over SECONDS duration (saved as animated GIF or image sequence)
      --fps <1-30>            Sampling framerate (FPS, 1-30) for continuous recording (default: 5)
      --open                  Open captured screenshot file with system default viewer
      --completions <SHELL>   Generate shell completion script to stdout [possible values: bash, elvish, fish, powershell, zsh]
      --json                  Output result as structured JSON (ideal for automation & AI agents)
  -v, --verbose               Enable verbose / debug logging
  -q, --quiet                 Suppress non-error output
  -h, --help                  Print help
  -V, --version               Print version
```

### Examples

#### 1. Copy directly to clipboard (no disk files created)
```bash
rgc-cli -c
```

#### 2. Pipe screenshot to another tool via stdout
```bash
# Save via shell redirection
rgc-cli -o - > screenshot.png

# Linux Wayland / X11 clipboard pipe
rgc-cli -o - | wl-copy
```

#### 3. Save as modern WebP with structured JSON output
```bash
rgc-cli -o shot.webp --json
```
Output:
```json
{
  "ok": true,
  "data": {
    "status": "success",
    "path": "shot.webp",
    "width": 2560,
    "height": 1440,
    "format": "Webp",
    "clipboard": false,
    "sizeBytes": 417652
  }
}
```

#### 4. Save into a directory with auto-generated collision-free timestamp
```bash
rgc-cli --output-dir ./screenshots --json
```
Output:
```json
{
  "ok": true,
  "data": {
    "status": "success",
    "path": "screenshots/screenshot_2026-10-16_00-00-19.163_0000.png",
    "width": 2560,
    "height": 1440,
    "format": "Png",
    "clipboard": false,
    "sizeBytes": 574834
  }
}
```

#### 5. Query monitors in JSON format
```bash
rgc-cli --list-monitors --json
```
Output:
```json
{
  "ok": true,
  "data": [
    {
      "index": 0,
      "name": "\\\\.\\DISPLAY1",
      "is_primary": true,
      "width": 2560,
      "height": 1440,
      "scale_factor": 1.5
    }
  ]
}
```

#### 6. Structured Error Response
When an error occurs in `--json` mode, machine-readable diagnostic details are returned to stdout with an appropriate non-zero exit code:
```bash
rgc-cli -m 999 --json
```
Output (Exit Code 3):
```json
{
  "ok": false,
  "error": {
    "errorCode": "MONITOR_NOT_FOUND",
    "message": "Monitor index 999 is out of bounds (found 1 monitors).",
    "stage": "monitor_selection",
    "suggestion": "Run 'rgc-cli --list-monitors' to see all valid monitor indices.",
    "exitCode": 3
  }
}
```

#### 7. Dual output: Copy to clipboard AND save to file
```bash
rgc-cli -c -o ./archive/
```

#### 8. Delayed regional capture (HiDPI / Retina aware)
```bash
rgc-cli -d 1500 -r 200,200,600,400 -o cropped.png
```

#### 9. Exact 1:1 physical pixel crop
```bash
rgc-cli --coords physical -r 0,0,1920,1080 -o raw_pixels.png
```

#### 10. Downscaling and Quality (AI Token & Bandwidth Optimization)
```bash
# Half-resolution capture (e.g. 3840x2160 -> 1920x1080, reducing payload size by ~75%)
rgc-cli --scale 0.5 -o shot.webp

# Constrain image maximum edge to 1280px preserving aspect ratio
rgc-cli --max-dimension 1280 -o preview.png

# JPEG output with custom compression quality
rgc-cli -f jpeg --quality 80 -o shot.jpg
```

#### 11. Automatically Open Saved Screenshot in Default Viewer
```bash
# Capture and immediately open in default image viewer
rgc-cli --open -o shot.png
```

#### 12. Shell Tab Completions Generation
```bash
# Generate PowerShell completions
rgc-cli --completions powershell | Out-String | Invoke-Expression

# Generate Bash / Zsh completions
rgc-cli --completions bash > /etc/bash_completion.d/rgc-cli
rgc-cli --completions zsh > "${fpath[1]}/_rgc-cli"
```

#### 13. Window-Level Precise Capture
```bash
# List all active windows (titles, processes, IDs)
rgc-cli --list-windows

# Capture a specific window by title substring (case-insensitive)
rgc-cli -w "Visual Studio Code" -o vscode.png

# Capture a window by numeric window ID and open immediately
rgc-cli -w 197430 --open

# Window capture with regional crop inside the window
rgc-cli -w "Chrome" -r 0,0,800,600 -o browser_top.png
```

#### 14. Privacy Redaction & Transparent Hole Punching (AI Agent Protection)
```bash
# 1. Punch transparent hole over sensitive area (Alpha=0, default effect)
rgc-cli --mask 100,100,200,200 -o safe.png

# 2. Combine multiple masks with different effects (blur, blackout, pixelate, transparent)
rgc-cli --mask 50,50,300,100:blur \
        --mask 400,200,150,50:blackout \
        --mask 600,600,200,200:pixelate \
        -o multi_masked.png

# 3. Auto-locate application windows and redact from screen capture
rgc-cli --mask-window "1Password:transparent" \
        --mask-window "WeChat:blur" \
        -o desktop_desensitized.png

# 4. HiDPI adaptation (e.g. 150% scaling: logical coordinates automatically scale by 1.5x)
rgc-cli --coords logical --mask 100,100,200,200 -o hidpi_safe.png

# 5. Redaction combined with downscaling (masks applied BEFORE downsampling to prevent leakage)
rgc-cli --mask 100,100,500,200:blur --scale 0.5 -o low_token_safe.webp
```

#### 15. Multi-Monitor Panorama Stitching (Multi-Screen Virtual Desktop)
```bash
# Automatically stitch all connected monitors into an ultra-wide virtual desktop canvas
rgc-cli -a -o panorama.png

# Panorama capture combined with downscaling (reduces multi-4K screen tokens for AI vision models)
rgc-cli -a --max-dimension 2048 -o pano_low_token.webp

# Panorama capture combined with sensitive window redaction (punches hole over 1Password across all monitors)
rgc-cli -a --mask-window "1Password:transparent" -o pano_safe.png
```

#### 16. Visual Grounding Grid & Coordinate Anchors (AI Agent Localization)
```bash
# Overlay subtle cyan grid with crosshairs and coordinate labels every 100 pixels
rgc-cli --grid -o grid_100.png

# Custom 50px high-density grid for fine-grained coordinate estimation
rgc-cli --grid 50 -o grid_50.png

# Panorama capture combined with grid overlay and downscaling for AI vision models
rgc-cli -a --grid 100 --max-dimension 2048 -o pano_grid.webp
```

#### 17. Screen Diff & Incremental Token-Saving Crop (Agent Action Verification)
```bash
# Compare against previous screenshot and return structured diff metadata
rgc-cli --diff-with prev.png --json

# Crop output to only the bounding box of changed UI elements (saves 80%+ tokens)
rgc-cli --diff-with prev.png --crop-changed -o delta.png --json

# Custom pixel tolerance threshold (0-255, default: 16)
rgc-cli --diff-with prev.png --diff-threshold 32 --json
```

#### 18. Highlight Changed Regions & Burn Timestamp Watermark
```bash
# Draw high-contrast red bounding box with DIFF:WxH label over changed areas
rgc-cli --diff-with prev.png --highlight-diff -o diff_highlighted.png

# Burn UTC/local timestamp and custom task/step badge onto screenshot
rgc-cli --timestamp --tag "STEP-LOGIN" -o tracked.png

# Combined workflow: panorama + grid + diff highlight + timestamp audit
rgc-cli -a --grid 100 --diff-with baseline.png --highlight-diff --timestamp --tag "TASK_042" -o trace.png
```

#### 19. Responsive Wait for Screen Changes (Zero-Latency UI Automation)
```bash
# Poll screen and capture immediately upon detecting UI visual update (5s timeout)
rgc-cli --wait-change --wait-timeout 5000 -o after_click.png

# Wait for window state transition and crop only the changed sub-region
rgc-cli -w "Calculator" --wait-change --crop-changed --diff-with prev.png -o calc_result.png
```

#### 20. Run system diagnostics
```bash
# Human-readable text report
rgc-cli --doctor

# Structured JSON report
rgc-cli --doctor --json
```
Output:
```json
{
  "ok": true,
  "data": {
    "os": "windows",
    "arch": "x86_64",
    "backend": "Windows Graphics Capture (WGC) / DXGI / GDI",
    "monitorsDetected": 1,
    "primaryMonitor": "[0] \\\\.\\DISPLAY1 - 2560x1440 @ 1.50x",
    "clipboardAvailable": true,
    "supportedFormats": [
      "PNG (Lossless)",
      "JPEG (Standard)",
      "WebP (High-efficiency native)"
    ],
    "status": "Operational"
  }
}
```

#### 21. Mouse Cursor Capture & Focus Halo Ring
```bash
# Capture screenshot with visible mouse cursor
rgc-cli --cursor -o shot_with_cursor.png

# Highlight mouse cursor position with an amber focus halo ring (ideal for GUI tutorials & AI grounding)
rgc-cli --highlight-cursor -o shot_highlighted.png
```

#### 22. Continuous Screen Recording to Animated GIF & Image Sequences
```bash
# Record screen for 3 seconds at 10 FPS into an animated GIF
rgc-cli --record 3.0 --fps 10 -o recording.gif

# Record specific window with cursor highlight and save frames into a directory
rgc-cli -w "Code" --record 5.0 --fps 5 --cursor --output-dir ./record_frames/
```

---

## 🤖 Model Context Protocol (MCP) Integration

`rgc-cli` natively acts as an MCP stdio server conforming to JSON-RPC 2.0. This allows AI coding agents (Claude Desktop, Cursor, Cline, Roo Code, Windsurf) to directly take screenshots, record screen activity, subscribe to visual changes, and inspect displays.

### Exposed MCP Tools

1. **`list_monitors`**: Enumerate all connected screens, resolutions, and DPI scale factors.
2. **`list_windows`**: Enumerate all active application windows, titles, IDs, and positions.
3. **`capture_screen`**: Capture screen, window, stitched multi-monitor panorama, or cropped region with direct base64 image payload (`image/png`, `image/webp`, `image/jpeg`, `image/gif`) or output file.
   - Arguments: `monitor` (int), `window` (string), `allMonitors` (bool), `region` (string `"x,y,w,h"`), `coords` (`"logical"` | `"physical"`), `format` (`"png"` | `"jpeg"` | `"webp"` | `"gif"`), `scale` (number `0.01..1.0`), `maxDimension` (int `>0`), `quality` (int `1..100`), `grid` (int, pixel step), `diffWith` (string, path or Base64), `diffThreshold` (int, 0..255), `cropChanged` (bool), `highlightDiff` (bool), `timestamp` (bool), `tag` (string), `waitChange` (bool), `waitTimeoutMs` (int), `waitIntervalMs` (int), `cursor` (bool), `highlightCursor` (bool), `redactions` (array), `maskWindows` (array), `outputPath` (string), `returnBase64` (bool).
4. **`record_screen`**: Continuously record screen, window, or panorama over a specified duration into an animated GIF or sequence of frames.
   - Arguments: `durationSecs` (number, required), `fps` (int, 1..30, default 5), `monitor` (int), `window` (string), `allMonitors` (bool), `cursor` (bool), `highlightCursor` (bool), `scale` (number), `maxDimension` (int), `outputPath` (string), `returnBase64` (bool).
5. **`doctor`**: Diagnostic check of display subsystem, active backend, and clipboard capabilities.

### Exposed MCP Resources & Subscriptions

`rgc-cli` supports active resource subscriptions (`resources/subscribe` & `resources/unsubscribe`) and live change push notifications (`notifications/resources/updated`):

1. **`screen://current`**: Live snapshot of the primary display (`image/png` binary blob).
2. **`screen://all`**: Live panorama snapshot of all connected displays stitched together (`image/png` binary blob).
3. **`screen://monitor/{index}`**: Dynamic snapshot of a specific display monitor by zero-based index (e.g. `screen://monitor/0`).
4. **`screen://window/{id}`**: Dynamic snapshot of a specific application window by numeric ID (e.g. `screen://window/1234`).
5. **`screen://diagnostics`**: Machine-readable JSON diagnostic report of hardware, display server, and capture subsystem.

### Client Configuration Examples

#### Claude Desktop (`claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "screen-capture": {
      "command": "rgc-cli",
      "args": ["--mcp"]
    }
  }
}
```

#### Cline / Roo Code / Cursor (`mcp_settings.json`)

```json
{
  "mcpServers": {
    "rgc": {
      "command": "/absolute/path/to/rgc-cli",
      "args": ["--mcp"]
    }
  }
}
```

---

## 🚦 Exit Codes & Error Specification

In `--json` mode or standard CLI usage, exit codes adhere strictly to the following specification:

| Exit Code | Error Code Identifier | Stage | Description |
|:---:|:---|:---|:---|
| **0** | `SUCCESS` | `complete` | Operation succeeded |
| **1** | `GENERIC_ERROR` | `execution` | Uncategorized failure |
| **2** | `BAD_ARGS` | `cli_parsing` | Invalid arguments or conflicting CLI flags |
| **3** | `NO_MONITOR` / `MONITOR_NOT_FOUND` | `monitor_selection` | No display found or specified index is out of bounds |
| **4** | `REGION_OUT_OF_BOUNDS` | `region_calculation` | Crop coordinates exceed target monitor dimensions |
| **5** | `CAPTURE_FAILED` | `capture` | OS capture API failed to capture display buffer |
| **6** | `FORMAT_MISMATCH` | `format_inference` | Explicit `--format` contradicts file extension |
| **7** | `IO_ERROR` / `ENCODE_FAILED` | `output_save` | Disk write error, missing permissions, or encoding failure |
| **8** | `CLIPBOARD_ERROR` | `clipboard_export` | System clipboard unavailable or write failed |

---

## 📐 Coordinate System & DPI Semantics

- **Origin `(0, 0)`**: Strictly defined as the top-left corner of the selected monitor (or primary monitor by default).
- **`--coords logical` (Default)**: User-provided values are treated as logical screen coordinates and multiplied by the display's DPI `scale_factor` (e.g. 1.25x, 1.5x, 2.0x) using overflow-safe rounded arithmetic.
- **`--coords physical`**: User-provided values represent raw display buffer pixels directly, 1:1 without scaling.
- **Safety**: Both modes strictly validate coordinate boundaries, non-zero positive dimensions, and verify against the actual captured frame buffer dimensions.

---

## 🏛 Architecture

- [`src/main.rs`](src/main.rs): Clean CLI entry point, centralized error handling, and top-level pipeline orchestration.
- [`src/cli.rs`](src/cli.rs): Type-safe command-line parsing powered by `clap` derive, including custom `Region` parser with boundary checks and serde derivations.
- [`src/api.rs`](src/api.rs): Common API error codes, unified response envelope, and structured JSON contracts.
- [`src/capture/`](src/capture/): Modular cross-platform screen and window capture package:
  - `types.rs`: Capture errors, display info, and captured image containers.
  - `crop.rs`: Overflow-safe and DPI-aware coordinate mapping and validation.
  - `redaction.rs`: Privacy mask region resolution and window intersection geometry.
  - `screen.rs`: Physical display enumeration and monitor screenshot capture.
  - `window.rs`: Window enumeration, fuzzy title matching, and window capture.
  - `panorama.rs`: Multi-monitor virtual desktop canvas stitching and negative coordinate normalization.
  - `mod.rs`: Backward-compatible public exports.
- [`src/transform.rs`](src/transform.rs): Pure image processing pipeline (redactions, 3x5 ASCII bitmap text rendering, grid overlay, pixel diff calculation, diff bounding box highlighting, and watermark badge).
- [`src/output.rs`](src/output.rs): Clipboard export via `arboard`, stdout streaming with BrokenPipe safety, collision-resistant timestamping, format validation, and persistence.
- [`src/doctor.rs`](src/doctor.rs): Environment and hardware diagnostics, subsystem capability checks, and structured reporting.
- [`src/mcp/`](src/mcp/): Modular Model Context Protocol stdio server:
  - `types.rs`: JSON-RPC 2.0 protocol structs and MCP entities.
  - `base64.rs`: Pure Safe Rust RFC 4648 base64 codec with test vectors.
  - `resources.rs`: MCP Resource URI handling and image encapsulation.
  - `tools.rs`: Tool JSON Schemas and dispatch execution.
  - `prompts.rs`: Standard MCP Prompts protocol and built-in agent workflow templates.
  - `mod.rs`: Stdio loop, request routing, and integration tests.

---

## 🧪 Testing

Run all unit tests in headless environments (no physical display or clipboard needed):

```bash
cargo test
```

Run hardware-dependent tests (requires active graphical desktop session and clipboard access):

```bash
cargo test -- --ignored
```

---

## 📄 License

Licensed under the [Apache License, Version 2.0](LICENSE).
