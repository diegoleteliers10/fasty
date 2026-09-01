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

use crate::daemon::{AttachMode, Request, Response};

#[cfg(unix)]
fn connect() -> Result<std::os::unix::net::UnixStream, String> {
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
    println!("{:<id_w$}  {:<title_w$}  ALIVE  CWD", "ID", "TITLE");
    for s in sessions {
        println!(
            "{:<id_w$}  {:<title_w$}  {:<5}  {}",
            s.id,
            s.title,
            if s.alive { "yes" } else { "no" },
            s.cwd.as_deref().unwrap_or("-"),
        );
    }
}

pub fn run_sessions_command(watch: bool, wait_secs: Option<u64>) -> ! {
    #[cfg(not(unix))]
    {
        let _ = (watch, wait_secs);
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
                    print_sessions_table(&sessions);
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
                    print_sessions_table(&sessions);
                    let _ = std::io::stdout().flush();
                }
                Ok(Response::SessionAdded { session }) => {
                    println!(
                        "+ [{}] {} ({})",
                        session.id,
                        session.title,
                        session.cwd.as_deref().unwrap_or("-")
                    );
                    let _ = std::io::stdout().flush();
                }
                Ok(Response::SessionRemoved { id }) => {
                    println!("- [{id}]");
                    let _ = std::io::stdout().flush();
                }
                Ok(Response::SessionUpdated { session }) => {
                    println!(
                        "~ [{}] {} ({})",
                        session.id,
                        session.title,
                        session.cwd.as_deref().unwrap_or("-")
                    );
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
                send_request(&mut stream, &Request::Attach { id, mode })?;
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
