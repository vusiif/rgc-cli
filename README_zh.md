# rgc-cli (Rust Graphics Capture)

[![CI](https://github.com/vusiif/wgc-cli-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/vusiif/wgc-cli-rust/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Language: Rust](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/vusiif/wgc-cli-rust)
[![MCP Compatible](https://img.shields.io/badge/MCP-Compatible-brightgreen.svg)](https://modelcontextprotocol.io)

**[English](README.md)** | **[简体中文](README_zh.md)**

基于纯 Rust 构建的极速、轻量、跨平台屏幕截图命令行工具与 Model Context Protocol (MCP) 服务端。

> [!NOTE]
> `rgc-cli` 是原 `wgc-cli-rust` 项目的纯 Rust 跨平台演进版本。彻底移除对原生 C++ 胶水代码和本地构建工具链的依赖，在 Windows、macOS 和 Linux 上原生无缝运行。

---

## 🌟 核心特性

- **动态 WebP 录屏与 GIF 编码**: 支持长时录屏（`--record <SECS>`），原生编码为真彩色动态 WebP（`-o rec.webp`）或流式 GIF 动图，极大提升画质与压缩率，内存占用极低。
- **窗口工作区自动裁剪**: 支持 `--client-only`（配合 `-w, --window`），自动剔除 OS 阴影发光留白与系统标题栏，仅保留应用实际工作画布，并自适应重映射点击坐标。
- **纯正跨平台**: 原生支持 Windows、macOS 以及 Linux (X11 与 Wayland)，采用 100% 内存安全代码（`#![forbid(unsafe_code)]`）。
- **Model Context Protocol (MCP)**: 内置 `--mcp` stdio 服务端，遵循官方 MCP JSON-RPC 2.0 规范，开箱即用支持 Claude Desktop、Cursor、Cline、Roo Code 等 AI 编程助手。
- **现代图片格式 (PNG, JPEG, WebP)**: 内置纯 Rust WebP 原生编码，相较传统 PNG 通常节省约 **20%~35%** 文件体积，且兼顾像素级清晰度。
- **结构化 JSON 输出**: `--json` 标志将显示器列表、截图元数据及错误回显为机器可读的 JSON 信封结构，专为脚本自动化与 Agent 设计。
- **系统自检诊断**: 内置 `--doctor` 标志，快速检测当前操作系统图形显示子系统、DPI 缩放、底层抓屏后端及系统剪贴板就绪状态。
- **系统剪贴板直写**: 支持 `--clipboard` / `-c` 直接将截图写入系统剪贴板（基于 `arboard` 跨平台支持），无需落盘。
- **标准输出流式管道**: 支持将编码图像直接流式输出至标准输出 (`-o -`)，优雅处理 `BrokenPipe`，可无缝对接 `wl-copy`、`xclip`、ImageMagick、ffmpeg 或 AI 图像处理流。
- **High-DPI 缩放感知**: 严密支持用户逻辑坐标系与显示物理像素之间的转换与 `scale_factor` 计算，在 150% 缩放、4K 与 Retina 视网膜屏上精准对齐。
- **AI Agent 隐私脱敏与透明打孔**: 支持 `--mask <x,y,w,h[:effect]>` 与 `--mask-window <ID|TITLE[:effect]>`，提供 `transparent`（Alpha 镂空打孔）、`blackout`（黑块遮挡）、`pixelate`（马赛克）、`blur`（双线性下采样高斯模糊）四大脱敏特效。脱敏在下采样前执行，杜绝插值泄密。
- **视觉网格定位标尺**: 支持 `--grid [STEP]` 在截图上叠加半透明辅助网格线与交叉点紧凑坐标文字（内置纯 safe Rust 3x5 像素点阵字体），为多模态 AI Agent 提供绝对视觉坐标参考系，显著提升 GUI 交互点击精准度。
- **动态差异比对与增量截图**: 支持 `--diff-with <PATH>`、`--diff-threshold <0-255>` 与 `--crop-changed`，自动计算界面变动像素与最小外接包围盒 (`changed_bbox`)，支持仅裁剪并传输变动区域，节省 80%+ 视觉 Token 传输开销。
- **多显示器管理与全景拼接**: 列出并支持指定屏幕索引截图（`--list-monitors`, `--monitor <INDEX>`），并独家支持 `-a, --all-monitors` 将双屏、三屏虚拟桌面自动缝合成一张超宽全景大图，跨屏负坐标与异构布局自适应。
- **矩形区域裁剪**: 支持 `--region x,y,w,h` 裁剪，并在全屏抓取前提供快速越界预校验，节省无谓内存与 GPU 开销。
- **原子防碰撞命名**: 自动生成的截屏文件名采用毫秒级时间戳搭配原子单调递增序列号，杜绝快速连拍下的文件覆盖。
- **延时捕获**: 支持毫秒级定时延时捕获（`--delay <MS>`）。

---

## 📦 安装与编译

确保已安装 Rust 工具链（推荐 1.75+）：

```bash
cargo build --release
```

编译产物位于 `target/release/rgc-cli`（Windows 下为 `rgc-cli.exe`），在 LTO 与符号裁剪（strip）优化下，二进制体积仅约 **2~3 MB**。

---

## 🚀 使用指南

```text
用法: rgc-cli [选项]

选项:
      --doctor                运行系统诊断环境检查，查看屏幕捕获与剪贴板就绪状态
      --mcp                   启动 Model Context Protocol (MCP) stdio 服务端供 AI 助手对接
  -l, --list-monitors         列出所有可用显示器并退出
      --list-windows          列出所有活动的应用程序窗口并退出
  -m, --monitor <INDEX>       指定要捕获的显示器索引 (从 0 开始，默认为主显示器)
  -w, --window <ID|TITLE>     通过窗口 ID 或标题子串捕获特定窗口 (如 -w 1234 或 -w "Code")
      --client-only           仅截取应用程序客户区/工作画布 (自动剔除 OS 透明阴影留白与标题栏，需搭配 -w)
  -a, --all-monitors          捕获所有已连接显示器并自动拼接为虚拟桌面全景大图 (与 -m、-w 互斥)
  -r, --region <x,y,w,h>      指定截取矩形区域坐标: 'x,y,w,h' (例如 '100,100,800,600')
      --coords <MODE>         区域坐标系: 'logical' (默认，按 DPI 缩放) 或 'physical' (原始物理像素)
      --mask <x,y,w,h[:effect]> 敏感区域脱敏遮罩: 'x,y,w,h[:effect]' (effect: transparent, blackout, pixelate, blur; 默认: transparent)
      --mask-window <TARGET>  自动定位窗口并脱敏: 'ID|TITLE[:effect]' (effect: transparent, blackout, pixelate, blur; 默认: transparent)
  -o, --output <PATH>         输出文件路径、目录或 '-' (流式输出至 stdout)。默认为当前目录带时间戳文件
      --output-file <FILE>    显式指定输出文件目标 (若父目录不存在将自动创建)
      --output-dir <DIR>      显式指定输出目录目标 (在指定目录内生成防重时间戳文件)
  -c, --clipboard             将截图直接复制至系统剪贴板
  -d, --delay <MS>            截图前延时等待毫秒数 (例如 1000 表示延时 1 秒)
  -f, --format <FORMAT>       输出图片格式，缺省时自动根据扩展名推断 [可选: png, jpeg, webp, gif]
      --scale <FACTOR>        按比例缩放图像 (0.01 到 1.0 之间，例如 0.5 缩小一半)
      --max-dimension <PX>    限制图像最大宽度或高度 (像素)，保持原始纵横比
      --quality <1-100>       JPEG 输出压缩质量 (1-100，默认 85)
      --grid [STEP]           叠加视觉定位网格线与坐标十字锚点 (默认步长: 100 像素)
      --diff-with <PATH>      与指定的历史参考截图比对并计算变动外接矩形
      --diff-threshold <0-255> 像素比对差异容差阈值 (默认 16)
      --crop-changed          自动裁剪至发生变动的最小外接矩形 (需配合 --diff-with)
      --highlight-diff        在图像上绘制高对比度红框突出显示变动区域 (需配合 --diff-with)
      --timestamp             在输出截图右下角叠加印制时间戳水印徽标
      --tag <TEXT>            在输出截图右下角叠加印制自定义文本标签 (如步骤名称、会话ID)
      --wait-change           等待屏幕/窗口发生视觉变动后立即捕获 (适合 UI 自动化与点击响应确认)
      --wait-timeout <MS>     等待视觉变动的最大超时毫秒数 (默认 5000 毫秒)
      --wait-interval <MS>    变动检测轮询间隔毫秒数 (默认 100 毫秒)
      --cursor                捕获并在截图中渲染鼠标光标箭头
      --highlight-cursor      在鼠标光标周围渲染醒目的琥珀金色聚焦光环
      --record <SECS>         连续时间片抽样并录制指定秒数屏幕 (保存为动态 WebP、GIF 动图或图像序列帧)
      --fps <1-30>            连续录制采样帧率 FPS (1-30，默认 5 FPS)
      --som                   自动检测可交互 UI 控件并叠加 Set-of-Mark (SoM) 编号徽标
      --som-threshold <1-100> 控件边缘检测灵敏度阈值 (默认 25)
      --som-max-elements <N>  最大标注控件元素数量 (默认 50)
      --click-at <X,Y[,L]>    在截图标注点击波纹扩散环与目标靶心 (格式: X,Y 或 X,Y,标签)
      --drag <X1,Y1,X2,Y2[,L]> 在截图标注拖拽/手势轨迹虚线与导向箭头
      --verify-click <X,Y>    验证交互后指定坐标半径区域内是否发生真实界面视觉变动
      --verify-radius <PX>    交互效果验证检测半径 (默认 100 像素，0 为全屏)
      --verify-timeout <MS>   二次采样等待超时毫秒数 (默认 500 毫秒)
      --verify-threshold <N>  交互变动检测色差阈值 (1-255，默认 10)
      --open                  截图保存后自动调用系统默认查看器打开
      --completions <SHELL>   生成终端 Tab 自动补全脚本到 stdout [可选: bash, elvish, fish, powershell, zsh]
      --json                  以结构化 JSON 格式输出结果 (非常适合自动化与 AI 智能体)
  -v, --verbose               启用详细/调试日志
  -q, --quiet                 静默模式，抑制非错误信息
  -h, --help                  打印帮助信息
  -V, --version               打印版本信息
```

### 常用示例

#### 1. 直接复制到剪贴板（零磁盘写入）
```bash
rgc-cli -c
```

#### 2. 通过管道流式传输至其他工具
```bash
# 通过 Shell 重定向保存
rgc-cli -o - > screenshot.png

# Linux Wayland 剪贴板管道
rgc-cli -o - | wl-copy
```

#### 3. 保存为 WebP 并获取结构化 JSON 返回
```bash
rgc-cli -o shot.webp --json
```
输出：
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

#### 4. 存入指定目录（自动防碰撞时间戳命名）
```bash
rgc-cli --output-dir ./screenshots --json
```
输出：
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

#### 5. 以 JSON 格式查询所有显示器
```bash
rgc-cli --list-monitors --json
```
输出：
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

#### 6. 统一的结构化错误回显
在 `--json` 模式下发生错误时，将输出带退出码的机器可读诊断信息：
```bash
rgc-cli -m 999 --json
```
输出 (退出码 3)：
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

#### 7. 双重输出：保存文件的同时写入剪贴板
```bash
rgc-cli -c -o ./archive/
```

#### 8. 延时区域截图（感知 HiDPI 缩放）
```bash
rgc-cli -d 1500 -r 200,200,600,400 -o cropped.png
```

#### 9. 1:1 绝对物理像素裁剪
```bash
rgc-cli --coords physical -r 0,0,1920,1080 -o raw_pixels.png
```

#### 10. 图像尺寸缩放与质量控制（优化 AI Token 与传输带宽）
```bash
# 50% 等比下采样（例如 3840x2160 -> 1920x1080，体积缩减约 75%）
rgc-cli --scale 0.5 -o shot.webp

# 限制图片最长边不超过 1280px，保持原始纵横比
rgc-cli --max-dimension 1280 -o preview.png

# 指定 JPEG 压缩质量为 80
rgc-cli -f jpeg --quality 80 -o shot.jpg
```

#### 11. 截图后自动调用系统默认看图工具打开
```bash
# 截取并在系统默认照片/看图软件中即时打开
rgc-cli --open -o shot.png
```

#### 12. 生成终端 Tab 自动补全脚本
```bash
# 自动为 PowerShell 会话注入补全
rgc-cli --completions powershell | Out-String | Invoke-Expression

# 为 Linux/macOS Bash 或 Zsh 写入补全文件
rgc-cli --completions bash > /etc/bash_completion.d/rgc-cli
rgc-cli --completions zsh > "${fpath[1]}/_rgc-cli"
```

#### 13. 窗口级别精准捕获
```bash
# 枚举当前所有活动窗口（ID、进程名、标题、尺寸）
rgc-cli --list-windows

# 通过窗口标题（模糊匹配，忽略大小写）截取窗口
rgc-cli -w "Visual Studio Code" -o vscode.png

# 通过窗口数字 ID 截取窗口并在默认看图工具中打开
rgc-cli -w 197430 --open

# 窗口内局部坐标二次裁剪
rgc-cli -w "Chrome" -r 0,0,800,600 -o browser_top.png
```

#### 14. 隐私区域遮罩与透明打孔脱敏 (AI Agent 敏感信息防护)
```bash
# 1. 矩形区域透明打孔（默认特效：Alpha 通道置 0，生成透明洞）
rgc-cli --mask 100,100,200,200 -o safe.png

# 2. 多个区域不同特效组合（模糊、马赛克、纯黑遮挡、镂空打孔）
rgc-cli --mask 50,50,300,100:blur \
        --mask 400,200,150,50:blackout \
        --mask 600,600,200,200:pixelate \
        -o multi_masked.png

# 3. 自动识别窗口位置并脱敏（如全屏截图中将密码管理器或聊天窗口打孔或模糊）
rgc-cli --mask-window "1Password:transparent" \
        --mask-window "WeChat:blur" \
        -o desktop_desensitized.png

# 4. 配合 150% DPI 屏幕缩放自适应（默认 --coords logical 会自动乘以 1.5 倍）
rgc-cli --coords logical --mask 100,100,200,200 -o hidpi_safe.png

# 5. 脱敏与图片压缩下采样联动（先脱敏再缩放，杜绝插值泄露）
rgc-cli --mask 100,100,500,200:blur --scale 0.5 -o low_token_safe.webp
```

#### 15. 多显示器全景拼接截屏（解决 AI Agent 跨屏迷路）
```bash
# 自动抓取所有屏幕并按虚拟桌面相对坐标拼缝为超宽全景图
rgc-cli -a -o panorama.png

# 全景截屏配合下采样（将双 4K 拼接图等比缩小，节省 AI 视觉 Token）
rgc-cli -a --max-dimension 2048 -o pano_low_token.webp

# 全景截屏联动敏感窗口脱敏（全景大图中自动将 1Password 打孔）
rgc-cli -a --mask-window "1Password:transparent" -o pano_safe.png
```

#### 16. 视觉定位网格与坐标十字锚点 (AI Agent 点击精准定位)
```bash
# 叠加默认 100 像素步长的淡青色辅助网格与十字坐标刻度
rgc-cli --grid -o grid_100.png

# 指定 50 像素密集高精度网格
rgc-cli --grid 50 -o grid_50.png

# 全景截屏结合定位网格与下采样（供视觉 Agent 大图定位）
rgc-cli -a --grid 100 --max-dimension 2048 -o pano_grid.webp
```

#### 17. 屏幕动态差异比对与增量 Token 裁剪 (验证 Agent 交互响应)
```bash
# 与上一帧截图对比，输出变动像素数与 changed_bbox
rgc-cli --diff-with prev.png --json

# 仅裁剪并保存发生变动的局部小图（节约 80%+ 传输与多模态 Token）
rgc-cli --diff-with prev.png --crop-changed -o delta.png --json

# 自定义差异灵敏度阈值 (0-255，默认 16)
rgc-cli --diff-with prev.png --diff-threshold 32 --json
```

#### 18. 变动区域高亮红框突出与时间戳水印
```bash
# 在原图上绘制高对比度变动红框 (DIFF:WxH 标签)，一目了然确认操作变化
rgc-cli --diff-with prev.png --highlight-diff -o diff_highlighted.png

# 叠加 UTC/本地时间戳与任务步骤标签 (纯 safe Rust 3x5 点阵字体渲染，零外部字体依赖)
rgc-cli --timestamp --tag "STEP-LOGIN" -o tracked.png

# 综合联动：全景截屏 + 网格标尺 + 变动高亮 + 时间戳溯源
rgc-cli -a --grid 100 --diff-with baseline.png --highlight-diff --timestamp --tag "TASK_042" -o trace.png
```

#### 19. 响应式等待变动捕获 (告别脆弱的固定 sleep)
```bash
# 启动后监测屏幕，一旦 UI 响应渲染完毕（检测到像素色差变动）立即触发截屏（超时 5 秒）
rgc-cli --wait-change --wait-timeout 5000 -o after_click.png

# 等待窗口内容变动后立即仅裁剪发生变动的局部小图
rgc-cli -w "Calculator" --wait-change --crop-changed --diff-with prev.png -o calc_result.png
```

#### 20. 运行系统诊断检查
```bash
# 人类可读文本摘要
rgc-cli --doctor

# 结构化 JSON 诊断
rgc-cli --doctor --json
```
输出：
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

#### 21. 鼠标光标捕获与热区聚焦光环
```bash
# 捕获并在截图中渲染鼠标光标箭头
rgc-cli --cursor -o shot_with_cursor.png

# 在鼠标光标处绘制高对比度醒目聚焦光环 (极大提升教程与多屏 Agent 视觉定位效率)
rgc-cli --highlight-cursor -o shot_highlighted.png
```

#### 22. 连续时间片抽样与连拍录制动图
```bash
# 录制屏幕 3 秒 (10 FPS 采样) 并自动保存为平滑 GIF 动图
rgc-cli --record 3.0 --fps 10 -o recording.gif

# 针对指定应用窗口录制 5 秒 (5 FPS 采样)，带光标高亮并保存到文件夹为连续帧序列
rgc-cli -w "Code" --record 5.0 --fps 5 --cursor --output-dir ./record_frames/
```

#### 23. Set-of-Mark (SoM) 视觉控件自动检测与编号打标
```bash
# 自动识别界面按钮与输入框，并叠加编号角标 [1], [2], [3]
rgc-cli --som -o som_marked.png

# 输出打标截图并获取结构化元素坐标与点击中心点 JSON
rgc-cli --som --json -o som_marked.png
```
输出：
```json
{
  "ok": true,
  "data": {
    "status": "success",
    "path": "som_marked.png",
    "width": 1920,
    "height": 1080,
    "elements": [
      { "id": 1, "x": 120, "y": 45, "width": 80, "height": 32, "centerX": 160, "centerY": 61 },
      { "id": 2, "x": 220, "y": 45, "width": 100, "height": 32, "centerX": 270, "centerY": 61 }
    ]
  }
}
```

#### 24. 交互操作视觉轨迹叠加与局部响应验证
```bash
# 在指定坐标 (500,300) 绘制点击同心波纹与靶心标记
rgc-cli --click-at 500,300 -o click_trace.png

# 标注从 (100,200) 到 (400,500) 的拖拽滑动手势方向箭头与轨迹虚线
rgc-cli --drag 100,200,400,500,swipe_left -o drag_trace.png

# 验证点击 (500,300) 后其 100px 半径范围内是否真实产生界面响应
rgc-cli --verify-click 500,300 --verify-timeout 500 --json -o verified.png
```
输出：
```json
{
  "ok": true,
  "data": {
    "status": "success",
    "path": "verified.png",
    "actionVerification": {
      "verified": true,
      "targetX": 500,
      "targetY": 300,
      "radius": 100,
      "changedPixels": 420,
      "changePercent": 1.34,
      "threshold": 10,
      "bbox": { "x": 480, "y": 290, "width": 40, "height": 22 }
    }
  }
}
```

---

## 🤖 Model Context Protocol (MCP) 智能体集成

`rgc-cli` 内置标准 MCP stdio 服务端，支持 JSON-RPC 2.0 协议。可直接集成到 Claude Desktop、Cursor、Cline、Roo Code、Windsurf 等 AI 智能体中，支持截图、录像、视觉变动主动推送与环境自检。

### 提供的 MCP 工具列表

1. **`list_monitors`**: 枚举所有屏幕、分辨率与 DPI 缩放系数。
2. **`list_windows`**: 枚举所有活动应用窗口的标题、ID 及坐标尺寸。
3. **`capture_screen`**: 捕获指定屏幕、窗口、全景多屏或局部区域，支持直接返回 Base64 编码图片数据（`image/png`, `image/webp`, `image/jpeg`, `image/gif`）或落盘至本地文件。
   - 参数：`monitor` (整数/字符串), `window` (窗口 ID/标题), `allMonitors` (布尔值，多屏全景拼接), `region` (字符串 `"x,y,w,h"`), `coords` (`"logical"` | `"physical"`), `format` (`"png"` | `"jpeg"` | `"webp"` | `"gif"`), `scale` (浮点数 `0.01..1.0`), `maxDimension` (最大像素 `>0`), `quality` (压缩质量 `1..100`), `grid` (网格步长整数), `diffWith` (历史图像路径或 Base64 字符串), `diffThreshold` (像素容差阈值 0-255), `cropChanged` (布尔值), `highlightDiff` (布尔值), `timestamp` (布尔值), `tag` (字符串), `waitChange` (布尔值), `waitTimeoutMs` (整数), `waitIntervalMs` (整数), `cursor` (布尔值), `highlightCursor` (布尔值), `som` (布尔值), `somThreshold` (整数), `somMaxElements` (整数), `clickAt` (字符串或数组), `drag` (字符串或数组), `redactions` (数组), `maskWindows` (数组), `outputPath` (字符串), `returnBase64` (布尔值)。
4. **`record_screen`**: 连续录制屏幕、窗口或全景多屏指定秒数，编码为动画 GIF 或连续序列帧。
   - 参数：`durationSecs` (浮点数，必填), `fps` (整数 1..30，默认 5), `monitor` (整数), `window` (字符串), `allMonitors` (布尔值), `cursor` (布尔值), `highlightCursor` (布尔值), `scale` (浮点数), `maxDimension` (整数), `outputPath` (字符串), `returnBase64` (布尔值)。
5. **`verify_action`**: 采样交互前后的界面状态，局部校验 (x, y) 半径范围内是否发生真实的界面变动。返回验证结果、变动像素统计、外接矩形以及带点击波纹标注的截图。
   - 参数：`x` (整数，必填), `y` (整数，必填), `radius` (整数，默认 100), `timeoutMs` (整数，默认 500), `threshold` (整数，默认 10), `monitor` (整数), `window` (字符串), `outputPath` (字符串), `returnBase64` (布尔值)。
6. **`doctor`**: 诊断检查当前图形显示子系统与捕获就绪状态。

### 提供的 MCP 资源 (Resources) 与变动主动订阅

`rgc-cli` 全面支持 MCP 资源主动订阅 (`resources/subscribe` & `resources/unsubscribe`) 与实时更新推送 (`notifications/resources/updated`)：

1. **`screen://current`**: 当前主显示器画面的实时快照（`image/png` Base64 二进制资源）。
2. **`screen://all`**: 所有活动显示器自动拼接的超宽全景快照（`image/png` Base64 二进制资源）。
3. **`screen://monitor/{index}`**: 动态捕获指定索引显示器的实时画面（如 `screen://monitor/0`）。
4. **`screen://window/{id}`**: 动态捕获指定窗口数字 ID 的实时画面（如 `screen://window/1234`）。
5. **`screen://diagnostics`**: 图形与捕获子系统环境就绪诊断报告（JSON 文本资源）。

### 客户端接入配置示例

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

## 🚦 退出码与错误状态定义

| 退出码 | 错误码标识 | 阶段 (stage) | 说明 |
|:---:|:---|:---|:---|
| **0** | `SUCCESS` | `complete` | 操作成功执行 |
| **1** | `GENERIC_ERROR` | `execution` | 未分类的运行时错误 |
| **2** | `BAD_ARGS` | `cli_parsing` | 命令行参数格式不合法或互斥冲突 |
| **3** | `NO_MONITOR` / `MONITOR_NOT_FOUND` | `monitor_selection` | 未检测到显示器或指定的显示器索引超出范围 |
| **4** | `REGION_OUT_OF_BOUNDS` | `region_calculation` | 裁剪区域坐标超出了目标显示器边界 |
| **5** | `CAPTURE_FAILED` | `capture` | 操作系统捕获后端抓屏失败 |
| **6** | `FORMAT_MISMATCH` | `format_inference` | 显式 `--format` 格式与文件扩展名冲突 |
| **7** | `IO_ERROR` / `ENCODE_FAILED` | `output_save` | 磁盘写入失败、权限不足或编码失败 |
| **8** | `CLIPBOARD_ERROR` | `clipboard_export` | 系统剪贴板不可用或写入失败 |

---

## 📐 坐标系与 DPI 规范说明

- **坐标原点 `(0, 0)`**: 严格限定为目标显示器（默认为主显示器）的左上角。不支持负数虚拟桌面坐标混淆。
- **`--coords logical` (默认)**: 用户传入的数值视为逻辑坐标，将根据显示器 `scale_factor`（如 1.25x, 1.5x, 2.0x）进行防溢出四舍五入计算。
- **`--coords physical`**: 用户传入数值视为原始物理像素，1:1 精确映射，不进行任何 DPI 换算。
- **安全性保证**: 两种模式均会在调用昂贵的硬件截图前进行严格的边界预校验。

---

## 🏛 模块架构

- [`src/main.rs`](src/main.rs): 统一命令行入口、集中式错误分发与顶层流程管线。
- [`src/cli.rs`](src/cli.rs): 基于 `clap` derive 的类型安全参数解析，包含自定义 `Region` 解析与互斥冲突矩阵。
- [`src/api.rs`](src/api.rs): 公共 API 错误码、统一响应数据信封与结构化模型定义。
- [`src/capture/`](src/capture/): 跨平台底层抓屏与坐标换算模块包：
  - `types.rs`: 抓屏错误与实体数据结构。
  - `crop.rs`: 溢出安全与 DPI 缩放矩阵适配的裁剪换算。
  - `redaction.rs`: 敏感区域脱敏与相交遮罩坐标解算。
  - `screen.rs`: 显示器探测枚举与单屏截取。
  - `window.rs`: 窗口枚举、标题模糊匹配与窗口精准捕获。
  - `panorama.rs`: 多显示器全景画布拼接与负坐标排版。
  - `mod.rs`: 模块统一向外兼容导出。
- [`src/transform.rs`](src/transform.rs): 纯图像处理流水线（脱敏特效、3x5 点阵像素渲染、网格标尺、逐像素色差比对、变动框高亮与水印徽标）。
- [`src/output.rs`](src/output.rs): 跨平台剪贴板直写、stdout 流式管道、原子毫秒级防重命名与格式编码持久化。
- [`src/doctor.rs`](src/doctor.rs): 运行环境与硬件诊断、图形子系统就绪检测与报告构建。
- [`src/mcp/`](src/mcp/): 模块化 Model Context Protocol 服务端：
  - `types.rs`: JSON-RPC 2.0 基础协议与 MCP 实体模型。
  - `base64.rs`: 纯 Safe Rust 高性能 Base64 编解码器与 RFC 向量测试。
  - `resources.rs`: MCP 资源 URI 读取与 Base64 封装。
  - `tools.rs`: 工具 JSON Schema 声明与调度执行。
  - `prompts.rs`: MCP Prompts 协议与开箱即用 Agent 工作流模板。
  - `mod.rs`: Stdio 消息路由分发主循环与集成测试。

---

## 🧪 自动化测试

运行纯逻辑单元测试（适用于无桌面、无剪贴板的 CI 环境）：

```bash
cargo test
```

运行依赖真实桌面与剪贴板的硬件测试：

```bash
cargo test -- --ignored
```

---

## 📄 开源许可证

本项目基于 [Apache License, Version 2.0](LICENSE) 开源。
