# fastty — Status & Pendientes

**Última actualización:** 2026-10-25
**Render:** GPUI (Zed) — macOS primero; Linux/Windows funcionales

> La versión anterior de este documento describía un renderer wgpu/winit
> (src/renderer/pipeline.rs, src/chrome_layout.rs) que ya no existe: la app
> compila hoy contra GPUI y el chrome se dibuja con `div()` (src/ui/). Los
> archivos muertos se eliminaron en octubre de 2026.

---

## 1. Startup

Cold start ~129ms → Warm ~79ms (verificado en M1, 2026-08):

| Cambio | Archivo |
|--------|---------|
| Fontconfig (fc-list/fc-match) solo en Linux | `src/font_discovery_macos.rs` |
| Backends específicos por plataforma | selección de plataforma en bootstrap |
| Shell integration escrita una vez por proceso (`OnceLock`) | `src/terminal_state.rs` |

---

## 2. Arquitectura (mapa rápido)

| Área | Archivos |
|------|----------|
| UI (tabs, splits, palette, settings, status bar) | `src/ui/*.rs`, `src/pane_tree.rs` |
| Terminal (PTY, parser OSC/CSI/DCS, imágenes, snapshot FST1) | `src/terminal_state.rs`, `src/parser/`, `src/server/` |
| Daemon (socket, protocolo NDJSON, sesiones host) | `src/daemon.rs`, `src/session_host.rs`, `src/protocol_compat.rs` |
| CLI (sessions/attach/spawn/write/capture/host/agent-status) | `src/main.rs`, `src/daemon_client.rs`, `src/host_service.rs` |
| Gateway web (HTTP/WS embebido + WASM) | `src/gateway.rs`, `crates/fastty-wasm/`, `web/` |
| MCP | `src/mcp.rs` |
| IA (sidebar, providers, tools, auto-allow) | `src/ai/` |

---

## 3. Pendientes / deuda conocida

| # | Tema | Estado |
|---|------|--------|
| 1 | sixel: el gateway web no muestra imágenes (FST1 lleva solo celdas). Falta canal de imágenes en el snapshot o en el WS. | Abierto |
| 2 | Windows: daemon sin named pipes (`start()` es no-op en `#[cfg(not(unix))]`). Requiere runner Windows en CI para verificarse. | Diferido |
| 3 | Kitty keyboard: el cliente WASM (`encode_key` en `crates/fastty-wasm`) sigue con encoding legacy; el encoder nuevo (`src/kitty_keyboard.rs`) no está compartido como crate. | Abierto |
| 4 | `src/ui/root_view.rs` (~21k líneas) necesita partición por dominio (input, overlays, render). | Abierto |
| 5 | Dock badge con porcentaje para OSC 9;4 (hoy: solo indicador en tab). | Abierto |
| 6 | Eventos kitty release: verificar comportamiento de `on_key_up` con IME activo y en Linux/Windows. | Abierto |

---

## 4. Verificación

```bash
cargo test                # 545+ tests
cargo build --release     # ~54s en M1
./target/release/fastty   # smoke test
```

Pruebas manuales de protocolo:

```sh
# kitty keyboard: consultar flags (responde CSI ? flags u)
printf '\x1b[?u'   # dentro de un pane, con una app que lo use
# progress en tab:
printf '\x1b]9;4;1;50\x1b\\'
# sixel:
chafa --format sixel imagen.png
```
