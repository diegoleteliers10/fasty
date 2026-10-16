# Integraciones de agentes

Fastty ya recibe estados de programas por OSC 7501. El daemon ahora expone
esos estados en `SessionInfo.program_status` y los cambios en `fastty events`.
La API no expone mensajes libres del agente.

## Orden de trabajo

| Orden | Proceso | Punto de integración | Estado |
| --- | --- | --- | --- |
| 1 | Claude Code | Hooks de sesión y fin de turno | Implementado |
| 2 | OpenCode v1 | Plugin del servidor | Implementado |
| 2 | OpenCode v2 | Plugin de la CLI | Implementado |
| 3 | Gemini CLI | Hooks de sesión y turnos | Implementado |
| 4 | Pi | Extensión de ciclo de vida | Implementado |
| 5 | Codex CLI | Hooks de turno y sesión | Implementado. Codex puede pedir aprobación de confianza. |
| 6 | Aider | Integración de shell y OSC | Sin hook estructurado verificado. |
| 7 | Hermes Agent | Plugin de turnos y solicitudes al usuario | Implementado |

Fuentes oficiales: [Claude Code hooks](https://code.claude.com/docs/en/hooks),
[OpenCode plugins](https://docs.opencode.ai/docs/plugins/),
[Gemini CLI hooks](https://geminicli.com/docs/hooks/reference/),
[Pi extensions](https://pi.dev/docs/latest/extensions) y
[Codex app-server](https://developers.openai.com/codex/app-server).

## Contrato del adaptador

Cada adaptador debe ser opcional y enviar eventos tipados al host de sesión.
Debe informar estados de trabajo, espera, fin y error. Debe usar el mismo
`session_id` que el PTY. No debe leer la salida de pantalla para inferir
permisos ni escribir en el PTY sin una acción explícita del usuario.

Las integraciones solo publican estado. No leen salida de pantalla, no envían
texto al PTY y no aprueban permisos del agente.

## Instalar o quitar

Usa `fastty agent-hook install <proveedor> --project` para instalar en el
proyecto actual. Usa `--user` para instalar en la configuración del usuario.
Usa `fastty agent-hook uninstall <proveedor> --project` o `--user` para quitar
solo las entradas de Fastty. Los archivos JSON existentes se conservan y se
respaldan antes de cada cambio. El instalador no reemplaza archivos de
extensión que ya tengan contenido distinto.

Proveedores: `claude`, `gemini`, `codex`, `opencode`, `opencode-v2`, `pi` y `hermes`. Claude, Gemini y
Codex usan hooks JSON. OpenCode y Pi usan archivos de extensión del proyecto o
del usuario. Codex puede pedir que confirmes la confianza del proyecto después
de la instalación.

```sh
fastty agent-hook install claude --project
fastty agent-hook install opencode --project
fastty agent-hook install gemini --project
fastty agent-hook install pi --project
fastty agent-hook install codex --project
```

## OpenCode

Para OpenCode v1, instala el adaptador de servidor:

```sh
fastty agent-hook install opencode --user
```

Para OpenCode v2, instala el adaptador de la CLI:

```sh
fastty agent-hook install opencode-v2 --user
```

Ambos aceptan `--project`. El instalador usa `OPENCODE_CONFIG_DIR`, después
`XDG_CONFIG_HOME/opencode` y después `~/.config/opencode` para el alcance de
usuario. El adaptador v2 se registra en `cli.json`, en la lista `plugins`.
Se instala fuera de la carpeta de plugins v1 para evitar que v1 intente cargarlo.
Si existe `cli.jsonc` o el JSON contiene comentarios, el instalador no lo cambia.
Usa el archivo `integrations/opencode/fastty-v2.js` del repositorio y registra su
ruta absoluta en la lista `plugins` de tu configuración CLI.

V1 y v2 tienen APIs distintas. No registres el adaptador v1 como plugin v2.
Abre OpenCode después de instalar el adaptador. Una instancia que ya está
abierta puede conservar sus plugins anteriores.

Los adaptadores informan trabajo, reintento, permiso, pregunta, fin y error.
Cada respuesta elimina solo su solicitud pendiente. Mientras queda otra
solicitud, Fastty conserva el estado bloqueado. Los informes salen en orden.
V1 conserva el error cuando llega el evento de reposo que lo sigue.

V1 informa el estado conjunto de las sesiones raíz de su servidor. Su API no
expone la selección de la TUI. V2 usa la sesión visible y las tabs abiertas de
la CLI. No atribuye formularios globales de MCP a una sesión del terminal.
Los eventos de subagentes no completan ni borran el estado de su sesión raíz.

Al reinstalar, Fastty puede actualizar su adaptador v1 original y guarda una
copia de respaldo. Si el archivo tiene otros cambios, Fastty no lo reemplaza.

Fuentes: [migración de plugins v2](https://opencode.ai/v2/docs/migrate-v1/#plugins),
[plugins de CLI v2](https://opencode.ai/v2/docs/build/plugins/cli/),
[eventos v1](https://github.com/anomalyco/opencode/blob/v1.2.27/packages/opencode/src/session/status.ts).

## Hermes Agent

Instala y habilita el plugin en el perfil que uses:

```sh
fastty agent-hook install hermes --user
hermes plugins enable fastty-status
```

El instalador usa `HERMES_HOME/plugins/fastty-status` si `HERMES_HOME` está
definido. En otro caso usa `~/.hermes/plugins/fastty-status`. `HERMES_HOME` debe
ser una ruta absoluta. Usa el mismo perfil para los dos comandos. El alcance
`--project` no está disponible para Hermes.

El plugin usa `pre_llm_call` para informar trabajo y `on_session_end` para
informar fin, error o interrupción. Las solicitudes `approval`, `clarify` y
`sudo` muestran bloqueo por permiso, pregunta o autenticación. No publica
preguntas, comandos, respuestas ni contraseñas. No interpreta un error de API
con reintento como un fallo final.

`/new` borra el estado anterior y permite informar el turno de la nueva sesión.
Los eventos de subagentes y del gateway no cambian el estado del terminal CLI.
Hermes debe incluir los hooks `on_human_input_request` y
`on_human_input_resolved` para informar esos bloqueos.

Para quitarlo, deshabilita el plugin y elimina sus archivos:

```sh
hermes plugins disable fastty-status
fastty agent-hook uninstall hermes --user
```

Fastty conserva `config.yaml` y los archivos de plugin que el usuario modificó.

Fuentes: [plugins de Hermes](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/plugins.md),
[hooks de entrada del usuario](https://github.com/NousResearch/hermes-agent/blob/main/tools/human_input_hooks.py),
[fin de turno](https://github.com/NousResearch/hermes-agent/blob/main/agent/turn_finalizer.py).

## Límites actuales

`fastty events` publica altas, bajas y cambios de sesión. Los estados de agente
aparecen cuando el proceso emite OSC 7501 o usa un adaptador. `fastty agent-hook`
publica estados de Claude, Gemini y Codex. Los complementos de OpenCode y Pi
publican estados desde sus eventos de ciclo de vida. Fastty no aprueba permisos
del agente. Los eventos de permiso solo muestran el estado bloqueado.

## Claude Code: contrato para el primer adaptador

Claude Code ofrece hooks de comando con JSON por `stdin`. Los eventos útiles
para el estado son `SessionStart`, `UserPromptSubmit`, `Stop`, `StopFailure`,
`Notification` y `SessionEnd`. `SessionStart` también ocurre al reanudar una
sesión. `Stop` indica que Claude terminó una respuesta. `StopFailure` indica un
error de API. [Referencia oficial de hooks](https://code.claude.com/docs/en/hooks).

| Evento de Claude | Estado Fastty propuesto | Regla |
| --- | --- | --- |
| `SessionStart` | `idle` | Inicio o reanudación. |
| `UserPromptSubmit` | `working` | Se envía una solicitud. |
| `Stop` | `done` | La respuesta terminó. Si hay tareas de fondo activas, no marcar `done`. |
| `StopFailure` | `error` | La solicitud terminó por un error de API. |
| `Notification` con `permission_prompt` | `blocked`, `kind=permission` | Solo publicar el tipo. No publicar el texto de la herramienta. |
| `Notification` con `idle_prompt` o `agent_needs_input` | `blocked`, `kind=question` | Solo publicar el tipo. |
| `SessionEnd` | limpiar estado | Claude cerró o cambió la sesión. |

El contrato OSC que ya entiende Fastty usa `ESC ] 7501 ;` seguido por campos
separados por `:` y termina con `ESC \\`. `ESC` representa el byte `0x1b`, no
texto literal. Ejemplos del payload:

```text
ESC ] 7501 ; state=working:app=claude ESC \\
ESC ] 7501 ; state=done:app=claude ESC \\
ESC ] 7501 ; state=blocked:app=claude:kind=permission ESC \\
ESC ] 7501 ; state=error:app=claude ESC \\
ESC ] 7501 ; state=clear:app=claude ESC \\
```

No incluir `last_assistant_message`, `tool_input`, `transcript_path`, comandos,
razones de permiso ni texto libre en `msg` o `title`. Fastty no expone esos
campos en `SessionInfo.program_status`.

Los adaptadores solo publican estados si el agente hereda
`FASTTY_SESSION_TOKEN` desde Fastty. Si no encuentra el token, el hook termina
sin cambiar el estado. Los hooks de Gemini y Codex devuelven `{}` en `stdout`
para conservar su contrato de salida JSON.

OpenCode usa `session.idle`, `session.error` y eventos de permiso. Pi usa
`agent_start` y `agent_settled`. Pi marca `done` cuando termina todo el trabajo
automático, no al final de un intento individual. [OpenCode plugins](https://docs.opencode.ai/docs/plugins/), [Pi extensions](https://pi.dev/docs/latest/extensions).

Gemini usa `BeforeAgent`, `AfterAgent`, `SessionStart`, `SessionEnd` y
`Notification`. [Gemini CLI hooks](https://geminicli.com/docs/hooks/reference/).
Codex usa `UserPromptSubmit`, `Stop`, `Interrupt`, `SessionEnd` y
`PermissionRequest`. Revisa la confianza de los hooks antes de permitir que
Codex los ejecute. [Codex hooks](https://developers.openai.com/codex/hooks).

El menú de terminal pega el contexto en la sesión del agente solo si la
selección pertenece a ese panel, el agente está reconocido y el modo de pegado
con corchetes está activo. Fastty no envía Enter. Revisa el texto en el agente
y envíalo desde allí.
