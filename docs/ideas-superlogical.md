# Ideas para fastty desde Superlogical (Mitchell Hashimoto)

**Fuente:** timeline de https://x.com/mitchellh, julio-agosto 2026, scrapeado con aside.
**Qué es Superlogical:** nueva compañía de Mitchell Hashimoto (creador de Ghostty). Empieza con un multiplexer de terminal con arquitectura distinta a tmux/zellij. El multiplexer es gratis; el negocio viene de lo que sigue (productos no-terminal).

---

## Lo que anunció Superlogical (cronología)

| Fecha | Publicación | Detalle |
|-------|-------------|---------|
| 2026-07-29 | Anuncio de la compañía | Multiplexer de terminal como punto de partida. Arquitectura "radicalmente distinta" a tmux/zellij: un servidor mux que es dueño del SSH, navegación desde una sola app o interfaz de browser. |
| 2026-07-30 | Integración Tailscale | Servidores se unen a tailscale automáticamente y se exponen como servicios; clientes los descubren con una sola config. |
| 2026-07-30 | Kitty Graphics | IO throughput mejorado ~25x en Ghostty nightly. En Superlogical "abusan" de Kitty graphics para hacer cosas de UI (previews, etc.). |
| 2026-07-30 | Video de arquitectura | Comparación con tmux/zellij. El servidor ni siquiera "conoce" terminales: multiplexa más que terminales. |
| 2026-08-04 | Deck icons | Iconos dinámicos según qué corre en cada sesión del multiplexer. SuperSplit (splits), SuperTabs (kit nativo de tabs propio), tabs verticales ya funcionando. |
| 2026-08-12 | Tab peek | Gesto de 3 dedos hacia abajo: previews en vivo de las otras tabs (renderizadas en Metal, incluyen splits). Si seguís arrastrando, entrás a un modo estilo Mission Control. |
| 2026-08-13 | Keyboard-first | Todo manejable por teclado. |
| 2026-08-13 | API + CLI | API para que terceros (ej. ghostel/Emacs sobre libghostty) muestren sesiones de Superlogical dentro de otra app. CLI "muy familiar para usuarios de tmux/zellij, con diferencias, fully unix-y". |
| 2026-08-13 | Wayland | Cada tab/ventana es su propia ventana Wayland y delegan en el window manager nativo en vez de reimplementar. |
| 2026-08-14 | libghostty-vt (Wasm) | Terminal embebible en Wasm comparado con xterm.js: mejor I/O, reflow y rendering. Cliente web de Superlogical ya funciona con libghostty. |
| 2026-08-17 | Memoria Wasm | Reemplazaron `std.heap.MemoryPool` de Zig por un pool custom para Wasm: -75% de memoria del terminal. |
| 2026-08-18 | Clientes sincronizados | Web + desktop sincronizados, móvil Apple en camino. Nada de wrapper multiplataforma: herramientas nativas por ecosistema (Swift/Apple frameworks). "Uncompromisingly native". |
| 2026-08-28 | Demo de velocidad | Énfasis en que el multiplexer es "el que no tenés que aprender: simplemente funciona". Scrollback default de libghostty: 10k BYTES (bug de config, lo van a subir). |
| 2026-08-29 | Resize multi-cliente | Hoy "last write wins" como tmux, pero temporal: pueden streamear la terminal redimensionada en background, mostrarla a cualquier tamaño y dar un botón "resync". Clientes de cualquier tamaño. |
| 2026-08-29 | Scrollback nativo | El buffer es local al cliente, no del server. Impacta búsqueda y accesibilidad. Performance IO "múltiplos por encima" de benchmarks de Alacritty/Kitty. |
| 2026-08-30 | Migración de configs | Detección automática de tmux y otros muxers/configs de terminal para ofrecer adoptar esos bindings como propios. Presets de compatibilidad. |

---

## Ideas concretas para fastty

Ordenadas por valor vs. esfuerzo, considerando que fastty ya tiene: tabs, status bar con git, command palette, SSH manager, TOML con live reload, temas.

### Alto valor, bajo esfuerzo

1. **Deck icons dinámicos.** [✅ Implementado] Los tabs y sidebar de fastty detectan el proceso en foreground (`vim`, `node`, `cargo`, `ssh`, `git`, `docker`, `python`, etc.) y muestran su icono respectivo.
2. **Detección y adopción de configs ajenas.** [✅ Implementado] Detección automática en Settings de `~/.tmux.conf`, configs de Ghostty, Alacritty, Kitty y WezTerm con importación 1-click a `fastty.toml`.
3. **Presets de keybindings.** [✅ Implementado] Presets integrados (`Default`, `Ghostty`, `tmux`, `iTerm2`) seleccionables desde Settings con resolución dinámica.
4. **Subir el default de scrollback y hacerlo config.** [✅ Implementado] Default elevado a 10.000 líneas (hasta 100.000 max), configurable en TOML y Settings con indicador de consumo de memoria RAM estimada.

### Alto valor, esfuerzo medio

5. **Tab peek / vista Mission Control.** [✅ Implementado] Modo overlay (`⌘⇧O` / `Ctrl+Shift+M`) con cuadrícula de miniaturas en vivo de cada pestaña y splits, icono de proceso, branch Git y navegación por teclado/mouse.
6. **Scrollback local con búsqueda unificada.** [✅ Implementado] Búsqueda global multi-tab (`⌘⇧F` / `Ctrl+Shift+F`) sobre el buffer de todas las pestañas y splits simultáneamente con fuzzy matching (`fff-search`) y salto directo.
7. **Multiplexado ligero estilo attach.** [✅ Implementado] Persistencia y restauración de sesiones/workspaces completos (con árbol de splits, paneles y CWDs) en `~/.config/fastty/sessions/`.
8. **SSH manager con mux semantics.** [✅ Implementado] Agrupación por tags/entornos (`#prod`, `#dev`, `#aws`, etc.), detección de sesiones activas (`● Active`), TCP keepalive automático y reconexión interactiva ante desconexión con tecla `r`.
9. **Gráficos Kitty protocol.** [✅ Implementado] Parser de protocolo de gráficos Kitty (`\x1b_G...`) con renderizado GPU directo en GPUI para imágenes inline PNG/RGBA/RGB en la cuadrícula de terminal.

### Apuesta mayor (Roadmap)

10. **Cliente web embebible.** [⏳ Roadmap] Compilar el core de emulación a Wasm para sincronización con visor web.
11. **API para embeber terminales fastty.** [⏳ Roadmap] IPC daemon para embeber sesiones dentro de editores (Zed, Neovim via plugin).

---

## Principios que Mitchell repite (y que fastty puede adoptar)

- **Cero fricción de aprendizaje.** "El mejor multiplexer es el que no tenés que aprender." Cada feature nueva de fastty debería funcionar sin leer docs.
- **Nativo sin compromiso.** Nada de parity forzada: usar lo mejor de cada plataforma (en fastty: GPUI ya es la apuesta nativa; seguir ahí en vez de abstraer).
- **Keyboard-first, mouse opcional.** Toda acción de UI alcanzable por teclado.
- **El buffer vive en el cliente.** Scrollback, búsqueda y accesibilidad mejoran si el estado es local.
- **Migrar, no competir por config.** Detectar la config existente del usuario y ofrecérsela, en vez de pedirle que aprenda la propia.
