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
        if [ "$ARCH" = "x86_64" ]; then
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

# ── 2. Resolve XDG/platform directories ───────────────────────────────────────
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

# ── 4. Query the GitHub API for the latest release ─────────────────────────────
echo "Fetching the latest version from GitHub..."
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
    echo "ERROR: Could not determine the latest version from GitHub." >&2
    exit 1
fi

echo "Latest version found: $LATEST_TAG"

# ── 5. Download the correct asset ──────────────────────────────────────────────
ASSET_NAME="$APP_NAME-$TARGET.tar.gz"
DOWNLOAD_URL="https://github.com/$GITHUB_USER/$GITHUB_REPO/releases/download/$LATEST_TAG/$ASSET_NAME"

TEMP_DIR=$(mktemp -d -t install-$APP_NAME.XXXXXX)
trap 'rm -rf "$TEMP_DIR"' EXIT

echo "Downloading $ASSET_NAME..."
curl -sSfL -o "$TEMP_DIR/$ASSET_NAME" "$DOWNLOAD_URL"

# ── 6. Extract ─────────────────────────────────────────────────────────────────
echo "Extracting files..."
tar -xzf "$TEMP_DIR/$ASSET_NAME" -C "$TEMP_DIR"

# ── 6.5. Copy default config if bundled ─────────────────────────────────────────
if [ ! -f "$CONFIG_DIR/config.toml" ] && [ ! -f "$CONFIG_DIR/config.json" ]; then
    if [ -f "$TEMP_DIR/config.toml" ]; then
        echo "Copying default config.toml..."
        cp "$TEMP_DIR/config.toml" "$CONFIG_DIR/config.toml"
    elif [ -f "$TEMP_DIR/config.json" ]; then
        echo "Copying default config.json..."
        cp "$TEMP_DIR/config.json" "$CONFIG_DIR/config.json"
    fi
fi

# ── 7. Install binary ─────────────────────────────────────────────────────────
if [ "$OS" = "darwin" ]; then
    # macOS: install .app bundle + symlink
    INSTALL_DIR="/Applications"
    if [ -w "$INSTALL_DIR" ]; then
        rm -rf "$INSTALL_DIR/Fastty.app"
        mv "$TEMP_DIR/Fastty.app" "$INSTALL_DIR/"
    else
        echo "Administrator privileges (sudo) are required to write to $INSTALL_DIR."
        sudo rm -rf "$INSTALL_DIR/Fastty.app"
        sudo mv "$TEMP_DIR/Fastty.app" "$INSTALL_DIR/"
    fi

    if [ -w "$BIN_DIR" ]; then
        rm -f "$BIN_DIR/fastty"
        ln -sf "$INSTALL_DIR/Fastty.app/Contents/MacOS/fastty" "$BIN_DIR/fastty"
    else
        sudo rm -f "$BIN_DIR/fastty"
        sudo ln -sf "$INSTALL_DIR/Fastty.app/Contents/MacOS/fastty" "$BIN_DIR/fastty"
    fi

    echo "$APP_NAME installed successfully at $INSTALL_DIR/Fastty.app!"
else
    # Linux: install binary directly
    SRC_BINARY=$(find "$TEMP_DIR" -maxdepth 2 -type f -name "$APP_NAME" | head -n 1)

    if [ ! -f "$SRC_BINARY" ]; then
        echo "ERROR: Could not find binary '$APP_NAME' in extracted files." >&2
        exit 1
    fi

    if [ -w "$BIN_DIR" ]; then
        rm -f "$BIN_DIR/$APP_NAME"
        cp -f "$SRC_BINARY" "$BIN_DIR/$APP_NAME"
        chmod 0755 "$BIN_DIR/$APP_NAME"
    else
        echo "Administrator privileges (sudo) are required to write to $BIN_DIR."
        sudo rm -f "$BIN_DIR/$APP_NAME"
        sudo cp -f "$SRC_BINARY" "$BIN_DIR/$APP_NAME"
        sudo chmod 0755 "$BIN_DIR/$APP_NAME"
    fi

    # Verify the binary was copied correctly
    NEW_HASH=$(sha256sum "$SRC_BINARY" 2>/dev/null | awk '{print $1}')
    INSTALLED_HASH=$(sha256sum "$BIN_DIR/$APP_NAME" 2>/dev/null | awk '{print $1}')
    if [ -z "$NEW_HASH" ] || [ -z "$INSTALLED_HASH" ] || [ "$NEW_HASH" != "$INSTALLED_HASH" ]; then
        echo "ERROR: the binary at $BIN_DIR/$APP_NAME does not match the new one (copy failed). Aborting." >&2
        exit 1
    fi

    # Set up icon and desktop entry
    ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/pixmaps"
    DESKTOP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    mkdir -p "$ICON_DIR" "$DESKTOP_DIR"

    RAW_ICON_URL="https://raw.githubusercontent.com/$GITHUB_USER/$GITHUB_REPO/main/assets/fasttyIcon.png"
    if [ -w "$ICON_DIR" ]; then
        curl -sSfL -o "$ICON_DIR/fastty.png" "$RAW_ICON_URL"
    else
        sudo mkdir -p "$ICON_DIR"
        sudo curl -sSfL -o "$ICON_DIR/fastty.png" "$RAW_ICON_URL"
    fi

    DESKTOP_CONTENT="[Desktop Entry]
Name=Fastty
Comment=GPU-accelerated Terminal Emulator
Exec=$BIN_DIR/$APP_NAME
Icon=$ICON_DIR/fastty.png
Terminal=false
Type=Application
Categories=System;TerminalEmulator;
Keywords=terminal;emulator;wgpu;"

    if [ -w "$DESKTOP_DIR" ]; then
        echo "$DESKTOP_CONTENT" > "$DESKTOP_DIR/fastty.desktop"
    else
        sudo mkdir -p "$DESKTOP_DIR"
        echo "$DESKTOP_CONTENT" | sudo tee "$DESKTOP_DIR/fastty.desktop" > /dev/null
    fi

    echo "$APP_NAME installed successfully at $BIN_DIR/$APP_NAME!"
    echo "You can launch it by searching '$APP_NAME' in your menu or typing it in a terminal."

    # Schedule a self-restart
    (
        sleep 3
        pkill -x fastty 2>/dev/null || true
        nohup "$BIN_DIR/$APP_NAME" >/dev/null 2>&1 &
        disown
    ) &
    disown
fi

# ── 8. Check PATH ──────────────────────────────────────────────────────────────
if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo ""
    echo "WARNING: $BIN_DIR is not in your PATH."
    echo "Add it by running one of the following:"
    echo ""
    echo "    bash  → add 'export PATH=\"$BIN_DIR:\$PATH\"' to ~/.bashrc"
    echo "    zsh   → add 'export PATH=\"$BIN_DIR:\$PATH\"' to ~/.zshrc"
    echo "    fish  → run: fish_add_path $BIN_DIR"
fi

# ── 9. Summary ─────────────────────────────────────────────────────────────────
echo ""
echo "    ✓ Binary    → $BIN_DIR/fastty"
echo "    ✓ Config    → $CONFIG_DIR"
echo "    ✓ Data      → $DATA_DIR"
echo "    ✓ State     → $STATE_DIR"
echo "    ✓ Cache     → $CACHE_DIR"
