#!/usr/bin/env bash
# ==============================================================================
# Script de Instalación para Linux y macOS (Fasty)
# ==============================================================================
# Ejecutar directamente desde internet con:
# curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.sh | bash
# ==============================================================================

set -euo pipefail

# CONFIGURACIÓN
GITHUB_USER="diegoleteliers10"
GITHUB_REPO="fasty"
APP_NAME="fasty"

echo "=== Iniciando instalación de $APP_NAME ==="

# 1. Detección automática del Sistema Operativo y la Arquitectura
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
    linux)
        if [ "$ARCH" = "x86_64" ]; then
            TARGET="x86_64-unknown-linux-gnu"
        else
            echo "❌ Error: La arquitectura Linux '$ARCH' no está soportada actualmente." >&2
            exit 1
        fi
        ;;
    darwin)
        if [ "$ARCH" = "x86_64" ]; then
            TARGET="x86_64-apple-darwin"
        elif [ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ]; then
            TARGET="aarch64-apple-darwin"
        else
            echo "❌ Error: La arquitectura macOS '$ARCH' no está soportada." >&2
            exit 1
        fi
        ;;
    *)
        echo "❌ Error: El sistema operativo '$OS' no es compatible con este script." >&2
        exit 1
        ;;
esac

echo "✅ Plataforma detectada: OS=$OS, Arch=$ARCH -> Target=$TARGET"

# 2. Consultar la API pública de GitHub para obtener la última release
echo "🔍 Consultando la última versión disponible en GitHub..."
API_URL="https://api.github.com/repos/$GITHUB_USER/$GITHUB_REPO/releases/latest"

LATEST_TAG=$(curl -sSf "$API_URL" | grep '"tag_name":' | sed -E 's/.*"tag_name":\s*"([^"]+)".*/\1/')

if [ -z "$LATEST_TAG" ]; then
    echo "❌ Error: No se pudo obtener la información de la última versión desde GitHub." >&2
    exit 1
fi

echo "📦 Última versión encontrada: $LATEST_TAG"

# 3. Descargar el archivo .tar.gz correcto
ASSET_NAME="$APP_NAME-$TARGET.tar.gz"
DOWNLOAD_URL="https://github.com/$GITHUB_USER/$GITHUB_REPO/releases/download/$LATEST_TAG/$ASSET_NAME"

TEMP_DIR=$(mktemp -d -t install-$APP_NAME.XXXXXX)
trap 'rm -rf "$TEMP_DIR"' EXIT

echo "📥 Descargando $ASSET_NAME..."
curl -sSL -o "$TEMP_DIR/$ASSET_NAME" "$DOWNLOAD_URL"

# 4. Descomprimir el binario/app bundle
echo "🔓 Descomprimiendo archivos..."
tar -xzf "$TEMP_DIR/$ASSET_NAME" -C "$TEMP_DIR"

# 5. Instalación específica por Sistema Operativo
if [ "$OS" = "darwin" ]; then
    # --- macOS: Instalación en /Applications con soporte de Launcher/Launchpad ---
    INSTALL_DIR="/Applications"
    echo "🚀 Copiando Fasty.app a $INSTALL_DIR..."
    
    if [ -w "$INSTALL_DIR" ]; then
        rm -rf "$INSTALL_DIR/Fasty.app"
        mv "$TEMP_DIR/Fasty.app" "$INSTALL_DIR/"
    else
        echo "🔑 Se requieren privilegios de administrador (sudo) para escribir en $INSTALL_DIR."
        sudo rm -rf "$INSTALL_DIR/Fasty.app"
        sudo mv "$TEMP_DIR/Fasty.app" "$INSTALL_DIR/"
    fi

    # Crear enlace simbólico en /usr/local/bin para poder ejecutar 'fasty' desde terminal
    BIN_DIR="/usr/local/bin"
    if [ ! -d "$BIN_DIR" ]; then
        if [ -w "/usr/local" ]; then
            mkdir -p "$BIN_DIR"
        else
            sudo mkdir -p "$BIN_DIR"
        fi
    fi

    echo "🔗 Creando enlace simbólico en $BIN_DIR/fasty..."
    if [ -w "$BIN_DIR" ]; then
        rm -f "$BIN_DIR/fasty"
        ln -sf "$INSTALL_DIR/Fasty.app/Contents/MacOS/fasty" "$BIN_DIR/fasty"
    else
        sudo rm -f "$BIN_DIR/fasty"
        sudo ln -sf "$INSTALL_DIR/Fasty.app/Contents/MacOS/fasty" "$BIN_DIR/fasty"
    fi

    echo "🎉 ¡$APP_NAME instalado con éxito en $INSTALL_DIR/Fasty.app!"
    echo "💡 Puedes iniciarlo desde tu Launchpad o escribiendo '$APP_NAME' en tu terminal."

elif [ "$OS" = "linux" ]; then
    # --- Linux: Mover binario y configurar lanzador de escritorio .desktop ---
    BIN_DIR="/usr/local/bin"
    
    if [ ! -d "$BIN_DIR" ]; then
        echo "📂 Creando el directorio $BIN_DIR..."
        if [ -w "/usr/local" ]; then
            mkdir -p "$BIN_DIR"
        else
            sudo mkdir -p "$BIN_DIR"
        fi
    fi

    echo "🚀 Copiando el binario a $BIN_DIR/$APP_NAME..."
    if [ -w "$BIN_DIR" ]; then
        mv "$TEMP_DIR/$APP_NAME" "$BIN_DIR/$APP_NAME"
        chmod +x "$BIN_DIR/$APP_NAME"
    else
        echo "🔑 Se requieren privilegios de administrador (sudo) para escribir en $BIN_DIR."
        sudo mv "$TEMP_DIR/$APP_NAME" "$BIN_DIR/$APP_NAME"
        sudo chmod +x "$BIN_DIR/$APP_NAME"
    fi

    # Configuración de icono PNG y archivo .desktop para menús del sistema
    echo "🎨 Configurando icono y acceso directo de escritorio para Linux..."
    ICON_DIR="/usr/local/share/pixmaps"
    DESKTOP_DIR="/usr/local/share/applications"
    RAW_ICON_URL="https://raw.githubusercontent.com/$GITHUB_USER/$GITHUB_REPO/main/assets/fastyIcon.png"

    # Descargar el icono PNG
    if [ -w "$ICON_DIR" ]; then
        curl -sSL -o "$ICON_DIR/fasty.png" "$RAW_ICON_URL"
    else
        sudo mkdir -p "$ICON_DIR"
        sudo curl -sSL -o "$ICON_DIR/fasty.png" "$RAW_ICON_URL"
    fi

    # Crear lanzador .desktop
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

    echo "🎉 ¡$APP_NAME instalado con éxito en $BIN_DIR/$APP_NAME!"
    echo "💡 Puedes ejecutarlo buscando '$APP_NAME' en tu menú o escribiéndolo en terminal."
fi
