# Release Notes - v2.6.0: AI Grounding, Action Verification & SIMD Performance

This milestone release brings powerful visual AI grounding capabilities, continuous recording, closed-loop action verification, and order-of-magnitude performance improvements to `rgc-cli`.

---

## 🚀 Key Highlights & New Features

### 1. Set-of-Mark (SoM) Interactive UI Detection (`--som`)
- **Visual Grounding for AI Vision Models**: Detects interactive buttons, input fields, and UI controls using pure Safe Rust adaptive edge gradient and contrast analysis.
- **Numbered Badges**: Overlays high-contrast, visually distinct numbered badges directly on detected interactive elements.
- **Structured JSON Metadata**: Outputs bounding box geometries, centers, and element identifiers via `--json` for direct ingestion by vision-language models (VLMs) and autonomous agents.

### 2. Action Verification Overlays (`--verify-click <X,Y>`)
- **Closed-Loop Feedback**: Automatically samples visual state before and after an action to verify whether a localized UI change occurred.
- **Visual Ripple & Change Highlight**: Annotates the screenshot with a click ripple marker and a high-contrast boundary box around detected visual differences.

### 3. Continuous Screen Recording to Animated GIF (`--record <SECS>`)
- **Effortless GIF Generation**: Captures screen activity, windows, or panoramas at configurable framerates (1–30 FPS) and compiles directly into compact animated GIFs or numbered image sequences.
- **Cursor Tracking**: Seamlessly tracks and embeds the mouse cursor position throughout the recording session.

### 4. Mouse Cursor Overlay & Halo Ring (`--cursor`, `--highlight-cursor`)
- Renders cross-platform mouse pointer position with an optional amber focus halo ring, perfect for GUI automation walkthroughs and agent action tracking.

### 5. Extended Model Context Protocol (MCP) Capabilities
- Added `record_screen`, `verify_action`, and `detect_ui_elements` tools.
- Added `resources/subscribe` and `notifications/resources/updated` for reactive display topology and window change tracking.

### 6. SIMD & Zero-Copy Performance Suite
- **50x–100x Panorama Stitching**: Replaced software Alpha blending with contiguous row `memcpy`, dropping multi-monitor stitching latency from ~180ms to < 2ms.
- **Hardware Memset Redaction**: Transparent and blackout window masks now compile directly to assembly-level hardware `memset` and 4-byte chunk writes.
- **Hardware Memcmp Diffing**: Equal-row short-circuiting skips 90%+ of unchanged desktop pixels in image diffing at memory bandwidth speeds.
- **Zero-Copy Pipelines**: RGBA slice borrowing prevents 33MB heap allocations during clipboard export, and vectorized 3-byte chunking boosts Base64 throughput 3x–5x.

---

# Release Notes - v2.0.0: The Cross-Platform Rewrite (`rgc-cli`)

This major release marks the complete transformation of the legacy Windows-only `wgc-cli` into **`rgc-cli` (Rust Graphics Capture)** — a purely cross-platform, high-performance, and lightweight CLI utility written 100% in Rust.

---

## 🚀 Key Highlights & New Features

### 1. Pure Cross-Platform Architecture
- **Zero OS-specific C/C++ bindings**: Completely eliminated the MSVC C++ WGC glue layer (`wgc_capture.cpp`) and native Windows dependencies.
- **Full Tier-1 Platform Support**: Works seamlessly on **Windows**, **macOS**, and **Linux** (both X11 and Wayland sessions) powered by `xcap`.

### 2. Modern Formats: Lossless Pure-Rust WebP
- Added first-class support for **WebP** (`--format webp`, or auto-inferred via `-o output.webp`), reducing screenshot file size by **~30%** compared to PNG while retaining lossless, pixel-perfect visual clarity.
- Continued full support for standard **PNG** and **JPEG** encoders.

### 3. Direct-to-Clipboard Export (`--clipboard` / `-c`)
- Directly copy captured frames to the system clipboard across all platforms without generating temporary files on disk.
- Supports dual export: copy to clipboard and save to disk simultaneously (`-c -o ./path`).

### 4. Unix Stdout Streaming (`-o -`)
- Stream raw encoded image bytes directly to standard output, enabling seamless shell piping into tools like `wl-copy`, `xclip`, `ImageMagick`, `ffmpeg`, or AI agent pipelines.

### 5. High-DPI & Retina Scaling Awareness
- Explicit translation between logical user coordinates and physical pixel buffers using monitor `scale_factor`.
- Region cropping (`--region x,y,w,h`) maps precisely across 100%, 125%, 150%, and 200%+ display scalings with zero-cost sub-image slicing in hot paths.

### 6. Structured JSON Output (`--json`)
- `--list-monitors --json`: Outputs monitor indices, IDs, physical resolutions, and DPI factors as a structured JSON array.
- `--json` on capture: Emits a structured JSON report including saved path, dimensions, format, file size, and clipboard status.

### 7. Multi-Monitor Selection & Delayed Capture
- `--list-monitors` (`-l`): Human-readable or JSON display enumeration.
- `--monitor` (`-m`): Capture a specific display by index (defaults to primary monitor).
- `--delay` (`-d`): Millisecond-level countdown timer before capture.

---

## 📊 Performance & Binary Size
- **Standalone Release Binary**: ~2.2 MB (fully optimized with LTO and symbol stripping).
- **Fast Startup & Clean Exit**: Zero background daemon overhead, zero memory leaks.

---

## 🛠 Breaking Changes & Migration from v1.x
- Binary name renamed from `wgccli.exe` to `rgc-cli`.
- Removed Windows-specific named pipe IPC server/client mode and D3D diagnostics in favor of standard cross-platform command-line execution and stdout streaming.
