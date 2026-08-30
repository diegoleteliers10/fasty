#!/usr/bin/env bash
# ==============================================================================
# Fastty installation script for Linux and macOS
# ==============================================================================
# Run directly from the internet with:
# curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.sh | bash
# ==============================================================================

set -euo pipefail

GITHUB_USER="diegoleteliers10"
GITHUB_REPO="fasty"
APP_NAME="fastty"

echo "=== Starting $APP_NAME installation ==="

# ── 1. Detect OS and architecture ─────────────────────────────────────────────
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
    linux)
        if [ "$ARCH" = "x86_64" ] || [ "$ARCH" = "amd64" ]; then
            TARGET="x86_64-unknown-linux-gnu"
        else
            echo "ERROR: Linux architecture '$ARCH' is not supported." >&2
            exit 1
        fi
        ;;
    darwin)
        if [ "$ARCH" = "x86_64" ]; then
            TARGET="x86_64-apple-darwin"
        elif [ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ]; then
            TARGET="aarch64-apple-darwin"
        else
            echo "ERROR: macOS architecture '$ARCH' is not supported." >&2
            exit 1
        fi
        ;;
    *)
        echo "ERROR: OS '$OS' is not compatible with this script." >&2
        exit 1
        ;;
esac

echo "Detected platform: OS=$OS, Arch=$ARCH -> Target=$TARGET"

# ── 1.5. Refuse to step on an install already owned by a package manager ──────
# Mirrors the "one owner per install" rule the app's own updater enforces
# (see src/updater.rs / self_update_blocked_reason): if Fastty is already
# managed by Homebrew or a system package, installing here too would create
# two copies that disagree about who is responsible for updating it.
FORCE_INSTALL="${FASTTY_FORCE_INSTALL:-0}"

if [ "$OS" = "darwin" ]; then
    for caskroom in "/opt/homebrew/Caskroom/$APP_NAME" "/usr/local/Caskroom/$APP_NAME"; do
        if [ -d "$caskroom" ] && [ "$FORCE_INSTALL" != "1" ]; then
            echo "NOTICE: Fastty is already installed via Homebrew (found $caskroom)." >&2
            echo "        Run 'brew upgrade --cask fastty' to update instead." >&2
            echo "        Re-run with FASTTY_FORCE_INSTALL=1 to install alongside it anyway." >&2
            exit 1
        fi
    done
else
    if command -v dpkg >/dev/null 2>&1 && dpkg -s "$APP_NAME" >/dev/null 2>&1 && [ "$FORCE_INSTALL" != "1" ]; then
        echo "NOTICE: Fastty is already installed via a .deb package." >&2
        echo "        Run 'sudo apt update && sudo apt upgrade $APP_NAME' to update instead." >&2
        echo "        Re-run with FASTTY_FORCE_INSTALL=1 to install alongside it anyway." >&2
        exit 1
    fi
fi

# ── 2. Resolve platform directories ───────────────────────────────────────────
if [ "$OS" = "darwin" ]; then
    CONFIG_DIR="$HOME/Library/Application Support/fastty"
    DATA_DIR="$HOME/Library/Application Support/fastty"
    STATE_DIR="$HOME/Library/Application Support/fastty"
    CACHE_DIR="$HOME/Library/Caches/fastty"
    if [ -w "/usr/local/bin" ]; then
        BIN_DIR="/usr/local/bin"
    else
        BIN_DIR="$HOME/.local/bin"
    fi
else
    CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/fastty"
    DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/fastty"
    STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/fastty"
    CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/fastty"
    BIN_DIR="$HOME/.local/bin"
fi

# ── 3. Create directories ─────────────────────────────────────────────────────
mkdir -p "$CONFIG_DIR" "$DATA_DIR" "$STATE_DIR" "$CACHE_DIR" "$BIN_DIR"

# ── 4. Query GitHub API for the latest release ────────────────────────────────
echo "Fetching latest version from GitHub..."
API_URL="https://api.github.com/repos/$GITHUB_USER/$GITHUB_REPO/releases/latest"
API_RESPONSE=$(curl -sSfL "$API_URL")

if command -v jq >/dev/null 2>&1; then
    LATEST_TAG=$(printf '%s' "$API_RESPONSE" | jq -r '.tag_name')
elif command -v python3 >/dev/null 2>&1; then
    LATEST_TAG=$(printf '%s' "$API_RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin)['tag_name'])")
elif command -v python >/dev/null 2>&1; then
    LATEST_TAG=$(printf '%s' "$API_RESPONSE" | python -c "import sys,json; print(json.load(sys.stdin)['tag_name'])")
else
    LATEST_TAG=$(printf '%s' "$API_RESPONSE" \
        | tr -d ' \t\r\n' \
        | grep -o '"tag_name":"[^"]*"' \
        | head -1 \
        | sed -E 's/"tag_name":"([^"]+)"/\1/')
fi

if [ -z "$LATEST_TAG" ] || [ "$LATEST_TAG" = "null" ]; then
    echo "ERROR: Could not determine latest release version from GitHub." >&2
    exit 1
fi

echo "Latest version found: $LATEST_TAG"

# ── 5. Download asset ─────────────────────────────────────────────────────────
ASSET_NAME="$APP_NAME-$TARGET.tar.gz"
DOWNLOAD_URL="https://github.com/$GITHUB_USER/$GITHUB_REPO/releases/download/$LATEST_TAG/$ASSET_NAME"

TEMP_DIR=$(mktemp -d -t install-$APP_NAME.XXXXXX)
trap 'rm -rf "$TEMP_DIR"' EXIT

echo "Downloading $ASSET_NAME..."
curl -sSfL -o "$TEMP_DIR/$ASSET_NAME" "$DOWNLOAD_URL"

# ── 6. Extract ─────────────────────────────────────────────────────────────────
echo "Extracting archive..."
tar -xzf "$TEMP_DIR/$ASSET_NAME" -C "$TEMP_DIR"

# ── 6.5. Initialize default configuration if absent ───────────────────────────
CONFIG_FILE="$CONFIG_DIR/fastty.toml"
LEGACY_CONFIG="$CONFIG_DIR/config.toml"

if [ ! -f "$CONFIG_FILE" ] && [ ! -f "$LEGACY_CONFIG" ] && [ ! -f "$CONFIG_DIR/config.json" ]; then
    if [ -f "$TEMP_DIR/fastty.toml" ]; then
        echo "Copying default fastty.toml..."
        cp "$TEMP_DIR/fastty.toml" "$CONFIG_FILE"
    elif [ -f "$TEMP_DIR/config.toml" ]; then
        echo "Copying default fastty.toml..."
        cp "$TEMP_DIR/config.toml" "$CONFIG_FILE"
    else
        echo "Generating default fastty.toml..."
        cat << 'EOF' > "$CONFIG_FILE"
# fastty configuration file
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
EOF
    fi
fi

# ── 7. Install binary and platform integrations ────────────────────────────────
if [ "$OS" = "darwin" ]; then
    INSTALL_DIR="/Applications"
    if [ -d "$TEMP_DIR/Fastty.app" ]; then
        if [ -w "$INSTALL_DIR" ]; then
            rm -rf "$INSTALL_DIR/Fastty.app"
            mv "$TEMP_DIR/Fastty.app" "$INSTALL_DIR/"
        else
            echo "Administrator privileges required to write to $INSTALL_DIR:"
            sudo rm -rf "$INSTALL_DIR/Fastty.app"
            sudo mv "$TEMP_DIR/Fastty.app" "$INSTALL_DIR/"
        fi
    fi

    # Symlink to bin directory
    if [ -w "$BIN_DIR" ]; then
        rm -f "$BIN_DIR/fastty"
        ln -sf "$INSTALL_DIR/Fastty.app/Contents/MacOS/fastty" "$BIN_DIR/fastty"
    else
        sudo rm -f "$BIN_DIR/fastty"
        sudo ln -sf "$INSTALL_DIR/Fastty.app/Contents/MacOS/fastty" "$BIN_DIR/fastty"
    fi

    # Remove macOS quarantine bit and refresh LaunchServices
    xattr -dr com.apple.quarantine "$INSTALL_DIR/Fastty.app" 2>/dev/null || true
    /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$INSTALL_DIR/Fastty.app" 2>/dev/null || true

    echo "$APP_NAME installed successfully at $INSTALL_DIR/Fastty.app"
else
    # Linux: locate binary and install
    SRC_BINARY=$(find "$TEMP_DIR" -maxdepth 2 -type f -name "$APP_NAME" | head -n 1)

    if [ ! -f "$SRC_BINARY" ]; then
        echo "ERROR: Binary '$APP_NAME' was not found in the extracted files." >&2
        exit 1
    fi

    if [ -w "$BIN_DIR" ]; then
        rm -f "$BIN_DIR/$APP_NAME"
        cp -f "$SRC_BINARY" "$BIN_DIR/$APP_NAME"
        chmod 0755 "$BIN_DIR/$APP_NAME"
    else
        echo "Administrator privileges required to write to $BIN_DIR:"
        sudo rm -f "$BIN_DIR/$APP_NAME"
        sudo cp -f "$SRC_BINARY" "$BIN_DIR/$APP_NAME"
        sudo chmod 0755 "$BIN_DIR/$APP_NAME"
    fi

    # Set up desktop icon and application launcher
    ICON_HICOLOR_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/512x512/apps"
    ICON_PIXMAP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/pixmaps"
    DESKTOP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    mkdir -p "$ICON_HICOLOR_DIR" "$ICON_PIXMAP_DIR" "$DESKTOP_DIR"

    RAW_ICON_URL="https://raw.githubusercontent.com/$GITHUB_USER/$GITHUB_REPO/main/assets/fasttyIcon.png"
    curl -sSfL -o "$ICON_HICOLOR_DIR/fastty.png" "$RAW_ICON_URL" 2>/dev/null || true
    cp -f "$ICON_HICOLOR_DIR/fastty.png" "$ICON_PIXMAP_DIR/fastty.png" 2>/dev/null || true

    cat << EOF > "$DESKTOP_DIR/fastty.desktop"
[Desktop Entry]
Name=Fastty
GenericName=Terminal Emulator
Comment=Fast GPU-accelerated Terminal Emulator
Exec=$BIN_DIR/$APP_NAME %U
Icon=fastty
Terminal=false
Type=Application
Categories=System;TerminalEmulator;Utility;
Keywords=shell;prompt;command;commandline;terminal;emulator;
StartupWMClass=fastty
Actions=NewWindow;

[Desktop Action NewWindow]
Name=New Window
Exec=$BIN_DIR/$APP_NAME
EOF

    chmod 0644 "$DESKTOP_DIR/fastty.desktop"
    update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
    gtk-update-icon-cache -f -t "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor" 2>/dev/null || true

    echo "$APP_NAME installed successfully at $BIN_DIR/$APP_NAME"
fi

# ── 8. Shell and PATH Configuration Check ─────────────────────────────────────
USER_SHELL="$(basename "${SHELL:-bash}")"

if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo ""
    echo "NOTICE: $BIN_DIR is not in your current PATH."
    case "$USER_SHELL" in
        fish)
            if command -v fish_add_path >/dev/null 2>&1; then
                fish_add_path "$BIN_DIR" 2>/dev/null || true
                echo "Added $BIN_DIR to fish universal path automatically."
            else
                echo "Add to ~/.config/fish/config.fish:  set -gx PATH $BIN_DIR \$PATH"
            fi
            ;;
        zsh)
            echo "Add to ~/.zshrc:  export PATH=\"$BIN_DIR:\$PATH\""
            ;;
        bash)
            echo "Add to ~/.bashrc:  export PATH=\"$BIN_DIR:\$PATH\""
            ;;
        nu|nushell)
            echo "Add to ~/.config/nushell/config.nu:  \$env.PATH = (\$env.PATH | split row (char esep) | prepend '$BIN_DIR')"
            ;;
        *)
            echo "Add '$BIN_DIR' to your shell's PATH configuration."
            ;;
    esac
fi

# ── 9. Summary ────────────────────────────────────────────────────────────────
echo ""
echo "=== Fastty installation complete ==="
echo "    ✓ Binary    → $BIN_DIR/$APP_NAME"
echo "    ✓ Config    → $CONFIG_DIR"
echo "    ✓ Data      → $DATA_DIR"
echo "    ✓ State     → $STATE_DIR"
echo "    ✓ Cache     → $CACHE_DIR"
echo ""
