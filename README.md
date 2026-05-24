# Fasty

Terminal emulator acelerado por GPU construido con **GPUI** (el framework UI de Zed) y **Rust**.

Inspirado en [Ghostty](https://github.com/ghostty-org/ghostty), pero usando GPUI en vez de implementación raw de Vulkan/Metal.

## Arquitectura

```
fasty/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point
│   ├── lib.rs               # Librería principal
│   ├── app.rs               # Estado de la aplicación + rendering
│   ├── config.rs            # Configuración global
│   ├── settings.rs          # Settings del usuario
│   ├── tabs.rs              # Sistema de tabs
│   ├── titlebar.rs          # Titlebar personalizado
│   ├── terminal/
│   │   └── mod.rs           # Grid de celdas + scrollback + SGR
│   ├── parser/
│   │   └── mod.rs           # Parser ANSI/VT100
│   ├── pty/
│   │   └── mod.rs           # PTY process + async reader
│   └── input/
│       └── mod.rs           # KeyEvent → secuencias PTY
```

## Stack Tecnológico

| Componente | Tecnología |
|------------|------------|
| UI Framework | GPUI 0.2.2 |
| Rendering GPU | GPUI (Metal/Vulkan/D3D12) |
| PTY | portable-pty |
| Terminal Parser | alacritty_terminal |
| Async | tokio (via futures) |
| Logging | tracing + tracing-subscriber |

## Dependencias del Sistema

### macOS
```bash
xcode-select --install
```

### Linux (Wayland)
```bash
sudo apt install libvulkan-dev libwayland-dev
```

### Linux (X11)
```bash
sudo apt install libvulkan-dev libx11-dev
```

### Windows
```bash
# Instalar Vulkan SDK
# https://vulkan.lunarg.com/
```

## Build

```bash
# Debug
cargo build

# Release (optimizado)
cargo build --release

# Con features específicas
cargo build --features wayland  # Linux Wayland
cargo build --features x11      # Linux X11
```

## Run

```bash
cargo run
```

## Características

- [x] Rendering acelerado por GPU via GPUI
- [x] Parser ANSI/VT100 (vía alacritty_terminal)
- [x] Soporte SGR completo (colores 16, 256, truecolor)
- [x] Atributos: bold, dim, italic, underline, strikethrough, inverse, hidden, blink
- [x] Scrollback buffer
- [x] PTY con proceso hijo
- [x] Sistema de tabs
- [x] Titlebar personalizado
- [x] Configuración persistente (JSON)

## Próximos Pasos

- [ ] Resize dinámico (reportar tamaño al PTY)
- [ ] Click en URLs
- [ ] Selection con mouse
- [ ] Copy/paste
- [ ] Config file para fonts y colors
- [ ] Search en scrollback
- [ ] Hyperlinks (RFC 6560)
- [ ] Portapapeles integrado (OSC 52)
- [ ] Keybindings configurables

## Inspiración y Recursos

- [Ghostty](https://github.com/ghostty-org/ghostty) — Terminal de Mitchell Hedaberg
- [Zed](https://zed.dev) — Editor de código que usa GPUI
- [GPUI](https://gpui.rs) — Documentación oficial
- [alacritty_terminal](https://github.com/alacritty/alacritty) — Terminal que usa el mismo parser
- [The TTY demystified](http://www.linasakesson.net/programming/tty/) — Explicación profunda de PTY/TTY