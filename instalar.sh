#!/usr/bin/env bash
# ==============================================================================
# Fasty installation script for Linux and macOS
# ==============================================================================
# Run directly from the internet with:
# curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.sh | bash
# ==============================================================================

set -euo pipefail

USE_USER_DIR=false
if [ "${FASTY_USER_INSTALL:-0}" = "1" ]; then
    USE_USER_DIR=true
fi

for arg in "$@"; do
    case $arg in
        --user)
            USE_USER_DIR=true
            shift
            ;;
    esac
done

GITHUB_USER="diegoleteliers10"
GITHUB_REPO="fasty"
APP_NAME="fasty"

echo "=== Starting $APP_NAME installation ==="

# 1. Detect OS and architecture
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

# 2. Query the GitHub API for the latest release
echo "Fetching the latest version from GitHub..."
API_URL="https://api.github.com/repos/$GITHUB_USER/$GITHUB_REPO/releases/latest"

# Fetch the full JSON response once and store it
API_RESPONSE=$(curl -sSfL "$API_URL")

# Extract tag_name robustly — tries tools in order of reliability:
#   1. jq          – most correct, handles any JSON formatting
#   2. python3     – available on virtually all modern Linux/macOS systems
#   3. python      – fallback for older systems
#   4. sed/grep    – pure POSIX fallback; strips all whitespace so it works
#                    on both compact (single-line) and pretty-printed JSON
if command -v jq >/dev/null 2>&1; then
    LATEST_TAG=$(printf '%s' "$API_RESPONSE" | jq -r '.tag_name')
elif command -v python3 >/dev/null 2>&1; then
    LATEST_TAG=$(printf '%s' "$API_RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin)['tag_name'])")
elif command -v python >/dev/null 2>&1; then
    LATEST_TAG=$(printf '%s' "$API_RESPONSE" | python -c "import sys,json; print(json.load(sys.stdin)['tag_name'])")
else
    # Remove all whitespace, then extract the value of "tag_name":"..."
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

# 3. Download the correct .tar.gz asset
ASSET_NAME="$APP_NAME-$TARGET.tar.gz"
DOWNLOAD_URL="https://github.com/$GITHUB_USER/$GITHUB_REPO/releases/download/$LATEST_TAG/$ASSET_NAME"

TEMP_DIR=$(mktemp -d -t install-$APP_NAME.XXXXXX)
trap 'rm -rf "$TEMP_DIR"' EXIT

echo "Downloading $ASSET_NAME..."
curl -sSfL -o "$TEMP_DIR/$ASSET_NAME" "$DOWNLOAD_URL"

# 4. Extract
echo "Extracting files..."
tar -xzf "$TEMP_DIR/$ASSET_NAME" -C "$TEMP_DIR"

# 5. OS-specific install
if [ "$OS" = "darwin" ]; then
    if [ "$USE_USER_DIR" = true ]; then
        INSTALL_DIR="$HOME/Applications"
        BIN_DIR="$HOME/.local/bin"
        mkdir -p "$INSTALL_DIR"
        mkdir -p "$BIN_DIR"
    else
        INSTALL_DIR="/Applications"
        BIN_DIR="/usr/local/bin"
    fi
    echo "Copying Fasty.app to $INSTALL_DIR..."

    if [ -w "$INSTALL_DIR" ]; then
        rm -rf "$INSTALL_DIR/Fasty.app"
        mv "$TEMP_DIR/Fasty.app" "$INSTALL_DIR/"
    else
        echo "Administrator privileges (sudo) are required to write to $INSTALL_DIR."
        sudo rm -rf "$INSTALL_DIR/Fasty.app"
        sudo mv "$TEMP_DIR/Fasty.app" "$INSTALL_DIR/"
    fi

    if [ ! -d "$BIN_DIR" ]; then
        if [ -w "$(dirname "$BIN_DIR")" ]; then
            mkdir -p "$BIN_DIR"
        else
            sudo mkdir -p "$BIN_DIR"
        fi
    fi

    echo "Creating symlink at $BIN_DIR/fasty..."
    if [ -w "$BIN_DIR" ]; then
        rm -f "$BIN_DIR/fasty"
        ln -sf "$INSTALL_DIR/Fasty.app/Contents/MacOS/fasty" "$BIN_DIR/fasty"
    else
        sudo rm -f "$BIN_DIR/fasty"
        sudo ln -sf "$INSTALL_DIR/Fasty.app/Contents/MacOS/fasty" "$BIN_DIR/fasty"
    fi

    echo "$APP_NAME installed successfully at $INSTALL_DIR/Fasty.app!"
    echo "You can launch it from Launchpad or by typing '$APP_NAME' in your terminal."

elif [ "$OS" = "linux" ]; then
    if [ "$USE_USER_DIR" = true ]; then
        BIN_DIR="$HOME/.local/bin"
        ICON_DIR="$HOME/.local/share/pixmaps"
        DESKTOP_DIR="$HOME/.local/share/applications"
        mkdir -p "$BIN_DIR"
        mkdir -p "$ICON_DIR"
        mkdir -p "$DESKTOP_DIR"
    else
        BIN_DIR="/usr/local/bin"
        ICON_DIR="/usr/local/share/pixmaps"
        DESKTOP_DIR="/usr/local/share/applications"
    fi

    # Find the binary in case it's in a subfolder within the tar
    SRC_BINARY=$(find "$TEMP_DIR" -maxdepth 2 -type f -name "$APP_NAME" | head -n 1)

    if [ ! -f "$SRC_BINARY" ]; then
        echo "ERROR: Could not find binary '$APP_NAME' in extracted files." >&2
        exit 1
    fi

    if [ ! -d "$BIN_DIR" ]; then
        echo "Creating directory $BIN_DIR..."
        if [ -w "$(dirname "$BIN_DIR")" ]; then
            mkdir -p "$BIN_DIR"
        else
            sudo mkdir -p "$BIN_DIR"
        fi
    fi

    echo "Replacing binary at $BIN_DIR/$APP_NAME..."
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

    # Verify the on-disk binary was actually replaced by comparing sha256.
    NEW_HASH=$(sha256sum "$SRC_BINARY" 2>/dev/null | awk '{print $1}')
    INSTALLED_HASH=$(sha256sum "$BIN_DIR/$APP_NAME" 2>/dev/null | awk '{print $1}')
    if [ -z "$NEW_HASH" ] || [ -z "$INSTALLED_HASH" ] || [ "$NEW_HASH" != "$INSTALLED_HASH" ]; then
        echo "ERROR: the binary at $BIN_DIR/$APP_NAME does not match the new one (copy failed). Aborting." >&2
        exit 1
    fi

    echo "$LATEST_TAG" > /tmp/fasty-update-done 2>/dev/null || true

    echo "Setting up icon and desktop entry for Linux..."
    RAW_ICON_URL="https://raw.githubusercontent.com/$GITHUB_USER/$GITHUB_REPO/main/assets/fastyIcon.png"

    if [ -w "$ICON_DIR" ]; then
        curl -sSfL -o "$ICON_DIR/fasty.png" "$RAW_ICON_URL"
    else
        sudo mkdir -p "$ICON_DIR"
        sudo curl -sSfL -o "$ICON_DIR/fasty.png" "$RAW_ICON_URL"
    fi

    DESKTOP_CONTENT="[Desktop Entry]
Name=Fasty
Comment=GPU-accelerated Terminal Emulator
Exec=$BIN_DIR/$APP_NAME
Icon=$ICON_DIR/fasty.png
Terminal=false
Type=Application
Categories=System;TerminalEmulator;
Keywords=terminal;emulator;wgpu;"

    if [ -w "$DESKTOP_DIR" ]; then
        echo "$DESKTOP_CONTENT" > "$DESKTOP_DIR/fasty.desktop"
    else
        sudo mkdir -p "$DESKTOP_DIR"
        echo "$DESKTOP_CONTENT" | sudo tee "$DESKTOP_DIR/fasty.desktop" > /dev/null
    fi

    echo "$APP_NAME installed successfully at $BIN_DIR/$APP_NAME!"
    echo "You can launch it by searching '$APP_NAME' in your menu or typing it in a terminal."

    # Schedule a self-restart: 3s after this script exits, kill the current
    # fasty process and relaunch it. The deferred subshell survives even when
    # the parent pty is destroyed, so the new fasty comes up cleanly.
    (
        sleep 3
        pkill -x fasty 2>/dev/null || true
        nohup "$BIN_DIR/$APP_NAME" >/dev/null 2>&1 &
        disown
    ) &
    disown
fi
