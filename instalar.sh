#!/usr/bin/env bash
# ==============================================================================
# Script de Instalación para Linux y macOS (Fasty)
# ==============================================================================
# Ejecutar directamente desde internet con:
# curl -fsSL https://raw.githubusercontent.com/diegoleteliers10/fasty/main/instalar.sh | bash
# ==============================================================================

set -euo pipefail

# Procesar argumentos y variables de entorno
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
    if [ "$USE_USER_DIR" = true ]; then
        INSTALL_DIR="$HOME/Applications"
        BIN_DIR="$HOME/.local/bin"
        mkdir -p "$INSTALL_DIR"
        mkdir -p "$BIN_DIR"
    else
        INSTALL_DIR="/Applications"
        BIN_DIR="/usr/local/bin"
    fi
    echo "🚀 Copiando Fasty.app a $INSTALL_DIR..."
    
    if [ -w "$INSTALL_DIR" ]; then
        rm -rf "$INSTALL_DIR/Fasty.app"
        mv "$TEMP_DIR/Fasty.app" "$INSTALL_DIR/"
    else
        echo "🔑 Se requieren privilegios de administrador (sudo) para escribir en $INSTALL_DIR."
        sudo rm -rf "$INSTALL_DIR/Fasty.app"
        sudo mv "$TEMP_DIR/Fasty.app" "$INSTALL_DIR/"
    fi

    # Crear enlace simbólico en BIN_DIR para poder ejecutar 'fasty' desde terminal
    if [ ! -d "$BIN_DIR" ]; then
        if [ -w "$(dirname "$BIN_DIR")" ]; then
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
    
    if [ ! -d "$BIN_DIR" ]; then
        echo "📂 Creando el directorio $BIN_DIR..."
        if [ -w "$(dirname "$BIN_DIR")" ]; then
            mkdir -p "$BIN_DIR"
        else
            sudo mkdir -p "$BIN_DIR"
        fi
    fi

    echo "🚀 Reemplazando el binario en $BIN_DIR/$APP_NAME..."
    # Reemplazo robusto en Linux:
    #   1. rm -f  -> elimina la entrada de directorio del binario viejo.
    #                Funciona aunque el binario esté en ejecución (sólo
    #                remueve el dirent; el inodo sigue vivo mientras el
    #                proceso lo tenga mapeado).
    #   2. cp     -> crea un inodo NUEVO con el contenido nuevo. No
    #                toca el inodo viejo, así que no hay ETXTBSY.
    #   3. chmod  -> fija permisos del archivo recién creado.
    # mv/rename(2) sobre un ejecutable en ejecución falla con ETXTBSY
    # ("Text file busy") en Linux y deja el binario viejo en disco,
    # por eso no usamos mv ni install (que internamente hace rename
    # atómico sobre el destino).
    if [ -w "$BIN_DIR" ]; then
        rm -f "$BIN_DIR/$APP_NAME"
        cp -f "$TEMP_DIR/$APP_NAME" "$BIN_DIR/$APP_NAME"
        chmod 0755 "$BIN_DIR/$APP_NAME"
    else
        echo "🔑 Se requieren privilegios de administrador (sudo) para escribir en $BIN_DIR."
        sudo rm -f "$BIN_DIR/$APP_NAME"
        sudo cp -f "$TEMP_DIR/$APP_NAME" "$BIN_DIR/$APP_NAME"
        sudo chmod 0755 "$BIN_DIR/$APP_NAME"
    fi

    # Verifica que el binario en disco realmente fue reemplazado
    # comparando hashes sha256. Si la copia falló (p.ej. cp no pudo
    # escribir) NO marcamos el update como completado para que el
    # usuario pueda reintentar.
    NEW_HASH=$(sha256sum "$TEMP_DIR/$APP_NAME" 2>/dev/null | awk '{print $1}')
    INSTALLED_HASH=$(sha256sum "$BIN_DIR/$APP_NAME" 2>/dev/null | awk '{print $1}')
    if [ -z "$NEW_HASH" ] || [ -z "$INSTALLED_HASH" ] || [ "$NEW_HASH" != "$INSTALLED_HASH" ]; then
        echo "❌ Error: el binario en $BIN_DIR/$APP_NAME no coincide con el nuevo (copia fallida). Aborta." >&2
        exit 1
    fi

    echo "$LATEST_TAG" > /tmp/fasty-update-done 2>/dev/null || true

    # Configuración de icono PNG y archivo .desktop para menús del sistema
    echo "🎨 Configurando icono y acceso directo de escritorio para Linux..."
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

    # Schedule a self-restart: 3s after this script exits, kill the
    # current fasty and relaunch it. The deferred subshell survives
    # even when the parent pty (the shell that ran this script) is
    # destroyed, so the new fasty comes up cleanly.
    (
        sleep 3
        pkill -x fasty 2>/dev/null || true
        nohup "$BIN_DIR/$APP_NAME" >/dev/null 2>&1 &
        disown
    ) &
    disown
fi
