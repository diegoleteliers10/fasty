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
- **Asynchronous Background Updater**: A non-blocking, automatic updater accessible via the topbar "Update" button. Selecting "Update" runs the installation script (`instalar.sh`/`instalar.ps1`) in a background thread, displaying "Updating...", and automatically launching the updated Fasty window and closing the old one on success.
- **About Context Menu**: Left-clicking the Fasty top-bar logo triggers the options context menu, providing quick access to the "About Fasty" panel.

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

## ⚡ Performance & Memory Optimizations

### Memory Footprint Reduction (30–50MB Linux, 50–70MB Windows)
Fasty has been optimized to run at a significantly lower memory footprint across all platforms:
1. **Scrollback Memory Cap**: The scrollback buffer is capped at a maximum of 3000 lines (down from 10,000), reducing allocations by ~37MB.
2. **GPU Texture Atlas Scaling**: Main and UI texture atlases are sized at `1536x1536` on Linux/macOS and `1024x1024` on Windows (down from `2048x2048`), freeing GPU and system memory.
3. **Logging Overhead Elimination**: Removed duplicate logging crates (`log`, `env_logger`) in favor of standard `tracing`. Tracing output is conditionalized to initialize only in debug configurations.
4. **Hardened Release Profile**: Recompiled with Link-Time Optimization (`lto = true`), unit splitting (`codegen-units = 1`), debug symbols stripping (`strip = true`), and panic unwinding disabled (`panic = "abort"`).

### Windows Memory & UX Hardening (v0.2.5)
v0.2.5 brings Windows to parity with the Linux/macOS footprint and eliminates two UX issues specific to the D3D12 backend:
1. **D3D12 Debug Layer Disabled in Release**: `wgpu::InstanceFlags` is now set via `from_build_config()` so the validation layer (which loads `d3d12sdklayers.dll` and spawns 3 helper processes — the source of the visible console flashes at startup) only runs in debug builds. **Cuts ~70MB of D3D12 debug-layer shadow copies.**
2. **Memory Hints on Windows**: `MemoryHints::MemoryUsage` (instead of `Performance`) is used on the D3D12 backend to prevent the driver from creating CPU-accessible staging copies of every GPU resource. **Cuts another ~20–50MB.**
3. **Atlas Sizing on Windows**: Atlases are 1024×1024 on Windows (1024² = 1M cells is more than enough for the basic + box-drawing + emoji set used). **Cuts ~10MB per renderer × up to 3 renderers (main + settings + about).**
4. **No-Console-Startup**: All Windows child process spawns (`curl` for update check, `reg` for font resolution, `cmd` for URL/file opening, `powershell` for update install, fasty relaunch on `Ctrl+Shift+N`) now set `creation_flags(0x08000000)` (`CREATE_NO_WINDOW`). The user no longer sees background terminal windows flash at startup or during user actions.
5. **No Dialog White Flash**: Settings and About dialogs now commit their first swapchain frame (`frame.present()`) before being made visible. On Windows D3D12, invisible HWNDs never receive `WM_PAINT`, so the previous approach of `set_visible(true)` inside `RedrawRequested` was unreachable and resulted in a white backbuffer being shown on the first user interaction.

**Result on Windows**: 200MB → **50–70MB** RAM, zero console flashes, zero dialog white flash.

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
  "scrollback": 3000
}
```

| Config Property | Type | Description |
| :--- | :--- | :--- |
| `font.family` | `string` | Name of the font family loaded via FontConfig / FreeType |
| `font.size` | `float` | Font size in logical points |
| `font.weight` | `float` | Numeric font weight value (e.g. `400.0` for Regular) |
| `font.ligatures` | `boolean` | Toggles rendering of font ligatures |
| `shell` | `string?` | Custom shell path (set to `null` to detect default system shell) |
| `scrollback` | `integer` | Lines of scrollback buffer history retained in memory (default 3000, capped at a maximum of 3000 for RAM efficiency) |

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

## 🗺️ Roadmap

Features under consideration for upcoming releases. Grouped by category and priority (**🔴 High** = high demand + low-medium effort, **🟡 Medium** = clear value but moderate effort, **🟢 Exploratory** = speculative, high effort or niche).

### 🖼️ Graphics & Terminal Protocols
| Priority | Feature | Notes |
| :--- | :--- | :--- |
| 🔴 | **OSC 8 Hyperlinks** | Make `\e]8;;url\e\\text\e]8;;\e\\` clickable inline (currently requires `Ctrl+hover`). Industry standard, used by `ls --color`, `gh`, modern CLI tools. |
| 🔴 | **Inline Image Protocol (iTerm2/Kitty)** | Render PNG/JPEG inline via the Kitty graphics protocol. Useful for `chafa`, `viu`, image previews in `yazi`/`ranger`. |
| 🟡 | **Sixel Graphics** | Legacy image protocol still used by some tools (`img2sixel`, `ls -6`). Optional, opt-in via config. |
| 🟢 | **Unicode 16 + Complex Shaping** | Better emoji ZWJ sequences, RTL text, Indic scripts. Current FreeType pipeline handles most cases; gaps remain. |

### ✂️ Productivity & Workflow
| Priority | Feature | Notes |
| :--- | :--- | :--- |
| 🔴 | **In-Scrollback Search** | `Ctrl+Shift+F` opens a search bar that highlights matches in the live + scrollback buffer. Should be GPU-accelerated. |
| 🔴 | **Command Palette** | `Ctrl+Shift+P` opens a fuzzy-search palette over settings, tab actions, themes. Inspired by VS Code/Sublime. |
| 🔴 | **Session Restore** | Persist open tabs, working directories, and PWD on shutdown; restore on next launch. Optional via config. |
| 🟡 | **Split Panes** | Horizontal/vertical splits per tab (like `tmux`/`Zellij`). Each split runs its own PTY. |
| 🟡 | **Copy on Select** | Mouse selection auto-copies to clipboard, like `tmux`/`kitty` (with a config toggle for the current `Ctrl+Shift+C` flow). |
| 🟡 | **Tab Reordering by Drag** | Currently read-only. Add drag-to-reorder + drag-to-detach (spawn new window). |
| 🟢 | **Quake-Mode / Drop-Down Terminal** | Global hotkey toggles a top-anchored sliding window. Implementation: fullscreen transparent window + slide-in offset animation. |
| 🟢 | **Shell Integration (Shell History, Last Command Markers)** | Mark command boundaries in scrollback, jump between them. Requires opt-in shell hooks (similar to `starship`/`fish`). |

### 🎨 Customization & Configuration
| Priority | Feature | Notes |
| :--- | :--- | :--- |
| 🔴 | **Themes (Color Schemes)** | First-class color palette support. Ship 4–6 built-in themes (Tomorrow Night, Solarized, One Dark, Catppuccin) + a `.theme.json` loader. |
| 🔴 | **TOML Config + Live Reload** | Replace the single `config.json` with a typed `fasty.toml` (sections: `[font]`, `[colors]`, `[keybindings]`, `[shell]`) that re-applies on save. |
| 🟡 | **Custom Keybindings** | User-rebindable shortcuts in `fasty.toml`. Required for split-pane UX. |
| 🟡 | **Per-Override Settings UI** | A visual theme/font picker, replacing the current text-number fields. |
| 🟢 | **Plugin System (Lua/WASM)** | Ghostty/WezTerm-style scripting. High effort, but unlocks third-party themes, status bars, integrations. |

### 🤖 Modern Integrations
| Priority | Feature | Notes |
| :--- | :--- | :--- |
| 🟡 | **AI Command Suggestions (opt-in)** | Local-only: pipe the last failed command to a small LLM (via Ollama / local API) for a fix suggestion. No telemetry. |
| 🟡 | **Inline Git Status in Topbar** | Render the current tab's repo branch + dirty status in the topbar. Hooks `git status` lazily (cached, 1s debounce). |
| 🟡 | **Built-in SSH Manager** | Quick `Ctrl+Shift+S` → "Connect to..." picker. Spawns `ssh user@host` inside a new tab. Replaces manual `fasty -e ssh user@host`. |
| 🟢 | **Remote / `fasty://` URL Scheme** | Register a protocol so `fasty://new-tab?cwd=...` opens a new tab from a browser link. |
| 🟢 | **Cloud Config Sync** | Optional encrypted sync of `fasty.toml` via a simple backend (or WebDAV). |

### ♿ Accessibility & Inclusion
| Priority | Feature | Notes |
| :--- | :--- | :--- |
| 🔴 | **Screen Reader Bridge (Windows UIA / Linux AT-SPI / macOS AX)** | Announce output to assistive tech. Biggest current gap. |
| 🟡 | **High-Contrast Theme** | WCAG AAA-compliant palette for users with low vision. |
| 🟡 | **Color-Blind Palettes** | Deuteranopia/protanopia-friendly variants. |
| 🟡 | **DPI Override Per-Monitor** | Already PerMonitorV2-aware; needs verified behavior on mixed-DPI multi-monitor setups. |
| 🟢 | **Touch / Gesture Input** | Long-press to select, two-finger scroll in scrollback. Targets 2-in-1 laptops. |

### ⚙️ Performance & Reliability
| Priority | Feature | Notes |
| :--- | :--- | :--- |
| 🔴 | **Scrollback-to-Disk** | Beyond 3000 lines, spill to a memory-mapped file. RAM-stays-low, infinite history. |
| 🟡 | **GPU-Accelerated Search** | Run the in-scrollback search as a compute shader on the GPU-stored glyph index. Sub-millisecond on 100k lines. |
| 🟡 | **Crash Reporting & Auto-Restart** | Optional opt-in crash dump upload (self-hosted) + automatic restart on panic. |
| 🟢 | **Wide-Gamut (P3) and HDR Output** | Detect HDR displays, emit 10-bit color where supported. Currently 8-bit sRGB only. |

### 🪟 Platform & Packaging
| Priority | Feature | Notes |
| :--- | :--- | :--- |
| 🔴 | **Signed MSIX / `.msi` Installer for Windows** | Replace the PowerShell installer. Microsoft's `makeappx` + a self-signed cert. |
| 🟡 | **Flatpak for Linux** | Sandbox-friendly distribution. |
| 🟡 | **Homebrew Formula Maintenance** | Already installable via the script; a real tap is overdue. |
| 🟢 | **Android (via `winit` + `wgpu` Mobile)** | The same codebase already builds for Android (the crates support it). Just need touch-friendly input and on-screen keyboard. |

> **Have a feature request?** Open an issue on GitHub. Anything that fits the "minimal, fast, GPU-native terminal" philosophy is welcome.

---

## 📚 Acknowledgements & Resources

- [wgpu](https://wgpu.rs) - Graphics framework for Rust.
- [Ghostty](https://github.com/ghostty-org/ghostty) - Inspiration for modern GPU terminal features.
- [alacritty_terminal](https://github.com/alacritty/alacritty) - ANSI parser and state wrapper.
- [portable-pty](https://docs.rs/portable-pty/latest/portable_pty/) - Cross-platform PTY manager.
- [The TTY demystified](http://www.linasakesson.net/programming/tty/) - Indispensable resource for terminal PTY structure.
