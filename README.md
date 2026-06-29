# Fastty

<div align="center">
  <img src="assets/fasttyIcon.png" alt="Fastty Logo" width="128" />
</div>

> GPU-accelerated terminal emulator built with Rust, winit, and wgpu

Fastty leverages `wgpu` (the modern, cross-platform GPU API powering GPUI/Zed) for rendering instead of raw Vulkan/Metal implementations, presenting a minimal UI footprint with ultra-high performance. Inspired by [Ghostty](https://github.com/ghostty-org/ghostty).

## Overview

Fastty is a modern terminal emulator that combines GPU acceleration with a minimal feature set. It runs on Vulkan, Metal, DX12, and OpenGL ES through wgpu, providing consistent performance across platforms.

**Key characteristics:**
- GPU-accelerated rendering with `<1%` CPU idle usage
- Native UI with vector icons and smooth animations
- Tabbed interface with session restore
- Git integration, SSH manager, and command palette
- TOML configuration with live reload
- Crash reporting and auto-restart

## Installation

### Quick Install

**Linux & macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fastty/main/instalar.sh | bash
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/diegoleteliers10/fastty/main/instalar.ps1 | iex
```

### Manual Install

Download from [Releases](https://github.com/diegoleteliers10/fastty/releases).

Extract and install:
```bash
# Linux
mkdir -p ~/.local/bin
mv fastty ~/.local/bin/

# macOS
mv Fastty.app /Applications/
```

### Build from Source

```bash
# System dependencies
# macOS: xcode-select --install
# Linux: sudo apt install libvulkan-dev libwayland-dev (or libx11-dev)
# Windows: Install Vulkan SDK

cargo build --release
```

## Getting Started

**Basic usage:**
```bash
fastty                                    # Default shell
fastty -e htop                            # Run command, auto-close on exit
fastty -e ssh user@server                 # SSH session
fastty -d ~/my-project -e bun run dev    # Dev server in specific directory
```

**Configuration:**
Fastty reads `fastty.toml` from:
1. `./fastty.toml` (portable)
2. `/etc/fastty/fastty.toml` (system)
3. `~/.config/fastty/fastty.toml` (user)

```toml
shell = "/bin/bash"
scrollback = 3000
theme = "default"
session_restore = true

[font]
family = "JetBrains Mono"
size = 14.0
ligatures = true

[keybindings]
ctrl+shift+t = "new_tab"
ctrl+shift+p = "command_palette"
```

## Features

**Performance & UX:**
- GPU-accelerated rendering via wgpu (Vulkan, Metal, DX12, GLES)
- `<1%` CPU idle usage with optimized cursor blink
- Shared GPU atlases for instant window creation and tab tearing
- Animated scrollbar fading and TUI mouse integration
- Memory footprint: 30-50MB (Linux), 50-70MB (Windows)

**Terminal Features:**
- Tabbed layout with drag-to-reorder and tear-out windows
- Session restore (saves and restores tab directories)
- Live-apply settings for fonts, scrollback, and themes
- Built-in themes: default, catppuccin, one-dark, solarized-dark, high-contrast
- Custom keybindings and TOML config with live reload

**Developer Tools:**
- Git status bottombar with branch, dirty counts, and commit info
- Built-in SSH manager (`Ctrl+Shift+O`)
- Project jumper for quick navigation between open tabs
- Git worktree picker (`Ctrl+Alt+W`)
- Shell snippets with Tab expansion
- Command palette (`Ctrl+Shift+P`)

**Reliability:**
- Crash reporting with auto-restart
- Asynchronous background updater
- URL hover detection and opening
- Copy on select

## Configuration

**Themes:**
Built-in themes available: `default`, `catppuccin`, `one-dark`, `solarized-dark`, `high-contrast`

Custom themes as JSON in `~/.config/fastty/themes/`:
```json
{
  "background": "#1a1b26",
  "foreground": "#c0caf5",
  "black": "#15161e",
  "red": "#f7768e",
  "green": "#9ece6a"
}
```

**Bottombar Widgets:**
```toml
[[bottombar.widgets]]
type = "git"
align = "left"

[[bottombar.widgets]]
type = "time"
format = "%H:%M"
align = "left"

[[bottombar.widgets]]
type = "command"
command = "hostname -I | awk '{print $1}'"
on_click = "copy"
interval_ms = 10000
```

Available widget types: `git`, `time`, `kube`, `aws`, `command`

**Snippets:**
Shell command expansion via Tab in `~/.config/fastty/snippets.toml`:
```toml
"gst" = "git status"
"gcm" = "git commit -m \"${1:message}\""
"ll" = "ls -lah"
"serve" = "python3 -m http.server"
```

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+T` | New tab |
| `Ctrl+Shift+W` | Close tab |
| `Ctrl+Shift+P` | Command palette |
| `Ctrl+Shift+O` | SSH manager |
| `Ctrl+Shift+J` | Project jumper |
| `Ctrl+Alt+W` | Git worktree picker |
| `Ctrl+Shift+F` | Search in scrollback |
| `Ctrl+Shift+S` | Settings |
| `Tab` | Expand snippet |
| `Ctrl+Plus/Minus` | Adjust font size |

## Architecture

```
src/
├── main.rs            # Entry point, event loop, UI state
├── terminal_state.rs  # PTY controller & parser wrapper
├── renderer/          # wgpu backend (pipeline, atlas, shaders)
├── config.rs          # TOML config & live reload
├── keybindings.rs     # Key combo parser & action resolver
├── git.rs             # Git status polling & worktree helpers
├── ssh.rs             # SSH config parser
├── snippets.rs        # Shell snippet expansion
├── session.rs         # Tab directory persistence
└── crash.rs           # Panic hook & crash reporter
```

## Performance

Fastty achieves sub-1% CPU idle through:
- `winit`'s `ControlFlow::Wait` for event-driven sleeping
- Blocking PTY read loop at OS kernel level
- Micro-optimized cursor blink with GPU fast-path (48-byte buffer write)
- 64KB PTY buffer to minimize syscalls
- Lazy font loading with batch pre-rasterization
- Cached WGSL shaders with `std::sync::OnceLock`

Memory optimizations:
- Scrollback capped at 3000 lines (37MB reduction)
- GPU texture atlases: 1536x1536 (Linux/macOS), 1024x1024 (Windows)
- LTO + stripped symbols in release builds
- Windows: D3D12 debug layer disabled, no staging copies

## Acknowledgments

Built with excellent open-source tools:
- [wgpu](https://wgpu.rs) - Modern cross-platform GPU API
- [Ghostty](https://github.com/ghostty-org/ghostty) - Inspiration for GPU-native terminal features
- [alacritty_terminal](https://github.com/alacritty/alacritty) - ANSI/VT parser
- [portable-pty](https://docs.rs/portable-pty) - Cross-platform PTY management
- [The TTY demystified](http://www.linasakesson.net/programming/tty/) - Terminal PTY reference

## License

See [LICENSE](LICENSE) file.