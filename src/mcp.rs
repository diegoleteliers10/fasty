//! Local MCP (Model Context Protocol) server for fastty.
//!
//! `fastty mcp` speaks JSON-RPC 2.0 over stdio (newline-delimited messages,
//! the MCP stdio transport) and exposes fastty's daemon as agent tools, so
//! Claude Code, Codex, OpenCode, or any MCP client can drive terminals:
//! list/spawn/write/read/close sessions, and a one-shot `run_command` that
//! runs a command in a throwaway headless session and returns its output.
//!
//! When fastty's GUI is running, tools operate on its live sessions. When
//! it isn't, the MCP process embeds its own daemon (same as `fastty
//! gateway`), giving agents a headless workspace that lives as long as the
//! MCP connection does.
//!
//! `fastty mcp setup [AGENT]` registers the server into an agent's config
//! file (same "migrate, don't compete" spirit as the terminal config
//! importer): claude-code, opencode, codex, gemini, cursor, zed, and
//! claude-desktop are supported. Setup is cross-platform; the tools
//! themselves need the Unix-socket daemon.

use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
        },
    })
}

fn tools_json() -> Vec<Value> {
    vec![
        tool(
            "fastty_list_sessions",
            "List fastty's live terminal sessions with id, title, working directory, size, and whether the shell process is still alive.",
            json!({}),
            &[],
        ),
        tool(
            "fastty_spawn_session",
            "Start a new terminal session inside fastty and return its id. Without a command it runs the user's shell. By default (open=true) the session also opens as a visible tab in the user's fastty window, so the user can watch and join in; set open=false to keep it headless. Use fastty_write_session to type into it and fastty_read_screen to see it.",
            json!({
                "command": { "type": "string", "description": "Program to run (defaults to the user's shell)." },
                "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments for the program." },
                "cwd": { "type": "string", "description": "Working directory (defaults to the home directory)." },
                "cols": { "type": "integer", "description": "Terminal width (default 80)." },
                "rows": { "type": "integer", "description": "Terminal height (default 24)." },
                "open": { "type": "boolean", "description": "Open the session as a visible tab in the running fastty window (default true)." },
            }),
            &[],
        ),
        tool(
            "fastty_write_session",
            "Type text into a session's terminal, as if the user typed it. Use enter=true to submit the line. The session echoes output to its grid; read it with fastty_read_screen.",
            json!({
                "id": { "type": "integer", "description": "Session id (from fastty_list_sessions or fastty_spawn_session)." },
                "text": { "type": "string", "description": "Text to type into the terminal." },
                "enter": { "type": "boolean", "description": "Append a carriage return to submit the line (default false)." },
            }),
            &["id", "text"],
        ),
        tool(
            "fastty_read_screen",
            "Read what a session's terminal currently shows, rendered as plain text (colors and styling are dropped). With include_history=true, scrollback lines are prepended before the visible screen.",
            json!({
                "id": { "type": "integer", "description": "Session id." },
                "include_history": { "type": "boolean", "description": "Prepend scrollback history above the visible screen (default false)." },
            }),
            &["id"],
        ),
        tool(
            "fastty_resize_session",
            "Resize a headless session's terminal grid. GUI panes always follow the window size, so this only matters for headless sessions; to change on-screen layout use fastty_resize_pane or fastty_resize_window.",
            json!({
                "id": { "type": "integer", "description": "Session id." },
                "cols": { "type": "integer", "description": "New width in columns." },
                "rows": { "type": "integer", "description": "New height in rows." },
            }),
            &["id", "cols", "rows"],
        ),
        tool(
            "fastty_layout",
            "Describe the fastty window layout: every tab with its panes (id, title, cwd, size, which is active). Use this to see which sessions share a tab before splitting or focusing. Returns an empty list when no fastty window is running.",
            json!({}),
            &[],
        ),
        tool(
            "fastty_split_pane",
            "Split a visible pane in the fastty window: create a new pane next to it (same tab) and return its session id. The new pane behaves like any session — read, write, resize, close all work on it. Requires a running fastty window.",
            json!({
                "id": { "type": "integer", "description": "Existing pane/session id to split (from fastty_layout or fastty_list_sessions)." },
                "direction": { "type": "string", "enum": ["left", "right", "top", "down"], "description": "Which side of the target the new pane takes." },
                "command": { "type": "string", "description": "Program to run in the new pane (defaults to the user's shell)." },
                "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments for the program." },
                "cwd": { "type": "string", "description": "Working directory (defaults to the target pane's cwd)." },
            }),
            &["id", "direction"],
        ),
        tool(
            "fastty_resize_pane",
            "Grow or shrink a visible pane by moving the divider on one of its sides. delta is a fraction of the split axis (0.05 moves it 5%). Requires a running fastty window.",
            json!({
                "id": { "type": "integer", "description": "Pane/session id." },
                "direction": { "type": "string", "enum": ["left", "right", "top", "down"], "description": "Which divider to move — the side of the pane to push." },
                "delta": { "type": "number", "description": "How far to move the divider, as a fraction of the axis (default 0.05)." },
            }),
            &["id", "direction"],
        ),
        tool(
            "fastty_focus_pane",
            "Bring a pane's tab to the front of the fastty window and make the pane active, so the user sees what you're working on. Requires a running fastty window.",
            json!({
                "id": { "type": "integer", "description": "Pane/session id." },
            }),
            &["id"],
        ),
        tool(
            "fastty_resize_window",
            "Resize the fastty window itself, approximately to cols x rows for the active pane. For moving pane dividers use fastty_resize_pane. Requires a running fastty window.",
            json!({
                "cols": { "type": "integer", "description": "Target width in columns (min 20)." },
                "rows": { "type": "integer", "description": "Target height in rows (min 5)." },
            }),
            &["cols", "rows"],
        ),
        tool(
            "fastty_close_session",
            "Terminate a session and free it. Headless sessions close directly. GUI tabs (the user's own panes) answer not_closable; retry with force=true to close them too, killing their running process.",
            json!({
                "id": { "type": "integer", "description": "Session id." },
                "force": { "type": "boolean", "description": "Also close GUI-owned tabs, killing their running process (default false)." },
            }),
            &["id"],
        ),
        tool(
            "fastty_run_command",
            "Run a shell command in a throwaway headless session and return its output as text: spawns `$SHELL -c <command>`, waits for the process to exit (up to timeout_ms), reads the terminal (recent scrollback + screen, last 300 non-empty lines), then closes the session. Best for builds, git status, file inspection, and other finite commands. For interactive programs (REPLs, watchers), spawn a session instead.",
            json!({
                "command": { "type": "string", "description": "Shell command to run." },
                "cwd": { "type": "string", "description": "Working directory (defaults to the home directory)." },
                "timeout_ms": { "type": "integer", "description": "Wall-clock budget in milliseconds (default 30000)." },
            }),
            &["command"],
        ),
    ]
}

// ---------------------------------------------------------------------------
// JSON-RPC dispatch (pure; testable without stdio)
// ---------------------------------------------------------------------------

/// Rows returned by `run_command`'s output renderer (most recent lines).
#[cfg(unix)]
const RUN_COMMAND_MAX_LINES: usize = 300;
/// Columns/rows for `run_command`'s throwaway session: wide enough for
/// build logs, tall enough to keep most output on the screen.
#[cfg(unix)]
const RUN_COMMAND_COLS: usize = 200;
#[cfg(unix)]
const RUN_COMMAND_ROWS: usize = 50;
/// Poll interval while waiting for a spawned command to exit.
#[cfg(unix)]
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Default wall-clock budget for `run_command`.
#[cfg(unix)]
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Handles one JSON-RPC message. Returns `Some(response)` for requests,
/// `None` for notifications and anything that must not be answered.
pub(crate) fn dispatch(msg: &Value) -> Option<Value> {
    let version = msg.get("jsonrpc").and_then(Value::as_str);
    if version != Some("2.0") {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        return Some(rpc_error(id, -32600, "jsonrpc must be exactly \"2.0\""));
    }
    let method = msg.get("method").and_then(Value::as_str)?;

    // Notifications (no id) are never answered.
    let is_notification = msg.get("id").is_none();
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let params = msg.get("params").cloned().unwrap_or(json!({}));

    let result = match method {
        "initialize" => Some(json!({
            "protocolVersion": params
                .get("protocolVersion")
                .cloned()
                .unwrap_or_else(|| json!("2025-06-18")),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": "fastty",
                "version": env!("CARGO_PKG_VERSION"),
            },
        })),
        "ping" => Some(json!({})),
        "tools/list" => Some(json!({ "tools": tools_json() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Some(match call_tool(name, &args) {
                Ok(text) => json!({
                    "content": [{ "type": "text", "text": text }],
                }),
                Err(e) => json!({
                    "content": [{ "type": "text", "text": e }],
                    "isError": true,
                }),
            })
        }
        _ => {
            return if is_notification {
                None
            } else {
                Some(rpc_error(id, -32601, &format!("method not found: {method}")))
            }
        }
    };

    if is_notification {
        return None;
    }
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result? }))
}

// ---------------------------------------------------------------------------
// Tool implementations (talk to the daemon; Unix sockets only)
// ---------------------------------------------------------------------------

#[cfg(unix)]
use crate::daemon::{Request, Response, SessionInfo};

#[cfg(unix)]
fn send_and_read(req: &Request) -> Result<Response, String> {
    let mut stream = crate::daemon_client::connect()?;
    {
        use std::io::Write as _;
        let mut line = serde_json::to_string(req).map_err(|e| e.to_string())?;
        line.push('\n');
        stream.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        stream.flush().map_err(|e| e.to_string())?;
    }
    let mut reader = std::io::BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    if line.is_empty() {
        return Err("connection closed by fastty's daemon".to_string());
    }
    serde_json::from_str(&line).map_err(|e| format!("invalid daemon response: {e}"))
}

#[cfg(unix)]
fn sessions() -> Result<Vec<SessionInfo>, String> {
    match send_and_read(&Request::List)? {
        Response::Sessions { sessions } => Ok(sessions),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        _ => Err("unexpected daemon response to list".to_string()),
    }
}

#[cfg(unix)]
fn session_alive(id: usize) -> Result<Option<bool>, String> {
    Ok(sessions()?
        .into_iter()
        .find(|s| s.id == id)
        .map(|s| s.alive))
}

/// Renders a FST1 (v1/v2) snapshot as plain text: scrollback (if requested)
/// followed by the screen, trailing blank lines trimmed.
fn render_snapshot_text(data: &[u8], include_history: bool) -> Result<String, String> {
    let (header, cells) = crate::server::binary_snapshot::decode_snapshot(data)
        .ok_or_else(|| "invalid binary snapshot".to_string())?;
    let cols = header.cols as usize;
    let rows = header.rows as usize;
    if cols == 0 || rows == 0 {
        return Err("empty snapshot".to_string());
    }
    let history_rows = header.history_rows() as usize;
    let screen = &cells[history_rows * cols..];

    let mut lines: Vec<String> = Vec::new();
    if include_history && history_rows > 0 {
        let history = &cells[..history_rows * cols];
        for row in history.chunks_exact(cols) {
            lines.push(row_to_text(row));
        }
    }
    for row in screen.chunks_exact(cols) {
        lines.push(row_to_text(row));
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    Ok(lines.into_iter().map(|l| l.trim_end().to_string()).collect::<Vec<_>>().join("\n"))
}

fn row_to_text(row: &[crate::server::binary_snapshot::FasttyPackedCell]) -> String {
    row.iter()
        .map(|cell| {
            if cell.flags & crate::server::binary_snapshot::CELL_FLAG_WIDE_CHAR_SPACER != 0 {
                // The wide glyph before it already spans the column pair.
                return ' ';
            }
            char::from_u32(cell.c).unwrap_or(' ')
        })
        .collect()
}

#[cfg(unix)]
fn call_tool(name: &str, args: &Value) -> Result<String, String> {
    match name {
        "fastty_list_sessions" => {
            let sessions = sessions()?;
            Ok(serde_json::to_string_pretty(&sessions).unwrap_or_else(|_| "[]".to_string()))
        }
        "fastty_spawn_session" => {
            let command = args.get("command").and_then(Value::as_str);
            let empty_args: Vec<String> = Vec::new();
            let tool_args = args
                .get("args")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or(empty_args);
            let cwd = args.get("cwd").and_then(Value::as_str);
            let cols = args.get("cols").and_then(Value::as_u64).map(|v| v as usize);
            let rows = args.get("rows").and_then(Value::as_u64).map(|v| v as usize);
            let open = args.get("open").and_then(Value::as_bool).unwrap_or(true);
            match send_and_read(&Request::Spawn { command: command.map(str::to_string), args: tool_args, cwd: cwd.map(str::to_string), cols, rows, open })? {
                Response::Spawned { id, opened } => Ok(json!({
                    "id": id,
                    "note": if open && opened {
                        "session started and opened as a tab in the fastty window; type with fastty_write_session, read with fastty_read_screen"
                    } else if open {
                        "session started headless (no fastty window is running); type with fastty_write_session, read with fastty_read_screen"
                    } else {
                        "headless session started; type with fastty_write_session, read with fastty_read_screen"
                    }
                })
                .to_string()),
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected daemon response to spawn".to_string()),
            }
        }
        "fastty_write_session" => {
            let id = args.get("id").and_then(Value::as_u64).ok_or("missing id")? as usize;
            let text = args.get("text").and_then(Value::as_str).ok_or("missing text")?;
            let enter = args.get("enter").and_then(Value::as_bool).unwrap_or(false);
            let payload = if enter { format!("{text}\r") } else { text.to_string() };
            match send_and_read(&Request::Write {
                id,
                data: crate::daemon::base64_encode(payload.as_bytes()),
                ack: true,
            })? {
                Response::Done { .. } => Ok(json!({ "ok": true }).to_string()),
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected daemon response to write".to_string()),
            }
        }
        "fastty_read_screen" => {
            let id = args.get("id").and_then(Value::as_u64).ok_or("missing id")? as usize;
            let include_history = args
                .get("include_history")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match send_and_read(&Request::BinarySnapshot { id })? {
                Response::BinarySnapshot { data, .. } => {
                    let bytes = crate::daemon::base64_decode(&data)
                        .ok_or("invalid base64 in snapshot")?;
                    render_snapshot_text(&bytes, include_history)
                }
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected daemon response to binary_snapshot".to_string()),
            }
        }
        "fastty_resize_session" => {
            let id = args.get("id").and_then(Value::as_u64).ok_or("missing id")? as usize;
            let cols = args.get("cols").and_then(Value::as_u64).ok_or("missing cols")? as usize;
            let rows = args.get("rows").and_then(Value::as_u64).ok_or("missing rows")? as usize;
            match send_and_read(&Request::Resize { id, cols, rows, ack: true })? {
                Response::Done { .. } => Ok(json!({ "ok": true }).to_string()),
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected daemon response to resize".to_string()),
            }
        }
        "fastty_close_session" => {
            let id = args.get("id").and_then(Value::as_u64).ok_or("missing id")? as usize;
            let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
            match send_and_read(&Request::Close { id, force })? {
                Response::Closed { id } => Ok(json!({ "closed": id }).to_string()),
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected daemon response to close".to_string()),
            }
        }
        "fastty_layout" => {
            match send_and_read(&Request::Layout)? {
                Response::Layout { tabs } => {
                    Ok(serde_json::to_string_pretty(&tabs).unwrap_or_else(|_| "[]".to_string()))
                }
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected daemon response to layout".to_string()),
            }
        }
        "fastty_split_pane" => {
            let id = args.get("id").and_then(Value::as_u64).ok_or("missing id")? as usize;
            let direction = args
                .get("direction")
                .and_then(Value::as_str)
                .ok_or("missing direction")?
                .to_string();
            let command = args.get("command").and_then(Value::as_str);
            let empty_args: Vec<String> = Vec::new();
            let split_args = args
                .get("args")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or(empty_args);
            let cwd = args.get("cwd").and_then(Value::as_str);
            match send_and_read(&Request::SplitPane {
                id,
                direction,
                command: command.map(str::to_string),
                args: split_args,
                cwd: cwd.map(str::to_string),
            })? {
                Response::PaneSplit { id } => Ok(json!({
                    "id": id,
                    "note": "pane created next to the target; it is a normal session (read/write/close work)"
                })
                .to_string()),
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected daemon response to split_pane".to_string()),
            }
        }
        "fastty_resize_pane" => {
            let id = args.get("id").and_then(Value::as_u64).ok_or("missing id")? as usize;
            let direction = args
                .get("direction")
                .and_then(Value::as_str)
                .ok_or("missing direction")?
                .to_string();
            let delta = args
                .get("delta")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .unwrap_or(0.05);
            match send_and_read(&Request::ResizePane { id, direction, delta })? {
                Response::Done { .. } => Ok(json!({ "ok": true }).to_string()),
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected daemon response to resize_pane".to_string()),
            }
        }
        "fastty_focus_pane" => {
            let id = args.get("id").and_then(Value::as_u64).ok_or("missing id")? as usize;
            match send_and_read(&Request::FocusPane { id })? {
                Response::Done { .. } => Ok(json!({ "ok": true }).to_string()),
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected daemon response to focus_pane".to_string()),
            }
        }
        "fastty_resize_window" => {
            let cols = args.get("cols").and_then(Value::as_u64).ok_or("missing cols")? as usize;
            let rows = args.get("rows").and_then(Value::as_u64).ok_or("missing rows")? as usize;
            match send_and_read(&Request::ResizeWindow { cols, rows })? {
                Response::Done { .. } => Ok(json!({ "ok": true }).to_string()),
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected daemon response to resize_window".to_string()),
            }
        }
        "fastty_run_command" => {
            let command = args
                .get("command")
                .and_then(Value::as_str)
                .ok_or("missing command")?;
            let cwd = args.get("cwd").and_then(Value::as_str);
            let timeout_ms = args
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_TIMEOUT_MS)
                .max(500);
            run_command(command, cwd, timeout_ms)
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

/// Spawns `$SHELL -c <command>` in a throwaway headless session, waits for
/// the process to exit, reads the terminal, and closes the session.
#[cfg(unix)]
fn run_command(command: &str, cwd: Option<&str>, timeout_ms: u64) -> Result<String, String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let shell_args = vec!["-c".to_string(), command.to_string()];
    let (shell, shell_args) = (shell.as_str(), shell_args);

    let id = match send_and_read(&Request::Spawn {
        command: Some(shell.to_string()),
        args: shell_args,
        cwd: cwd.map(str::to_string),
        cols: Some(RUN_COMMAND_COLS),
        rows: Some(RUN_COMMAND_ROWS),
        open: false,
    })? {
        Response::Spawned { id, .. } => id,
        Response::Error { code, message } => return Err(format!("{code}: {message}")),
        _ => return Err("unexpected daemon response to spawn".to_string()),
    };

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut timed_out = false;
    loop {
        match session_alive(id) {
            Ok(Some(false)) => break,
            Ok(Some(true)) => {}
            Ok(None) => break, // session vanished
            Err(e) => {
                let _ = close_session_quietly(id);
                return Err(e);
            }
        }
        if Instant::now() >= deadline {
            timed_out = true;
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    let output = match send_and_read(&Request::BinarySnapshot { id })? {
        Response::BinarySnapshot { data, .. } => {
            let bytes = crate::daemon::base64_decode(&data).ok_or("invalid base64 in snapshot")?;
            render_snapshot_text(&bytes, true).unwrap_or_default()
        }
        _ => String::new(),
    };
    close_session_quietly(id)?;

    // Keep the context bounded: agents care about the tail of build logs.
    let non_empty: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = non_empty.len().saturating_sub(RUN_COMMAND_MAX_LINES);
    let mut text = non_empty[start..].join("\n");
    if text.is_empty() {
        text = "(no output)".to_string();
    }
    if timed_out {
        text.push_str(&format!(
            "\n(fastty_run_command: command did not exit within {timeout_ms}ms; output above may be partial, session closed)"
        ));
    }
    Ok(text)
}

#[cfg(unix)]
fn close_session_quietly(id: usize) -> Result<(), String> {
    match send_and_read(&Request::Close { id, force: false })? {
        Response::Closed { .. } => Ok(()),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        _ => Ok(()),
    }
}

/// Non-Unix fallback: the daemon only listens on Unix sockets, so tool
/// calls can't work. `fastty mcp setup` remains usable on every platform.
#[cfg(not(unix))]
fn call_tool(_name: &str, _args: &Value) -> Result<String, String> {
    Err(
        "fastty MCP tools are not available on this platform: the daemon only \
         listens on Unix sockets."
            .to_string(),
    )
}

// ---------------------------------------------------------------------------
// stdio loop
// ---------------------------------------------------------------------------

/// Entry point for `fastty mcp`. Never returns; exits when stdin closes.
#[cfg(unix)]
pub fn run_mcp_server() -> ! {
    let _ = crate::paths::init();

    // Talk to fastty's GUI daemon when it's up; otherwise embed one, like
    // `fastty gateway` does, so agents work without the app open.
    #[cfg(unix)]
    {
        let sock = crate::daemon::socket_path();
        if std::os::unix::net::UnixStream::connect(&sock).is_err() {
            crate::daemon::start();
            let _ = crate::daemon::ensure_default_session();
        }
    }

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => std::process::exit(0), // client closed the pipe
            Ok(_) => {}
            Err(_) => std::process::exit(1),
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(trimmed) {
            Ok(msg) => dispatch(&msg).map(|v| v.to_string()),
            Err(e) => Some(
                rpc_error(Value::Null, -32700, &format!("parse error: {e}")).to_string(),
            ),
        };
        if let Some(resp) = response {
            if out.write_all(resp.as_bytes()).is_err() || out.write_all(b"\n").is_err() {
                std::process::exit(1);
            }
            let _ = out.flush();
        }
    }
}

/// Non-Unix fallback for `fastty mcp`: server mode needs the socket daemon.
#[cfg(not(unix))]
pub fn run_mcp_server() -> ! {
    eprintln!(
        "fastty mcp: not supported on this platform yet (the daemon only \
         listens on Unix sockets)."
    );
    std::process::exit(1)
}

// ---------------------------------------------------------------------------
// `fastty mcp setup` — register the server into AI agents' configs
// ---------------------------------------------------------------------------

/// The agents `fastty mcp setup` can write a registration into.
const SETUP_AGENTS: &[(&str, &str)] = &[
    ("claude-code", "Project .mcp.json (Claude Code / Claude CLI)"),
    ("opencode", "OpenCode (opencode.json)"),
    ("codex", "OpenAI Codex CLI (config.toml)"),
    ("gemini", "Google Gemini CLI (settings.json)"),
    ("cursor", "Cursor (mcp.json)"),
    ("zed", "Zed editor (settings.json)"),
    ("claude-desktop", "Claude Desktop app"),
];

/// Config file path for an agent, by platform. All of them live under the
/// home directory except Claude Desktop and Zed, which follow each OS's
/// app-config convention.
fn agent_config_path(agent: &str) -> Option<std::path::PathBuf> {
    let home = dirs::home_dir()?;
    let p = |home: &std::path::Path, rel: &[&str]| {
        let mut buf = home.to_path_buf();
        for part in rel {
            buf.push(part);
        }
        buf
    };
    match agent {
        "claude-code" => std::env::current_dir().map(|cwd| cwd.join(".mcp.json")).ok(),
        "opencode" => Some(p(&home, &[".config", "opencode", "opencode.json"])),
        "codex" => Some(p(&home, &[".codex", "config.toml"])),
        "gemini" => Some(p(&home, &[".gemini", "settings.json"])),
        "cursor" => Some(p(&home, &[".cursor", "mcp.json"])),
        "claude-desktop" => {
            #[cfg(target_os = "macos")]
            {
                Some(p(&home, &["Library", "Application Support", "Claude", "claude_desktop_config.json"]))
            }
            #[cfg(windows)]
            {
                std::env::var("APPDATA")
                    .ok()
                    .map(|d| std::path::PathBuf::from(d).join("Claude").join("claude_desktop_config.json"))
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                Some(p(&home, &[".config", "Claude", "claude_desktop_config.json"]))
            }
        }
        "zed" => {
            #[cfg(target_os = "macos")]
            {
                Some(p(&home, &["Library", "Application Support", "Zed", "settings.json"]))
            }
            #[cfg(windows)]
            {
                std::env::var("APPDATA")
                    .ok()
                    .map(|d| std::path::PathBuf::from(d).join("Zed").join("settings.json"))
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                Some(p(&home, &[".config", "zed", "settings.json"]))
            }
        }
        _ => None,
    }
}

/// The `fastty` executable path written into agent configs: this process's
/// own binary, so registration works even when fastty isn't on the agent's
/// PATH.
fn fastty_command() -> String {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "fastty".to_string())
}

/// The generic `mcpServers` shape shared by Claude Code, Gemini CLI, Cursor
/// and Claude Desktop. Returns (new content, changed).
fn merge_json_mcp_servers(content: &str, exe: &str) -> Result<(String, bool), String> {
    let mut root: Value = if content.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(content).map_err(|e| format!("not valid JSON: {e}"))?
    };
    let entry = json!({ "command": exe, "args": ["mcp"] });
    let servers = root
        .as_object_mut()
        .ok_or("JSON root must be an object")?
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    let servers = servers
        .as_object_mut()
        .ok_or("\"mcpServers\" must be an object")?;
    if servers.get("fastty") == Some(&entry) {
        return Ok((content.to_string(), false));
    }
    servers.insert("fastty".to_string(), entry);
    Ok((serde_json::to_string_pretty(&root).unwrap(), true))
}

/// OpenCode's shape: `mcp.fastty = { type: "local", command: [...], enabled }`.
fn merge_opencode(content: &str, exe: &str) -> Result<(String, bool), String> {
    let mut root: Value = if content.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(content).map_err(|e| format!("not valid JSON: {e}"))?
    };
    let entry = json!({ "type": "local", "command": [exe, "mcp"], "enabled": true });
    let mcp = root
        .as_object_mut()
        .ok_or("JSON root must be an object")?
        .entry("mcp")
        .or_insert_with(|| json!({}));
    let mcp = mcp.as_object_mut().ok_or("\"mcp\" must be an object")?;
    if mcp.get("fastty") == Some(&entry) {
        return Ok((content.to_string(), false));
    }
    mcp.insert("fastty".to_string(), entry);
    Ok((serde_json::to_string_pretty(&root).unwrap(), true))
}

/// Zed's shape: `context_servers.fastty = { source: "custom", command, args }`.
fn merge_zed(content: &str, exe: &str) -> Result<(String, bool), String> {
    let mut root: Value = if content.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(content).map_err(|e| format!("not valid JSON: {e}"))?
    };
    let entry = json!({ "source": "custom", "command": exe, "args": ["mcp"] });
    let servers = root
        .as_object_mut()
        .ok_or("JSON root must be an object")?
        .entry("context_servers")
        .or_insert_with(|| json!({}));
    let servers = servers
        .as_object_mut()
        .ok_or("\"context_servers\" must be an object")?;
    if servers.get("fastty") == Some(&entry) {
        return Ok((content.to_string(), false));
    }
    servers.insert("fastty".to_string(), entry);
    Ok((serde_json::to_string_pretty(&root).unwrap(), true))
}

/// Codex CLI's TOML: `[mcp_servers.fastty]`. toml_edit preserves the user's
/// comments and formatting.
fn merge_codex(content: &str, exe: &str) -> Result<(String, bool), String> {
    let mut doc = if content.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        content
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| format!("not valid TOML: {e}"))?
    };
    let table = &mut doc["mcp_servers"];
    if !table.is_table() {
        *table = toml_edit::Item::Table(toml_edit::Table::new());
    }
    if let Some(servers) = table.as_table_mut() {
        let existing = servers
            .get("fastty")
            .and_then(|i| i.as_table())
            .and_then(|t| t.get("command"))
            .and_then(|i| i.as_str())
            .map(str::to_string);
        if existing.as_deref() == Some(exe) {
            return Ok((content.to_string(), false));
        }
        let mut fastty = toml_edit::Table::new();
        fastty["command"] = toml_edit::value(exe);
        let mut args = toml_edit::Array::new();
        args.push("mcp");
        fastty["args"] = toml_edit::value(args);
        servers.insert("fastty", toml_edit::Item::Table(fastty));
    }
    Ok((doc.to_string(), true))
}

/// Merges fastty's MCP registration into `agent`'s config file, creating
/// the file (and parent dirs) when missing. Returns a user-facing message.
fn register_agent(agent: &str) -> Result<String, String> {
    let path = agent_config_path(agent)
        .ok_or_else(|| format!("unknown agent: {agent} (see `fastty mcp setup`)"))?;
    let exe = fastty_command();
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let (new_content, changed) = match agent {
        "opencode" => merge_opencode(&content, &exe)?,
        "zed" => merge_zed(&content, &exe)?,
        "codex" => merge_codex(&content, &exe)?,
        _ => merge_json_mcp_servers(&content, &exe)?,
    };
    if !changed {
        return Ok(format!(
            "fastty MCP is already registered in {agent} ({}).",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("couldn't create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, new_content)
        .map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    Ok(format!(
        "Registered fastty MCP into {agent}: {}\nRestart the agent so it picks up the new server.",
        path.display()
    ))
}

/// Entry point for `fastty mcp setup [AGENT]`. Without an agent, lists the
/// supported ones and where their configs live. Returns a process exit code.
pub fn run_mcp_setup(agent: Option<&str>) -> i32 {
    match agent {
        Some(agent) => match register_agent(agent) {
            Ok(msg) => {
                println!("{msg}");
                0
            }
            Err(e) => {
                eprintln!("fastty mcp setup: {e}");
                1
            }
        },
        None => {
            println!("Register fastty's MCP server into an AI agent's config:\n");
            println!("  fastty mcp setup <agent>\n");
            println!("Supported agents:");
            for (name, description) in SETUP_AGENTS {
                let path = agent_config_path(name)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "?".to_string());
                println!("  {name:<15} {description} ({path})");
            }
            println!("\nThen talk to your terminals with fastty_list_sessions, fastty_run_command, and friends.");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::prelude::*;

    fn fst1_bytes(history: usize, rows: usize, cols: usize, fill: char) -> Vec<u8> {
        let mut header = crate::server::binary_snapshot::FasttyBinarySnapshotHeader::default();
        header.cols = cols as u16;
        header.rows = rows as u16;
        header.cell_count = ((history + rows) * cols) as u32;
        header.set_history_rows(history as u32);
        let mut cells = Vec::new();
        for r in 0..history + rows {
            for c in 0..cols {
                cells.push(crate::server::binary_snapshot::FasttyPackedCell {
                    c: if c == 0 { fill as u32 } else if c < 5 {
                        (b'0' + (r % 10) as u8) as u32
                    } else {
                        ' ' as u32
                    },
                    fg: 0,
                    bg: 0,
                    flags: 0,
                    _reserved: 0,
                });
            }
        }
        crate::server::binary_snapshot::encode_snapshot(&header, &cells)
    }

    #[test]
    fn render_snapshot_text_history_and_screen() {
        let data = fst1_bytes(2, 3, 8, 'H');
        let text = render_snapshot_text(&data, true).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        // 2 history + 3 screen rows, blank tails trimmed.
        assert_eq!(lines.len(), 5);
        assert!(lines[0].starts_with('H'));
        assert!(lines[4].starts_with('H'));

        let screen_only = render_snapshot_text(&data, false).unwrap();
        assert_eq!(screen_only.lines().count(), 3);
    }

    #[test]
    fn render_snapshot_rejects_garbage() {
        assert!(render_snapshot_text(b"nope", true).is_err());
    }

    #[test]
    fn dispatch_initialize_and_ping() {
        let msg = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "clientInfo": { "name": "test", "version": "0" } }
        });
        let resp = dispatch(&msg).unwrap();
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["serverInfo"]["name"], "fastty");
        assert_eq!(resp["result"]["protocolVersion"], "2025-06-18");
        assert!(resp["result"]["capabilities"]["tools"].is_object());

        let ping = json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" });
        let resp = dispatch(&ping).unwrap();
        assert_eq!(resp["result"], json!({}));
    }

    #[test]
    fn dispatch_lists_all_tools_with_schemas() {
        let msg = json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" });
        let resp = dispatch(&msg).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        let expected = [
            "fastty_list_sessions",
            "fastty_spawn_session",
            "fastty_write_session",
            "fastty_read_screen",
            "fastty_resize_session",
            "fastty_close_session",
            "fastty_run_command",
            "fastty_layout",
            "fastty_split_pane",
            "fastty_resize_pane",
            "fastty_focus_pane",
            "fastty_resize_window",
        ];
        assert_eq!(tools.len(), expected.len());
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for name in expected {
            assert!(names.contains(&name), "missing tool {name}");
        }
        for t in tools {
            assert!(t["inputSchema"]["properties"].is_object(), "tool {} schema", t["name"]);
        }
    }

    #[test]
    fn dispatch_notifications_get_no_response() {
        let msg = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(dispatch(&msg).is_none());
    }

    #[test]
    fn dispatch_unknown_method_is_32601() {
        let msg = json!({ "jsonrpc": "2.0", "id": 9, "method": "resources/list" });
        let resp = dispatch(&msg).unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn dispatch_unknown_tool_returns_is_error() {
        let msg = json!({
            "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": { "name": "fastty_nope", "arguments": {} }
        });
        let resp = dispatch(&msg).unwrap();
        assert_eq!(resp["result"]["isError"], true);
    }

    /// End-to-end through an embedded daemon: run_command must come back
    /// with the command's output. Unix only (the daemon is socket-based).
    #[cfg(unix)]
    #[test]
    fn run_command_echoes_output_via_embedded_daemon() {
        let _ = crate::paths::init();
        let sock = crate::daemon::socket_path();
        if std::os::unix::net::UnixStream::connect(&sock).is_err() {
            crate::daemon::start();
        }
        let _ = crate::daemon::ensure_default_session();

        let out = call_tool(
            "fastty_run_command",
            &json!({ "command": "echo fastty-mcp-e2e-ok", "timeout_ms": 15000 }),
        )
        .expect("run_command should succeed");
        assert!(out.contains("fastty-mcp-e2e-ok"), "output: {out}");
    }

    #[test]
    fn merge_json_mcp_servers_creates_and_is_idempotent() {
        let exe = "/opt/fastty/bin/fastty";
        let (out, changed) = merge_json_mcp_servers("", exe).unwrap();
        assert!(changed);
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["mcpServers"]["fastty"]["command"], exe);
        assert_eq!(parsed["mcpServers"]["fastty"]["args"], json!(["mcp"]));
        let (out2, changed2) = merge_json_mcp_servers(&out, exe).unwrap();
        assert!(!changed2);
        assert_eq!(out2, out);
    }

    #[test]
    fn merge_json_mcp_servers_preserves_existing_keys() {
        let existing = r#"{
  "mcpServers": {
    "other": { "command": "npx", "args": ["-y", "other-mcp"] }
  },
  "otherTopLevel": true
}"#;
        let (out, changed) = merge_json_mcp_servers(existing, "/usr/bin/fastty").unwrap();
        assert!(changed);
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["mcpServers"]["other"]["command"], "npx");
        assert_eq!(parsed["otherTopLevel"], true);
        assert_eq!(parsed["mcpServers"]["fastty"]["command"], "/usr/bin/fastty");
    }

    #[test]
    fn merge_opencode_uses_local_command_array() {
        let (out, changed) = merge_opencode("", "/usr/local/bin/fastty").unwrap();
        assert!(changed);
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["mcp"]["fastty"]["type"], "local");
        assert_eq!(parsed["mcp"]["fastty"]["command"], json!(["/usr/local/bin/fastty", "mcp"]));
        assert_eq!(parsed["mcp"]["fastty"]["enabled"], true);
        let (_, changed2) = merge_opencode(&out, "/usr/local/bin/fastty").unwrap();
        assert!(!changed2);
    }

    #[test]
    fn merge_zed_uses_context_servers_custom() {
        let (out, changed) = merge_zed("", "/usr/local/bin/fastty").unwrap();
        assert!(changed);
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["context_servers"]["fastty"]["source"], "custom");
        assert_eq!(parsed["context_servers"]["fastty"]["command"], "/usr/local/bin/fastty");
        assert_eq!(parsed["context_servers"]["fastty"]["args"], json!(["mcp"]));
    }

    #[test]
    fn merge_codex_toml_preserves_comments_and_is_idempotent() {
        let existing = "# my codex config\nmodel = \"gpt-5.5\"\n\n[mcp_servers.other]\ncommand = \"bun\"\nargs = [\"x\", \"other\"]\n";
        let (out, changed) = merge_codex(existing, "/usr/local/bin/fastty").unwrap();
        assert!(changed);
        assert!(out.contains("# my codex config"), "comments must survive");
        assert!(out.contains("model = \"gpt-5.5\""));
        assert!(out.contains("[mcp_servers.other]"));
        assert!(out.contains("[mcp_servers.fastty]"));
        assert!(out.contains("command = \"/usr/local/bin/fastty\""));
        let (out2, changed2) = merge_codex(&out, "/usr/local/bin/fastty").unwrap();
        assert!(!changed2);
        assert_eq!(out2, out);
    }

    #[test]
    fn every_setup_agent_has_a_config_path() {
        for (name, _) in SETUP_AGENTS {
            assert!(agent_config_path(name).is_some(), "agent {name} has no path");
        }
        assert!(agent_config_path("not-an-agent").is_none());
    }

    #[test]
    fn register_agent_rejects_unknown_names() {
        assert!(register_agent("not-an-agent").is_err());
    }
}
