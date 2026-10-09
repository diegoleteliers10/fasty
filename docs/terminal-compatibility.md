# Compatibilidad de terminal

Esta tabla describe la compatibilidad que muestra el código actual de Fastty. «Parcial» indica que el protocolo funciona con límites o que falta una parte importante.

| Protocolo o función | Estado | Evidencia y límites |
|---|---|---|
| `TERM` y color verdadero | Soportado | Fastty define `TERM=xterm-256color` y `COLORTERM=truecolor`. El parser de Alacritty admite color RGB de 24 bits. `TERM` no anuncia extensiones específicas de Fastty. ([terminal_state.rs](../src/terminal_state.rs)) |
| Hooks de shell: Bash, Zsh, Fish, PowerShell y Nushell | Parcial | Fastty carga integración OSC 133 en el shell interactivo predeterminado y conserva los perfiles del usuario. La integración depende de cada shell. Bash no reemplaza un trap `DEBUG` existente en versiones anteriores a 4.4. Los argumentos explícitos de shell omiten la integración. ([shell_integration.rs](../src/shell_integration.rs), [terminal_state.rs](../src/terminal_state.rs)) |
| OSC 133 y OSC 633 | Parcial | El parser reconoce A/B/C/D y el código de salida en D; ambos códigos actualizan el estado y la duración de comandos. OSC 633 no añade la integración con VS Code completa. Los hooks actuales solo generan OSC 133. ([terminal_state.rs](../src/terminal_state.rs)) |
| OSC 8, enlaces | Soportado | Alacritty VTE almacena los enlaces OSC 8 por celda. Fastty detecta el enlace bajo el cursor y lo muestra como URL activa. ([root_view.rs](../src/ui/root_view.rs)) |
| Gráficos Kitty | Parcial | Fastty decodifica y muestra imágenes, con transmisión directa, por archivo y por archivo temporal, fragmentos y operaciones de ubicación/borrado. El soporte no cubre todas las opciones del protocolo Kitty. ([kitty_graphics.rs](../src/parser/kitty_graphics.rs), [terminal_state.rs](../src/terminal_state.rs)) |
| Portapapeles Kitty, OSC 5522 | Parcial | Fastty admite escritura y lectura de texto e imágenes. La lectura está habilitada por defecto en la configuración y se puede desactivar con `clipboard_read`. ([config.rs](../src/config.rs), [kitty_clipboard.rs](../src/parser/kitty_clipboard.rs)) |
| Ratón y SGR 1006 | Soportado | Fastty informa clic, arrastre, movimiento, rueda y modificadores en X10/1000/1002/1003/1005/1006. Restablece los modos al volver al prompt. ([terminal_state.rs](../src/terminal_state.rs), [root_view.rs](../src/ui/root_view.rs)) |
| Eventos de foco, modo 1004 | Soportado | Fastty reconoce el modo y envía foco dentro/fuera cuando cambia el foco de la terminal. ([parser/mod.rs](../src/parser/mod.rs), [terminal_state.rs](../src/terminal_state.rs)) |
| Pegado entre corchetes, modo 2004 | Soportado | Fastty reconoce el modo y encierra el texto pegado con las secuencias de inicio y fin. ([parser/mod.rs](../src/parser/mod.rs), [terminal_state.rs](../src/terminal_state.rs)) |
| Salida sincronizada, modo 2026 | Soportado | Fastty detecta el inicio y fin del modo 2026 y agrupa las actualizaciones. Un límite de 150 ms libera una actualización si la aplicación no envía el fin del modo. ([terminal_state.rs](../src/terminal_state.rs)) |
| Protocolo de teclado Kitty | Parcial | Fastty activa los modos Kitty y envía secuencias `CSI u` cuando la aplicación activa el modo correspondiente. Admite teclas Unicode comunes, teclas funcionales y eventos de pulsación, repetición y liberación. No implementa todos los códigos de tecla ni todos los casos de texto asociado. ([terminal_state.rs](../src/terminal_state.rs), [kitty_keyboard.rs](../src/kitty_keyboard.rs)) |
| Sixel | No soportado | No hay parser DCS Sixel ni decodificador Sixel en el código. Añadirlo requiere reconocer el flujo DCS `q`, decodificar los datos comprimidos y de color, ubicar la imagen en la cuadrícula y limitar dimensiones, memoria y trabajo de decodificación. Es una función separada de los gráficos Kitty. |

## Auditoría de Sixel y matriz

La búsqueda del código actual no encuentra una ruta Sixel. La matriz cubre funciones que se pueden confirmar en el código. No representa una certificación de compatibilidad con Ghostty, Warp o Alacritty. Una matriz completa requiere casos por protocolo para el parser, el estado del terminal, la entrada y el renderizado. Fastty tampoco incluye todavía una auditoría general de secuencias VT.
