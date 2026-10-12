# Integraciones de agentes

Fastty ya recibe estados de programas por OSC 7501. El daemon ahora expone
esos estados en `SessionInfo.program_status` y los cambios en `fastty events`.
La API no expone mensajes libres del agente.

## Orden de trabajo

| Orden | Proceso | Punto de integración | Estado |
| --- | --- | --- | --- |
| 1 | Claude Code | Hooks de sesión y fin de turno | Implementado |
| 2 | OpenCode | Plugin de ciclo de vida | Implementado |
| 3 | Gemini CLI | Hooks de sesión y turnos | Implementado |
| 4 | Pi | Extensión de ciclo de vida | Implementado |
| 5 | Codex CLI | Hooks de turno y sesión | Implementado. Codex puede pedir aprobación de confianza. |
| 6 | Aider | Integración de shell y OSC | Sin hook estructurado verificado. |

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

Proveedores: `claude`, `gemini`, `codex`, `opencode` y `pi`. Claude, Gemini y
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
