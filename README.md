# Fastty

<div align="center">
  <img src="assets/fasttyIcon.png" alt="Fastty Logo" width="128" />
  <h3>Fast, GPU-accelerated terminal emulator built with Rust & GPUI</h3>
</div>

Fastty is a modern, high-performance terminal emulator designed for speed, low memory usage, and seamless developer workflows across macOS, Linux, and Windows.

## Key Features

- **GPU-Accelerated Rendering**: Fast layout and rasterization powered by GPUI and Metal/Vulkan.
- **TUI & CLI Compatibility**: Pixel-perfect rendering for OpenCode, Claude Code, lazygit, htop, and complex box-drawing/block ASCII art.
- **Modern Tab System**: Underline selection styling, double-click/right-click tab renaming (`⌘⇧R` / `Ctrl+Shift+R`), duplicate tab, and dynamic process title updates.
- **Cross-Platform**: First-class support for macOS, Linux (Wayland/X11), and Windows with automatic shell detection (`zsh`, `fish`, `bash`, `powershell`, `pwsh`, `cmd`).
- **Built-in Status Bar & Git Integration**: Branch indicator, worktree switcher (`⌘⌥W` / `Ctrl+Alt+W`), sync status, and command execution timer.
- **Command Palette & SSH Manager**: Fast command navigation (`⌘P` / `Ctrl+Shift+P`) and SSH connection manager (`⌘O` / `Ctrl+Shift+O`).
- **Config & Theming**: TOML configuration (`fastty.toml`) with live reload and built-in themes (*Default*, *Catppuccin*, *One Dark*, *Solarized Dark*, *High Contrast*).

---

## Installation

### Quick Install

#### macOS & Linux
```bash
curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.sh | bash
```

#### Windows (PowerShell)
```powershell
irm https://raw.githubusercontent.com/diegoleteliers10/fastty/main/instalar.ps1 | iex
```

---

### Manual Installation

Download the latest pre-compiled archive for your platform from [Releases](https://github.com/diegoleteliers10/fasty/releases):

- **macOS (Apple Silicon & Intel)**: Extract `fastty-aarch64-apple-darwin.tar.gz` or `fastty-x86_64-apple-darwin.tar.gz` and move `Fastty.app` to `/Applications/`.
- **Linux**: Extract `fastty-x86_64-unknown-linux-gnu.tar.gz` and place `fastty` into `~/.local/bin/`.
- **Windows**: Extract `fastty-x86_64-pc-windows-msvc.zip` and place `fastty.exe` in your `PATH`.

---

### Build from Source

```bash
# Clone the repository
git clone https://github.com/diegoleteliers10/fasty.git
cd fasty

# Build in release mode
cargo build --release

# The compiled binary is located at target/release/fastty
```

---

## Configuration

Fastty searches for configuration in the following locations:
1. `./fastty.toml` or `./config.toml` (portable mode)
2. `/etc/fastty/fastty.toml` (system-wide)
3. User directory:
   - **macOS**: `~/Library/Application Support/fastty/fastty.toml`
   - **Linux**: `~/.config/fastty/fastty.toml`
   - **Windows**: `%APPDATA%\fastty\config\fastty.toml`

### Example `fastty.toml`

```toml
theme = "default"
opacity = 1.0
scrollback = 1000
session_restore = true
copy_on_select = false
notify_on_command_finish = true

[font]
family = "monospace"
size = 14.0
weight = 400.0
ligatures = true

[bottombar]
enabled = true
layout = "balanced"
left_widgets = ["git_branch", "git_status"]
right_widgets = ["cwd", "duration", "exit_code"]

[keybindings]
# Custom shortcuts can be specified here
```

---

## Keyboard Shortcuts

| Action | macOS | Linux / Windows |
|---|---|---|
| **New Tab** | `⌘T` | `Ctrl+Shift+T` |
| **Close Tab** | `⌘W` | `Ctrl+Shift+W` |
| **Rename Tab** | `⌘⇧R` | `Ctrl+Shift+R` |
| **Next / Previous Tab** | `⌘⇧]` / `⌘⇧[` or `Ctrl+Tab` | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| **Command Palette** | `⌘P` | `Ctrl+Shift+P` |
| **SSH Manager** | `⌘O` | `Ctrl+Shift+O` |
| **Git Worktrees** | `⌘⌥W` | `Ctrl+Alt+W` |
| **Project Jumper** | `⌘J` | `Ctrl+Shift+J` |
| **Search in Scrollback** | `⌘F` | `Ctrl+F` |
| **Settings** | `⌘,` | `Ctrl+,` |
| **Clear Scrollback** | `⌘K` | `Ctrl+Shift+K` |
| **Zoom In / Out / Reset** | `⌘+` / `⌘-` / `⌘0` | `Ctrl+Plus` / `Ctrl+Minus` / `Ctrl+0` |

---

## License

MIT License. See [LICENSE](LICENSE) for details.
