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

## 📦 Installation & Setup

### 1. Automatic Installation via Scripts

#### 🐧 Linux & 🍎 macOS
You can install Fasty automatically by running the installer script:
```bash
curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.sh | bash
```

#### 🪟 Windows
Run the PowerShell installer script (no administrator privileges required):
```powershell
irm https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.ps1 | iex
```

---

### 2. Manual Installation from Release Archives

If you prefer to install Fasty manually, download the correct bundle for your platform from the [Releases page](https://github.com/diegoleteliers10/fasty/releases).

#### 🐧 Linux & 🍎 macOS (`.tar.gz`)
1. Download the archive for your architecture:
   - Linux: `fasty-x86_64-unknown-linux-gnu.tar.gz`
   - macOS (Intel): `fasty-x86_64-apple-darwin.tar.gz`
   - macOS (Apple Silicon): `fasty-aarch64-apple-darwin.tar.gz`
2. Extract the archive:
   ```bash
   tar -xzf fasty-*.tar.gz
   ```
3. Move the binary/app bundle to your installation directory:
   - On **Linux**:
     ```bash
     mkdir -p ~/.local/bin
     mv fasty ~/.local/bin/
     ```
   - On **macOS**:
     ```bash
     mv Fasty.app /Applications/
     ln -s /Applications/Fasty.app/Contents/MacOS/fasty /usr/local/bin/fasty
     ```

#### 🪟 Windows (`.zip`)
1. Download `fasty-x86_64-pc-windows-msvc.zip`.
2. Extract the `.zip` archive.
3. Move the extracted folder or copy `fasty.exe` to a folder in your path, for example:
   ```powershell
   Move-Item -Path .\fasty.exe -Destination "$env:USERPROFILE\.local\bin\fasty.exe" -Force
   ```
4. Ensure `$env:USERPROFILE\.local\bin` is added to your environment `PATH` variable.

---

### 3. Build & Setup from Source

If you want to compile Fasty from source:

#### Install System Dependencies

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

#### Build Fasty

```bash
# Debug profile
cargo build

# Release profile (with LTO and optimizations)
cargo build --release

# Build specifying a specific backend (Linux only)
cargo build --features wayland
cargo build --features x11
```

---

## 💻 Command Line Interface (CLI)

Fasty can be launched with several command-line flags to customize its startup behavior.

### CLI Options

| Option | Alias | Description |
| :--- | :--- | :--- |
| `-e` | `--command` | Spawns a specific command directly and auto-closes the window once it exits. |
| `-d` | `--working-dir` | Overrides the PTY startup working directory (e.g. `-d ~/projects`). |
| | `--title` | Sets a custom window title. |

### Usage Examples

```bash
# Open fasty with the default user shell
fasty

# Open htop directly, closing the window automatically when htop exits
fasty -e htop

# Open a specific file inside neovim
fasty -e nvim src/main.rs

# Start an ssh session
fasty -e ssh user@server

# Run a development server in a specific working directory
fasty -d ~/my-project -e bun run dev

# Open terminal in a specific directory with a custom window title
fasty --title "Dev Server" -d ~/my-project -e bun run dev

# Run compound command inside bash
fasty -e bash -c "cargo build && cargo test"
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
