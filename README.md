# Fasty

GPU-accelerated terminal emulator built with Rust, `winit`, and `wgpu`.

Inspired by [Ghostty](https://github.com/ghostty-org/ghostty), Fasty leverages `wgpu` (the modern, cross-platform GPU API powering GPUI/Zed) for rendering instead of raw Vulkan/Metal implementations, presenting a minimal UI footprint with ultra-high performance.

---

## Key Features

- **GPU-Accelerated Rendering**: Built on `wgpu` (Vulkan, Metal, DX12, GLES) and WGSL shaders for low-latency cell grid updates.
- **Modern SVG Vector Icons**: High-contrast vector icons loaded from `assets/icons/` via `resvg`, `usvg`, and `tiny-skia` — replaces default unicode/font glyphs in context menus, topbar, tabs, and settings.
- **Ultra-Low CPU Idle Footprint**: `<1%` CPU at idle via `winit`'s `ControlFlow::Wait`, blocking PTY read loop, and micro-optimized cursor blink GPU fast-path.
- **Animated Scrollbar Fading**: Smoothly fades the scrollbar out during TUI mouse reporting or alternate screen buffers, fades back in instantly on exit.
- **Seamless TUI Mouse Integration**: Perfect mouse clicks and drag selections passed directly to the PTY (htop, vim, Claude Code) without interfering with desktop text selection.
- **Tabbed Layout**: Multiple independent tabs, each running a native shell process.
- **Live-Apply Settings**: Font family, size (1pt step), scrollback, and theme applied instantly — no Save/Cancel buttons.
- **Built-in Color Themes**: `default` (Fasty), `catppuccin`, `one-dark`, `solarized-dark`. Switches apply live across main window, settings, and about dialogs.
- **Custom Keybindings**: User-rebindable shortcuts via `[keybindings]` in `fasty.toml`. 13 actions available; defaults preserved when omitted.
- **Session Restore**: Saves open tab working directories on exit, restores to saved paths on next launch. Enabled by default; opt-out via `session_restore = false`.
- **Command Palette**: `Ctrl+Shift+P` opens a fuzzy-search palette for quick access to settings, tab actions, themes, and font size controls.
- **TOML Config + Live Reload**: `fasty.toml` edits re-apply on save — no restart needed. Comments and formatting preserved across Settings-dialog writes via `toml_edit` round-tripping.
- **Asynchronous Background Updater**: Non-blocking update check on startup with one-click install from the topbar "Update" button. Restarts automatically on success.

---

## SVG Icon UI System

Fasty integrates custom vector icons mapped into the GPU texture atlas as non-color stencils. Sharp rendering at all DPI scales without system-installed icon fonts.

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

> Icons like `add.svg` and `less.svg` are custom-drawn with a stroke thickness of `2.5` to ensure subpixel visibility and high contrast at tiny render dimensions.

---

## Performance & Memory Optimizations

### Memory Footprint Reduction (30-50MB Linux, 50-70MB Windows)

1. **Scrollback Memory Cap**: Capped at 3000 lines (down from 10,000), reducing allocations by ~37MB.
2. **GPU Texture Atlas Scaling**: Main and UI texture atlases sized at `1536x1536` on Linux/macOS and `1024x1024` on Windows (down from `2048x2048`).
3. **Logging Overhead Elimination**: Removed duplicate logging crates in favor of standard `tracing`. Output conditionalized to debug configurations only.
4. **Hardened Release Profile**: LTO (`lto = true`), unit splitting (`codegen-units = 1`), debug symbols stripped (`strip = true`), panic unwinding disabled (`panic = "abort"`).

### Windows Memory & UX Hardening (v0.2.5)

1. **D3D12 Debug Layer Disabled in Release**: `wgpu::InstanceFlags::from_build_config()` skips the validation layer. Cuts ~70MB of D3D12 debug-layer shadow copies.
2. **MemoryHints::MemoryUsage**: Prevents the D3D12 driver from creating CPU-accessible staging copies. Cuts ~20-50MB.
3. **Atlas Sizing on Windows**: 1024x1024 atlases (sufficient for basic + box-drawing + emoji set). Cuts ~10MB per renderer.
4. **No-Console-Startup**: All Windows child process spawns use `creation_flags(0x08000000)` (`CREATE_NO_WINDOW`). Zero console flashes.
5. **No Dialog White Flash**: Settings and About dialogs commit first swapchain frame before `set_visible(true)`.

**Result on Windows**: 200MB -> 50-70MB RAM, zero console flashes, zero dialog white flash.

### Micro-Optimized Cursor Blinking

Instead of full layout rebuild on cursor blink (~8% CPU), Fasty implements a fast-path renderer:
1. `RenderReason::CursorBlink` updates only the transparency byte of the cursor quad in `cached_final_instances`.
2. Single 48-byte buffer write (`queue.write_buffer`) directly to GPU instance buffer.
3. Command encoder submitted immediately with cached draw count values — skips cell iterations, atlas dirty checks, and font rasterization.

### OS-Level Sleeping

- **Event Loop**: Rests in `Wait` state until window event, keypress, or PTY output.
- **PTY Reader Thread**: Blocks at OS kernel level on read calls, generating `0.0%` CPU wakeups when idle.

---

## Codebase Architecture

```
src/
├── main.rs            # Entry point, event loop, tab manager, UI state
├── terminal_state.rs  # PTY controller & alacritty_terminal parser wrapper
├── keybindings.rs     # Key combo parser, action resolver, user overrides
├── session.rs         # Tab cwd persistence (save/restore)
├── renderer/          # wgpu backend components
│   ├── mod.rs         # Renderer definitions, render passes
│   ├── pipeline.rs    # Cell instance drawing, UI layouts, cursor fast-path
│   └── atlas.rs       # Dynamic Glyph and SVG Stencil GPU texture cache
├── config.rs          # TOML config, live-reload watcher, atomic save
└── event_listener.rs  # PTY write proxy
```

---

## Installation & Setup

### Automatic Installation via Scripts

#### Linux & macOS
```bash
curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.sh | bash
```

#### Windows
```powershell
irm https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.ps1 | iex
```

### Manual Installation from Release Archives

Download from the [Releases page](https://github.com/diegoleteliers10/fasty/releases).

#### Linux & macOS (`.tar.gz`)
1. Download the archive for your architecture:
   - Linux: `fasty-x86_64-unknown-linux-gnu.tar.gz`
   - macOS (Intel): `fasty-x86_64-apple-darwin.tar.gz`
   - macOS (Apple Silicon): `fasty-aarch64-apple-darwin.tar.gz`
2. Extract: `tar -xzf fasty-*.tar.gz`
3. Install:
   - **Linux**:
     ```bash
     mkdir -p ~/.local/bin
     mv fasty ~/.local/bin/
     ```
   - **macOS**:
     ```bash
     mv Fasty.app /Applications/
     ln -s /Applications/Fasty.app/Contents/MacOS/fasty /usr/local/bin/fasty
     ```

#### Windows (`.zip`)
1. Download `fasty-x86_64-pc-windows-msvc.zip`.
2. Extract the archive.
3. Move `fasty.exe` to a folder in your path:
   ```powershell
   Move-Item -Path .\fasty.exe -Destination "$env:USERPROFILE\.local\bin\fasty.exe" -Force
   ```
4. Ensure `$env:USERPROFILE\.local\bin` is in your `PATH`.

### Build from Source

#### Install System Dependencies

- **macOS**: `xcode-select --install`
- **Linux (Wayland)**: `sudo apt install libvulkan-dev libwayland-dev`
- **Linux (X11)**: `sudo apt install libvulkan-dev libx11-dev`
- **Windows**: Install the [Vulkan SDK](https://vulkan.lunarg.com/).

#### Build

```bash
cargo build              # Debug profile
cargo build --release    # Release profile (LTO + optimizations)

# Linux backend selection
cargo build --features wayland
cargo build --features x11
```

---

## Command Line Interface

| Option | Alias | Description |
| :--- | :--- | :--- |
| `-e` | `--command` | Spawn a specific command and auto-close on exit |
| `-d` | `--working-dir` | Override the PTY startup working directory |
| | `--title` | Set a custom window title |

```bash
fasty                                    # Default shell
fasty -e htop                            # Run htop, auto-close on exit
fasty -e nvim src/main.rs                # Open file in neovim
fasty -e ssh user@server                 # SSH session
fasty -d ~/my-project -e bun run dev    # Dev server in specific directory
fasty --title "Dev Server" -d ~/my-project -e bun run dev
fasty -e bash -c "cargo build && cargo test"
```

---

## Configuration

Fasty reads `fasty.toml` from the first existing path:

1. `./fasty.toml` (current working directory -- portable mode)
2. `/etc/fasty/fasty.toml` (system-wide)
3. `~/.config/fasty/fasty.toml` (user -- default)

If no file is found, defaults are applied. On startup, window dimensions default to **800 x 520** pixels.

**Live reload (v0.2.8+):** Edits to `fasty.toml` re-apply on save. Settings changed via the Settings dialog persist to this same file. Comments and formatting are preserved via `toml_edit` round-tripping.

**Live-reload exceptions:**
- `font.ligatures`: requires restart (atlas-level cache rebuild).
- `shell`: applies only to newly spawned tabs.

**Migration from v0.2.7 or earlier:** On first launch, v0.2.8 auto-converts `~/.config/fasty/config.json` into `~/.config/fasty/fasty.toml` and renames the original to `config.json.bak`.

### Configuration Template

```toml
shell = "/bin/bash"        # optional; omit to detect system default
scrollback = 3000
theme = "default"
session_restore = true     # restore last tabs on launch

[font]
family = "JetBrains Mono"
size = 14.0
weight = 400.0
ligatures = true

[keybindings]
# All bindings are optional. Omitted keys use defaults.
# Example overrides:
# ctrl+shift+t = "new_tab"
# ctrl+shift+w = "close_tab"
# ctrl+shift+p = "command_palette"
```

### Config Properties

| Property | Type | Description |
| :--- | :--- | :--- |
| `font.family` | `string` | Font family name loaded via FontConfig / FreeType |
| `font.size` | `float` | Font size in logical points |
| `font.weight` | `float` | Numeric font weight (e.g. `400.0` for Regular) |
| `font.ligatures` | `boolean` | Toggle font ligatures (restart required) |
| `shell` | `string?` | Custom shell path; omit to detect system default |
| `scrollback` | `integer` | Lines of scrollback buffer (default 3000, capped at 3000) |
| `theme` | `string` | Color scheme: `default`, `catppuccin`, `one-dark`, `solarized-dark`, or custom name |
| `session_restore` | `boolean` | Restore previously open tabs on launch (default: `true`) |

### Keybindings

The `[keybindings]` section maps key combinations to actions. All bindings are optional; omitted keys use defaults.

**Available actions:**

| Action | Default Binding | Description |
| :--- | :--- | :--- |
| `new_tab` | `ctrl+shift+t` | Open a new tab |
| `close_tab` | `ctrl+shift+w` | Close current tab |
| `new_window` | `ctrl+shift+n` | Open a new window |
| `copy` | `ctrl+shift+c` | Copy selection to clipboard |
| `paste` | `ctrl+shift+v` | Paste from clipboard |
| `open_search` | `ctrl+shift+f` | Open search bar |
| `open_settings` | `ctrl+shift+s` | Open settings dialog |
| `reload_config` | `ctrl+shift+r` | Reload configuration |
| `command_palette` | `ctrl+shift+p` | Open command palette |
| `increase_font_size` | `ctrl+equal` / `ctrl+plus` | Increase font size |
| `decrease_font_size` | `ctrl+minus` | Decrease font size |
| `reset_font_size` | `ctrl+0` | Reset font size |
| `next_tab` | `ctrl+tab` | Switch to next tab |
| `prev_tab` | `ctrl+shift+tab` | Switch to previous tab |
| `select_tab_N` | `alt+N` (1-9) | Switch to tab N |

### Built-in Themes

| Theme | Background | Foreground | Notes |
| :--- | :--- | :--- | :--- |
| `default` | `#0C0C0C` | `#C5C8C6` | Fasty -- original terminal bg, Tomorrow Night palette |
| `catppuccin` | `#24273A` | `#CAD3F5` | Soft pastel, easy on the eyes |
| `one-dark` | `#282C34` | `#ABB2BF` | Atom One Dark |
| `solarized-dark` | `#002B36` | `#839496` | Classic Solarized dark |

Theme changes take effect immediately on the live terminal. Each cell instance looks up its named color through `named_color_rgb()` / `index_to_ansi_color()` which dispatch to the active theme.

**Themed surfaces** (all re-render on theme change): main window bg, topbar, active tab fill, scrollback area, settings dialog, about dialog, context menu.

### Custom Themes

Drop a `.json` file into `~/.config/fasty/themes/` (filename minus `.json` becomes the theme name). All 18 fields are optional except `background` and `foreground`; missing ANSI colors fall back to `foreground`.

```json
{
  "background": "#1a1b26",
  "foreground": "#c0caf5",
  "black":   "#15161e",
  "red":     "#f7768e",
  "green":   "#9ece6a",
  "yellow":  "#e0af68",
  "blue":    "#7aa2f7",
  "magenta": "#bb9af7",
  "cyan":    "#7dcfff",
  "white":   "#a9b1d6",
  "bright_black":   "#414868",
  "bright_red":     "#f7768e",
  "bright_green":   "#9ece6a",
  "bright_yellow":  "#e0af68",
  "bright_blue":    "#7aa2f7",
  "bright_magenta": "#bb9af7",
  "bright_cyan":    "#7dcfff",
  "bright_white":   "#c0caf5"
}
```

---

## Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Ctrl + Shift + T` | Open a new tab |
| `Ctrl + Shift + W` | Close current tab |
| `Ctrl + Shift + P` | Open command palette |
| `Ctrl + Shift + L` | Snap scroll to bottom |
| `Ctrl + C` / `Ctrl + Shift + C` | Copy selection |
| `Ctrl + V` / `Ctrl + Shift + V` | Paste clipboard |
| `Ctrl + Left Click` | Open URL in browser |
| `Left Mouse Drag` | Highlight text |

---

## Roadmap

Features under consideration for upcoming releases:
- **High** = high demand + low-medium effort
- **Medium** = clear value but moderate effort
- **Exploratory** = speculative, high effort or niche

`[x]` = implemented  |  `[ ]` = planned

### Graphics & Terminal Protocols
- [x] **OSC 8 Hyperlinks** -- Clickable inline hyperlinks via OSC 8 escape sequences.
- [ ] **Inline Image Protocol (iTerm2/Kitty)** -- Render PNG/JPEG inline via the Kitty graphics protocol.
- [ ] **Sixel Graphics** -- Legacy image protocol for `img2sixel`, `ls -6`.
- [ ] **Unicode 16 + Complex Shaping** -- Better emoji ZWJ sequences, RTL text, Indic scripts.
- [x] **OpenType Font Ligatures** -- Configurable via `font.ligatures`; rendered via `rustybuzz` shaping.

### Productivity & Workflow
- [x] **In-Scrollback Search** -- `Ctrl+Shift+F` opens a search bar highlighting matches in live + scrollback buffer.
- [x] **Command Palette** -- `Ctrl+Shift+P` opens a fuzzy-search palette over settings, tab actions, themes.
- [x] **Session Restore** -- Persist open tab working directories on shutdown; restore on next launch.
- [ ] **Split Panes** -- Horizontal/vertical splits per tab (like `tmux`/`Zellij`).
- [x] **Copy on Select** -- Mouse selection auto-copies to clipboard on release.
- [ ] **Tab Reordering by Drag** -- Drag-to-reorder + drag-to-detach.
- [ ] **Quake-Mode / Drop-Down Terminal** -- Global hotkey toggles a top-anchored sliding window.
- [ ] **Shell Integration (Command Markers)** -- Mark command boundaries in scrollback, jump between them.
- [x] **Click-to-Cursor Prompt Positioning** -- Click in prompt area to move cursor.
- [x] **URL Hover Detection** -- `Ctrl+hover` highlights URLs; `Ctrl+click` opens in browser.

### Customization & Configuration
- [x] **Themes (Color Schemes)** -- Built-in `default`, `catppuccin`, `one-dark`, `solarized-dark`. Live switching.
- [x] **TOML Config + Live Reload (v0.2.8)** -- `fasty.toml` re-applies on save. Round-tripped with `toml_edit`.
- [x] **Custom Keybindings** -- User-rebindable shortcuts in `fasty.toml` `[keybindings]` section.
- [ ] **Visual Settings Picker** -- Replace text-number fields with visual theme/font picker.
- [ ] **Plugin System (Lua/WASM)** -- Ghostty/WezTerm-style scripting.

### Modern Integrations
- [ ] **AI Command Suggestions (opt-in)** -- Local-only: pipe last failed command to a small LLM for fix suggestion.
- [ ] **Inline Git Status in Topbar** -- Render current tab's repo branch + dirty status.
- [ ] **Built-in SSH Manager** -- `Ctrl+Shift+S` -> "Connect to..." picker.
- [ ] **Remote / `fasty://` URL Scheme** -- Register protocol for browser-to-terminal links.
- [ ] **Cloud Config Sync** -- Optional encrypted sync of `fasty.toml`.
- [x] **Background Auto-Updater** -- Non-blocking update check + one-click install from topbar.

### Accessibility
- [ ] **Screen Reader Bridge (Windows UIA / Linux AT-SPI / macOS AX)** -- Announce output to assistive tech.
- [ ] **High-Contrast Theme** -- WCAG AAA-compliant palette.
- [ ] **Color-Blind Palettes** -- Deuteranopia/protanopia-friendly variants.
- [ ] **DPI Override Per-Monitor** -- Verified behavior on mixed-DPI setups.
- [ ] **Touch / Gesture Input** -- Long-press to select, two-finger scroll. Targets 2-in-1 laptops.

### Performance & Reliability
- [x] **D3D12 Debug Layer Disabled in Release (v0.2.5)** -- Skips validation layer.
- [x] **MemoryHints::MemoryUsage on Windows (v0.2.5)** -- Prevents D3D12 shadow copies.
- [x] **CREATE_NO_WINDOW on All Windows Spawns (v0.2.5)** -- Zero console flashes.
- [x] **First-Frame Render Before Show on Dialogs (v0.2.5)** -- Eliminates white backbuffer flash.
- [ ] **Scrollback-to-Disk** -- Spill to memory-mapped file beyond 3000 lines.
- [ ] **GPU-Accelerated Search** -- In-scrollback search as compute shader.
- [ ] **Crash Reporting & Auto-Restart** -- Opt-in crash dump + automatic restart on panic.
- [ ] **Wide-Gamut (P3) and HDR Output** -- Detect HDR displays, emit 10-bit color.

### Platform & Packaging
- [ ] **Signed MSIX / `.msi` Installer for Windows** -- Replace PowerShell installer.
- [x] **Start Menu Shortcut on Windows** -- Auto-registered on first launch.
- [ ] **Flatpak for Linux** -- Sandbox-friendly distribution.
- [ ] **Homebrew Formula Maintenance** -- Real tap overdue.
- [ ] **Android (via `winit` + `wgpu` Mobile)** -- Touch-friendly input and on-screen keyboard.

> Have a feature request? Open an issue on GitHub. Anything that fits the "minimal, fast, GPU-native terminal" philosophy is welcome.

---

## Acknowledgements & Resources

- [wgpu](https://wgpu.rs) - Graphics framework for Rust.
- [Ghostty](https://github.com/ghostty-org/ghostty) - Inspiration for modern GPU terminal features.
- [alacritty_terminal](https://github.com/alacritty/alacritty) - ANSI parser and state wrapper.
- [portable-pty](https://docs.rs/portable-pty/latest/portable_pty/) - Cross-platform PTY manager.
- [The TTY demystified](http://www.linasakesson.net/programming/tty/) - Indispensable resource for terminal PTY structure.
