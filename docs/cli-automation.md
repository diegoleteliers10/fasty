# Automation contract for the CLI

Scripts need two things that do not change: the exit code, and the JSON on
stdout. Both are versioned. This file is the contract; a change to either is a
breaking change and bumps `schema_version`.

## Commands with a stable contract

| Command | JSON | Contract version |
|---|---|---|
| `fastty capture <id> [--json] [--wait[=SECONDS]]` | yes | 1 |
| `fastty send <id> [--json] [--enter] [--wait[=SECONDS]] -- <TEXT>` | yes | 1 |
| `fastty wait <id> --until <TEXT> [--timeout[=SECONDS]] [--json]` | yes | 1 |

`--wait` retries the connection until the daemon answers. It does not wait for
output from the session.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | The command succeeded |
| 1 | Bad flags or arguments. Nothing was sent to the daemon |
| 2 | The daemon was unreachable, the protocol failed, or it reported an error that is not a missing session |
| 3 | The session id does not exist, or it disappeared before the request |
| 4 | `capture` returned a partial screen because the output hit the 1 MiB cap |
| 5 | `wait` reached its timeout without seeing the pattern |

Code 4 replaces an older undocumented behaviour where a truncated capture
exited with 2, the same code as a failure. Nothing documented that behaviour.

## JSON shape

One object, one line, on stdout. With `--json`, stderr stays empty and stdout
carries the document even when the command fails, so a caller parses one
stream and branches on `ok`.

Success:

```json
{"schema_version":1,"command":"send","ok":true,"session":4,"bytes":12,"enter":true}
{"schema_version":1,"command":"capture","ok":true,"session":4,"data":"total 0\n","bytes":8,"truncated":false}
```

Failure:

```json
{"schema_version":1,"command":"capture","ok":false,"exit_code":3,"error":{"code":"no_such_session","message":"..."}}
```

`exit_code` repeats the process exit code inside the document, so a script can
branch without reading the shell status.

### Fields

| Field | Present on | Meaning |
|---|---|---|
| `schema_version` | always | `1` |
| `command` | always | `send` or `capture` |
| `ok` | always | Whether the command succeeded |
| `session` | success | Session id from the request |
| `bytes` | success | Bytes written, or bytes of captured data |
| `enter` | `send` success | Whether Enter was appended |
| `data` | `capture` success | The captured screen as text |
| `truncated` | `capture` success | Whether the capture hit the cap |
| `exit_code` | failure | The process exit code |
| `error.code` | failure | Daemon error code, or `daemon_unreachable`, `protocol_error`, `unsupported_platform` |
| `error.message` | failure | Human-readable detail |

## Daemon error codes that reach `error.code`

`no_such_session`, `access_denied`, `bad_request`, `not_attached`,
`not_closable`, `spawn_failed`, `invalid_snapshot`, `invalid_status`,
`unknown_method`.

Only `no_such_session` maps to exit 3. Every other daemon code maps to 2.

## Without `--json`

`capture` writes the screen text to stdout and keeps its message on stderr.
`send` prints nothing on success. Exit codes are the same as with `--json`.

## `wait`: block until a pane prints a string

`fastty wait <id> --until <TEXT> [--timeout[=SECONDS]] [--json]`

| Behaviour | Contract |
|---|---|
| Output scanned | Only output produced after the command starts. The initial screen is read and discarded, so text that was already on screen cannot answer the wait |
| Match | Literal substring. No regular expression |
| Consumption | None. The command reads a read-only attachment and never writes to the pane |
| Timeout | 60 seconds by default; `--timeout <SECONDS>` overrides it |
| Memory | At most 8 KiB of filtered text is kept, so an hours-long wait uses the same memory as a short one |
| Success | Exit 0, with the matched line |
| Timeout | Exit 5, `error.code: timeout` |
| Pane closed first | Exit 2, `error.code: session_closed` |
| No session | Exit 3, `error.code: no_such_session` |

ANSI escape sequences are removed before matching, including a sequence split
across two reads, so a colour code cannot sit inside the pattern.

The two `--wait` flags above are unrelated: on `send` and `capture` they retry the
connection until the daemon answers. `fastty wait` is the command that waits for
output.

## Not covered by this version

- `sessions --json` still prints a bare array without an envelope. Changing it
  would break existing callers, so it keeps its current shape.
- A regular-expression pattern. `--until` is literal only.
- There is no embedded scripting layer, and adding one would mean a C runtime in
  every release target plus an API surface, a permission model and resource
  limits. A shell plus this JSON contract covers the same automation today.
