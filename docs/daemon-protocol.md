# fasttyd — protocolo del daemon local (v2)

Implementación: `src/daemon.rs`. Expone el control y la observación local de
sesiones de Fastty sin quitarle la pestaña a la ventana GPUI.

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
{"cmd": "hello", "protocol_version": 2}
{"cmd": "set_access_scope", "scope": "observe"}
{"cmd": "set_access_scope", "scope": "interact"}
{"cmd": "list"}
{"cmd": "session_status", "id": 1}
{"cmd": "close_if_idle", "id": 1}
{"cmd": "close_if_idle_batch", "ids": [1, 2]}
{"cmd": "subscribe_sessions"}
{"cmd": "subscribe_events"}
{"cmd": "capture", "id": 1}
{"cmd": "attach", "id": 1}
{"cmd": "attach", "id": 1, "mode": "read_only"}
{"cmd": "attach", "id": 1, "binary_snapshot": true}
{"cmd": "detach", "id": 1}
{"cmd": "write", "id": 1, "data": "<base64>"}
{"cmd": "external_action", "id": 1, "action": {"action": "send", "text": "git status\r"}}
{"cmd": "external_action", "id": 1, "action": {"action": "close"}}
{"cmd": "resize", "id": 1, "cols": 80, "rows": 24}
{"cmd": "spawn", "command": "zsh", "args": ["-l"], "cwd": "/home/user", "cols": 80, "rows": 24, "open": true}
{"cmd": "spawn_status", "request_id": "<client request ID>"}
{"cmd": "close", "id": 1}
{"cmd": "close", "id": 1, "force": true}
{"cmd": "split_pane", "id": 1, "direction": "right", "cwd": "/home/user"}
{"cmd": "resize_pane", "id": 3, "direction": "left", "delta": 0.05}
{"cmd": "focus_pane", "id": 3}
{"cmd": "layout"}
{"cmd": "resize_window", "cols": 120, "rows": 40}
```

`session_status` returns whether the shell is alive, whether it has a foreground
child process, and the foreground command when the host can read it. Older
hosts can omit `foreground_process_label`.

`close_if_idle` closes the session only when it has no foreground command.
It returns `closed` or `close_blocked`. `close_blocked` includes the command
name when the host can read it. The host checks the process and stops the
session in one operation.

`close_if_idle_batch` checks every listed session under its writer locks before
it stops any session. It returns `close_batch` with all `closed_ids` when every
session is idle. If any session has a foreground command, it stops none and
returns the blocked session IDs and command labels.

The `hello` response reports the protocol version, Fastty version, and optional
capabilities. A client must check capabilities before it sends newer commands.
When an older host does not support `close_if_idle`, Fastty checks process status
and sends `close` with `force: true` only when the host reports no foreground
process. The old protocol cannot make these two requests atomic, so a process
that starts between them can still be stopped. Fastty does not use this fallback
for batch close because separate checks could close only part of the batch.

`attach` sends an ANSI snapshot by default. Set `binary_snapshot` to `true` to
get a compressed FST1 snapshot with scrollback in the `snapshot` event. The
attach snapshot keeps the newest history that fits in 16 MiB of cells. The
field is optional so older clients keep the ANSI format.

The `terminal_state_snapshot_v1` capability enables `snapshot_format: "terminal_state"`
on `attach`. This format takes priority over `binary_snapshot`. It returns an FST2
snapshot with both screen buffers, cursor state, wrap state, symbolic colors,
and complete cells. Native clients request this format only when the host reports
the capability. Legacy requests and WASM clients keep FST1 or ANSI.

FST2 uses a 12-byte header with magic, schema version, encoding, and decoded payload
length. Version 1 uses a bounded JSON state payload, optionally compressed with
Deflate. The decoder checks the state before it changes the terminal. The format
does not store a parser that is inside an incomplete escape sequence.

- `id` es el `PaneId` de fastty (un `usize`, único por proceso — cada split
  cuenta como su propia sesión, no solo cada tab).
- `hello` es opcional pero recomendado como primer mensaje: te confirma que
  hablás con la versión de protocolo que esperás antes de mandar nada más. Acepta
  el campo opcional `"read_only": true` para fijar la conexión completa en modo
  solo-lectura (sticky). En una conexión `read_only`, cualquier llamada a `write`,
  `spawn` o `close` es rechazada con código de error `"read_only"`.
  La respuesta incluye `fastty_version` y `capabilities`. Clientes antiguos pueden
  omitir `capabilities`. Los clientes deben usar esta lista para detectar comandos
  opcionales y mantener sesiones cuando el host no los admite. Una versión distinta
  de Fastty no implica por sí sola que el host sea incompatible.
- `set_access_scope` limita la conexión a `observe`, `interact` o `manage`.
  La conexión puede bajar su permiso, nunca subirlo. `observe` permite leer.
  `interact` permite enviar texto y redimensionar una terminal. `manage`
  permite crear, cerrar, enfocar y cambiar la estructura de sesiones. Las
  conexiones antiguas que no envían este request conservan el permiso actual.
- `subscribe_sessions`: inicia una suscripción push a los cambios de la
  lista de sesiones. Envía un evento inicial `sessions` con el estado actual,
  seguido de eventos `session_added`, `session_removed`, y `session_updated`
  a medida que cambian las pestañas/splits en fastty.
- `subscribe_events`: inicia un flujo JSONL con una foto inicial y eventos de
  sesión con versión, secuencia, hora y `session_id`. Fastty conserva los últimos
  4.096 eventos. Si el cliente queda atrás, recibe `event_stream_gap` con una
  foto completa y debe reemplazar su estado local antes de continuar desde la
  nueva secuencia. `session_updated` incluye el campo opcional
  `terminal_state_changed: true` cuando cambia el proceso visible, el cwd, el
  estado del shell o el estado OSC 7501. `fastty events` presenta esos cambios
  como `terminal_state_changed`. El campo `session.title` es el nombre del
  proceso en primer plano cuando el sistema puede leerlo. `session.alive`
  indica si el proceso del shell sigue vivo. `program_status` incluye el estado
  que un programa informó por OSC 7501. Estos valores salen de la foto de sesión
  que Fastty ya expone y se actualizan por sondeo cada 100 ms. Fastty limita
  cada texto a 256 caracteres y cada lista de estados a 16 registros. El flujo
  no informa inicio ni fin de comandos.
- `capture`: devuelve la pantalla visible como texto ANSI. La salida tiene un
  límite de generación de 1 MiB y no incluye el scrollback.
- `attach`: se adjunta a una sesión. Acepta el campo opcional `mode`:
  `"read_write"` (por defecto) o `"read_only"`. En modo `"read_only"`, cualquier
  `write` posterior a esa sesión en esa conexión será rechazado con error
  `read_only`.
- `data` en `write` son bytes crudos en base64 (para poder mandar teclas de
  control, secuencias de escape, etc. sin pelear con el escaping de JSON).
- `external_action` pide aprobación visible antes de enviar texto o cerrar una
  sesión. Fastty responde `done` tras aprobar y ejecutar. Responde
  `approval_denied`, `approval_timeout` o `approval_unavailable` si la acción
  no se aprueba, vence el plazo de 60 segundos o no hay una ventana visible.
  El permiso `interact` permite enviar texto y `manage` permite cerrar.
- `detach` es por sesión **y por conexión**: solo corta el streaming de esa
  sesión en la conexión que lo pide, no afecta a otros clientes adjuntados
  a la misma sesión desde otra conexión.
- `resize`: actualiza dinámicamente las dimensiones en columnas y filas de la
  terminal (`cols`, `rows`) y emite `SIGWINCH` en el kernel PTY.
- `spawn`: crea una nueva sesión de terminal headless/remota y devuelve `{"event": "spawned", "id": <id>, "opened": <bool>}`. El campo opcional `request_id` hace que los reintentos de la misma solicitud devuelvan el mismo ID. Con `"open": true` la sesión además se abre como un tab visible en la ventana de fastty; `"opened"` en la respuesta indica si eso sucedió (`false` cuando no hay ventana corriendo o no se pidió `open`). La sesión sigue siendo la misma — mismo `id`, cerrable vía `close` — y el renderizado la redimensiona a la geometría real del tab.
- `spawn_status`: busca el resultado de un `spawn` por su `request_id`. Fastty usa este comando si pierde la respuesta inicial, para recuperar el ID sin crear una segunda sesión. Los hosts antiguos pueden no tener este comando.
- `close`: cierra y desregistra la sesión especificada por `id`. Las sesiones headless (creadas vía `spawn`, con o sin `open`) se cierran directo. Las sesiones GUI abiertas por el usuario en la ventana devuelven error `not_closable` — son tabs del usuario — salvo que se mande `"force": true`, que mata su proceso (igual que cerrar el tab en la ventana) y la desregistra.
- `split_pane`: parte un pane visible de la ventana creando un pane nuevo a su lado (mismo tab, `"direction"`: `left`/`right`/`top`/`down`) y devuelve `{"event": "pane_split", "id": <nuevo id>}`. El pane nuevo es una sesión GUI normal (list/write/close funcionan). `command`/`args`/`cwd` opcionales (por defecto el shell del usuario y el cwd del pane objetivo). Sin ventana corriendo: error `no_gui`.
- `resize_pane`: mueve el divisor del lado indicado (`"direction"`) del pane, `delta` es una fracción del eje del split (default 0.05, se clampea a ±0.5). Responde `done` con `cmd: "resize_pane"`.
- `focus_pane`: trae el tab del pane al frente y lo hace activo, para que el usuario vea lo que el agente está tocando.
- `layout`: devuelve `{"event": "layout", "tabs": [...]}` con la estructura completa de la ventana — cada tab con sus panes (id, title, cwd, activo, tamaño). Sesiones headless sin tab no aparecen aquí; están en `list`.
- `resize_window`: redimensiona la ventana de fastty aproximándose a `cols` x `rows` del pane activo (con splits es aproximado; para mover divisores usar `resize_pane`). Responde `done` con `cmd: "resize_window"`.
- `binary_snapshot`: solicita un snapshot binario con cabecera `FST1` (comprimido con Deflate). `screen_only: true` excluye el scrollback. Los clientes antiguos omiten el campo y conservan el snapshot completo.

## Compatibilidad entre versiones

Un cliente anuncia su versión de protocolo en `hello` con el campo opcional
`protocol_version`. Un host nuevo adapta cada respuesta a esa versión. Un host
antiguo ignora el campo.

Casos:

1. Sin `hello`, el host responde con la forma nativa.
2. Con `hello` y sin `protocol_version`, el cliente recibe la forma más antigua
   que el host todavía soporta. Es el caso de un cliente que no conoce la
   negociación.
3. Con `protocol_version`, el host aplica las reglas de compatibilidad entre esa
   versión y la suya.

Cada cambio de API es una regla en `src/protocol_compat.rs`. La regla nombra la
versión donde empezó el cambio, un JSON pointer (RFC 6901) y el tipo de cambio:
campo agregado, campo renombrado o campo eliminado. El primer token del pointer
es la etiqueta del mensaje (`cmd`, `event` o `type`). El token `*` recorre
cualquier clave de objeto o cualquier elemento de arreglo.

El host aplica las reglas a las respuestas que salen y a las solicitudes que
llegan. El cliente que abre un host aplica las reglas a las solicitudes que
salen. Los mensajes sin reglas se envían sin tocar el JSON.

Un evento desconocido no rompe un cliente. El cliente lo lee como
`Response::Unknown` y sigue leyendo la conexión. Un comando desconocido sí
devuelve error `bad_request`, porque el host no puede ejecutarlo.

Los campos nuevos se marcan con `#[serde(default)]`. Un cliente antiguo los
ignora. Los campos eliminados o renombrados necesitan una regla.

### Responses (daemon → cliente)

```jsonc
{"event": "hello", "version": 2, "fastty_version": "0.22.9", "capabilities": ["close_if_idle", "close_if_idle_batch", "binary_snapshot_attach", "attached_process_running", "spawn_request_status"]}
{"event": "sessions", "sessions": [
  {"id": 1, "title": "zsh", "cwd": "/Users/you", "cols": 89, "rows": 32, "alive": true, "history_size": 0}
]}
{"event": "session_status", "alive": true, "process_running": true, "foreground_process_label": "claude"}
{"event": "spawn_status", "id": 42}
{"event": "session_added", "session": {"id": 2, "title": "zsh", "cwd": "/tmp", "cols": 89, "rows": 32, "alive": true, "history_size": 0}}
{"event": "session_removed", "id": 2}
{"event": "session_updated", "session": {"id": 1, "title": "vim", "cwd": "/Users/you", "cols": 89, "rows": 32, "alive": true, "history_size": 120}}
{"event": "event_stream_started", "version": 1, "sequence": 0, "sessions": []}
{"event": "stream_event", "envelope": {"version": 1, "sequence": 1, "timestamp_unix_ms": 1791750000000, "session_id": 2, "kind": {"type": "session_added", "session": {"id": 2}}}}
{"event": "event_stream_gap", "after_sequence": 1, "oldest_sequence": 4098, "latest_sequence": 8193, "sessions": []}
{"event": "error", "code": "approval_denied", "message": "the user denied this action"}
{"event": "captured", "id": 1, "data": "<ANSI screen>", "truncated": false}
{"event": "done", "cmd": "set_access_scope"}
{"event": "attached", "id": 1, "cols": 89, "rows": 32, "mode": "read_only", "program_status": []}
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
- `program_status` es opcional y solo aparece cuando Fastty detecta estados
  OSC 7501. Incluye el ID, estado, aplicación, progreso y tipo de espera. Omite
  mensajes y texto libre para reducir la exposición de datos.
- `attached.program_status` conserva el estado OSC 7501 al reconectar una
  sesión. Es opcional para clientes antiguos y omite mensajes vacíos.
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
  `invalid_base64`, `not_attached`, `read_only`, `access_denied`,
  `permission_escalation`, `approval_denied`, `approval_timeout`,
  `approval_unavailable`, `session_changed`, `input_too_large` y `unsupported`. Pueden
  agregarse códigos nuevos con el tiempo; un cliente debería tratar uno que
  no reconoce igual que una falla genérica.

## Cliente de referencia en el propio binario

`fastty sessions` y `fastty attach <id>` son clientes de referencia
integrados:

```bash
# Listar sesiones activas
fastty sessions

# Capturar la pantalla visible y enfocar una sesión
fastty capture 1
fastty focus 1

# Enviar texto. Fastty pide aprobación antes de escribirlo
fastty send 1 --enter -- 'git status'

# Leer cambios de sesión como JSONL
fastty events --json

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

### Límites de acceso

- `attach --read-only` limita la escritura en esa sesión y conexión.
- `attach` con `mode: "read_only"` aplica el permiso de observación a esa
  sesión y conexión. `mode: "read_write"` necesita el permiso `interact`.
- `hello` con `read_only: true` limita las acciones de escritura de toda la
  conexión. Los eventos y las capturas son de solo lectura.
- `send` necesita un ID explícito. No ejecuta texto hasta que el cliente usa
  `--enter`. Fastty pide aprobación visible antes de enviar el texto.
  `close --force` pide aprobación visible antes de cerrar una sesión.
- Los permisos de scope son límites que el cliente declara. Ayudan a evitar
  errores de integraciones cooperativas. No autentican al proceso cliente.
- La aprobación visible protege el nuevo `external_action`. Las peticiones
  directas `write` y `close` siguen disponibles para compatibilidad y para el
  transporte interno de sesiones. No son una barrera contra procesos del mismo
  usuario.
- El socket `0600` bloquea a otros usuarios del sistema. Los procesos del mismo
  usuario comparten el mismo límite de confianza. Estas reglas evitan acciones
  accidentales de clientes cooperativos, no aíslan procesos hostiles del mismo
  usuario.

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


## Host identity and restart

The `hello` reply includes `host_instance`, a UUID for this host process. The
`session_identity_v1` capability permits `expected_host_instance` in `attach`.
The host checks this value before it looks up the session. A mismatch returns
`host_instance_changed`.

Each session record includes `identity` with `host_instance` and `session_id`.
The host reserves numeric IDs in blocks of 1024. It saves and syncs the upper
limit before it uses an ID. A host restart skips unused IDs in the last block.

Saved panes retain `host_session_identity`. A saved numeric reference without
an identity stays unavailable. The app does not infer its identity from a
session number, title, or directory.

The `host_status` request returns `host_status` with `status`. This record has
the host instance, app version, protocol version, capabilities, session count,
other connection count, and `restart_available`.

The `restart_host` request requires `expected_host_instance`. The host accepts
it only with zero sessions and zero other connections. The host applies this
check under the same lock that permits new sessions and connections. It then
returns `host_restarting`, removes its socket, and exits. A blocked request
returns `host_restart_blocked`. The app starts its current host executable
after the old socket closes. The app does not stop a shell to permit an upgrade.

The command palette has `Session Host: Status` and
`Session Host: Restart or Upgrade`. Old hosts retain their negotiated snapshot
format. The app does not force a restart when a host lacks the restart command.

## Process events

Clients must set `"process_events": true` in `subscribe_events` to get process
states and events. The default is `false`. Existing clients keep the same
response types. Filtered events still advance the stream sequence.

The `process_events_v1` capability adds `process_changed` events to the typed
event stream. One host sampler reads the process table every 350 ms.
Each state has a session identity, a generation, and an observation.
The generation changes only when the observation changes.

An available observation has the shell identity, foreground process group,
group members, and ancestors. A process identity has its PID, start time in
Unix seconds, and name. It excludes command arguments. Ancestor traversal
stops after 64 parents per member and detects cycles.

A failed or incomplete observation has `status: "unavailable"` and a reason.
It does not prove that the terminal is idle. The sample can miss processes
that start and exit between reads. Close safety still uses a live query.

The host sends `process_states` after `event_stream_started` and after
`event_stream_gap`. This baseline includes the current generations.
`fastty events` prints it as `process_snapshot`. Later changes use
`process_changed`. Observe access can read these events.


Each GUI pane holds its saved identity. The app passes that identity to each
attach, retry, status, and close request. Panes with the same numeric ID do not
share an identity cache. A fresh session cannot change a saved pane reference.

If the host instance changes, workspace restore can start a new shell. A
connection retry keeps the pane unavailable until the user selects recovery.
A saved pane with no identity offers `Start new shell`. This action replaces
the GUI pane and leaves the old host session alone. Closing an unavailable
legacy pane removes only its GUI reference.
