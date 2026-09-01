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
- **Local IPC Daemon Protocol**: Built-in multiplexer server via Unix domain socket (`fasttyd.sock`). List live sessions, attach remotely, or integrate with Neovim, Zed, and VS Code plugins.
- **Native Web Gateway & Fastty-Wasm**: Embedded zero-dependency HTTP & WebSocket server (`fastty gateway`) and pure WebAssembly VT emulator to access your active terminals from any browser, tablet, or mobile device.
- **Cross-Platform**: First-class support for macOS, Linux (Wayland/X11), and Windows with automatic shell detection (`zsh`, `fish`, `bash`, `powershell`, `pwsh`, `cmd`).
- **Built-in Status Bar & Git Integration**: Branch indicator, worktree switcher (`⌘⌥W` / `Ctrl+Alt+W`), sync status, and command execution timer.
- **Command Palette & SSH Manager**: Fast command navigation (`⌘P` / `Ctrl+Shift+P`) and SSH connection manager (`⌘O` / `Ctrl+Shift+O`).
- **Config & Theming**: TOML configuration (`fastty.toml`) with live reload and built-in themes (*Default*, *Catppuccin*, *One Dark*, *Solarized Dark*, *High Contrast*).

---

## Daemon Protocol & Web Gateway

Fastty includes a built-in session multiplexer and an embedded web server accessible via the CLI:

### CLI Subcommands

```bash
# List all active tabs and splits with PID, CWD, and window title
fastty sessions

# Watch session changes in real time (reactive stream)
fastty sessions --watch

# Attach interactively to an existing session
fastty attach <session-id>

# Attach in read-only mode (viewing logs/builds without input interference)
fastty attach <session-id> --read-only

# Wait for a session to become available before attaching
fastty attach <session-id> --wait=30
```

### Web Gateway & Browser Access

Launch the native, zero-dependency web gateway:

```bash
# Start web gateway on default port 8765
fastty gateway

# Bind to custom port and network interface (e.g. for Tailscale or LAN access)
fastty gateway --port 8765 --host 0.0.0.0

# Enforce read-only access for all connected browsers
fastty gateway --port 8765 --read-only
```

Then visit `http://localhost:8765` in your browser. The embedded WebAssembly engine (`fastty-wasm`) provides:
- Batched 60/120 FPS HTML5 Canvas 2D rendering.
- Interactive scrollback buffer, mouse wheel, and touch navigation.
- Dynamic font size zoom controls and real-time tab switching.

See [docs/daemon-protocol.md](docs/daemon-protocol.md) for the complete JSON IPC protocol specification.

---

## Installation

### Quick Install

#### macOS & Linux
```bash
curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.sh | bash
```

#### Windows (PowerShell)
```powershell
irm https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.ps1 | iex
```

---

### Homebrew (macOS)

```bash
brew tap diegoleteliers10/fasty https://github.com/diegoleteliers10/fasty
brew install --cask fastty
```

Or install directly from the Cask URL without tapping:

```bash
brew install --cask https://raw.githubusercontent.com/diegoleteliers10/fasty/main/Casks/fastty.rb
```

### Manual Installation

Download the latest pre-compiled archive for your platform from [Releases](https://github.com/diegoleteliers10/fasty/releases):

- **macOS (Apple Silicon & Intel)**: Download `fastty-aarch64-apple-darwin.dmg` or `fastty-x86_64-apple-darwin.dmg`, open it, and drag `Fastty.app` to `Applications`. (Alternatively, extract the matching `.tar.gz` and move `Fastty.app` to `/Applications/` yourself.)
- **Windows**: Run `Fastty_<version>_x64.msi` for a standard installer (adds Fastty to the Start Menu and to *Settings > Apps* / *Control Panel > Programs and Features* for clean uninstalling). It installs per-user under `%LOCALAPPDATA%\Programs\Fastty`, so it needs no administrator rights to install *or* to self-update later. Alternatively, extract `fastty-x86_64-pc-windows-msvc.zip` and place `fastty.exe` in your `PATH`.
- **Linux (Debian/Ubuntu)**: `sudo dpkg -i fastty_<version>_amd64.deb` (or double-click it in a graphical file manager). Installs the binary, a desktop menu entry, and icon. Update it the same way `apt`/`dpkg` updates any package (see [Updates](#updates) below).
- **Linux (any distro)**: Download `Fastty_<version>_amd64.AppImage`, `chmod +x` it, and run it directly — no installation required.
- **Linux (manual)**: Extract `fastty-x86_64-unknown-linux-gnu.tar.gz` and place `fastty` into `~/.local/bin/`.

> **Note for macOS:** Fastty uses ad-hoc code signing. Installing via Homebrew or `instalar.sh` automatically configures macOS Gatekeeper permissions. If you download the DMG directly through a browser and macOS reports the app as damaged or unverified, run this command once:
> ```bash
> xattr -cr /Applications/Fastty.app
> ```

---

## Updates

Fastty checks GitHub's latest release on startup and shows an in-app "vX.Y.Z" button when a newer one exists. What happens when you click it depends on how Fastty got installed — the same split that Zed and Ghostty make between their own self-updater and a system package manager's:

| Install method | Update path |
|---|---|
| `.tar.gz` / `.zip` (manual/`instalar.sh`) | Self-updates in place |
| `.dmg` (drag to Applications) | Self-updates in place |
| Homebrew Cask | `brew upgrade --cask fastty` (self-update is disabled so it doesn't desync brew's own record) |
| `.msi` (per-user) | Self-updates in place (no admin needed, same as the install) |
| `.deb` | `sudo apt update && sudo apt upgrade fastty` (self-update is disabled — the binary is owned by dpkg) |
| `.AppImage` | Download the newest `Fastty_*.AppImage` manually (AppImages can't persist a self-replace across restarts) |

When self-update isn't available, clicking the update button shows a modal explaining how to update instead of trying (and possibly failing) to overwrite files it doesn't own. Third-party packagers (AUR, Nix, Flatpak, ...) can force this behavior with their own message by setting the `FASTTY_UPDATE_EXPLANATION` environment variable when launching Fastty, the same escape hatch Zed exposes as `ZED_UPDATE_EXPLANATION`.

---

### Build from Source

```bash
# Clone the repository
git clone https://github.com/diegoleteliers10/fasty.git
cd fasty

# Build in release mode
cargo build --release

# Build WebAssembly package
cargo build --target wasm32-unknown-unknown -p fastty-wasm --release

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
