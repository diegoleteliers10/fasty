//! `fastty sessions` / `fastty attach <id>` — a CLI reference client for the
//! daemon in `crate::daemon`, speaking the exact protocol documented in
//! `docs/daemon-protocol.md` using the exact same `Request`/`Response` types
//! the daemon itself serializes, so this can never drift out of sync with
//! what fastty actually understands on the wire.
//!
//! This exists mainly as living documentation and a debugging tool: before
//! this, the only way to poke at the daemon was to hand-write a socket
//! client in another language. `fastty attach <id>` is also a genuinely
//! useful thing on its own — a terminal-native way to peek at (and type
//! into) a tab that's open in another window, or over SSH into a machine
//! that's already running fastty.

#[cfg(unix)]
use crate::daemon::{AttachMode, Request, Response};

#[cfg(unix)]
pub(crate) fn connect() -> Result<std::os::unix::net::UnixStream, String> {
    let path = crate::daemon::socket_path();
    std::os::unix::net::UnixStream::connect(&path).map_err(|e| {
        format!(
            "couldn't reach fastty's daemon socket at {}: {e}\n(is fastty running?)",
            path.display()
        )
    })
}

/// Like `connect`, but if `wait_secs` is set, retries every 250ms instead of
/// failing on the first attempt — for a script or plugin that wants to say
/// "wait for fastty to start" instead of racing it. `None` keeps the
/// original one-shot behavior.
#[cfg(unix)]
fn connect_with_retry(wait_secs: Option<u64>) -> Result<std::os::unix::net::UnixStream, String> {
    let Some(wait_secs) = wait_secs else {
        return connect();
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_secs);
    let mut printed_waiting = false;
    loop {
        match connect() {
            Ok(s) => return Ok(s),
            Err(e) => {
                if std::time::Instant::now() >= deadline {
                    return Err(format!("{e}\n(gave up after waiting {wait_secs}s)"));
                }
                if !printed_waiting {
                    eprintln!("Waiting for fastty to start (up to {wait_secs}s)...");
                    printed_waiting = true;
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
}

#[cfg(unix)]
fn send_request(stream: &mut std::os::unix::net::UnixStream, req: &Request) -> Result<(), String> {
    use std::io::Write;
    let mut line = serde_json::to_string(req).map_err(|e| e.to_string())?;
    line.push('\n');
    stream.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(unix)]
fn read_response(reader: &mut impl std::io::BufRead) -> Result<Response, String> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("connection closed by fastty".to_string());
    }
    serde_json::from_str(line.trim_end()).map_err(|e| format!("bad response from daemon: {e}"))
}

#[cfg(unix)]
fn print_sessions_table(sessions: &[crate::daemon::SessionInfo]) {
    if sessions.is_empty() {
        println!("No fastty sessions.");
        return;
    }
    let id_w = sessions
        .iter()
        .map(|s| s.id.to_string().len())
        .max()
        .unwrap_or(2)
        .max(2);
    let title_w = sessions
        .iter()
        .map(|s| s.title.len())
        .max()
        .unwrap_or(5)
        .max(5);
    println!("{:<id_w$}  {:<title_w$}  ALIVE  LINES  CWD", "ID", "TITLE");
    for s in sessions {
        println!(
            "{:<id_w$}  {:<title_w$}  {:<5}  {:<5}  {}",
            s.id,
            s.title,
            if s.alive { "yes" } else { "no" },
            s.history_size,
            s.cwd.as_deref().unwrap_or("-"),
        );
    }
}

pub fn run_sessions_command(watch: bool, wait_secs: Option<u64>, json: bool) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (watch, wait_secs, json);
        eprintln!(
            "fastty sessions: not supported on this platform yet \
             (the daemon only listens on Unix domain sockets)."
        );
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        let mut stream = match connect_with_retry(wait_secs) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fastty sessions: {e}");
                std::process::exit(1);
            }
        };
        let request = if watch {
            Request::SubscribeSessions
        } else {
            Request::List
        };
        if let Err(e) = send_request(&mut stream, &request) {
            eprintln!("fastty sessions: {e}");
            std::process::exit(1);
        }
        let mut reader = std::io::BufReader::new(stream);

        if !watch {
            match read_response(&mut reader) {
                Ok(Response::Sessions { sessions }) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&sessions)
                                .unwrap_or_else(|_| "[]".to_string())
                        );
                    } else {
                        print_sessions_table(&sessions);
                    }
                    std::process::exit(0);
                }
                Ok(Response::Error { code, message }) => {
                    eprintln!("fastty sessions: {code}: {message}");
                    std::process::exit(1);
                }
                Ok(other) => {
                    eprintln!("fastty sessions: unexpected response from daemon: {other:?}");
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("fastty sessions: {e}");
                    std::process::exit(1);
                }
            }
        }

        // --watch: print the baseline table, then one line per delta as
        // they arrive, until the daemon disconnects (fastty quit) or we're
        // killed (Ctrl+C in the default, non-raw terminal mode this runs in
        // -- there's nothing to forward here, so no need to fight SIGINT).
        loop {
            match read_response(&mut reader) {
                Ok(Response::Sessions { sessions }) => {
                    if json {
                        println!("{}", serde_json::to_string(&sessions).unwrap_or_default());
                    } else {
                        print_sessions_table(&sessions);
                    }
                    let _ = std::io::stdout().flush();
                }
                Ok(Response::SessionAdded { session }) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({ "event": "session_added", "session": session })
                        );
                    } else {
                        println!(
                            "+ [{}] {} ({})",
                            session.id,
                            session.title,
                            session.cwd.as_deref().unwrap_or("-")
                        );
                    }
                    let _ = std::io::stdout().flush();
                }
                Ok(Response::SessionRemoved { id }) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({ "event": "session_removed", "id": id })
                        );
                    } else {
                        println!("- [{id}]");
                    }
                    let _ = std::io::stdout().flush();
                }
                Ok(Response::SessionUpdated { session }) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({ "event": "session_updated", "session": session })
                        );
                    } else {
                        println!(
                            "~ [{}] {} ({})",
                            session.id,
                            session.title,
                            session.cwd.as_deref().unwrap_or("-")
                        );
                    }
                    let _ = std::io::stdout().flush();
                }
                Ok(Response::Error { code, message }) => {
                    eprintln!("fastty sessions: {code}: {message}");
                    std::process::exit(1);
                }
                Ok(_) => continue,
                Err(e) => {
                    eprintln!("fastty sessions: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}

pub fn run_capture_command(id: usize, wait_secs: Option<u64>) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (id, wait_secs);
        eprintln!("fastty capture: not supported on this platform yet.");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        let mut stream = match connect_with_retry(wait_secs) {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("fastty capture: {error}");
                std::process::exit(1);
            }
        };
        if let Err(error) = send_request(&mut stream, &Request::Hello { read_only: true }) {
            eprintln!("fastty capture: {error}");
            std::process::exit(1);
        }
        let mut reader = std::io::BufReader::new(stream);
        if !matches!(read_response(&mut reader), Ok(Response::Hello { .. })) {
            eprintln!("fastty capture: daemon did not accept read-only mode");
            std::process::exit(1);
        }
        let mut stream = reader.into_inner();
        if let Err(error) = send_request(&mut stream, &Request::Capture { id }) {
            eprintln!("fastty capture: {error}");
            std::process::exit(1);
        }
        match read_response(&mut std::io::BufReader::new(stream)) {
            Ok(Response::Captured {
                data, truncated, ..
            }) => {
                if std::io::stdout().write_all(data.as_bytes()).is_err() {
                    std::process::exit(1);
                }
                if truncated {
                    eprintln!("fastty capture: output was truncated at 1 MiB");
                    std::process::exit(2);
                }
                std::process::exit(0);
            }
            Ok(Response::Error { code, message }) => {
                eprintln!("fastty capture: {code}: {message}");
                std::process::exit(1);
            }
            Ok(_) => {
                eprintln!("fastty capture: unexpected response from fastty");
                std::process::exit(1);
            }
            Err(error) => {
                eprintln!("fastty capture: {error}");
                std::process::exit(1);
            }
        }
    }
}

pub fn run_focus_command(id: usize, wait_secs: Option<u64>) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (id, wait_secs);
        eprintln!("fastty focus: not supported on this platform yet.");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        let mut stream = match connect_with_retry(wait_secs) {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("fastty focus: {error}");
                std::process::exit(1);
            }
        };
        if let Err(error) = send_request(&mut stream, &Request::FocusPane { id }) {
            eprintln!("fastty focus: {error}");
            std::process::exit(1);
        }
        match read_response(&mut std::io::BufReader::new(stream)) {
            Ok(Response::Done { .. }) => std::process::exit(0),
            Ok(Response::Error { code, message }) => {
                eprintln!("fastty focus: {code}: {message}");
                std::process::exit(1);
            }
            Ok(_) => {
                eprintln!("fastty focus: unexpected response from fastty");
                std::process::exit(1);
            }
            Err(error) => {
                eprintln!("fastty focus: {error}");
                std::process::exit(1);
            }
        }
    }
}

pub fn run_events_command(wait_secs: Option<u64>) -> ! {
    #[cfg(not(unix))]
    {
        let _ = wait_secs;
        eprintln!("fastty events: not supported on this platform yet.");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        let mut stream = match connect_with_retry(wait_secs) {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("fastty events: {error}");
                std::process::exit(1);
            }
        };
        if let Err(error) = send_request(&mut stream, &Request::Hello { read_only: true }) {
            eprintln!("fastty events: {error}");
            std::process::exit(1);
        }
        let mut reader = std::io::BufReader::new(stream);
        if !matches!(read_response(&mut reader), Ok(Response::Hello { .. })) {
            eprintln!("fastty events: daemon did not accept read-only mode");
            std::process::exit(1);
        }
        let mut stream = reader.into_inner();
        if let Err(error) = send_request(&mut stream, &Request::SubscribeEvents) {
            eprintln!("fastty events: {error}");
            std::process::exit(1);
        }
        let mut reader = std::io::BufReader::new(stream);
        loop {
            let response = match read_response(&mut reader) {
                Ok(response) => response,
                Err(error) => {
                    eprintln!("fastty events: {error}");
                    std::process::exit(1);
                }
            };
            let event = match response {
                Response::EventStreamStarted {
                    version,
                    sequence,
                    sessions,
                } => {
                    serde_json::json!({
                        "version": version,
                        "type": "snapshot",
                        "sequence": sequence,
                        "sessions": sessions,
                    })
                }
                Response::EventStreamGap {
                    version,
                    after_sequence,
                    oldest_sequence,
                    latest_sequence,
                    sessions,
                } => {
                    let [gap, snapshot] = event_stream_gap_jsonl(
                        version,
                        after_sequence,
                        oldest_sequence,
                        latest_sequence,
                        sessions,
                    );
                    for event in [gap, snapshot] {
                        if writeln!(std::io::stdout(), "{event}").is_err() {
                            std::process::exit(1);
                        }
                    }
                    let _ = std::io::stdout().flush();
                    continue;
                }
                Response::StreamEvent { envelope } => {
                    let metadata = serde_json::json!({
                        "version": envelope.version,
                        "sequence": envelope.sequence,
                        "timestamp_unix_ms": envelope.timestamp_unix_ms,
                        "session_id": envelope.session_id,
                    });
                    match envelope.kind {
                        crate::daemon::SessionEvent::SessionAdded { session } => {
                            serde_json::json!({
                                "version": metadata["version"],
                                "type": "session_added",
                                "sequence": metadata["sequence"],
                                "timestamp_unix_ms": metadata["timestamp_unix_ms"],
                                "session_id": metadata["session_id"],
                                "session": session,
                            })
                        }
                        crate::daemon::SessionEvent::SessionRemoved { id } => serde_json::json!({
                            "version": metadata["version"],
                            "type": "session_removed",
                            "sequence": metadata["sequence"],
                            "timestamp_unix_ms": metadata["timestamp_unix_ms"],
                            "session_id": metadata["session_id"],
                            "id": id,
                        }),
                        crate::daemon::SessionEvent::SessionUpdated {
                            session,
                            terminal_state_changed,
                        } => {
                            serde_json::json!({
                                "version": metadata["version"],
                                "type": if terminal_state_changed { "terminal_state_changed" } else { "session_updated" },
                                "sequence": metadata["sequence"],
                                "timestamp_unix_ms": metadata["timestamp_unix_ms"],
                                "session_id": metadata["session_id"],
                                "session": session,
                            })
                        }
                    }
                }
                Response::Error { code, message } => {
                    eprintln!("fastty events: {code}: {message}");
                    std::process::exit(1);
                }
                _ => continue,
            };
            if writeln!(std::io::stdout(), "{event}").is_err() {
                std::process::exit(1);
            }
            let _ = std::io::stdout().flush();
        }
    }
}

fn event_stream_gap_jsonl<S: serde::Serialize>(
    version: u16,
    after_sequence: u64,
    oldest_sequence: u64,
    latest_sequence: u64,
    sessions: S,
) -> [serde_json::Value; 2] {
    [
        serde_json::json!({
            "version": version,
            "type": "gap",
            "after_sequence": after_sequence,
            "oldest_sequence": oldest_sequence,
            "latest_sequence": latest_sequence,
        }),
        serde_json::json!({
            "version": version,
            "type": "snapshot",
            "sequence": latest_sequence,
            "sessions": sessions,
        }),
    ]
}

#[cfg(test)]
mod event_stream_cli_tests {
    use super::event_stream_gap_jsonl;
    use serde_json::json;

    #[test]
    fn event_gap_outputs_recovery_marker_and_replacement_snapshot() {
        let lines = event_stream_gap_jsonl(1, 4, 10, 12, json!([{"id": 7}]));

        assert_eq!(lines[0]["type"], "gap");
        assert_eq!(lines[0]["after_sequence"], 4);
        assert_eq!(lines[0]["oldest_sequence"], 10);
        assert_eq!(lines[0]["latest_sequence"], 12);
        assert_eq!(lines[1]["type"], "snapshot");
        assert_eq!(lines[1]["sequence"], 12);
        assert_eq!(lines[1]["sessions"][0]["id"], 7);
    }
}

#[cfg(unix)]
struct RawModeGuard {
    original: libc::termios,
    active: bool,
}

#[cfg(unix)]
impl RawModeGuard {
    /// Puts *this process's* stdin into raw mode (no line buffering, no
    /// local echo, signal-generating keys like Ctrl+C passed through as
    /// plain bytes) so every keystroke goes straight to the remote pane,
    /// same as `ssh`/`tmux attach`. Restored automatically on drop or
    /// explicit `disable()`.
    fn enable() -> Option<Self> {
        unsafe {
            let mut original: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut original) != 0 {
                return None;
            }
            let mut raw = original;
            libc::cfmakeraw(&mut raw);
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) != 0 {
                return None;
            }
            Some(Self {
                original,
                active: true,
            })
        }
    }

    fn disable(&mut self) {
        if self.active {
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original);
            }
            self.active = false;
        }
    }
}

#[cfg(unix)]
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        self.disable();
    }
}

/// Byte for the local "detach and exit" shortcut, Ctrl+\ (ASCII FS). Chosen
/// because it's the traditional SIGQUIT key in cooked mode and essentially
/// never typed on purpose inside a normal shell session, unlike Ctrl+C/D
/// which must be forwarded to the remote pane untouched.
const DETACH_BYTE: u8 = 0x1c;

pub fn run_attach_command(id: usize, read_only: bool, wait_secs: Option<u64>) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (id, read_only, wait_secs);
        eprintln!(
            "fastty attach: not supported on this platform yet \
             (the daemon only listens on Unix domain sockets)."
        );
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::io::{Read, Write};

        let mode = if read_only {
            AttachMode::ReadOnly
        } else {
            AttachMode::ReadWrite
        };

        // With --wait, retry the whole connect-then-attach handshake, not
        // just the connect: a plugin asking to wait wants "wait until this
        // session exists", not just "wait until fastty's socket exists".
        let deadline =
            wait_secs.map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
        let mut printed_waiting = false;
        let (mut reader, write_half, cols, rows) = loop {
            let attempt = connect().and_then(|mut stream| {
                send_request(&mut stream, &Request::Attach { id, mode, binary_snapshot: false })?;
                let write_half = stream.try_clone().map_err(|e| e.to_string())?;
                let mut reader = std::io::BufReader::new(stream);
                match read_response(&mut reader) {
                    Ok(Response::Attached { cols, rows, .. }) => {
                        Ok((reader, write_half, cols, rows))
                    }
                    Ok(Response::Error { code, message }) => Err(format!("{code}: {message}")),
                    Ok(other) => Err(format!("unexpected response from daemon: {other:?}")),
                    Err(e) => Err(e),
                }
            });
            match attempt {
                Ok(result) => break result,
                Err(e) => match deadline {
                    Some(deadline) if std::time::Instant::now() < deadline => {
                        if !printed_waiting {
                            eprintln!("Waiting for session {id} ({e})...");
                            printed_waiting = true;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                    Some(_) => {
                        eprintln!("fastty attach: {e}\n(gave up waiting)");
                        std::process::exit(1);
                    }
                    None => {
                        eprintln!("fastty attach: {e}");
                        std::process::exit(1);
                    }
                },
            }
        };

        let mode_note = if read_only { ", read-only" } else { "" };
        eprintln!(
            "fastty attach: attached to session {id} ({cols}x{rows}{mode_note}). Ctrl+\\ to detach."
        );

        let raw_guard = RawModeGuard::enable();
        if raw_guard.is_none() {
            eprintln!(
                "fastty attach: couldn't set stdin to raw mode; input will still work \
                 but keys like arrow keys or Ctrl+C may not behave as expected."
            );
        }

        // Forward local stdin to the remote pane on its own thread; the main
        // thread stays free to print incoming output/snapshot bytes as they
        // arrive, matching what a real interactive attach should feel like.
        // In read-only mode this thread still runs, but only ever watches
        // for the detach shortcut -- everything else typed is discarded
        // instead of being sent as a `write` the daemon would reject anyway.
        std::thread::spawn(move || {
            let mut write_half = write_half;
            let mut stdin = std::io::stdin();
            let mut buf = [0u8; 4096];
            loop {
                let n = match stdin.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let chunk = &buf[..n];
                let Some(detach_at) = chunk.iter().position(|&b| b == DETACH_BYTE) else {
                    if !read_only {
                        let req = Request::Write {
                            id,
                            data: crate::daemon::base64_encode(chunk),
                            ack: false,
                        };
                        if send_request(&mut write_half, &req).is_err() {
                            break;
                        }
                    }
                    continue;
                };
                if !read_only && detach_at > 0 {
                    let req = Request::Write {
                        id,
                        data: crate::daemon::base64_encode(&chunk[..detach_at]),
                        ack: false,
                    };
                    if send_request(&mut write_half, &req).is_err() {
                        break;
                    }
                }
                let _ = send_request(&mut write_half, &Request::Detach { id });
                break;
            }
        });

        let mut stdout = std::io::stdout();
        let exit_code: i32 = loop {
            match read_response(&mut reader) {
                Ok(Response::Output { data, .. }) | Ok(Response::Snapshot { data, .. }) => {
                    if let Some(bytes) = crate::daemon::base64_decode(&data) {
                        let _ = stdout.write_all(&bytes);
                        let _ = stdout.flush();
                    }
                }
                Ok(Response::Closed { .. }) => {
                    eprintln!("\nfastty attach: session {id} was closed.");
                    break 0;
                }
                Ok(Response::Detached { .. }) => {
                    eprintln!("\nfastty attach: detached.");
                    break 0;
                }
                Ok(Response::Error { code, message }) => {
                    eprintln!("\nfastty attach: {code}: {message}");
                    break 1;
                }
                Ok(_) => continue,
                Err(e) => {
                    eprintln!("\nfastty attach: {e}");
                    break 1;
                }
            }
        };

        // Explicitly restore cooked mode before exiting so the calling shell
        // is never left in raw state.
        if let Some(mut guard) = raw_guard {
            guard.disable();
        }

        std::process::exit(exit_code);
    }
}

/// `fastty spawn [--cwd DIR] [--cols N] [--rows N] [--open] [--json] [--wait[=N]] [--] [COMMAND [ARGS...]]`
///
/// Starts a session in the running fastty daemon and prints its id.
/// Without a command the session runs the default shell. With `--open` it
/// also surfaces as a tab in the fastty window (when one is running).
pub fn run_spawn_command(
    command: Option<String>,
    args: Vec<String>,
    cwd: Option<String>,
    cols: Option<usize>,
    rows: Option<usize>,
    open: bool,
    wait_secs: Option<u64>,
    json: bool,
) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (command, args, cwd, cols, rows, open, wait_secs, json);
        eprintln!("fastty spawn: not supported on this platform yet.");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        let mut stream = match connect_with_retry(wait_secs) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fastty spawn: {e}");
                std::process::exit(1);
            }
        };
        let req = Request::Spawn {
            command,
            args,
            cwd,
            cols,
            rows,
            open,
        };
        if let Err(e) = send_request(&mut stream, &req) {
            eprintln!("fastty spawn: {e}");
            std::process::exit(1);
        }
        let mut reader = std::io::BufReader::new(stream);
        match read_response(&mut reader) {
            Ok(Response::Spawned { id, opened }) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({ "event": "spawned", "id": id, "opened": opened })
                    );
                } else {
                    println!("{id}");
                }
                std::process::exit(0);
            }
            Ok(Response::Error { code, message }) => {
                eprintln!("fastty spawn: {code}: {message}");
                std::process::exit(1);
            }
            Ok(_) => {
                eprintln!("fastty spawn: unexpected response from fastty");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("fastty spawn: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// `fastty split <pane-id> [--direction left|right|top|down] [--cwd DIR]
/// [--json] [--wait[=N]] [--] [COMMAND [ARGS...]]` — splits a visible pane
/// in the running fastty GUI and prints the new pane's session id.
pub fn run_split_pane_command(
    pane_id: usize,
    direction: String,
    command: Option<String>,
    args: Vec<String>,
    cwd: Option<String>,
    wait_secs: Option<u64>,
    json: bool,
) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (pane_id, direction, command, args, cwd, wait_secs, json);
        eprintln!("fastty split: not supported on this platform yet.");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        let mut stream = match connect_with_retry(wait_secs) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fastty split: {e}");
                std::process::exit(1);
            }
        };
        let req = Request::SplitPane {
            id: pane_id,
            direction,
            command,
            args,
            cwd,
        };
        if let Err(e) = send_request(&mut stream, &req) {
            eprintln!("fastty split: {e}");
            std::process::exit(1);
        }
        let mut reader = std::io::BufReader::new(stream);
        match read_response(&mut reader) {
            Ok(Response::PaneSplit { id }) => {
                if json {
                    println!("{}", serde_json::json!({ "event": "pane_split", "id": id }));
                } else {
                    println!("{id}");
                }
                std::process::exit(0);
            }
            Ok(Response::Error { code, message }) => {
                eprintln!("fastty split: {code}: {message}");
                std::process::exit(1);
            }
            Ok(_) => {
                eprintln!("fastty split: unexpected response from fastty");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("fastty split: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// `fastty close <id> [--force]` — terminates a headless session in the
/// daemon. GUI panes answer `not_closable` unless `--force` is given, which
/// kills their process and closes the tab.
pub fn run_close_command(id: usize, force: bool, wait_secs: Option<u64>) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (id, force, wait_secs);
        eprintln!("fastty close: not supported on this platform yet.");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        let mut stream = match connect_with_retry(wait_secs) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fastty close: {e}");
                std::process::exit(1);
            }
        };
        let request = if force {
            Request::ExternalAction {
                id,
                action: crate::daemon::ExternalAction::Close { force: true },
            }
        } else {
            Request::Close { id, force }
        };
        if let Err(e) = send_request(&mut stream, &request) {
            eprintln!("fastty close: {e}");
            std::process::exit(1);
        }
        let mut reader = std::io::BufReader::new(stream);
        match read_response(&mut reader) {
            Ok(Response::Closed { id: closed }) => {
                println!("closed {closed}");
                std::process::exit(0);
            }
            Ok(Response::Error { code, message }) => {
                eprintln!("fastty close: {code}: {message}");
                std::process::exit(1);
            }
            Ok(_) => {
                eprintln!("fastty close: unexpected response from fastty");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("fastty close: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// `fastty resize <id> <cols> <rows>` — resizes a session, acked.
pub fn run_resize_command(id: usize, cols: usize, rows: usize, wait_secs: Option<u64>) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (id, cols, rows, wait_secs);
        eprintln!("fastty resize: not supported on this platform yet.");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        let mut stream = match connect_with_retry(wait_secs) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fastty resize: {e}");
                std::process::exit(1);
            }
        };
        let req = Request::Resize {
            id,
            cols,
            rows,
            ack: true,
        };
        if let Err(e) = send_request(&mut stream, &req) {
            eprintln!("fastty resize: {e}");
            std::process::exit(1);
        }
        let mut reader = std::io::BufReader::new(stream);
        match read_response(&mut reader) {
            Ok(Response::Done { .. }) => {
                println!("resized {id} to {cols}x{rows}");
                std::process::exit(0);
            }
            Ok(Response::Error { code, message }) => {
                eprintln!("fastty resize: {code}: {message}");
                std::process::exit(1);
            }
            Ok(_) => {
                eprintln!("fastty resize: unexpected response from fastty");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("fastty resize: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// `fastty write <id> [--enter] <TEXT...>` — types text into a session's
/// PTY. With `--enter` a carriage return is appended, submitting the line.
pub fn run_write_command(id: usize, text: String, enter: bool, wait_secs: Option<u64>) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (id, text, enter, wait_secs);
        eprintln!("fastty write: not supported on this platform yet.");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        let mut stream = match connect_with_retry(wait_secs) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fastty write: {e}");
                std::process::exit(1);
            }
        };
        if let Err(error) = send_request(&mut stream, &Request::Hello { read_only: false }) {
            eprintln!("fastty write: {error}");
            std::process::exit(1);
        }
        let mut reader = std::io::BufReader::new(stream);
        if !matches!(read_response(&mut reader), Ok(Response::Hello { .. })) {
            eprintln!("fastty write: daemon did not accept the hello request");
            std::process::exit(1);
        }
        let mut stream = reader.into_inner();
        if let Err(error) = send_request(
            &mut stream,
            &Request::SetAccessScope {
                scope: crate::daemon::AccessScope::Interact,
            },
        ) {
            eprintln!("fastty write: {error}");
            std::process::exit(1);
        }
        let mut reader = std::io::BufReader::new(stream);
        match read_response(&mut reader) {
            Ok(Response::Done { .. }) => {}
            // Older hosts do not know the optional access-scope request.
            // Keep the existing explicit `write` command behavior with them.
            Ok(Response::Error { code, .. }) if code == "bad_request" => {}
            Ok(Response::Error { code, message }) => {
                eprintln!("fastty write: {code}: {message}");
                std::process::exit(1);
            }
            Ok(_) => {
                eprintln!("fastty write: unexpected access-scope response");
                std::process::exit(1);
            }
            Err(error) => {
                eprintln!("fastty write: {error}");
                std::process::exit(1);
            }
        }
        let mut stream = reader.into_inner();
        let payload = if enter { format!("{text}\r") } else { text };
        let req = Request::Write {
            id,
            data: crate::daemon::base64_encode(payload.as_bytes()),
            ack: true,
        };
        if let Err(e) = send_request(&mut stream, &req) {
            eprintln!("fastty write: {e}");
            std::process::exit(1);
        }
        let mut reader = std::io::BufReader::new(stream);
        match read_response(&mut reader) {
            Ok(Response::Done { .. }) => {
                std::process::exit(0);
            }
            Ok(Response::Error { code, message }) => {
                eprintln!("fastty write: {code}: {message}");
                std::process::exit(1);
            }
            Ok(_) => {
                eprintln!("fastty write: unexpected response from fastty");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("fastty write: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// `fastty send` asks the visible Fastty window to approve input before it
/// reaches the session.
pub fn run_send_command(id: usize, text: String, enter: bool, wait_secs: Option<u64>) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (id, text, enter, wait_secs);
        eprintln!("fastty send: not supported on this platform yet.");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        let result = (|| {
            let mut stream = connect_with_retry(wait_secs)?;
            send_request(&mut stream, &Request::Hello { read_only: false })?;
            let mut reader = std::io::BufReader::new(stream);
            if !matches!(read_response(&mut reader)?, Response::Hello { .. }) {
                return Err("daemon did not accept the hello request".to_string());
            }
            let mut stream = reader.into_inner();
            send_request(
                &mut stream,
                &Request::SetAccessScope {
                    scope: crate::daemon::AccessScope::Interact,
                },
            )?;
            let mut reader = std::io::BufReader::new(stream);
            match read_response(&mut reader)? {
                Response::Done { .. } => {}
                Response::Error { code, message } => return Err(format!("{code}: {message}")),
                _ => return Err("unexpected access-scope response".to_string()),
            }
            let payload = if enter { format!("{text}\r") } else { text };
            let mut stream = reader.into_inner();
            send_request(
                &mut stream,
                &Request::ExternalAction {
                    id,
                    action: crate::daemon::ExternalAction::Send { text: payload },
                },
            )?;
            match read_response(&mut std::io::BufReader::new(stream))? {
                Response::Done { .. } => Ok(()),
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected response from fastty".to_string()),
            }
        })();
        match result {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("fastty send: {error}");
                std::process::exit(1);
            }
        }
    }
}

pub fn run_agent_status_command(state: String, app: String, kind: Option<String>) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (state, app, kind);
        eprintln!("fastty agent-status: not supported on this platform yet.");
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        let result = (|| {
            let source_id = std::env::var("FASTTY_SESSION_TOKEN")
                .map_err(|_| "FASTTY_SESSION_TOKEN is missing. Start the agent inside Fastty.".to_string())?
                .parse::<u64>()
                .map_err(|_| "FASTTY_SESSION_TOKEN is invalid".to_string())?;
            let valid_state = matches!(state.as_str(), "idle" | "working" | "done" | "blocked" | "error" | "clear");
            let valid_app = matches!(app.as_str(), "claude" | "opencode" | "gemini" | "pi" | "codex");
            let valid_kind = kind.as_deref().is_none_or(|value| matches!(value, "permission" | "question" | "auth"));
            if !valid_state || !valid_app || !valid_kind {
                return Err("invalid state, agent, or blocked kind".to_string());
            }
            let mut payload = format!("state={state}:id=agent:app={app}");
            if state == "blocked" {
                let kind = kind.as_deref().unwrap_or("question");
                payload.push_str(&format!(":kind={kind}"));
            }
            let mut stream = connect()?;
            send_request(&mut stream, &Request::Hello { read_only: false })?;
            let mut reader = std::io::BufReader::new(stream);
            if !matches!(read_response(&mut reader)?, Response::Hello { .. }) {
                return Err("daemon did not accept the hello request".to_string());
            }
            let mut stream = reader.into_inner();
            send_request(
                &mut stream,
                &Request::SetAccessScope {
                    scope: crate::daemon::AccessScope::Interact,
                },
            )?;
            let mut reader = std::io::BufReader::new(stream);
            match read_response(&mut reader)? {
                Response::Done { .. } => {}
                Response::Error { code, message } => return Err(format!("{code}: {message}")),
                _ => return Err("unexpected access-scope response".to_string()),
            }
            let mut stream = reader.into_inner();
            send_request(
                &mut stream,
                &Request::ReportProgramStatus { source_id, payload },
            )?;
            match read_response(&mut std::io::BufReader::new(stream))? {
                Response::Done { .. } => Ok(()),
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                _ => Err("unexpected response from Fastty".to_string()),
            }
        })();
        match result {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("fastty agent-status: {error}");
                std::process::exit(1);
            }
        }
    }
}

fn status_for_agent_hook(
    provider: &str,
    event: Option<&str>,
    notification: Option<&str>,
) -> Option<(&'static str, Option<&'static str>)> {
    match (provider, event) {
        ("claude", Some("SessionStart"))
        | ("gemini", Some("SessionStart"))
        | ("codex", Some("SessionStart")) => Some(("idle", None)),
        ("claude", Some("UserPromptSubmit"))
        | ("gemini", Some("BeforeAgent"))
        | ("codex", Some("UserPromptSubmit")) => Some(("working", None)),
        ("claude", Some("Stop"))
        | ("gemini", Some("AfterAgent"))
        | ("codex", Some("Stop")) => Some(("done", None)),
        ("claude", Some("StopFailure")) => Some(("error", None)),
        ("claude", Some("SessionEnd"))
        | ("gemini", Some("SessionEnd"))
        | ("codex", Some("SessionEnd")) => Some(("clear", None)),
        ("codex", Some("Interrupt")) => Some(("idle", None)),
        ("codex", Some("PermissionRequest")) => Some(("blocked", Some("permission"))),
        ("claude", Some("Notification")) if notification == Some("permission_prompt") => {
            Some(("blocked", Some("permission")))
        }
        ("claude", Some("Notification"))
            if matches!(notification, Some("idle_prompt" | "agent_needs_input")) =>
        {
            Some(("blocked", Some("question")))
        }
        ("gemini", Some("Notification")) if notification == Some("ToolPermission") => {
            Some(("blocked", Some("permission")))
        }
        _ => None,
    }
}

#[cfg(test)]
mod agent_hook_tests {
    use super::status_for_agent_hook;

    #[test]
    fn provider_hooks_map_only_supported_lifecycle_states() {
        assert_eq!(
            status_for_agent_hook("claude", Some("UserPromptSubmit"), None),
            Some(("working", None))
        );
        assert_eq!(
            status_for_agent_hook("gemini", Some("Notification"), Some("ToolPermission")),
            Some(("blocked", Some("permission")))
        );
        assert_eq!(
            status_for_agent_hook("codex", Some("PermissionRequest"), None),
            Some(("blocked", Some("permission")))
        );
        assert_eq!(status_for_agent_hook("pi", Some("agent_end"), None), None);
    }
}

pub fn run_agent_hook_command(provider: String) -> ! {
    use std::io::Read;

    if !matches!(provider.as_str(), "claude" | "gemini" | "codex") {
        eprintln!("fastty agent-hook: supported providers: claude, gemini, codex");
        std::process::exit(2);
    }
    let emit_result = || {
        if provider != "claude" {
            println!("{{}}");
        }
    };
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        emit_result();
        std::process::exit(0);
    }
    let parsed = serde_json::from_str::<serde_json::Value>(&input).ok();
    let Some(value) = parsed else {
        emit_result();
        std::process::exit(0);
    };
    let event = value.get("hook_event_name").and_then(serde_json::Value::as_str);
    let notification = value.get("notification_type").and_then(serde_json::Value::as_str)
        .or_else(|| value.get("type").and_then(serde_json::Value::as_str));
    let status = status_for_agent_hook(provider.as_str(), event, notification);
    if let (Some((state, kind)), Some(executable)) =
        (status, std::env::current_exe().ok())
    {
        if std::env::var_os("FASTTY_SESSION_TOKEN").is_some() {
            let mut command = std::process::Command::new(executable);
            command
                .args(["agent-status", state, provider.as_str()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            if let Some(kind) = kind {
                command.arg(kind);
            }
            let _ = command.status();
        }
    }
    emit_result();
    std::process::exit(0);
}
