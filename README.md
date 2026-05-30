# Fasty

GPU-accelerated terminal emulator built with Rust, `winit`, and `wgpu`.

Inspired by [Ghostty](https://github.com/ghostty-org/ghostty), Fasty leverages `wgpu` (the modern, cross-platform GPU API powering GPUI/Zed) for rendering instead of raw Vulkan/Metal implementations, presenting a minimal UI footprint with ultra-high performance.

---

## 🚀 Key Features

- **GPU-Accelerated Rendering**: Built on top of `wgpu` (targeting Vulkan, Metal, DX12, GLES) and WGSL shaders for low-latency cell grid updates.
- **Modern SVG Vector Icons**: Replaced default unicode/font glyphs in context menus, topbar window controls, tabs, and settings panels with high-contrast vector icons loaded from `assets/icons/` via `resvg`, `usvg`, and `tiny-skia`.
- **Ultra-Low CPU Idle Footprint**: Achieves `<1%` CPU consumption at idle by utilizing `winit`'s `ControlFlow::Wait` mode, employing a blocking PTY read loop, and implementing a micro-optimized cursor blink GPU fast-path.
- **Animated Scrollbar Fading**: Smoothly fades the scrollbar out (using dynamic alpha opacity lerping) when running a text user interface (TUI) that owns mouse reporting or active alternate screen buffers, fading it back in instantly upon exit.
- **Seamless TUI Mouse Integration**: Perfect mouse clicks and drag selections passed directly to the terminal PTY (such as inside Claude Code, htop, or vim) without interfering with desktop text selection.
- **Tabbed Layout**: Support for multiple independent tabs, each running a native shell process.
- **Configurable Settings Panel**: Adjust font family, font size, and scrollback capacity on-the-fly.

---

## 🎨 SVG Icon UI System

Fasty integrates custom vector icons mapped into the GPU texture atlas as non-color stencils. This guarantees sharp rendering at all DPI scales without reliance on system-installed icon fonts.

| UI Component | Action / Function | SVG Icon Asset | Render Size |
| :--- | :--- | :--- | :--- |
| **Topbar** | Close Window | `close.svg` | `14x14 px` |
| | Maximize Window | `maximize.svg` | `14x14 px` |
| | Minimize Window | `less.svg` | `14x14 px` |
| | Open Settings Panel | `settings.svg` | `16x16 px` |
| **Tabs Bar** | Add New Tab | `add.svg` | `16x16 px` |
| | Close Tab | `close.svg` | `12x12 px` |
| **Settings Panel** | Close Settings | `close.svg` | `14x14 px` |
| | Font Family Selector | `text-font.svg` | `14x14 px` |
| | Increase/Decrease Font | `add.svg` / `less.svg` | `14x14 px` |
| | Increase/Decrease Scrollback | `add.svg` / `less.svg` | `14x14 px` |
| **Context Menu** | Copy Text | `copy.svg` | `14x14 px` |
| | Paste Text | `paste.svg` | `14x14 px` |
| | Open New Tab | `add.svg` | `14x14 px` |
| | Close Tab | `close.svg` | `14x14 px` |

> [!TIP]
> Icons like `add.svg` and `less.svg` are custom-drawn with a stroke thickness of `2.5` to ensure subpixel visibility and high contrast at tiny render dimensions.

---

## ⚡ Performance Optimizations

### Micro-Optimized Cursor Blinking
Instead of triggering a full layout and rebuilding cell instances across the grid when the cursor blinks (which consumed ~8% CPU), Fasty implements a fast-path renderer:
1. An enum `RenderReason` defines the draw request:
   ```rust
   pub enum RenderReason {
       CursorBlink, // Redraw only the cursor quad
       GridChanged, // Rebuild and redraw the entire screen
   }
   ```
2. When performing a `CursorBlink`, Fasty updates only the transparency byte of the cursor quad in `cached_final_instances`.
3. It performs a single, minimal 48-byte buffer write (`queue.write_buffer`) directly to the GPU's instance buffer.
4. It submits the command encoder immediately with cached draw count values, skipping cell iterations, Atlas dirty checks, and font rasterization.

### OS-Level Sleeping
- **Event Loop**: By default, the winit event loop rests in `Wait` state until a window event, keypress, or PTY output occurs.
- **PTY Reader Thread**: The reader thread blocks at the OS kernel level on read calls from the PTY master, generating `0.0%` CPU wakeups when the shell is idle.

---

## 📂 Codebase Architecture

```
src/
├── main.rs            # Entry point, event loop, tab manager, UI state
├── terminal_state.rs  # PTY controller & alacritty_terminal parser wrapper
├── renderer/          # wgpu backend components
│   ├── mod.rs         # Renderer definitions, render passes
│   ├── pipeline.rs    # Cell instance drawing, UI layouts, cursor fast-path
│   └── atlas.rs       # Dynamic Glyph and SVG Stencil GPU texture cache
├── config.rs         # Config validation and local JSON reading
└── event_listener.rs # PTY write proxy
```

---

## 🛠️ Build & Setup

### 1. Install System Dependencies

* **macOS**:
  ```bash
  xcode-select --install
  ```
* **Linux (Wayland)**:
  ```bash
  sudo apt install libvulkan-dev libwayland-dev
  ```
* **Linux (X11)**:
  ```bash
  sudo apt install libvulkan-dev libx11-dev
  ```
* **Windows**:
  Install the [Vulkan SDK](https://vulkan.lunarg.com/).

### 2. Build

```bash
# Debug profile
cargo build

# Release profile (with LTO and optimizations)
cargo build --release

# Build specifying a specific backend (Linux only)
cargo build --features wayland
cargo build --features x11
```

### 3. Run

```bash
cargo run
```

---

## ⚙️ Configuration

Fasty loads a file named `config.json` situated in the binary's execution directory. If not present, defaults are applied. On startup, Fasty initializes window dimensions at a default logical footprint of **`800 x 520`** pixels.

### Configuration Template
```json
{
  "font": {
    "family": "JetBrains Mono",
    "size": 14.0,
    "weight": 400.0,
    "ligatures": true
  },
  "shell": null,
  "scrollback": 10000
}
```

| Config Property | Type | Description |
| :--- | :--- | :--- |
| `font.family` | `string` | Name of the font family loaded via FontConfig / FreeType |
| `font.size` | `float` | Font size in logical points |
| `font.weight` | `float` | Numeric font weight value (e.g. `400.0` for Regular) |
| `font.ligatures` | `boolean` | Toggles rendering of font ligatures |
| `shell` | `string?` | Custom shell path (set to `null` to detect default system shell) |
| `scrollback` | `integer` | Lines of scrollback buffer history retained in memory |

---

## ⌨️ Keyboard Shortcuts

| Shortcut Key | Action Performed |
| :--- | :--- |
| **`Ctrl + Shift + T`** | Open a new tab |
| **`Ctrl + Shift + W`** | Close current tab |
| **`Ctrl + Shift + L`** | Snap scroll position to the bottom of the buffer |
| **`Ctrl + C`** / **`Ctrl + Shift + C`** | Copy current mouse text selection |
| **`Ctrl + V`** / **`Ctrl + Shift + V`** | Paste clipboard contents to PTY |
| **`Left Mouse Drag`** | Highlight text |
| **`Ctrl + Left Click`** | Open highlighted URL in default browser |

---

## 📚 Acknowledgements & Resources

- [wgpu](https://wgpu.rs) - Graphics framework for Rust.
- [Ghostty](https://github.com/ghostty-org/ghostty) - Inspiration for modern GPU terminal features.
- [alacritty_terminal](https://github.com/alacritty/alacritty) - ANSI parser and state wrapper.
- [portable-pty](https://docs.rs/portable-pty/latest/portable_pty/) - Cross-platform PTY manager.
- [The TTY demystified](http://www.linasakesson.net/programming/tty/) - Indispensable resource for terminal PTY structure.
