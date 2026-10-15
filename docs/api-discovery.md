# API discovery

Use the running session host to get the supported commands and their JSON schemas.

```sh
fastty api list --json
fastty api describe attach --json
fastty api describe spawn --json
```

`list` returns the server version, protocol version, capabilities, and every request name. `describe` also returns the request and response schemas for one method. Without `--json`, the commands print text.

The server generates the schemas from its Rust protocol types with Schemars. Each schema uses JSON Schema Draft 7. The request schema includes the `cmd` field. The response schema includes the `event` field and all possible response events for that method, including `error`. Each document includes its referenced type definitions.

The `response_mode` field is `single`, `stream`, or `ack_optional`. For `write` and `resize`, `ack_optional` means the server sends `done` only when the request sets `ack` to `true`. A stream can send more than one response. Its schema does not specify event order.

The CLI queries the host socket directly. It uses the schema from that server version. It does not start a host or generate a local schema. If the host is absent, the method is unknown, or the host does not support discovery, the command exits with status 1. Socket reads and writes have a 5 s timeout.

The wire requests are:

```json
{"cmd":"api_list"}
{"cmd":"api_describe","method":"attach"}
```

The responses use `event: "api_list"` or `event: "api_describe"`. Their `api` field contains the result. Discovery requires no session ID and does not create a session.

Schemas specify the JSON data shape. Access scope, GUI availability, session state, and host identity can also restrict a command. See [the daemon protocol](daemon-protocol.md).
