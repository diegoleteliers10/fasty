# Biovity Terminal

GPU-native terminal emulator powered by **GPUI** (Zed's UI framework) y **Rust**.

Inspirado en [Ghostty](https://github.com/ghostty-org/ghostty), pero usando GPUI en vez de implementación raw de Vulkan/Metal.

## Arquitectura

```
biovity-terminal/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point — inicializa GPUI app
│   ├── app/
│   │   └── mod.rs           # AppState — estado de la ventana terminal
│   │                         #   + Render trait (GPUI rendering)
│   │                         #   + EntityInputHandler (keyboard/mouse)
│   ├── terminal/
│   │   └── mod.rs           # Grid de celdas + scrollback
│   │                         #   + Cell, Color, CellAttrs
│   │                         #   + SGR (colores, bold, italic, etc)
│   ├── parser/
│   │   └── mod.rs           # Parser ANSI/VT100 state machine
│   │                         #   + CSI sequences (CUU, CUD, SGR, etc)
│   │                         #   + OSC strings (window title)
│   ├── pty/
│   │   └── mod.rs           # PTY process + async reader
│   └── input/
│       └── mod.rs           # KeyEvent → secuencias PTY
│
├── docs/
│   └── architecture.svg      # Diagrama de arquitectura
```

## Stack Tecnológico

| Componente | Tecnología | Alternativas |
|------------|------------|---------------|
| UI Framework | GPUI 0.2.2 | — |
| Rendering GPU | GPUI (Metal/Vulkan/D3D12) | wgpu raw |
| PTY | portable-pty | rustix + manual |
| Terminal Parser | Custom (state machine) | vte |
| Font Shaping | GPUI text system | harfbuzz单独 |
| Async | tokio | smol, async-std |

## Dependencias del Sistema

### macOS
```bash
xcode-select --install
```

### Linux (Wayland)
```bash
# Instalar librerías de desarrollo Vulkan
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

# Release
cargo build --release

# Con features específicas
cargo build --features wayland  # Linux Wayland
cargo build --features x11     # Linux X11
```

## Run

```bash
cargo run
```

## Diseño de GPUI

GPUI usa un modelo híbrido immediate/retained:

1. **`Render` trait**: Cada frame, GPUI llama `render()` en la root view
2. **Element tree**: `render()` construye un árbol de elementos (div, canvas, text)
3. **Layout**: Taffy (layout engine tipo CSS) calcula posiciones
4. **Paint**: GPUI convierte elementos a draw calls GPU
5. **Prepaint/Paint separation**: Prepaint prepara datos, Paint dibuja

### Para Terminal Rendering

Para un terminal necesitamos control fino → usamos `canvas()` element:

```rust
canvas(
    prepaint: |bounds, window, cx| {
        // Preparar datos para este frame
        (grid.clone(), cursor, selection)
    },
    paint: |bounds, state, window, cx| {
        // Dibujar usando GPUI paint API
        window.paint_quad(...);
        window.paint_path(...);
    },
)
```

## Parser ANSI

State machine que maneja:

- **C0 controls**: BS, LF, CR, TAB, BEL, etc
- **CSI sequences**: CUP, SGR, ED, EL, CUU/CUD/CUF/CUB, DECSET/DECRST
- **OSC strings**: Window title, color palette
- **DCS**: Device control strings

### Secuencias CSI implementadas

| Secuencia | Nombre | Descripción |
|-----------|--------|-------------|
| `CSI A` | CUU | Cursor up |
| `CSI B` | CUD | Cursor down |
| `CSI C` | CUF | Cursor forward |
| `CSI D` | CUB | Cursor back |
| `CSI H` | CUP | Cursor position |
| `CSI J` | ED | Erase display |
| `CSI K` | EL | Erase line |
| `CSI m` | SGR | Graphic rendition |
| `CSI S` | SU | Scroll up |
| `CSI T` | SD | Scroll down |

## SGR (Select Graphic Rendition)

Soporta todos los modos estándar:

- **Colores 16**: ANSI 16-color palette
- **Colores 256**: Palette index (0-255)
- **Truecolor**: 24-bit RGB
- **Atributos**: bold, dim, italic, underline, strikethrough, inverse, hidden, blink

## Scrollback

- 10,000 líneas de scrollback buffer
- Se preserva al hacer resize de ventana
- Scroll del mouse envía secuencias PTY (no se intercepta scroll del trackpad)

## Próximos Pasos

- [ ] Implementar resize dinámico (CSI resize reporting)
- [ ] Click en URLs para abrirlas
- [ ] Selection con mouse (arrastrar para seleccionar)
- [ ] Copy/paste con selección
- [ ] Config file para fonts y colors
- [ ] Tabs (múltiples terminales)
- [ ] Search en scrollback
- [ ] Hyperlinks (terminal hyperlinks RFC 6560)
- [ ] Portapapeles integrado (osc 52)
- [ ] Configurable keybindings

## Inspiración y Recursos

- [Ghostty](https://github.com/ghostty-org/ghostty) — Terminal de Mitchell Hedaberg
- [Zed](https://zed.dev) — Editor de código que usa GPUI
- [GPUI](https://gpui.rs) — Documentación oficial
- [vte](https://docs.rs/vte/latest/vte/) — Parser VT100 para referencia
- [The TTY demystified](http://www.linusakesson.net/programming/tty/) — Explicación profunda de PTY/TTY
