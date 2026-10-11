//! Local process host for PTY sessions.

use crate::daemon::{AttachMode, Request, Response, SessionInfo};
use std::io::{BufRead, BufReader, Write};

const HOST_ARGUMENT: &str = "--fastty-session-host";

#[cfg(unix)]
pub fn ensure_running() -> Result<(), String> {
    use std::os::unix::net::UnixStream;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let path = crate::daemon::socket_path();
    if UnixStream::connect(&path).is_ok() {
        return Ok(());
    }

    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    Command::new(executable)
        .arg(HOST_ARGUMENT)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start Fastty session host: {error}"))?;

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if UnixStream::connect(&path).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Fastty session host did not start at {}",
                path.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(not(unix))]
pub fn ensure_running() -> Result<(), String> {
    Ok(())
}

pub fn is_host_argument(argument: Option<&str>) -> bool {
    argument == Some(HOST_ARGUMENT)
}

pub fn run_host() {
    crate::daemon::run_host();
}

#[cfg(unix)]
pub struct RemoteSession {
    pub id: usize,
    pub snapshot: Vec<u8>,
    pub reader: BufReader<std::os::unix::net::UnixStream>,
    pub control: std::sync::Arc<RemoteControl>,
}

#[cfg(not(unix))]
pub struct RemoteSession {
    pub id: usize,
    pub snapshot: Vec<u8>,
    pub control: std::sync::Arc<RemoteControl>,
}

#[cfg(unix)]
pub struct RemoteControl {
    id: usize,
    stream: std::sync::Mutex<std::os::unix::net::UnixStream>,
    closed: std::sync::atomic::AtomicBool,
}

#[cfg(not(unix))]
pub struct RemoteControl;

#[cfg(not(unix))]
impl RemoteControl {
    pub fn write(&self, _bytes: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "session host is not supported",
        ))
    }

    pub fn resize(&self, _cols: usize, _rows: usize) {}

    pub fn detach(&self) {}

    pub fn stop(&self) {}
}

#[cfg(unix)]
pub fn session_process_running(id: usize) -> Result<bool, String> {
    use std::os::unix::net::UnixStream;

    ensure_running()?;
    let mut stream = UnixStream::connect(crate::daemon::socket_path())
        .map_err(|error| error.to_string())?;
    write_request(&mut stream, &Request::SessionStatus { id })
        .map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::SessionStatus { process_running, .. } => Ok(process_running),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected session status response: {other:?}")),
    }
}

#[cfg(not(unix))]
pub fn session_process_running(_id: usize) -> Result<bool, String> {
    Ok(false)
}

#[cfg(not(unix))]
impl Write for RemoteControl {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        RemoteControl::write(self, bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
impl RemoteControl {
    pub fn write(&self, bytes: &[u8]) -> std::io::Result<()> {
        self.send(&Request::Write {
            id: self.id,
            data: crate::daemon::base64_encode(bytes),
            ack: false,
        })
    }

    pub fn resize(&self, cols: usize, rows: usize) {
        let _ = self.send(&Request::Resize {
            id: self.id,
            cols,
            rows,
            ack: false,
        });
    }

    pub fn detach(&self) {
        if self.closed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let _ = self.send(&Request::Detach { id: self.id });
        self.shutdown();
    }

    pub fn stop(&self) {
        if self.closed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let _ = self.send(&Request::Close {
            id: self.id,
            force: true,
        });
        self.shutdown();
    }

    fn shutdown(&self) {
        let stream = self
            .stream
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }

    fn send(&self, request: &Request) -> std::io::Result<()> {
        let mut stream = self
            .stream
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        write_request(&mut *stream, request)
    }
}

#[cfg(unix)]
impl Drop for RemoteControl {
    fn drop(&mut self) {
        self.detach();
    }
}

#[cfg(unix)]
impl Write for RemoteControl {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        RemoteControl::write(self, bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
pub fn spawn_attached(
    command: &str,
    args: &[String],
    cwd: Option<&str>,
    cols: usize,
    rows: usize,
) -> Result<RemoteSession, String> {
    use std::os::unix::net::UnixStream;

    ensure_running()?;
    let mut stream =
        UnixStream::connect(crate::daemon::socket_path()).map_err(|error| error.to_string())?;
    write_request(
        &mut stream,
        &Request::Spawn {
            command: Some(command.to_string()),
            args: args.to_vec(),
            cwd: cwd.map(str::to_string),
            cols: Some(cols),
            rows: Some(rows),
            open: false,
        },
    )
    .map_err(|error| error.to_string())?;
    let mut spawn_reader = BufReader::new(stream);
    let id = match read_response(&mut spawn_reader)? {
        Response::Spawned { id, .. } => id,
        Response::Error { code, message } => return Err(format!("{code}: {message}")),
        other => return Err(format!("unexpected spawn response: {other:?}")),
    };

    let mut stream =
        UnixStream::connect(crate::daemon::socket_path()).map_err(|error| error.to_string())?;
    write_request(
        &mut stream,
        &Request::Attach {
            id,
            mode: AttachMode::ReadWrite,
        },
    )
    .map_err(|error| error.to_string())?;
    let control_stream = stream.try_clone().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);
    match read_response(&mut reader)? {
        Response::Attached { .. } => {}
        Response::Error { code, message } => return Err(format!("{code}: {message}")),
        other => return Err(format!("unexpected attach response: {other:?}")),
    }
    let snapshot = match read_response(&mut reader)? {
        Response::Snapshot { data, .. } => crate::daemon::base64_decode(&data)
            .ok_or_else(|| "session host sent an invalid snapshot".to_string())?,
        Response::Error { code, message } => return Err(format!("{code}: {message}")),
        other => return Err(format!("unexpected snapshot response: {other:?}")),
    };

    Ok(RemoteSession {
        id,
        snapshot,
        reader,
        control: std::sync::Arc::new(RemoteControl {
            id,
            stream: std::sync::Mutex::new(control_stream),
            closed: std::sync::atomic::AtomicBool::new(false),
        }),
    })
}

#[cfg(unix)]
pub fn attach(id: usize) -> Result<RemoteSession, String> {
    use std::os::unix::net::UnixStream;

    ensure_running()?;
    let mut stream =
        UnixStream::connect(crate::daemon::socket_path()).map_err(|error| error.to_string())?;
    write_request(
        &mut stream,
        &Request::Attach {
            id,
            mode: AttachMode::ReadWrite,
        },
    )
    .map_err(|error| error.to_string())?;
    let control_stream = stream.try_clone().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);
    match read_response(&mut reader)? {
        Response::Attached { .. } => {}
        Response::Error { code, message } => return Err(format!("{code}: {message}")),
        other => return Err(format!("unexpected attach response: {other:?}")),
    }
    let snapshot = match read_response(&mut reader)? {
        Response::Snapshot { data, .. } => crate::daemon::base64_decode(&data)
            .ok_or_else(|| "session host sent an invalid snapshot".to_string())?,
        Response::Error { code, message } => return Err(format!("{code}: {message}")),
        other => return Err(format!("unexpected snapshot response: {other:?}")),
    };
    Ok(RemoteSession {
        id,
        snapshot,
        reader,
        control: std::sync::Arc::new(RemoteControl {
            id,
            stream: std::sync::Mutex::new(control_stream),
            closed: std::sync::atomic::AtomicBool::new(false),
        }),
    })
}

#[cfg(unix)]
fn write_request(stream: &mut impl Write, request: &Request) -> std::io::Result<()> {
    let mut line = serde_json::to_string(request)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
    line.push('\n');
    stream.write_all(line.as_bytes())
}

#[cfg(unix)]
fn read_response(reader: &mut impl BufRead) -> Result<Response, String> {
    let mut line = String::new();
    let count = reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    if count == 0 {
        return Err("session host closed the connection".to_string());
    }
    serde_json::from_str(line.trim_end())
        .map_err(|error| format!("invalid session-host response: {error}"))
}

#[cfg(unix)]
pub fn read_output(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, String> {
    match read_response(reader)? {
        Response::Output { data, .. } => crate::daemon::base64_decode(&data)
            .map(Some)
            .ok_or_else(|| "session host sent invalid output".to_string()),
        Response::Closed { .. } => Ok(None),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        _ => Ok(Some(Vec::new())),
    }
}

#[cfg(unix)]
pub fn list_sessions() -> Result<Vec<SessionInfo>, String> {
    use std::os::unix::net::UnixStream;
    ensure_running()?;
    let mut stream =
        UnixStream::connect(crate::daemon::socket_path()).map_err(|error| error.to_string())?;
    write_request(&mut stream, &Request::List).map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::Sessions { sessions } => Ok(sessions),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected session-list response: {other:?}")),
    }
}

#[cfg(unix)]
pub fn session_exists(id: usize) -> Result<bool, String> {
    use std::os::unix::net::UnixStream;
    ensure_running()?;
    let mut stream =
        UnixStream::connect(crate::daemon::socket_path()).map_err(|error| error.to_string())?;
    write_request(&mut stream, &Request::SessionExists { id })
        .map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::SessionExists { exists } => Ok(exists),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected session-exists response: {other:?}")),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::os::unix::net::UnixStream;

    #[test]
    fn remote_input_uses_the_existing_write_request_shape() {
        let (client, server) = UnixStream::pair().unwrap();
        let control = RemoteControl {
            id: 27,
            stream: std::sync::Mutex::new(client),
            closed: std::sync::atomic::AtomicBool::new(false),
        };

        control.write(b"printf host\n").unwrap();

        let mut line = String::new();
        BufReader::new(server).read_line(&mut line).unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["cmd"], "write");
        assert_eq!(request["id"], 27);
        assert_eq!(request["data"], "cHJpbnRmIGhvc3QK");
    }

    #[test]
    fn stop_sends_force_close_and_detach_sends_detach() {
        let (stop_client, stop_server) = UnixStream::pair().unwrap();
        let stop = RemoteControl {
            id: 27,
            stream: std::sync::Mutex::new(stop_client),
            closed: std::sync::atomic::AtomicBool::new(false),
        };
        stop.stop();

        let mut stop_line = String::new();
        BufReader::new(stop_server)
            .read_line(&mut stop_line)
            .unwrap();
        let stop_request: serde_json::Value = serde_json::from_str(&stop_line).unwrap();
        assert_eq!(stop_request["cmd"], "close");
        assert_eq!(stop_request["id"], 27);
        assert_eq!(stop_request["force"], true);
        drop(stop);

        let (detach_client, detach_server) = UnixStream::pair().unwrap();
        let detach = RemoteControl {
            id: 28,
            stream: std::sync::Mutex::new(detach_client),
            closed: std::sync::atomic::AtomicBool::new(false),
        };
        detach.detach();

        let mut detach_line = String::new();
        BufReader::new(detach_server)
            .read_line(&mut detach_line)
            .unwrap();
        let detach_request: serde_json::Value = serde_json::from_str(&detach_line).unwrap();
        assert_eq!(detach_request["cmd"], "detach");
        assert_eq!(detach_request["id"], 28);
        drop(detach);
    }

    #[test]
    fn dropping_remote_control_detaches_without_stopping_session() {
        let (client, server) = UnixStream::pair().unwrap();
        let control = RemoteControl {
            id: 29,
            stream: std::sync::Mutex::new(client),
            closed: std::sync::atomic::AtomicBool::new(false),
        };

        drop(control);

        let mut line = String::new();
        BufReader::new(server).read_line(&mut line).unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["cmd"], "detach");
        assert_eq!(request["id"], 29);
    }

    #[test]
    fn host_argument_matches_only_the_internal_entrypoint() {
        assert!(is_host_argument(Some(HOST_ARGUMENT)));
        assert!(!is_host_argument(Some("sessions")));
        assert!(!is_host_argument(None));
    }

    #[test]
    fn host_survives_gui_projection_drop_and_reattach() {
        match std::env::var("FASTTY_TEST_SESSION_HOST_ROLE").as_deref() {
            Ok("server") => {
                crate::paths::init().expect("initialize isolated server paths");
                crate::daemon::run_host();
            }
            Ok("driver") => run_isolated_gui_driver(),
            Ok("parent") => run_isolated_lifecycle_parent(),
            Ok(role) => panic!("unknown test role: {role}"),
            Err(_) => run_lifecycle_in_isolated_process(),
        }
    }

    struct ChildGuard(std::process::Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }

    struct HostLaunchGuard(Option<std::process::Child>);

    impl HostLaunchGuard {
        fn child_mut(&mut self) -> &mut std::process::Child {
            self.0.as_mut().expect("host child is still guarded")
        }

        fn detach(mut self) {
            let _ = self.0.take();
        }
    }

    impl Drop for HostLaunchGuard {
        fn drop(&mut self) {
            if let Some(host) = self.0.as_mut() {
                if host.try_wait().ok().flatten().is_none() {
                    let _ = host.kill();
                }
                let _ = host.wait();
            }
        }
    }

    struct HostPidGuard(Option<u32>);

    impl HostPidGuard {
        fn terminate(&mut self) -> Result<(), String> {
            use std::process::Command;
            use std::time::{Duration, Instant};

            let Some(pid) = self.0 else {
                return Ok(());
            };
            let pid = pid.to_string();
            let signal = Command::new("/bin/kill")
                .args(["-TERM", &pid])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|error| format!("failed to signal isolated host: {error}"))?;
            if !signal.success() && !host_pid_exists(&pid) {
                self.0 = None;
                return Ok(());
            }
            if !signal.success() {
                return Err("failed to signal isolated host process".to_string());
            }

            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if !host_pid_exists(&pid) {
                    self.0 = None;
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err("isolated host process did not exit after SIGTERM".to_string())
        }
    }

    impl Drop for HostPidGuard {
        fn drop(&mut self) {
            let _ = self.terminate();
        }
    }

    fn host_pid_exists(pid: &str) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-0", pid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    struct SessionGuard(Option<usize>);

    impl SessionGuard {
        fn disarm(&mut self) {
            self.0 = None;
        }
    }

    impl Drop for SessionGuard {
        fn drop(&mut self) {
            if let Some(id) = self.0 {
                if let Ok(session) = attach(id) {
                    session.control.stop();
                }
            }
        }
    }

    struct HostSessionsGuard(bool);

    impl HostSessionsGuard {
        fn disarm(&mut self) {
            self.0 = false;
        }
    }

    impl Drop for HostSessionsGuard {
        fn drop(&mut self) {
            if !self.0 {
                return;
            }
            if let Ok(sessions) = list_sessions() {
                for session in sessions {
                    if let Ok(session) = attach(session.id) {
                        session.control.stop();
                    }
                }
            }
        }
    }

    struct TestRootGuard(std::path::PathBuf);

    impl Drop for TestRootGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run_lifecycle_in_isolated_process() {
        use std::process::{Command, Stdio};
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after the Unix epoch")
            .as_nanos();
        let root = std::path::PathBuf::from(format!("/tmp/fth-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create isolated test home");
        let _root = TestRootGuard(root.clone());

        let parent = Command::new(std::env::current_exe().expect("get test executable"))
            .arg("--exact")
            .arg("session_host::tests::host_survives_gui_projection_drop_and_reattach")
            .arg("--nocapture")
            .env("FASTTY_TEST_SESSION_HOST_ROLE", "parent")
            .env("HOME", &root)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .stdin(Stdio::null())
            .spawn()
            .expect("start isolated lifecycle parent process");
        let mut parent = ChildGuard(parent);
        let status = wait_for_child_exit_for(&mut parent.0, std::time::Duration::from_secs(35));

        assert!(status.success(), "isolated lifecycle parent process failed");
    }

    fn run_isolated_lifecycle_parent() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        crate::paths::init().expect("initialize isolated parent paths");
        let executable = std::env::current_exe().expect("get test executable");
        let test_home = std::path::PathBuf::from(std::env::var_os("HOME").expect("isolated home"));
        let host_pid_path = test_home.join("host-pid");
        let session_id_path = test_home.join("session-id");
        let mut driver = ChildGuard(
            Command::new(&executable)
                .arg("--exact")
                .arg("session_host::tests::host_survives_gui_projection_drop_and_reattach")
                .arg("--nocapture")
                .env("FASTTY_TEST_SESSION_HOST_ROLE", "driver")
                .env("FASTTY_TEST_HOST_PID_FILE", &host_pid_path)
                .env("FASTTY_TEST_SESSION_ID_FILE", &session_id_path)
                .stdin(Stdio::null())
                .spawn()
                .expect("start isolated GUI driver"),
        );
        let driver_status = try_wait_for_child_exit_for(&mut driver.0, Duration::from_secs(20));
        if driver_status.is_none() {
            let _ = driver.0.kill();
            let _ = driver.0.wait();
        }
        let host_pid = std::fs::read_to_string(&host_pid_path)
            .expect("GUI driver must write its host PID")
            .parse::<u32>()
            .expect("GUI driver host PID must be numeric");
        let mut host_guard = HostPidGuard(Some(host_pid));
        let mut sessions_guard = HostSessionsGuard(true);
        let session_id = std::fs::read_to_string(&session_id_path)
            .expect("GUI driver must write its host session ID")
            .parse::<usize>()
            .expect("GUI driver session ID must be numeric");
        let mut session_guard = SessionGuard(Some(session_id));
        let driver_status = driver_status.expect("isolated GUI driver did not exit before timeout");
        assert!(driver_status.success(), "isolated GUI driver failed");

        let duplicate = Command::new(&executable)
            .arg("--exact")
            .arg("session_host::tests::host_survives_gui_projection_drop_and_reattach")
            .arg("--nocapture")
            .env("FASTTY_TEST_SESSION_HOST_ROLE", "server")
            .stdin(Stdio::null())
            .spawn()
            .expect("start duplicate host contender");
        let mut duplicate = ChildGuard(duplicate);
        let duplicate_status = wait_for_child_exit_for(&mut duplicate.0, Duration::from_secs(3));
        assert!(
            duplicate_status.success(),
            "duplicate host contender failed"
        );

        std::thread::sleep(Duration::from_millis(1_500));
        let sessions = list_sessions().expect("list host sessions after GUI driver exit");
        let session = sessions
            .iter()
            .find(|session| session.id == session_id)
            .expect("same host session remains registered after GUI driver exit");
        assert!(
            session.alive,
            "host shell remains alive after GUI driver exit"
        );

        let resumed = attach(session_id).expect("reattach after GUI driver exit");
        let snapshot = String::from_utf8_lossy(&resumed.snapshot);
        assert!(
            snapshot.contains("output-after-detach-marker"),
            "reattach snapshot contains output produced after GUI driver exit"
        );
        resumed.control.stop();

        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if list_sessions()
                .map(|sessions| !sessions.iter().any(|session| session.id == session_id))
                .unwrap_or(false)
            {
                session_guard.disarm();
                sessions_guard.disarm();
                host_guard
                    .terminate()
                    .expect("terminate host by the PID reported by the GUI driver");
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("explicit stop did not remove the isolated host session");
    }

    fn run_isolated_gui_driver() {
        use std::process::{Command, Stdio};

        let host_pid_path = std::path::PathBuf::from(
            std::env::var_os("FASTTY_TEST_HOST_PID_FILE").expect("host PID file path"),
        );
        let session_id_path = std::path::PathBuf::from(
            std::env::var_os("FASTTY_TEST_SESSION_ID_FILE").expect("session ID file path"),
        );
        crate::paths::init().expect("initialize isolated GUI driver paths");
        let executable = std::env::current_exe().expect("get test executable");
        let mut host = HostLaunchGuard(Some(
            Command::new(&executable)
                .arg("--exact")
                .arg("session_host::tests::host_survives_gui_projection_drop_and_reattach")
                .arg("--nocapture")
                .env("FASTTY_TEST_SESSION_HOST_ROLE", "server")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start isolated session host from GUI driver"),
        ));
        std::fs::write(host_pid_path, host.child_mut().id().to_string())
            .expect("write host PID for lifecycle parent");
        wait_for_socket(&crate::daemon::socket_path(), host.child_mut());
        assert!(!session_exists(usize::MAX).expect("query missing session"));
        assert!(!session_exists(usize::MAX).expect("query registry after missing ID"));

        let idle_projection = crate::terminal_state::TerminalState::new_hosted(
            "/bin/sh",
            &[],
            None,
            2_000,
            crate::config::FontConfig::default(),
            10.0,
            20.0,
            800.0,
            480.0,
            crate::event_listener::EventSender::None,
        )
        .expect("spawn idle hosted shell");
        let idle_session_id = idle_projection
            .remote_session_id()
            .expect("idle terminal has a host session id");
        assert!(!idle_projection.needs_close_confirmation());
        attach(idle_session_id)
            .expect("attach to idle hosted shell")
            .control
            .stop();
        drop(idle_projection);

        let args = vec![
            "-c".to_string(),
            "sleep 1; printf 'output-after-detach-marker\\n'; sleep 30".to_string(),
        ];
        let projection = crate::terminal_state::TerminalState::new_hosted(
            "/bin/sh",
            &args,
            None,
            2_000,
            crate::config::FontConfig::default(),
            10.0,
            20.0,
            800.0,
            480.0,
            crate::event_listener::EventSender::None,
        )
        .expect("spawn isolated hosted shell");
        let session_id = projection
            .remote_session_id()
            .expect("hosted terminal has a host session id");
        let process_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !session_process_running(session_id).unwrap_or(false)
            && std::time::Instant::now() < process_deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(
            projection.needs_close_confirmation(),
            "an active hosted shell keeps the close confirmation"
        );
        std::fs::write(session_id_path, session_id.to_string())
            .expect("write host session ID for lifecycle parent");
        drop(projection);
        host.detach();
    }

    fn wait_for_socket(path: &std::path::Path, host: &mut std::process::Child) {
        use std::os::unix::net::UnixStream;
        use std::time::{Duration, Instant};

        let deadline = Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Some(status) = host.try_wait().expect("check host process") {
                panic!("isolated host exited before binding: {status}");
            }
            if UnixStream::connect(path).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("isolated host did not bind its private socket");
    }

    fn wait_for_child_exit_for(
        child: &mut std::process::Child,
        timeout: std::time::Duration,
    ) -> std::process::ExitStatus {
        try_wait_for_child_exit_for(child, timeout)
            .expect("test child process did not exit before timeout")
    }

    fn try_wait_for_child_exit_for(
        child: &mut std::process::Child,
        timeout: std::time::Duration,
    ) -> Option<std::process::ExitStatus> {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Some(status) = child.try_wait().expect("check duplicate host process") {
                return Some(status);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        None
    }
}
