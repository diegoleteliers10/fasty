# fastty MCP — conectando agentes a fastty

`fastty mcp` es un servidor MCP (Model Context Protocol) local que expone el
daemon de fastty como tools para agentes de IA: Claude Code, Codex, Cursor,
o cualquier cliente MCP. Los agentes pueden listar sesiones, crear
terminales headless, escribir comandos, leer la pantalla (y el scrollback),
y ejecutar comandos one-shot con output de vuelta.

## Cómo funciona

- Transporte stdio: JSON-RPC 2.0 con mensajes delimitados por newline (el
  transporte estándar MCP). El stdout del proceso es exclusivo del
  protocolo; los errores van a stderr.
- Si la GUI de fastty está corriendo, las tools operan sobre sus sesiones
  en vivo (¡los agentes ven las mismas terminales que tú!).
- Si no hay fastty abierto, `fastty mcp` embebe su propio daemon (mismo
  patrón que `fastty gateway`): los agentes obtienen un workspace headless
  efímero que vive mientras dure la conexión MCP.
- Requiere Unix (el daemon solo escucha en Unix sockets; no soportado en
  Windows todavía).

## Registro

### Claude Code

```bash
claude mcp add fastty -- fastty mcp
```

o en `.mcp.json` del proyecto:

```json
{
  "mcpServers": {
    "fastty": { "command": "fastty", "args": ["mcp"] }
  }
}
```

### Codex CLI

En `~/.codex/config.toml`:

```toml
[mcp_servers.fastty]
command = "fastty"
args = ["mcp"]
```

### Cursor y otros clientes

El formato genérico de MCP es el mismo: comando `fastty`, argumento `mcp`.

## Tools expuestas

| Tool | Qué hace |
|------|----------|
| `fastty_list_sessions` | Lista sesiones vivas (id, título, cwd, tamaño, alive). |
| `fastty_spawn_session` | Crea una sesión headless (shell o comando custom) y devuelve su id. |
| `fastty_write_session` | Escribe texto en la PTY de una sesión, con `enter: true` para submittear. |
| `fastty_read_screen` | Lee la pantalla como texto plano; `include_history: true` antepone el scrollback. |
| `fastty_resize_session` | Redimensiona el grid de una sesión. |
| `fastty_close_session` | Termina una sesión headless. |
| `fastty_run_command` | Ejecuta un comando en una sesión desechable (`$SHELL -c`), espera a que el proceso salga (timeout configurable, default 30s), devuelve las últimas 300 líneas de output y cierra la sesión. La vía rápida para builds, `git status`, inspección de archivos. |

`fastty_read_screen` y `fastty_run_command` usan el snapshot binario FST1
comprimido del daemon (v2, con scrollback) y lo renderizan a texto en el
proceso MCP — el output que ve el agente es la grilla real de la terminal.

## Notas de seguridad

- Las tools de escritura (`spawn`, `write`, `run_command`, `close`) actúan
  sobre el daemon local del usuario; el gating de permisos corresponde al
  cliente MCP (Claude Code/Codex preguntan por cada tool call según su
  propia política).
- `fastty_close_session` no puede cerrar panes GUI (`not_closable`): los
  terminales visibles del usuario están protegidos del protocolo.
- El proceso MCP es read-only hacia el sistema de archivos salvo por los
  comandos que el agente decida correr — igual que cualquier terminal.
