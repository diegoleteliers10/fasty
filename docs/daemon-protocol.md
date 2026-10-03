# fasttyd — protocolo del daemon local (v1)

Implementación: `src/daemon.rs`. Habilita el ítem 10/11 del roadmap
(`docs/plans/ideas-superlogical.md`): listar y adjuntarse a sesiones de
fastty desde otro proceso (plugin de Neovim, extensión de Zed, cliente web
Wasm) sin quitarle la pestaña a la ventana GPUI.

## Transporte

- Un socket Unix por usuario en `<state_dir>/fasttyd.sock`:
  - macOS: `~/Library/Application Support/fastty/fasttyd.sock`
  - Linux: `$XDG_STATE_HOME/fastty/fasttyd.sock` (o `~/.local/state/fastty/...`)
- Permisos `0600` (solo el usuario que corre fastty puede conectarse — el
  socket puede escribir en el PTY de cualquier sesión registrada).
- **Windows: no implementado.** `daemon::start()` es un no-op ahí; no hay
  pipes con nombre todavía.
- El registro de sesiones vive solo en memoria del proceso `fastty` que
  está corriendo. No hay estado entre reinicios ni recuperación de sesiones
  huérfanas — cuando fastty cierra, el socket y el registro desaparecen con
  él.

## Mensajes

Un objeto JSON por línea (`\n`-terminated) en ambas direcciones.

### Requests (cliente → daemon)

```jsonc
{"cmd": "hello"}
{"cmd": "hello", "read_only": true}
{"cmd": "list"}
{"cmd": "subscribe_sessions"}
{"cmd": "attach", "id": 1}
{"cmd": "attach", "id": 1, "mode": "read_only"}
{"cmd": "detach", "id": 1}
{"cmd": "write", "id": 1, "data": "<base64>"}
{"cmd": "resize", "id": 1, "cols": 80, "rows": 24}
{"cmd": "spawn", "command": "zsh", "args": ["-l"], "cwd": "/home/user", "cols": 80, "rows": 24, "open": true}
{"cmd": "close", "id": 1}
{"cmd": "close", "id": 1, "force": true}
{"cmd": "split_pane", "id": 1, "direction": "right", "cwd": "/home/user"}
{"cmd": "resize_pane", "id": 3, "direction": "left", "delta": 0.05}
{"cmd": "focus_pane", "id": 3}
{"cmd": "layout"}
{"cmd": "resize_window", "cols": 120, "rows": 40}
```

- `id` es el `PaneId` de fastty (un `usize`, único por proceso — cada split
  cuenta como su propia sesión, no solo cada tab).
- `hello` es opcional pero recomendado como primer mensaje: te confirma que
  hablás con la versión de protocolo que esperás antes de mandar nada más. Acepta
  el campo opcional `"read_only": true` para fijar la conexión completa en modo
  solo-lectura (sticky). En una conexión `read_only`, cualquier llamada a `write`,
  `spawn` o `close` es rechazada con código de error `"read_only"`.
- `subscribe_sessions`: inicia una suscripción push a los cambios de la
  lista de sesiones. Envía un evento inicial `sessions` con el estado actual,
  seguido de eventos `session_added`, `session_removed`, y `session_updated`
  a medida que cambian las pestañas/splits en fastty.
- `attach`: se adjunta a una sesión. Acepta el campo opcional `mode`:
  `"read_write"` (por defecto) o `"read_only"`. En modo `"read_only"`, cualquier
  `write` posterior a esa sesión en esa conexión será rechazado con error
  `read_only`.
- `data` en `write` son bytes crudos en base64 (para poder mandar teclas de
  control, secuencias de escape, etc. sin pelear con el escaping de JSON).
- `detach` es por sesión **y por conexión**: solo corta el streaming de esa
  sesión en la conexión que lo pide, no afecta a otros clientes adjuntados
  a la misma sesión desde otra conexión.
- `resize`: actualiza dinámicamente las dimensiones en columnas y filas de la
  terminal (`cols`, `rows`) y emite `SIGWINCH` en el kernel PTY.
- `spawn`: crea una nueva sesión de terminal headless/remota y devuelve `{"event": "spawned", "id": <id>, "opened": <bool>}`. Con `"open": true` la sesión además se abre como un tab visible en la ventana de fastty; `"opened"` en la respuesta indica si eso sucedió (`false` cuando no hay ventana corriendo o no se pidió `open`). La sesión sigue siendo la misma — mismo `id`, cerrable vía `close` — y el renderizado la redimensiona a la geometría real del tab.
- `close`: cierra y desregistra la sesión especificada por `id`. Las sesiones headless (creadas vía `spawn`, con o sin `open`) se cierran directo. Las sesiones GUI abiertas por el usuario en la ventana devuelven error `not_closable` — son tabs del usuario — salvo que se mande `"force": true`, que mata su proceso (igual que cerrar el tab en la ventana) y la desregistra.
- `split_pane`: parte un pane visible de la ventana creando un pane nuevo a su lado (mismo tab, `"direction"`: `left`/`right`/`top`/`down`) y devuelve `{"event": "pane_split", "id": <nuevo id>}`. El pane nuevo es una sesión GUI normal (list/write/close funcionan). `command`/`args`/`cwd` opcionales (por defecto el shell del usuario y el cwd del pane objetivo). Sin ventana corriendo: error `no_gui`.
- `resize_pane`: mueve el divisor del lado indicado (`"direction"`) del pane, `delta` es una fracción del eje del split (default 0.05, se clampea a ±0.5). Responde `done` con `cmd: "resize_pane"`.
- `focus_pane`: trae el tab del pane al frente y lo hace activo, para que el usuario vea lo que el agente está tocando.
- `layout`: devuelve `{"event": "layout", "tabs": [...]}` con la estructura completa de la ventana — cada tab con sus panes (id, title, cwd, activo, tamaño). Sesiones headless sin tab no aparecen aquí; están en `list`.
- `resize_window`: redimensiona la ventana de fastty aproximándose a `cols` x `rows` del pane activo (con splits es aproximado; para mover divisores usar `resize_pane`). Responde `done` con `cmd: "resize_window"`.
- `binary_snapshot`: solicita un snapshot binario ultrarrápido con cabecera `FST1` (comprimido con Deflate).

### Responses (daemon → cliente)

```jsonc
{"event": "hello", "version": 1, "fastty_version": "0.7.6"}
{"event": "sessions", "sessions": [
  {"id": 1, "title": "zsh", "cwd": "/Users/you", "cols": 89, "rows": 32, "alive": true, "history_size": 0}
]}
{"event": "session_added", "session": {"id": 2, "title": "zsh", "cwd": "/tmp", "cols": 89, "rows": 32, "alive": true, "history_size": 0}}
{"event": "session_removed", "id": 2}
{"event": "session_updated", "session": {"id": 1, "title": "vim", "cwd": "/Users/you", "cols": 89, "rows": 32, "alive": true, "history_size": 120}}
{"event": "attached", "id": 1, "cols": 89, "rows": 32, "mode": "read_only"}
{"event": "snapshot", "id": 1, "data": "<base64, ANSI/SGR>"}
{"event": "binary_snapshot", "id": 1, "data": "<base64, FST1 binary snapshot>"}
{"event": "detached", "id": 1}
{"event": "closed", "id": 1}
{"event": "output", "id": 1, "data": "<base64>"}
{"event": "error", "code": "read_only", "message": "attached to session 1 in read-only mode on this connection"}
```

- `title` es el nombre del proceso en foreground de la sesión (mismo dato
  que ya usan los deck icons), o `"shell"` si no se pudo determinar.
- `alive` refleja si el proceso de shell de la sesión sigue vivo
  (`TerminalState::is_alive`) — no si tiene un comando corriendo en este
  instante.
- Un `attach` manda **un `snapshot` inmediatamente después del `attached`**,
  con el contenido actual de la pantalla (no el scrollback completo, solo lo
  visible) como bytes ANSI/SGR ya coloreados — ver
  `TerminalState::snapshot_ansi`. Un cliente que no le importe la
  distinción puede tratarlo exactamente igual que un `output`. Los colores
  con nombre que dependen del tema activo de fastty
  (`Foreground`/`Background`/`Cursor`/variantes `Dim*`) caen al color por
  defecto del terminal del cliente — el daemon no tiene el tema resuelto en
  ese punto.
- `closed` se manda si la sesión a la que estabas adjuntado se cierra
  (`close_tab`/`close_active_pane`) mientras el `attach` seguía activo. No
  va a llegar más `output` para ese `id` en esa conexión después de esto.
- Los códigos de `error` son estables y pensados para matchear
  programáticamente, no solo mostrar: `bad_request`, `no_such_session`,
  `invalid_base64`, `not_attached`, `read_only`, `unsupported`. Pueden
  agregarse códigos nuevos con el tiempo; un cliente debería tratar uno que
  no reconoce igual que una falla genérica.

## Cliente de referencia en el propio binario

`fastty sessions` y `fastty attach <id>` son clientes de referencia
integrados:

```bash
# Listar sesiones activas
fastty sessions

# Observar cambios en vivo (subscribe_sessions)
fastty sessions --watch

# Esperar a que fastty inicie si aún no está corriendo
fastty sessions --wait=10

# Adjuntarse interactivamente (read-write)
fastty attach 1

# Adjuntarse en modo solo lectura (preview)
fastty attach 1 --read-only

# Esperar hasta que la sesión exista
fastty attach 1 --wait=30
```

`fastty attach` muestra el snapshot ANSI de la pantalla actual, streamea el
output en vivo, y reenvía todo lo que tipeés por stdin como `write` a la
sesión real (poniendo la terminal local en modo raw, igual que `ssh`/`tmux attach`).
`Ctrl+\` hace `detach` limpio sin cerrar la sesión. En modo `--read-only`, el input
local se descarta excepto por `Ctrl+\` para desadjuntarse.

### Control desde el CLI (`spawn` / `close` / `split` / `resize` / `write` / `list`)

El mismo binario expone subcomandos de control sobre el daemon, pensados
para scripts y agentes:

```bash
# Crear una sesión headless (imprime el id; sin comando corre el shell)
id=$(fastty spawn --cwd ~/api -- bun run dev)

# Escribir en su PTY (con --enter agrega \r y submita la línea)
fastty write $id --enter 'git status'

# Partir un pane visible de la ventana (imprime el id del pane nuevo)
new=$(fastty split $id --direction right)

# Redimensionar y cerrar
fastty resize $id 120 40
fastty close $id

# `list` es alias de `sessions` (mismos flags)
fastty list --json
```

`resize` y `write` usan el campo opt-in `ack: true` de sus requests y
esperan la respuesta `done` (o `error`) antes de salir, así el exit code
del CLI refleja si la operación llegó. Los clientes que omiten `ack`
mantienen el comportamiento silencioso original. `fastty close <id>` sobre
una sesión GUI devuelve `not_closable` (solo las headless se cierran por
protocolo); `fastty close <id> --force` también cierra tabs GUI matando su
proceso. `fastty spawn --open` abre la sesión como tab en la ventana.
`spawn` fallido devuelve `spawn_failed`.

Viven en `src/daemon_client.rs`, usando los mismos tipos `Request`/`Response`
que `src/daemon.rs` serializa — no hay una copia separada del protocolo del
lado del cliente que se pueda desincronizar.

## Web Gateway y Cliente Wasm Embebido (`fastty gateway`)

Fastty incluye un **servidor HTTP y WebSocket nativo en Rust embebido directamente en el binario** (`src/gateway.rs`), además del motor VT compilado a WebAssembly (`crates/fastty-wasm`).

No necesitas Python ni servidores web externos:

```bash
# Iniciar el Web Gateway en el puerto por defecto (8765)
fastty gateway

# Especificar puerto o interfaz de red (ej. para Tailscale o LAN)
fastty gateway --port 8765 --host 0.0.0.0

# Modo de solo lectura
fastty gateway --read-only
```

Al abrir `http://localhost:8765` en cualquier navegador:
1. Fastty sirve los archivos HTML/CSS/JS y el binario WebAssembly embebidos.
2. Establece una conexión WebSocket automática a `ws://localhost:8765/ws`.
3. El gateway nativo puentea los frames WebSocket directamente al socket Unix local `fasttyd.sock`.
4. El motor Wasm parsea y dibuja las terminales por lotes (*batched canvas rendering*) a 60/120 FPS.

## Ejemplo (Python, ver `scripts/daemon_test_client.py`)

```python
import json, socket, base64

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect("/Users/you/Library/Application Support/fastty/fasttyd.sock")

s.sendall(b'{"cmd": "list"}\n')
print(s.recv(65536))

s.sendall(b'{"cmd": "attach", "id": 1, "mode": "read_only"}\n')
while True:
    line = s.recv(65536)
    msg = json.loads(line.splitlines()[0])
    if msg.get("event") in ("output", "snapshot"):
        print(base64.b64decode(msg["data"]).decode(errors="replace"), end="")
    elif msg.get("event") == "closed":
        break
```
