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

## Not covered by this version

- `sessions --json` still prints a bare array without an envelope. Changing it
  would break existing callers, so it keeps its current shape.
- A command that waits until the output matches a pattern does not exist.
- There is no embedded scripting layer.
