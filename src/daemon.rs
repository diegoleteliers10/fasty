//! Local IPC daemon: lets other processes (a Neovim plugin, a Zed extension,
//! a future web bridge) list fastty's live tabs and attach to one — seeing
//! the same output the GPUI window renders and optionally sending input —
//! without taking the tab away from the GUI.
//!
//! - One Unix domain socket per user, at `<state_dir>/fasttyd.sock`.
//! - Newline-delimited JSON control messages (see `docs/daemon-protocol.md`).
//! - The registry is in-memory only and owned by the running `fastty`
//!   process — it holds no state across restarts, and cleans itself up when
//!   the process exits (the socket file goes with it).
//! - Windows has no Unix domain sockets in the same shape; `start()` is a
//!   no-op there for now rather than half-supporting named pipes.
//!
//! Panes register themselves in `RootView::spawn_terminal_pane` and
//! unregister on the two removal paths that exist today (`close_tab`,
//! `close_active_pane`). The registry is memory-only and dies with the
//! process, so a pane that somehow skips both (e.g. the whole app being
//! killed) never leaks past that point either -- it just means `list` can
//! serve a stale entry for the remainder of *this* run. `list` doesn't hide
//! those; it reports them with `alive: false` (see `TerminalState::is_alive`)
//! so a client can decide what to do with a dead session itself.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::pane_tree::PaneId;
use crate::terminal_state::TerminalState;

/// Bumped whenever a breaking change is made to the request/response shapes
/// documented in `docs/daemon-protocol.md`. Additive changes (a new optional
/// field, a new `cmd`/`event` variant) don't need a bump; a client should
/// tolerate unknown fields and events either way.
pub const PROTOCOL_VERSION: u32 = 1;

type Registry = Mutex<HashMap<PaneId, Arc<TerminalState>>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

/// Called when a pane's `TerminalState` is created, so it becomes visible to
/// `list`/`attach` immediately.
pub fn register(pane_id: PaneId, terminal: Arc<TerminalState>) {
    registry().lock().insert(pane_id, terminal);
}

/// Called from the pane-removal paths (`close_tab`, `close_active_pane`).
pub fn unregister(pane_id: PaneId) {
    registry().lock().remove(&pane_id);
}

fn is_registered(pane_id: PaneId) -> bool {
    registry().lock().contains_key(&pane_id)
}

/// Path to the daemon's control socket: `<state_dir>/fasttyd.sock`.
pub fn socket_path() -> std::path::PathBuf {
    crate::paths::get().state_dir.join("fasttyd.sock")
}

/// Also used directly by `crate::daemon_client` (the `fastty sessions` /
/// `fastty attach` CLI subcommands), which serializes these to talk to a
/// running daemon over the exact same wire format documented in
/// `docs/daemon-protocol.md` -- there's no separate hand-rolled client-side
/// copy of the protocol to drift out of sync with this one.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Hello,
    List,
    /// Push-updated session list: an initial `sessions` event with everything
    /// currently registered, followed by `session_added` / `session_removed`
    /// / `session_updated` as the registry changes, for as long as this
    /// connection stays open. No explicit unsubscribe in v1 -- a client that
    /// wants this typically dedicates one connection to it.
    SubscribeSessions,
    Attach {
        id: PaneId,
        #[serde(default)]
        mode: AttachMode,
    },
    Detach {
        id: PaneId,
    },
    Write {
        id: PaneId,
        data: String,
    },
    Resize {
        id: PaneId,
        cols: usize,
        rows: usize,
    },
    Spawn {
        #[serde(default)]
        command: Option<String>,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        cols: Option<usize>,
        #[serde(default)]
        rows: Option<usize>,
    },
    Close {
        id: PaneId,
    },
}

/// Whether an `attach` is allowed to `write` to the session it attached to,
/// on the connection that attached. Omitting `mode` in an `attach` request
/// defaults to `read_write`, matching the original (mode-less) behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachMode {
    #[default]
    ReadWrite,
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: PaneId,
    pub title: String,
    pub cwd: Option<String>,
    pub cols: usize,
    pub rows: usize,
    pub alive: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Response {
    Hello {
        version: u32,
        fastty_version: String,
    },
    /// Full session list -- the reply to `list`, and also the first thing
    /// sent after `subscribe_sessions` (as a baseline before any
    /// `session_added`/`session_removed`/`session_updated` deltas).
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    /// A session appeared (new tab/split) while `subscribe_sessions` was
    /// active on this connection.
    SessionAdded {
        session: SessionInfo,
    },
    /// A session disappeared (`close_tab`/`close_active_pane`) while
    /// `subscribe_sessions` was active on this connection.
    SessionRemoved {
        id: PaneId,
    },
    /// Any field of a still-alive session changed (title, cwd, alive) since
    /// the last `sessions`/`session_added`/`session_updated` for it, while
    /// `subscribe_sessions` was active. Polled at a coarse interval (see
    /// `unix_impl::handle_request`'s `SubscribeSessions` arm) rather than
    /// pushed instantly -- a few hundred ms of staleness is invisible in a
    /// sidebar and this avoids wiring a second notification path through
    /// every place `RootView` can change a pane's title/cwd.
    SessionUpdated {
        session: SessionInfo,
    },
    Attached {
        id: PaneId,
        cols: usize,
        rows: usize,
        mode: AttachMode,
    },
    /// Sent once, immediately after `attached`, with the pane's current
    /// on-screen contents as ANSI/SGR bytes (see
    /// `TerminalState::snapshot_ansi`) so a client can paint something
    /// before the first live `output` event arrives. A client that doesn't
    /// care about this can treat it exactly like an `output` event.
    Snapshot {
        id: PaneId,
        data: String,
    },
    Detached {
        id: PaneId,
    },
    /// The session this connection was attached to went away (its pane was
    /// closed) while the attach was still active. No further `output` for
    /// this `id` will follow.
    Closed {
        id: PaneId,
    },
    Output {
        id: PaneId,
        /// Base64-encoded raw PTY bytes — JSON strings can't carry arbitrary
        /// binary safely, and this protocol is meant to be easy to speak from
        /// any language, not maximally throughput-efficient.
        data: String,
    },
    Spawned {
        id: PaneId,
    },
    Error {
        /// Stable, machine-matchable reason: `"bad_request"`,
        /// `"no_such_session"`, `"invalid_base64"`, `"not_attached"`,
        /// `"read_only"`, or `"unsupported"`. New codes may be added over
        /// time; treat an unrecognized one the same as a generic failure.
        code: String,
        message: String,
    },
}

pub fn spawn_headless_session(
    cmd: Option<&str>,
    args: &[String],
    cwd: Option<&str>,
    cols: Option<usize>,
    rows: Option<usize>,
) -> Result<PaneId, String> {
    static NEXT_PANE_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(100);

    let shell = cmd
        .map(|s| s.to_string())
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(crate::paths::default_system_shell);

    let home_str = dirs::home_dir().map(|p| p.to_string_lossy().into_owned());
    let default_cwd = cwd.or(home_str.as_deref());
    let cols = cols.unwrap_or(80);
    let rows = rows.unwrap_or(24);

    let terminal = TerminalState::new_headless(&shell, args, default_cwd, cols, rows)
        .map_err(|e| format!("failed to spawn terminal: {e}"))?;

    let mut id = NEXT_PANE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    while is_registered(id) {
        id = NEXT_PANE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    register(id, Arc::new(terminal));
    Ok(id)
}

pub fn ensure_default_session() -> Option<PaneId> {
    if registry().lock().is_empty() {
        spawn_headless_session(None, &[], None, Some(80), Some(24)).ok()
    } else {
        None
    }
}

fn session_info(id: PaneId, terminal: &TerminalState) -> SessionInfo {
    let (cols, rows) = terminal.dimensions();
    SessionInfo {
        id,
        title: terminal
            .get_foreground_process_name()
            .unwrap_or_else(|| "shell".to_string()),
        cwd: terminal
            .get_current_working_directory()
            .map(|p| p.to_string_lossy().into_owned()),
        cols,
        rows,
        alive: terminal.is_alive(),
    }
}

/// Start the daemon's accept loop in a background thread. Safe to call more
/// than once (e.g. from more than one window) — only the first call binds
/// the socket.
pub fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        #[cfg(unix)]
        std::thread::spawn(unix_impl::run);
        #[cfg(not(unix))]
        {
            // No Unix domain sockets on this platform. A named-pipe backend
            // would live in its own module; not implemented yet.
        }
    });
}

#[cfg(unix)]
mod unix_impl {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    pub fn run() {
        let path = socket_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Remove a stale socket from a previous (crashed/killed) run before
        // binding — UnixListener::bind fails if the path already exists.
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            Err(_) => return, // e.g. state_dir not writable; daemon is best-effort
        };
        // Per-user only: the control socket can write to any registered
        // pane's PTY, so it must not be reachable by other local users.
        if let Ok(meta) = std::fs::metadata(&path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(&path, perms);
        }

        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || handle_connection(stream));
        }
    }

    /// One currently-attached session on a connection: its stop-flag (so
    /// `Detach{id}` can stop just this attach thread) and whether `write` to
    /// this `id` is allowed on this connection.
    struct Attachment {
        alive: Arc<AtomicBool>,
        read_only: bool,
    }

    /// Per-connection state: everything a `handle_request` call needs beyond
    /// the request itself.
    struct Conn {
        out: Arc<Mutex<UnixStream>>,
        /// Flips to false once the read loop ends (client disconnected or
        /// sent unparseable input); every attach/subscription thread checks
        /// this on each poll tick and stops promptly instead of leaking past
        /// the connection's lifetime.
        conn_alive: Arc<AtomicBool>,
        /// One entry per currently-attached session on *this* connection.
        /// Only ever touched from the connection's single reader thread
        /// (inserts here, removals in `Detach`), so a plain `HashMap` behind
        /// no lock is enough.
        attachments: HashMap<PaneId, Attachment>,
    }

    fn handle_connection(stream: UnixStream) {
        let write_half = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut conn = Conn {
            out: Arc::new(Mutex::new(write_half)),
            conn_alive: Arc::new(AtomicBool::new(true)),
            attachments: HashMap::new(),
        };

        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let request: Request = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "bad_request".to_string(),
                            message: format!("bad request: {e}"),
                        },
                    );
                    continue;
                }
            };
            handle_request(request, &mut conn);
        }
        conn.conn_alive.store(false, Ordering::Relaxed);
        for attachment in conn.attachments.values() {
            attachment.alive.store(false, Ordering::Relaxed);
        }
    }

    fn handle_request(request: Request, conn: &mut Conn) {
        match request {
            Request::Hello => {
                send(
                    &conn.out,
                    &Response::Hello {
                        version: PROTOCOL_VERSION,
                        fastty_version: env!("CARGO_PKG_VERSION").to_string(),
                    },
                );
            }
            Request::List => {
                let _ = ensure_default_session();
                let sessions = registry()
                    .lock()
                    .iter()
                    .map(|(id, terminal)| session_info(*id, terminal))
                    .collect();
                send(&conn.out, &Response::Sessions { sessions });
            }
            Request::Attach { id, mode } => {
                let terminal = registry().lock().get(&id).cloned();
                let Some(terminal) = terminal else {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "no_such_session".to_string(),
                            message: "no such session".to_string(),
                        },
                    );
                    return;
                };
                let (cols, rows) = terminal.dimensions();
                send(
                    &conn.out,
                    &Response::Attached {
                        id,
                        cols,
                        rows,
                        mode,
                    },
                );
                send(
                    &conn.out,
                    &Response::Snapshot {
                        id,
                        data: base64_encode(&terminal.snapshot_ansi()),
                    },
                );

                // Replacing a stale entry (re-attaching to the same id on the
                // same connection) stops the old thread cleanly first.
                if let Some(old) = conn.attachments.remove(&id) {
                    old.alive.store(false, Ordering::Relaxed);
                }
                let alive = Arc::new(AtomicBool::new(true));
                conn.attachments.insert(
                    id,
                    Attachment {
                        alive: Arc::clone(&alive),
                        read_only: mode == AttachMode::ReadOnly,
                    },
                );

                let rx = terminal.subscribe_output();
                let out = Arc::clone(&conn.out);
                let conn_alive = Arc::clone(&conn.conn_alive);
                std::thread::spawn(move || {
                    let should_run =
                        || conn_alive.load(Ordering::Relaxed) && alive.load(Ordering::Relaxed);
                    while should_run() {
                        match rx.try_recv() {
                            Ok(mut bytes) => {
                                // Coalesce consecutive waiting chunks to avoid NDJSON
                                // framing & IPC context-switch overhead during high throughput.
                                while let Ok(next) = rx.try_recv() {
                                    bytes.extend_from_slice(&next);
                                    if bytes.len() >= 65536 {
                                        break;
                                    }
                                }
                                let data = base64_encode(&bytes);
                                if !send(&out, &Response::Output { id, data }) {
                                    break;
                                }
                            }
                            Err(async_channel::TryRecvError::Empty) => {
                                if !is_registered(id) {
                                    send(&out, &Response::Closed { id });
                                    break;
                                }
                                std::thread::sleep(Duration::from_millis(20));
                            }
                            Err(async_channel::TryRecvError::Closed) => {
                                if !is_registered(id) {
                                    send(&out, &Response::Closed { id });
                                }
                                break;
                            }
                        }
                    }
                });
            }
            Request::Detach { id } => {
                if let Some(attachment) = conn.attachments.remove(&id) {
                    attachment.alive.store(false, Ordering::Relaxed);
                    send(&conn.out, &Response::Detached { id });
                } else {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "not_attached".to_string(),
                            message: format!("not attached to session {id} on this connection"),
                        },
                    );
                }
            }
            Request::Write { id, data } => {
                if conn.attachments.get(&id).is_some_and(|a| a.read_only) {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "read_only".to_string(),
                            message: format!(
                                "attached to session {id} in read-only mode on this connection"
                            ),
                        },
                    );
                    return;
                }
                let Some(bytes) = base64_decode(&data) else {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "invalid_base64".to_string(),
                            message: "invalid base64 in write".to_string(),
                        },
                    );
                    return;
                };
                if let Some(terminal) = registry().lock().get(&id) {
                    terminal.write_to_pty(&bytes);
                } else {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "no_such_session".to_string(),
                            message: "no such session".to_string(),
                        },
                    );
                }
            }
            Request::SubscribeSessions => {
                let _ = ensure_default_session();
                let initial: Vec<SessionInfo> = registry()
                    .lock()
                    .iter()
                    .map(|(id, terminal)| session_info(*id, terminal))
                    .collect();
                send(
                    &conn.out,
                    &Response::Sessions {
                        sessions: initial.clone(),
                    },
                );

                let out = Arc::clone(&conn.out);
                let conn_alive = Arc::clone(&conn.conn_alive);
                std::thread::spawn(move || {
                    let mut last: HashMap<PaneId, SessionInfo> =
                        initial.into_iter().map(|s| (s.id, s)).collect();
                    while conn_alive.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(350));
                        if !conn_alive.load(Ordering::Relaxed) {
                            return;
                        }
                        let current: HashMap<PaneId, SessionInfo> = registry()
                            .lock()
                            .iter()
                            .map(|(id, terminal)| (*id, session_info(*id, terminal)))
                            .collect();

                        for id in last.keys() {
                            if !current.contains_key(id)
                                && !send(&out, &Response::SessionRemoved { id: *id }) {
                                    return;
                                }
                        }
                        for (id, info) in &current {
                            match last.get(id) {
                                None => {
                                    if !send(
                                        &out,
                                        &Response::SessionAdded {
                                            session: info.clone(),
                                        },
                                    ) {
                                        return;
                                    }
                                }
                                Some(prev) if prev != info
                                    && !send(
                                        &out,
                                        &Response::SessionUpdated {
                                            session: info.clone(),
                                        },
                                    ) => {
                                        return;
                                    }
                                _ => {}
                            }
                        }
                        last = current;
                    }
                });
            }
            Request::Resize { id, cols, rows } => {
                if let Some(terminal) = registry().lock().get(&id) {
                    terminal.resize(cols, rows);
                } else {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "no_such_session".to_string(),
                            message: "no such session".to_string(),
                        },
                    );
                }
            }
            Request::Spawn {
                command,
                args,
                cwd,
                cols,
                rows,
            } => {
                match spawn_headless_session(command.as_deref(), &args, cwd.as_deref(), cols, rows) {
                    Ok(id) => {
                        send(&conn.out, &Response::Spawned { id });
                    }
                    Err(e) => {
                        send(
                            &conn.out,
                            &Response::Error {
                                code: "spawn_failed".to_string(),
                                message: e,
                            },
                        );
                    }
                }
            }
            Request::Close { id } => {
                unregister(id);
                send(&conn.out, &Response::Closed { id });
            }
        }
    }

    fn send(out: &Arc<Mutex<UnixStream>>, response: &Response) -> bool {
        let Ok(mut line) = serde_json::to_string(response) else {
            return false;
        };
        line.push('\n');
        out.lock().write_all(line.as_bytes()).is_ok()
    }
}

/// Minimal base64 (standard alphabet, `=` padding) -- shared by the daemon
/// (encoding PTY bytes into `output`/`snapshot` events) and
/// `crate::daemon_client` (encoding keystrokes into `write` requests,
/// decoding those same event payloads back to bytes to print).
pub(crate) fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut chunks = input.chunks_exact(3);
    for c in &mut chunks {
        let n = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | (c[2] as u32);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        1 => {
            let n = (rem[0] as u32) << 16;
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

pub(crate) fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let input = input.trim().as_bytes();
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut chunks = input.chunks_exact(4);
    for c in &mut chunks {
        let pad = c.iter().filter(|&&b| b == b'=').count();
        let v0 = val(c[0])?;
        let v1 = val(c[1])?;
        let v2 = if c[2] == b'=' { 0 } else { val(c[2])? };
        let v3 = if c[3] == b'=' { 0 } else { val(c[3])? };
        let n = ((v0 as u32) << 18) | ((v1 as u32) << 12) | ((v2 as u32) << 6) | (v3 as u32);
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    if !chunks.remainder().is_empty() {
        return None; // not a multiple of 4 — malformed
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_roundtrip() {
        let cases: &[&[u8]] = &[
            b"",
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            b"Hello, world!\n\x1b[31mRed\x1b[0m",
            &[0, 1, 2, 3, 253, 254, 255],
        ];
        for case in cases {
            let encoded = base64_encode(case);
            let decoded = base64_decode(&encoded).expect("valid base64 decode");
            assert_eq!(&decoded, case);
        }
    }

    #[test]
    fn test_base64_invalid() {
        assert!(base64_decode("invalid!").is_none());
        assert!(base64_decode("abc").is_none());
    }

    #[test]
    fn test_request_serialization_roundtrip() {
        let reqs = vec![
            Request::Hello,
            Request::List,
            Request::SubscribeSessions,
            Request::Attach {
                id: 42,
                mode: AttachMode::ReadWrite,
            },
            Request::Attach {
                id: 42,
                mode: AttachMode::ReadOnly,
            },
            Request::Detach { id: 42 },
            Request::Write {
                id: 42,
                data: "aGVsbG8=".to_string(),
            },
            Request::Resize {
                id: 42,
                cols: 80,
                rows: 24,
            },
            Request::Spawn {
                command: Some("bash".to_string()),
                args: vec!["-l".to_string()],
                cwd: Some("/tmp".to_string()),
                cols: Some(100),
                rows: Some(30),
            },
            Request::Close { id: 42 },
        ];

        for req in reqs {
            let json = serde_json::to_string(&req).expect("serialize");
            let parsed: Request = serde_json::from_str(&json).expect("deserialize");
            match (&req, &parsed) {
                (Request::Hello, Request::Hello) => {}
                (Request::List, Request::List) => {}
                (Request::SubscribeSessions, Request::SubscribeSessions) => {}
                (Request::Attach { id: a, mode: m1 }, Request::Attach { id: b, mode: m2 }) => {
                    assert_eq!(a, b);
                    assert_eq!(m1, m2);
                }
                (Request::Detach { id: a }, Request::Detach { id: b }) => assert_eq!(a, b),
                (Request::Write { id: a, data: d1 }, Request::Write { id: b, data: d2 }) => {
                    assert_eq!(a, b);
                    assert_eq!(d1, d2);
                }
                (
                    Request::Resize {
                        id: a,
                        cols: c1,
                        rows: r1,
                    },
                    Request::Resize {
                        id: b,
                        cols: c2,
                        rows: r2,
                    },
                ) => {
                    assert_eq!(a, b);
                    assert_eq!(c1, c2);
                    assert_eq!(r1, r2);
                }
                (
                    Request::Spawn {
                        command: c1,
                        args: a1,
                        cwd: cw1,
                        cols: co1,
                        rows: r1,
                    },
                    Request::Spawn {
                        command: c2,
                        args: a2,
                        cwd: cw2,
                        cols: co2,
                        rows: r2,
                    },
                ) => {
                    assert_eq!(c1, c2);
                    assert_eq!(a1, a2);
                    assert_eq!(cw1, cw2);
                    assert_eq!(co1, co2);
                    assert_eq!(r1, r2);
                }
                (Request::Close { id: a }, Request::Close { id: b }) => assert_eq!(a, b),
                _ => panic!("mismatched variant: {req:?} vs {parsed:?}"),
            }
        }
    }

    #[test]
    fn test_attach_request_defaults_to_read_write() {
        let json = r#"{"cmd":"attach","id":1}"#;
        let req: Request = serde_json::from_str(json).expect("deserialize");
        match req {
            Request::Attach { id, mode } => {
                assert_eq!(id, 1);
                assert_eq!(mode, AttachMode::ReadWrite);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_response_serialization() {
        let res = Response::Attached {
            id: 1,
            cols: 80,
            rows: 24,
            mode: AttachMode::ReadOnly,
        };
        let json = serde_json::to_string(&res).unwrap();
        assert!(json.contains(r#""mode":"read_only""#));

        let res = Response::Error {
            code: "read_only".to_string(),
            message: "attached to session 1 in read-only mode".to_string(),
        };
        let json = serde_json::to_string(&res).unwrap();
        assert!(json.contains(r#""code":"read_only""#));
    }
}
