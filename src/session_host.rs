//! Local process host for PTY sessions.

use crate::daemon::{AttachMode, BlockedSession, Request, Response, SessionInfo};
use std::io::{BufRead, BufReader, Write};

#[derive(Debug)]
pub struct SpawnAttachError {
    pub session_id: Option<usize>,
    pub retry_safe: bool,
    pub message: String,
}

impl std::fmt::Display for SpawnAttachError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SpawnAttachError {}

impl From<String> for SpawnAttachError {
    fn from(message: String) -> Self {
        Self {
            session_id: None,
            retry_safe: true,
            message,
        }
    }
}

impl SpawnAttachError {
    pub fn ambiguous(message: String) -> Self {
        Self {
            session_id: None,
            retry_safe: false,
            message: format!("{message}; the session may still be running"),
        }
    }

    pub fn orphaned_session(session_id: usize, message: String) -> Self {
        Self {
            session_id: Some(session_id),
            retry_safe: false,
            message: format!("{message}; session {session_id} may still be running"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct HostInfo {
    version: u32,
    fastty_version: String,
    capabilities: Vec<String>,
    host_instance: Option<crate::daemon::HostInstanceId>,
}

impl HostInfo {
    fn supports(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|item| item == capability)
    }
}

static HOST_INFO: std::sync::OnceLock<std::sync::Mutex<Option<HostInfo>>> =
    std::sync::OnceLock::new();

fn remember_host_info(info: HostInfo) -> bool {
    let cache = HOST_INFO.get_or_init(|| std::sync::Mutex::new(None));
    let mut cached = cache.lock().unwrap_or_else(|error| error.into_inner());
    let changed = cached.as_ref() != Some(&info);
    *cached = Some(info);
    changed
}

fn forget_host_info() {
    if let Some(cache) = HOST_INFO.get() {
        *cache.lock().unwrap_or_else(|error| error.into_inner()) = None;
    }
}

fn host_info() -> Option<HostInfo> {
    HOST_INFO.get().and_then(|cache| cache.lock().ok()?.clone())
}

pub fn current_host_instance() -> Option<crate::daemon::HostInstanceId> {
    host_info().and_then(|info| info.host_instance)
}

#[derive(Debug)]
pub enum SessionReferenceStatus {
    Present,
    Absent,
    Ended,
    Unknown(String),
}

pub fn probe_session_reference(
    id: usize,
    identity: Option<crate::daemon::SessionIdentity>,
) -> SessionReferenceStatus {
    let expected = match reference_host(id, identity) {
        Ok(expected) => expected,
        Err(error) => return SessionReferenceStatus::Unknown(error),
    };
    #[cfg(unix)]
    {
        use std::io::ErrorKind;
        let stream = match std::os::unix::net::UnixStream::connect(crate::daemon::socket_path()) {
            Ok(stream) => stream,
            Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
                return SessionReferenceStatus::Absent;
            }
            Err(error) => return SessionReferenceStatus::Unknown(error.to_string()),
        };
        let probe = || -> Result<SessionReferenceStatus, String> {
            stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).map_err(|error| error.to_string())?;
            stream.set_write_timeout(Some(std::time::Duration::from_secs(3))).map_err(|error| error.to_string())?;
            let mut stream = stream;
            validate_host_connection(&mut stream, &expected)?;
            write_request(&mut stream, &Request::SessionExists { id }).map_err(|error| error.to_string())?;
            match read_response(&mut BufReader::new(&mut stream))? {
                Response::SessionExists { exists: false } => return Ok(SessionReferenceStatus::Absent),
                Response::SessionExists { exists: true } => {},
                Response::Error { code, message } => return Err(format!("{code}: {message}")),
                other => return Err(format!("unexpected session existence response: {other:?}")),
            }
            write_request(&mut stream, &Request::SessionStatus { id }).map_err(|error| error.to_string())?;
            match read_response(&mut BufReader::new(stream))? {
                Response::SessionStatus { alive: true, .. } => Ok(SessionReferenceStatus::Present),
                Response::SessionStatus { alive: false, .. } => Ok(SessionReferenceStatus::Ended),
                Response::Error { code, message } => Err(format!("{code}: {message}")),
                other => Err(format!("unexpected session status response: {other:?}")),
            }
        };
        match probe() {
            Ok(status) => status,
            Err(error) if error.starts_with("host_instance_changed:") || error.starts_with("no_such_session:") => SessionReferenceStatus::Absent,
            Err(error) => SessionReferenceStatus::Unknown(error),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = expected;
        SessionReferenceStatus::Unknown("host sessions are not supported on this platform".into())
    }
}

pub fn host_status_label() -> String {
    match host_info() {
        Some(info) => format!(
            "Session Host: {} · App {}",
            info.fastty_version,
            env!("CARGO_PKG_VERSION")
        ),
        None => "Session Host: Unavailable".into(),
    }
}

fn attach_snapshot_format() -> crate::daemon::SnapshotFormat {
    match host_info() {
        Some(info) if info.supports("terminal_state_snapshot_v1") => {
            crate::daemon::SnapshotFormat::TerminalState
        }
        _ => crate::daemon::SnapshotFormat::Legacy,
    }
}

#[cfg(unix)]
fn connect_for_handshake() -> Result<std::os::unix::net::UnixStream, String> {
    use std::time::Duration;

    let stream = std::os::unix::net::UnixStream::connect(crate::daemon::socket_path())
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    Ok(stream)
}

#[cfg(unix)]
fn probe_host(stream: std::os::unix::net::UnixStream) -> Result<HostInfo, String> {
    probe_host_with_access(stream, true)
}

#[cfg(unix)]
fn probe_host_with_access(
    mut stream: std::os::unix::net::UnixStream,
    read_only: bool,
) -> Result<HostInfo, String> {
    write_request(&mut stream, &Request::Hello { read_only }).map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::Hello {
            version,
            fastty_version,
            capabilities,
            host_instance,
        } => Ok(HostInfo {
            version,
            fastty_version,
            capabilities,
            host_instance,
        }),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected session-host hello response: {other:?}")),
    }
}

fn reference_host(
    id: usize,
    identity: Option<crate::daemon::SessionIdentity>,
) -> Result<crate::daemon::HostInstanceId, String> {
    identity
        .filter(|identity| identity.session_id == id)
        .map(|identity| identity.host_instance)
        .ok_or_else(|| "session_identity_unknown: the saved session has no host identity".into())
}

#[cfg(unix)]
pub fn host_status() -> Result<crate::daemon::HostStatus, String> {
    let mut stream = connect_for_handshake()?;
    write_request(&mut stream, &Request::HostStatus).map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::HostStatus { status } => Ok(status),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected host status response: {other:?}")),
    }
}

#[cfg(unix)]
pub fn restart_host() -> Result<(), String> {
    let mut stream = connect_for_handshake()?;
    write_request(&mut stream, &Request::HostStatus).map_err(|error| error.to_string())?;
    let status = match read_response(&mut BufReader::new(&mut stream))? {
        Response::HostStatus { status } => status,
        Response::Error { code, message } => return Err(format!("{code}: {message}")),
        other => return Err(format!("unexpected host status response: {other:?}")),
    };
    if !status
        .capabilities
        .iter()
        .any(|capability| capability == "safe_host_restart_v1")
    {
        return Err("host_restart_unsupported: this host does not support a safe restart".into());
    }
    write_request(
        &mut stream,
        &Request::RestartHost {
            expected_host_instance: status.host_instance.clone(),
        },
    )
    .map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::HostRestarting => {}
        Response::Error { code, message } => return Err(format!("{code}: {message}")),
        other => return Err(format!("unexpected host restart response: {other:?}")),
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::os::unix::net::UnixStream::connect(crate::daemon::socket_path()).is_ok() {
        if std::time::Instant::now() >= deadline {
            return Err("host restart timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    forget_host_info();
    ensure_running()
}

const HOST_ARGUMENT: &str = "--fastty-session-host";

#[cfg(unix)]
pub fn ensure_running() -> Result<(), String> {
    use std::os::unix::net::UnixStream;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let path = crate::daemon::socket_path();
    if UnixStream::connect(&path).is_ok() {
        if let Ok(stream) = connect_for_handshake() {
            match probe_host(stream) {
                Ok(info) => {
                    let changed = remember_host_info(info.clone());
                    if changed
                        && (info.fastty_version != env!("CARGO_PKG_VERSION")
                            || info.version != crate::daemon::PROTOCOL_VERSION)
                    {
                        eprintln!(
                            "Fastty session host differs from this app: app={}, host={}, protocol={}, capabilities={:?}",
                            env!("CARGO_PKG_VERSION"),
                            info.fastty_version,
                            info.version,
                            info.capabilities
                        );
                    }
                    return Ok(());
                }
                Err(error) => {
                    forget_host_info();
                    eprintln!("Fastty session host hello failed: {error}");
                    return Ok(());
                }
            }
        }
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
        if let Ok(stream) = connect_for_handshake() {
            if let Ok(info) = probe_host(stream) {
                remember_host_info(info);
                return Ok(());
            }
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
    pub identity: Option<crate::daemon::SessionIdentity>,
    pub snapshot: Vec<u8>,
    pub program_status: Vec<crate::program_status::ProgramRecord>,
    pub process_running: Option<bool>,
    pub cwd: Option<String>,
    pub shell_pid: Option<u32>,
    pub restore_legacy_mouse_modes: bool,
    pub reader: BufReader<std::os::unix::net::UnixStream>,
    pub control: std::sync::Arc<RemoteControl>,
}

#[cfg(not(unix))]
pub struct RemoteSession {
    pub id: usize,
    pub identity: Option<crate::daemon::SessionIdentity>,
    pub snapshot: Vec<u8>,
    pub program_status: Vec<crate::program_status::ProgramRecord>,
    pub process_running: Option<bool>,
    pub cwd: Option<String>,
    pub shell_pid: Option<u32>,
    pub restore_legacy_mouse_modes: bool,
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

    pub fn stop_with_result(&self) -> Result<(), String> {
        self.stop();
        Ok(())
    }
}

#[cfg(unix)]
pub fn session_process_running(id: usize) -> Result<bool, String> {
    session_process_status(id).map(|(process_running, _)| process_running)
}

#[cfg(unix)]
pub fn session_process_status(id: usize) -> Result<(bool, Option<String>), String> {
    ensure_running()?;
    let mut stream = connect_for_handshake()?;
    validate_session_connection(&mut stream, id)?;
    write_request(&mut stream, &Request::SessionStatus { id })
        .map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::SessionStatus {
            process_running,
            foreground_process_label,
            ..
        } => Ok((process_running, foreground_process_label)),
        Response::Error { code, .. } if code == "bad_request" => {
            let session = legacy_session_info(id)?;
            let process_running = legacy_session_process_running(&session)
                .ok_or_else(|| "legacy host returned an unknown process name".to_string())?;
            let process_name = process_running.then(|| session.title.clone());
            Ok((process_running, process_name))
        }
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected session status response: {other:?}")),
    }
}

fn reference_request(
    id: usize,
    identity: Option<crate::daemon::SessionIdentity>,
    request: Request,
) -> Result<Response, String> {
    let expected = reference_host(id, identity)?;
    #[cfg(unix)]
    {
        let mut stream = connect_for_handshake()?;
        validate_host_connection(&mut stream, &expected)?;
        write_request(&mut stream, &request).map_err(|error| error.to_string())?;
        return read_response(&mut BufReader::new(stream));
    }
    #[cfg(not(unix))]
    {
        let _ = (expected, request);
        Err("host sessions are not supported on this platform".into())
    }
}

pub fn session_process_status_reference(
    id: usize,
    identity: Option<crate::daemon::SessionIdentity>,
) -> Result<(bool, Option<String>), String> {
    match reference_request(id, identity, Request::SessionStatus { id })? {
        Response::SessionStatus {
            process_running,
            foreground_process_label,
            ..
        } => Ok((process_running, foreground_process_label)),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected session status response: {other:?}")),
    }
}

pub fn session_exists_reference(
    id: usize,
    identity: Option<crate::daemon::SessionIdentity>,
) -> Result<bool, String> {
    match reference_request(id, identity, Request::SessionExists { id })? {
        Response::SessionExists { exists } => Ok(exists),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected session existence response: {other:?}")),
    }
}

pub fn close_if_idle_reference(
    id: usize,
    identity: Option<crate::daemon::SessionIdentity>,
) -> Result<CloseIfIdleResult, String> {
    let response = match reference_request(id, identity.clone(), Request::CloseIfIdle { id }) {
        Ok(response) => response,
        Err(error) if error.starts_with("host_instance_changed:") => {
            return Ok(CloseIfIdleResult::Closed)
        }
        Err(error) => {
            if matches!(probe_session_reference(id, identity), SessionReferenceStatus::Absent) {
                return Ok(CloseIfIdleResult::Closed);
            }
            return Err(error);
        }
    };
    match response {
        Response::Closed { .. } => Ok(CloseIfIdleResult::Closed),
        Response::CloseBlocked {
            foreground_process_label,
            ..
        } => Ok(CloseIfIdleResult::Running(foreground_process_label)),
        Response::Error { code, .. }
            if code == "no_such_session" || code == "host_instance_changed" =>
        {
            Ok(CloseIfIdleResult::Closed)
        }
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected close response: {other:?}")),
    }
}

fn current_batch_reference_ids(
    references: &[(usize, Option<crate::daemon::SessionIdentity>)],
    current: Option<&crate::daemon::HostInstanceId>,
) -> Result<Vec<usize>, String> {
    let mut ids = Vec::new();
    for (id, identity) in references {
        let expected = reference_host(*id, identity.clone())?;
        if Some(&expected) == current {
            ids.push(*id);
        }
    }
    Ok(ids)
}

pub fn close_if_idle_batch_references(
    references: Vec<(usize, Option<crate::daemon::SessionIdentity>)>,
) -> Result<CloseIfIdleBatchResult, String> {
    for (id, identity) in &references {
        reference_host(*id, identity.clone())?;
    }
    if references.is_empty() {
        return Ok(CloseIfIdleBatchResult::Closed);
    }
    #[cfg(unix)]
    {
        let close = || -> Result<CloseIfIdleBatchResult, String> {
        let mut stream = connect_for_handshake()?;
        let info = probe_host_with_access(
            stream.try_clone().map_err(|error| error.to_string())?,
            false,
        )?;
        let ids = current_batch_reference_ids(&references, info.host_instance.as_ref())?;
        if ids.is_empty() {
            return Ok(CloseIfIdleBatchResult::Closed);
        }
        write_request(&mut stream, &Request::CloseIfIdleBatch { ids })
            .map_err(|error| error.to_string())?;
        match read_response(&mut BufReader::new(stream))? {
            Response::CloseBatch { blocked, .. } if blocked.is_empty() => {
                Ok(CloseIfIdleBatchResult::Closed)
            }
            Response::CloseBatch { blocked, .. } => Ok(CloseIfIdleBatchResult::Running(blocked)),
            Response::Error { code, message } => Err(format!("{code}: {message}")),
            other => Err(format!("unexpected batch close response: {other:?}")),
        }
        };
        match close() {
            Ok(result) => Ok(result),
            Err(error) => {
                if references.iter().all(|(id, identity)| matches!(probe_session_reference(*id, identity.clone()), SessionReferenceStatus::Absent)) {
                    Ok(CloseIfIdleBatchResult::Closed)
                } else {
                    Err(error)
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        Err("host sessions are not supported on this platform".into())
    }
}

pub enum CloseIfIdleResult {
    Closed,
    Running(Option<String>),
}

pub enum CloseIfIdleBatchResult {
    Closed,
    Running(Vec<BlockedSession>),
}

#[cfg(unix)]
pub fn close_if_idle_batch(ids: Vec<usize>) -> Result<CloseIfIdleBatchResult, String> {
    ensure_running()?;
    if host_info().is_some_and(|info| !info.supports("close_if_idle_batch")) {
        return Err(legacy_batch_close_error());
    }
    let mut stream = connect_for_handshake()?;
    for id in &ids {
        validate_session_connection(&mut stream, *id)?;
    }
    write_request(&mut stream, &Request::CloseIfIdleBatch { ids })
        .map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::CloseBatch { blocked, .. } if blocked.is_empty() => {
            Ok(CloseIfIdleBatchResult::Closed)
        }
        Response::CloseBatch { blocked, .. } => Ok(CloseIfIdleBatchResult::Running(blocked)),
        Response::Error { code, message }
            if code == "bad_request"
                && message.contains("unknown variant `close_if_idle_batch`") =>
        {
            Err(legacy_batch_close_error())
        }
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected batch close response: {other:?}")),
    }
}

#[cfg(not(unix))]
pub fn close_if_idle_batch(_ids: Vec<usize>) -> Result<CloseIfIdleBatchResult, String> {
    Err("host sessions are not supported on this platform".to_string())
}

#[cfg(unix)]
pub fn close_if_idle(id: usize) -> Result<CloseIfIdleResult, String> {
    ensure_running()?;
    if host_info().is_some_and(|info| !info.supports("close_if_idle")) {
        return close_if_idle_on_legacy_host(id);
    }
    let mut stream = connect_for_handshake()?;
    validate_session_connection(&mut stream, id)?;
    write_request(&mut stream, &Request::CloseIfIdle { id }).map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::Closed { .. } => Ok(CloseIfIdleResult::Closed),
        Response::CloseBlocked {
            foreground_process_label,
            ..
        } => Ok(CloseIfIdleResult::Running(foreground_process_label)),
        Response::Error { code, .. } if code == "no_such_session" => Ok(CloseIfIdleResult::Closed),
        Response::Error { code, message }
            if code == "bad_request" && message.contains("unknown variant `close_if_idle`") =>
        {
            close_if_idle_on_legacy_host(id)
        }
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected close response: {other:?}")),
    }
}

#[cfg(unix)]
fn close_if_idle_on_legacy_host(id: usize) -> Result<CloseIfIdleResult, String> {
    close_legacy_session_if_idle(id, session_process_status, force_close_session)
}

fn close_legacy_session_if_idle(
    id: usize,
    status: impl FnOnce(usize) -> Result<(bool, Option<String>), String>,
    close: impl FnOnce(usize) -> Result<(), String>,
) -> Result<CloseIfIdleResult, String> {
    match status(id) {
        Ok((true, process_name)) => Ok(CloseIfIdleResult::Running(process_name)),
        Ok((false, _)) => {
            // Old hosts cannot make this status check and close atomic. Only
            // close after the host reports that the foreground process is idle.
            close(id)
                .map(|()| CloseIfIdleResult::Closed)
                .map_err(|error| format!("legacy idle close failed: {error}"))
        }
        Err(error) if error.starts_with("no_such_session:") => Ok(CloseIfIdleResult::Closed),
        Err(error) => Err(format!(
            "cannot verify that legacy host session {id} is idle. The tab stays open: {error}"
        )),
    }
}

fn legacy_batch_close_error() -> String {
    "the session host cannot close several tabs as one safe operation. Close tabs one at a time."
        .to_string()
}

#[cfg(not(unix))]
pub fn close_if_idle(_id: usize) -> Result<CloseIfIdleResult, String> {
    Err("host sessions are not supported on this platform".to_string())
}

#[cfg(not(unix))]
pub fn session_process_status(_id: usize) -> Result<(bool, Option<String>), String> {
    Ok((false, None))
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
        let _ = self.stop_with_result();
    }

    pub fn stop_with_result(&self) -> Result<(), String> {
        if self.closed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        let result = self
            .send(&Request::Close {
                id: self.id,
                force: true,
            })
            .map_err(|error| error.to_string());
        self.shutdown();
        result
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
) -> Result<RemoteSession, SpawnAttachError> {
    ensure_running().map_err(SpawnAttachError::from)?;
    let spawn_started = std::time::Instant::now();
    let request_id = next_spawn_request_id();
    let mut stream = connect_for_handshake()?;
    let info = probe_host_with_access(
        stream.try_clone().map_err(|error| error.to_string())?,
        false,
    )?;
    let expected_host_instance = info.host_instance.clone();
    remember_host_info(info);
    let spawn_response = write_request(
        &mut stream,
        &Request::Spawn {
            request_id: Some(request_id.clone()),
            command: Some(command.to_string()),
            args: args.to_vec(),
            cwd: cwd.map(str::to_string),
            cols: Some(cols),
            rows: Some(rows),
            open: false,
        },
    );
    let id = match spawn_response {
        Err(error) => resolve_ambiguous_spawn(&request_id, expected_host_instance.as_ref())
            .map_err(|_| SpawnAttachError::ambiguous(error.to_string()))?,
        Ok(()) => match read_response(&mut BufReader::new(stream)) {
            Ok(Response::Spawned { id, .. }) => id,
            Ok(Response::Error { code, message }) => {
                return Err(SpawnAttachError::from(format!("{code}: {message}")));
            }
            Ok(other) => resolve_ambiguous_spawn(&request_id, expected_host_instance.as_ref())
                .map_err(|_| {
                    SpawnAttachError::ambiguous(format!("unexpected spawn response: {other:?}"))
                })?,
            Err(error) => resolve_ambiguous_spawn(&request_id, expected_host_instance.as_ref())
                .map_err(|_| SpawnAttachError::ambiguous(error))?,
        },
    };
    eprintln!(
        "Fastty session host spawn complete: session_id={id}, elapsed_ms={}",
        spawn_started.elapsed().as_millis()
    );

    let attach_started = std::time::Instant::now();
    let attached = (|| -> Result<RemoteSession, String> {
        let mut stream = connect_for_handshake()?;
        validate_session_connection(&mut stream, id)?;
        write_request(
            &mut stream,
            &Request::Attach {
                id,
                expected_host_instance: expected_host_instance.clone(),
                mode: AttachMode::ReadWrite,
                binary_snapshot: false,
                snapshot_format: attach_snapshot_format(),
            },
        )
        .map_err(|error| error.to_string())?;
        let control_stream = stream.try_clone().map_err(|error| error.to_string())?;
        let mut reader = BufReader::new(stream);
        let (program_status, process_running, cwd, shell_pid) = match read_response(&mut reader)? {
            Response::Attached {
                program_status,
                process_running,
                cwd,
                shell_pid,
                ..
            } => (program_status, process_running, cwd, shell_pid),
            Response::Error { code, message } => return Err(format!("{code}: {message}")),
            other => return Err(format!("unexpected attach response: {other:?}")),
        };
        let snapshot = match read_response(&mut reader)? {
            Response::Snapshot { data, .. } => crate::daemon::base64_decode(&data)
                .ok_or_else(|| "session host sent an invalid snapshot".to_string())?,
            Response::Error { code, message } => return Err(format!("{code}: {message}")),
            other => return Err(format!("unexpected snapshot response: {other:?}")),
        };
        eprintln!(
            "Fastty session host attach complete: session_id={id}, snapshot_bytes={}, elapsed_ms={}",
            snapshot.len(),
            attach_started.elapsed().as_millis()
        );
        reader
            .get_mut()
            .set_read_timeout(None)
            .map_err(|error| error.to_string())?;
        Ok(RemoteSession {
            id,
            identity: expected_host_instance.clone().map(|host_instance| {
                crate::daemon::SessionIdentity {
                    host_instance,
                    session_id: id,
                }
            }),
            snapshot,
            program_status,
            process_running,
            cwd,
            shell_pid,
            restore_legacy_mouse_modes: false,
            reader,
            control: std::sync::Arc::new(RemoteControl {
                id,
                stream: std::sync::Mutex::new(control_stream),
                closed: std::sync::atomic::AtomicBool::new(false),
            }),
        })
    })();
    attached.map_err(|message| {
        let cleanup = force_close_reference(
            id,
            expected_host_instance
                .clone()
                .map(|host_instance| crate::daemon::SessionIdentity {
                    host_instance,
                    session_id: id,
                }),
        )
        .err();
        SpawnAttachError {
            session_id: cleanup.as_ref().map(|_| id),
            retry_safe: true,
            message: match cleanup {
                Some(cleanup_error) => {
                    format!("{message}; session {id} could not be stopped: {cleanup_error}")
                }
                None => message,
            },
        }
    })
}

#[cfg(unix)]
fn next_spawn_request_id() -> String {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!(
        "{}-{timestamp}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

#[cfg(unix)]
fn resolve_ambiguous_spawn(
    request_id: &str,
    expected_host_instance: Option<&crate::daemon::HostInstanceId>,
) -> Result<usize, String> {
    let expected = expected_host_instance.ok_or_else(|| {
        "spawn_recovery_unsupported: the original host has no instance identity".to_string()
    })?;
    let mut stream = connect_for_handshake()?;
    validate_host_connection(&mut stream, expected)?;
    write_request(
        &mut stream,
        &Request::SpawnStatus {
            request_id: request_id.to_string(),
        },
    )
    .map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::SpawnStatus { id: Some(id) } => Ok(id),
        Response::SpawnStatus { id: None } => {
            Err("host has no record of this spawn request".into())
        }
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected spawn-status response: {other:?}")),
    }
}

fn force_close_reference(
    id: usize,
    identity: Option<crate::daemon::SessionIdentity>,
) -> Result<(), String> {
    match reference_request(id, identity, Request::Close { id, force: true })? {
        Response::Closed { .. } => Ok(()),
        Response::Error { code, .. } if code == "no_such_session" => Ok(()),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected close response: {other:?}")),
    }
}

#[cfg(unix)]
fn force_close_session(id: usize) -> Result<(), String> {
    let mut stream = connect_for_handshake()?;
    validate_session_connection(&mut stream, id)?;
    write_request(&mut stream, &Request::Close { id, force: true })
        .map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::Closed { .. } => Ok(()),
        Response::Error { code, .. } if code == "no_such_session" => Ok(()),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected close response: {other:?}")),
    }
}

#[cfg(unix)]
pub fn attach(id: usize) -> Result<RemoteSession, String> {
    ensure_running()?;
    attach_expected(id, host_info().and_then(|info| info.host_instance))
}

#[cfg(unix)]
pub fn attach_reference(
    id: usize,
    identity: Option<crate::daemon::SessionIdentity>,
) -> Result<RemoteSession, String> {
    let expected = reference_host(id, identity)?;
    attach_expected(id, Some(expected))
}

#[cfg(unix)]
fn attach_expected(
    id: usize,
    expected_host_instance: Option<crate::daemon::HostInstanceId>,
) -> Result<RemoteSession, String> {
    let mut stream = connect_for_handshake()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    if let Some(expected) = &expected_host_instance {
        validate_host_connection(&mut stream, expected)?;
    }
    write_request(&mut stream, &Request::List).map_err(|error| error.to_string())?;
    let legacy_session = match read_response(&mut BufReader::new(&mut stream))? {
        Response::Sessions { sessions } => sessions.into_iter().find(|session| session.id == id),
        Response::Error { code, message } => return Err(format!("{code}: {message}")),
        other => return Err(format!("unexpected session-list response: {other:?}")),
    };
    write_request(
        &mut stream,
        &Request::Attach {
            id,
            expected_host_instance: expected_host_instance.clone(),
            mode: AttachMode::ReadWrite,
            binary_snapshot: true,
            snapshot_format: attach_snapshot_format(),
        },
    )
    .map_err(|error| error.to_string())?;
    let control_stream = stream.try_clone().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);
    let (program_status, attached_process_running, cwd, shell_pid) = match read_response(&mut reader)? {
        Response::Attached {
            program_status,
            process_running,
            cwd,
            shell_pid,
            ..
        } => (program_status, process_running, cwd, shell_pid),
        Response::Error { code, message } => return Err(format!("{code}: {message}")),
        other => return Err(format!("unexpected attach response: {other:?}")),
    };
    let snapshot = match read_response(&mut reader)? {
        Response::Snapshot { data, .. } => crate::daemon::base64_decode(&data)
            .ok_or_else(|| "session host sent an invalid snapshot".to_string())?,
        Response::Error { code, message } => return Err(format!("{code}: {message}")),
        other => return Err(format!("unexpected snapshot response: {other:?}")),
    };
    reader
        .get_mut()
        .set_read_timeout(None)
        .map_err(|error| error.to_string())?;
    let (restore_legacy_mouse_modes, process_running) = match attached_process_running {
        Some(process_running) => (false, Some(process_running)),
        None => match query_session_status(id, expected_host_instance.as_ref())? {
            Response::SessionStatus {
                process_running, ..
            } => (false, Some(process_running)),
            Response::Error { code, .. } if code == "bad_request" => {
                let session = legacy_session.as_ref().ok_or_else(|| {
                    format!("no_such_session: session ID {id} not found")
                })?;
                (
                    legacy_session_needs_mouse_modes(session),
                    legacy_session_process_running(session),
                )
            }
            Response::Error { code, message } => return Err(format!("{code}: {message}")),
            other => return Err(format!("unexpected session status response: {other:?}")),
        },
    };
    Ok(RemoteSession {
        id,
        identity: expected_host_instance.clone().map(|host_instance| {
            crate::daemon::SessionIdentity {
                host_instance,
                session_id: id,
            }
        }),
        snapshot,
        program_status,
        process_running,
        cwd: cwd.or_else(|| legacy_session.and_then(|session| session.cwd)),
        shell_pid,
        restore_legacy_mouse_modes,
        reader,
        control: std::sync::Arc::new(RemoteControl {
            id,
            stream: std::sync::Mutex::new(control_stream),
            closed: std::sync::atomic::AtomicBool::new(false),
        }),
    })
}

#[cfg(unix)]
fn query_session_status(
    id: usize,
    expected_host_instance: Option<&crate::daemon::HostInstanceId>,
) -> Result<Response, String> {
    let mut stream = connect_for_handshake()?;
    if let Some(expected) = expected_host_instance {
        validate_host_connection(&mut stream, expected)?;
    } else {
        validate_session_connection(&mut stream, id)?;
    }
    write_request(&mut stream, &Request::SessionStatus { id })
        .map_err(|error| error.to_string())?;
    read_response(&mut BufReader::new(stream))
}

#[cfg(unix)]
fn legacy_session_info(id: usize) -> Result<SessionInfo, String> {
    list_sessions()?
        .into_iter()
        .find(|session| session.id == id)
        .ok_or_else(|| format!("no_such_session: session ID {id} not found"))
}

fn legacy_session_process_running(session: &SessionInfo) -> Option<bool> {
    if !session.alive {
        return Some(false);
    }
    let process_name = session
        .title
        .trim()
        .rsplit('/')
        .next()?
        .to_ascii_lowercase();
    if process_name.is_empty() {
        return None;
    }
    if matches!(
        process_name.as_str(),
        "shell" | "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "csh" | "tcsh"
    ) {
        return None;
    }
    Some(true)
}

fn legacy_session_needs_mouse_modes(session: &SessionInfo) -> bool {
    let process_name = session
        .title
        .trim()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        process_name.as_str(),
        "claude" | "opencode" | "agy" | "pi" | "codex"
    )
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
    ensure_running()?;
    let mut stream = connect_for_handshake()?;
    write_request(&mut stream, &Request::List).map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::Sessions { sessions } => Ok(sessions),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected session-list response: {other:?}")),
    }
}

#[cfg(unix)]
fn validate_session_connection(
    stream: &mut std::os::unix::net::UnixStream,
    _id: usize,
) -> Result<(), String> {
    let expected = host_info().and_then(|info| info.host_instance);
    if let Some(expected) = expected {
        validate_host_connection(stream, &expected)?;
    }
    Ok(())
}

#[cfg(unix)]
fn validate_host_connection(
    stream: &mut std::os::unix::net::UnixStream,
    expected: &crate::daemon::HostInstanceId,
) -> Result<(), String> {
    let info = probe_host_with_access(
        stream.try_clone().map_err(|error| error.to_string())?,
        false,
    )?;
    if info.host_instance.as_ref() != Some(expected) {
        return Err(
            "host_instance_changed: the saved session belongs to another host instance".into(),
        );
    }
    Ok(())
}

#[cfg(unix)]
pub fn session_exists(id: usize) -> Result<bool, String> {
    ensure_running()?;
    let mut stream = connect_for_handshake()?;
    validate_session_connection(&mut stream, id)?;
    write_request(&mut stream, &Request::SessionExists { id })
        .map_err(|error| error.to_string())?;
    match read_response(&mut BufReader::new(stream))? {
        Response::SessionExists { exists } => Ok(exists),
        Response::Error { code, message } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected session-exists response: {other:?}")),
    }
}

#[cfg(unix)]
type CwdCallback = dyn Fn(std::path::PathBuf) + Send + Sync;

#[cfg(unix)]
struct CwdConsumer {
    identity: crate::daemon::SessionIdentity,
    callback: Box<CwdCallback>,
    revision: std::sync::Mutex<u64>,
}

#[cfg(unix)]
#[derive(Default)]
struct CwdObservers {
    consumers: Vec<std::sync::Weak<CwdConsumer>>,
    running: bool,
    sessions: Vec<SessionInfo>,
    revision: u64,
}

#[cfg(unix)]
struct CwdObserver {
    host: crate::daemon::HostInstanceId,
    state: std::sync::Mutex<CwdObservers>,
}

#[cfg(unix)]
pub(crate) struct CwdObservation {
    _consumer: std::sync::Arc<CwdConsumer>,
    _observer: std::sync::Arc<CwdObserver>,
}

#[cfg(not(unix))]
pub(crate) struct CwdObservation;

#[cfg(not(unix))]
pub(crate) fn observe_cwd(
    _identity: crate::daemon::SessionIdentity,
    _callback: impl Fn(std::path::PathBuf) + Send + Sync + 'static,
) -> Option<CwdObservation> {
    None
}

#[cfg(unix)]
pub(crate) fn observe_cwd(
    identity: crate::daemon::SessionIdentity,
    callback: impl Fn(std::path::PathBuf) + Send + Sync + 'static,
) -> Option<CwdObservation> {
    use std::sync::{Arc, Mutex, OnceLock, Weak};
    static OBSERVERS: OnceLock<Mutex<Vec<Weak<CwdObserver>>>> = OnceLock::new();
    let mut observers = OBSERVERS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    observers.retain(|observer| observer.strong_count() > 0);
    let observer = observers
        .iter()
        .filter_map(Weak::upgrade)
        .find(|observer| observer.host == identity.host_instance)
        .unwrap_or_else(|| {
            let observer = Arc::new(CwdObserver {
                host: identity.host_instance.clone(),
                state: Mutex::new(CwdObservers::default()),
            });
            observers.push(Arc::downgrade(&observer));
            observer
        });
    let consumer = Arc::new(CwdConsumer {
        identity,
        callback: Box::new(callback),
        revision: std::sync::Mutex::new(0),
    });
    let mut state = observer
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    state.consumers.push(Arc::downgrade(&consumer));
    let cached = state
        .sessions
        .iter()
        .find(|session| session.id == consumer.identity.session_id && session.alive)
        .and_then(|session| session.cwd.as_ref())
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute());
    let revision = state.revision;
    if !state.running {
        state.running = true;
        let worker = Arc::clone(&observer);
        #[cfg(test)]
        let test_connection = CWD_TEST_CONNECTION.with(|slot| slot.borrow_mut().take());
        let started = std::thread::Builder::new().name("fastty-session-cwd".into()).spawn(move || {
            #[cfg(test)]
            let test_only = test_connection.is_some();
            #[cfg(test)]
            let mut test_connection = test_connection;
            let mut backoff_ms = 100;
            loop {
                if worker.consumers().is_empty() {
                    let mut state = worker.state.lock().unwrap_or_else(|error| error.into_inner());
                    state.consumers.retain(|consumer| consumer.strong_count() > 0);
                    if state.consumers.is_empty() {
                        state.running = false;
                        return;
                    }
                }
                #[cfg(test)]
                let connection = match test_connection.take() {
                    Some(stream) => Ok(stream),
                    None if test_only => Err("test connection closed".into()),
                    None => connect_for_handshake(),
                };
                #[cfg(not(test))]
                let connection = connect_for_handshake();
                if let Ok(stream) = connection {
                    if matches!(worker.follow(stream), Err(error) if error == "host_instance_changed") {
                        let mut state = worker.state.lock().unwrap_or_else(|error| error.into_inner());
                        state.consumers.clear();
                        state.sessions.clear();
                        state.running = false;
                        return;
                    }
                }
                for _ in 0..backoff_ms / 100 {
                    if worker.consumers().is_empty() { break; }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                backoff_ms = (backoff_ms * 2).min(5_000);
            }
        });
        if started.is_err() {
            state.running = false;
            return None;
        }
    }
    drop(state);
    drop(observers);
    if let Some(path) = cached {
        consumer.publish(revision, path);
    }
    Some(CwdObservation {
        _consumer: consumer,
        _observer: observer,
    })
}

#[cfg(unix)]
impl CwdConsumer {
    fn publish(&self, revision: u64, path: std::path::PathBuf) {
        let mut applied = self
            .revision
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if revision > *applied {
            *applied = revision;
            (self.callback)(path);
        }
    }
}

#[cfg(unix)]
impl CwdObserver {
    fn consumers(&self) -> Vec<std::sync::Arc<CwdConsumer>> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state
            .consumers
            .retain(|consumer| consumer.strong_count() > 0);
        state
            .consumers
            .iter()
            .filter_map(std::sync::Weak::upgrade)
            .collect()
    }

    fn valid_session(&self, session: &SessionInfo) -> bool {
        session.identity.as_ref().is_none_or(|identity| {
            identity.host_instance == self.host && identity.session_id == session.id
        })
    }

    fn apply(&self, response: Response) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.revision += 1;
        let revision = state.revision;
        let sessions = match response {
            Response::Sessions { sessions } => {
                let sessions: Vec<_> = sessions
                    .into_iter()
                    .filter(|session| self.valid_session(session))
                    .collect();
                state.sessions = sessions.clone();
                sessions
            }
            Response::SessionAdded { session } | Response::SessionUpdated { session }
                if self.valid_session(&session) =>
            {
                state.sessions.retain(|cached| cached.id != session.id);
                state.sessions.push(session.clone());
                vec![session]
            }
            Response::SessionRemoved { id } => {
                state.sessions.retain(|session| session.id != id);
                state.consumers.retain(|consumer| {
                    consumer
                        .upgrade()
                        .is_some_and(|consumer| consumer.identity.session_id != id)
                });
                Vec::new()
            }
            _ => Vec::new(),
        };
        let mut updates = Vec::new();
        for session in sessions.into_iter().filter(|session| session.alive) {
            let Some(path) = session
                .cwd
                .map(std::path::PathBuf::from)
                .filter(|path| path.is_absolute())
            else {
                continue;
            };
            for consumer in state.consumers.iter().filter_map(std::sync::Weak::upgrade) {
                if consumer.identity.session_id == session.id {
                    updates.push((consumer, path.clone()));
                }
            }
        }
        drop(state);
        for (consumer, path) in updates {
            consumer.publish(revision, path);
        }
    }

    fn follow(&self, mut stream: std::os::unix::net::UnixStream) -> Result<(), String> {
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(std::time::Duration::from_secs(3)))
            .map_err(|error| error.to_string())?;
        let info =
            probe_host_with_access(stream.try_clone().map_err(|error| error.to_string())?, true)?;
        if info.host_instance.as_ref() != Some(&self.host) {
            return Err("host_instance_changed".into());
        }
        let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
        let mut attached = false;
        for consumer in self.consumers() {
            write_request(
                &mut stream,
                &Request::SessionExists {
                    id: consumer.identity.session_id,
                },
            )
            .map_err(|error| error.to_string())?;
            match read_response(&mut reader)? {
                Response::SessionExists { exists: true } => attached = true,
                Response::SessionExists { exists: false } => {
                    self.state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .consumers
                        .retain(|registered| {
                            registered.upgrade().is_some_and(|registered| {
                                !std::sync::Arc::ptr_eq(&registered, &consumer)
                            })
                        });
                }
                Response::Error { code, message } => return Err(format!("{code}: {message}")),
                other => return Err(format!("unexpected session-exists response: {other:?}")),
            }
        }
        if !attached || self.consumers().is_empty() {
            return Ok(());
        }
        write_request(&mut stream, &Request::SubscribeSessions)
            .map_err(|error| error.to_string())?;
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(250)))
            .map_err(|error| error.to_string())?;
        let mut frame = Vec::new();
        while !self.consumers().is_empty() {
            match reader.read_until(b'\n', &mut frame) {
                Ok(0) => return Err("session host closed the connection".into()),
                Ok(_) if frame.last() == Some(&b'\n') => {
                    let response =
                        serde_json::from_slice(&frame).map_err(|error| error.to_string())?;
                    frame.clear();
                    if let Response::Error { code, message } = response {
                        return Err(format!("{code}: {message}"));
                    }
                    self.apply(response);
                }
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => return Err(error.to_string()),
            }
            if frame.len() > 4 * 1024 * 1024 {
                return Err("session registry frame is too large".into());
            }
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
thread_local! {
    static CWD_TEST_CONNECTION: std::cell::RefCell<Option<std::os::unix::net::UnixStream>> = const { std::cell::RefCell::new(None) };
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn cwd_baseline_keeps_a_consumer_registered_after_the_snapshot() {
        let (tx, rx) = std::sync::mpsc::channel();
        let consumer = std::sync::Arc::new(CwdConsumer {
            identity: crate::daemon::SessionIdentity {
                host_instance: crate::daemon::HostInstanceId("baseline-host".into()),
                session_id: 42,
            },
            callback: Box::new(move |path| {
                tx.send(path).unwrap();
            }),
            revision: std::sync::Mutex::new(0),
        });
        let observer = CwdObserver {
            host: consumer.identity.host_instance.clone(),
            state: std::sync::Mutex::new(CwdObservers {
                consumers: vec![std::sync::Arc::downgrade(&consumer)],
                ..Default::default()
            }),
        };
        observer.apply(Response::Sessions { sessions: vec![] });
        observer.apply(Response::SessionAdded {
            session: SessionInfo {
                id: 42,
                cwd: Some("/work/new-repository".into()),
                ..legacy_session("zsh", true)
            },
        });
        assert_eq!(
            rx.try_recv().unwrap(),
            std::path::PathBuf::from("/work/new-repository")
        );
    }

    #[test]
    fn cwd_consumer_rejects_a_cached_revision_after_a_new_update() {
        let (tx, rx) = std::sync::mpsc::channel();
        let consumer = CwdConsumer {
            identity: crate::daemon::SessionIdentity {
                host_instance: crate::daemon::HostInstanceId("revision-host".into()),
                session_id: 42,
            },
            callback: Box::new(move |path| {
                tx.send(path).unwrap();
            }),
            revision: std::sync::Mutex::new(0),
        };
        consumer.publish(2, "/work/current".into());
        consumer.publish(1, "/work/stale".into());
        assert_eq!(
            rx.try_recv().unwrap(),
            std::path::PathBuf::from("/work/current")
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn legacy_remote_projection_tracks_cwd_without_osc_after_spawn_and_restore() {
        for restored in [false, true] {
            let (reader, peer) = UnixStream::pair().unwrap();
            let control = reader.try_clone().unwrap();
            let (observer_client, mut observer_server) = UnixStream::pair().unwrap();
            CWD_TEST_CONNECTION.with(|slot| *slot.borrow_mut() = Some(observer_client));
            let host_instance = crate::daemon::HostInstanceId(format!("cwd-test-{restored}"));
            let identity = crate::daemon::SessionIdentity {
                host_instance: host_instance.clone(),
                session_id: 731,
            };
            let legacy: Response = serde_json::from_str(
                r#"{"event":"attached","id":731,"cols":80,"rows":24,"mode":"read_write"}"#,
            )
            .unwrap();
            let Response::Attached { cwd, shell_pid, .. } = legacy else {
                panic!("attached")
            };
            assert!(cwd.is_none() && shell_pid.is_none());
            let remote = super::RemoteSession {
                id: 731,
                identity: Some(identity.clone()),
                snapshot: Vec::new(),
                program_status: Vec::new(),
                process_running: Some(false),
                cwd,
                shell_pid,
                restore_legacy_mouse_modes: restored,
                reader: BufReader::new(reader),
                control: std::sync::Arc::new(super::RemoteControl {
                    id: 731,
                    stream: std::sync::Mutex::new(control),
                    closed: std::sync::atomic::AtomicBool::new(false),
                }),
            };
            let (events, received) = std::sync::mpsc::channel();
            let terminal = crate::terminal_state::TerminalState::attach_hosted_from_remote(
                remote,
                "/bin/sh",
                &[],
                Some("/home/stale"),
                100,
                crate::config::FontConfig::default(),
                10.0,
                20.0,
                800.0,
                480.0,
                crate::event_listener::EventSender::None,
            )
            .unwrap();
            terminal.set_event_sender(crate::event_listener::EventSender::Callback(
                std::sync::Arc::new(move |event| {
                    if let crate::terminal_state::AppEvent::CwdChanged(path) = event {
                        let _ = events.send(path);
                    }
                }),
            ));
            observer_server
                .set_read_timeout(Some(std::time::Duration::from_millis(500)))
                .unwrap();
            let mut requests = BufReader::new(observer_server.try_clone().unwrap());
            let mut line = String::new();
            // A legacy host exposes the cwd only in the session registry.
            let hello = requests.read_line(&mut line);
            assert!(
                hello.is_ok_and(|bytes| bytes > 0),
                "remote projection must observe the host registry"
            );
            assert!(matches!(
                serde_json::from_str::<Request>(&line).unwrap(),
                Request::Hello { read_only: true }
            ));
            writeln!(
                observer_server,
                "{}",
                serde_json::to_string(&Response::Hello {
                    version: 1,
                    fastty_version: "0.22.10".into(),
                    capabilities: Vec::new(),
                    host_instance: Some(host_instance),
                })
                .unwrap()
            )
            .unwrap();
            line.clear();
            requests.read_line(&mut line).unwrap();
            assert!(matches!(
                serde_json::from_str::<Request>(&line).unwrap(),
                Request::SessionExists { id: 731 }
            ));
            writeln!(
                observer_server,
                "{}",
                serde_json::to_string(&Response::SessionExists { exists: true }).unwrap()
            )
            .unwrap();
            line.clear();
            requests.read_line(&mut line).unwrap();
            assert!(matches!(
                serde_json::from_str::<Request>(&line).unwrap(),
                Request::SubscribeSessions
            ));
            let mut session = crate::daemon::SessionInfo {
                id: 731,
                identity: None,
                title: "sh".into(),
                cwd: Some("/home/live".into()),
                cols: 80,
                rows: 24,
                alive: true,
                history_size: 0,
                program_status: Vec::new(),
            };
            writeln!(
                observer_server,
                "{}",
                serde_json::to_string(&Response::Sessions {
                    sessions: vec![session.clone()]
                })
                .unwrap()
            )
            .unwrap();
            assert_eq!(
                received
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap(),
                std::path::PathBuf::from("/home/live")
            );
            assert_eq!(
                terminal.get_current_working_directory(),
                Some("/home/live".into())
            );
            let saved_snapshot = terminal.snapshot_state_for_save().unwrap();
            let (late_events, late_received) = std::sync::mpsc::channel();
            let late = super::observe_cwd(identity.clone(), move |path| {
                let _ = late_events.send(path);
            })
            .unwrap();
            assert_eq!(
                late_received
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .unwrap(),
                std::path::PathBuf::from("/home/live")
            );
            // Replay and updates use the same subscription, including consumers added after the baseline.
            assert_eq!(late._observer.consumers().len(), 2);
            for invalid in [
                crate::daemon::SessionInfo {
                    id: 999,
                    cwd: Some("/wrong/session".into()),
                    ..session.clone()
                },
                crate::daemon::SessionInfo {
                    cwd: Some("relative/path".into()),
                    ..session.clone()
                },
                crate::daemon::SessionInfo {
                    identity: Some(crate::daemon::SessionIdentity {
                        host_instance: crate::daemon::HostInstanceId("other-host".into()),
                        session_id: 731,
                    }),
                    cwd: Some("/wrong/host".into()),
                    ..session.clone()
                },
                session.clone(),
            ] {
                writeln!(
                    observer_server,
                    "{}",
                    serde_json::to_string(&Response::SessionUpdated { session: invalid }).unwrap()
                )
                .unwrap();
            }
            assert!(received
                .recv_timeout(std::time::Duration::from_millis(80))
                .is_err());
            assert_eq!(
                terminal.get_current_working_directory(),
                Some("/home/live".into())
            );
            session.cwd = Some("/work/biovity".into());
            writeln!(
                observer_server,
                "{}",
                serde_json::to_string(&Response::SessionUpdated {
                    session: session.clone()
                })
                .unwrap()
            )
            .unwrap();
            assert_eq!(
                received
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap(),
                std::path::PathBuf::from("/work/biovity")
            );
            assert_eq!(
                terminal.get_current_working_directory(),
                Some("/work/biovity".into())
            );
            let (restored_reader, restored_peer) = UnixStream::pair().unwrap();
            let restored_control = restored_reader.try_clone().unwrap();
            let restored_terminal = crate::terminal_state::TerminalState::attach_hosted_from_remote(
                super::RemoteSession {
                    id: 731,
                    identity: Some(identity.clone()),
                    snapshot: saved_snapshot,
                    program_status: Vec::new(),
                    process_running: Some(false),
                    cwd: None,
                    shell_pid: None,
                    restore_legacy_mouse_modes: false,
                    reader: BufReader::new(restored_reader),
                    control: std::sync::Arc::new(super::RemoteControl {
                        id: 731,
                        stream: std::sync::Mutex::new(restored_control),
                        closed: std::sync::atomic::AtomicBool::new(false),
                    }),
                },
                "/bin/sh",
                &[],
                Some("/home/stale"),
                100,
                crate::config::FontConfig::default(),
                10.0,
                20.0,
                800.0,
                480.0,
                crate::event_listener::EventSender::None,
            )
            .unwrap();
            assert_eq!(
                restored_terminal.get_current_working_directory(),
                Some("/work/biovity".into())
            );
            // The late observer also gets changes from the existing stream.
            while late_received.try_recv().is_ok() {}
            writeln!(
                observer_server,
                "{}",
                serde_json::to_string(&Response::SessionRemoved { id: 731 }).unwrap()
            )
            .unwrap();
            writeln!(
                observer_server,
                "{}",
                serde_json::to_string(&Response::SessionUpdated {
                    session: crate::daemon::SessionInfo {
                        cwd: Some("/removed".into()),
                        ..session.clone()
                    },
                })
                .unwrap()
            )
            .unwrap();
            assert!(received
                .recv_timeout(std::time::Duration::from_millis(80))
                .is_err());
            drop(late);
            drop(restored_terminal);
            drop(restored_peer);
            drop(terminal);
            drop(peer);
        }
    }

    #[test]
    fn connection_identity_check_keeps_manage_access_and_rejects_legacy_host() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let responder = std::thread::spawn(move || {
            let mut request = String::new();
            BufReader::new(server.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let request: Request = serde_json::from_str(&request).unwrap();
            assert!(matches!(request, Request::Hello { read_only: false }));
            server
                .write_all(b"{\"event\":\"hello\",\"version\":1,\"fastty_version\":\"0.22.6\"}\n")
                .unwrap();
        });
        let error = validate_host_connection(
            &mut client,
            &crate::daemon::HostInstanceId("saved-host".into()),
        )
        .unwrap_err();
        assert!(error.starts_with("host_instance_changed:"));
        responder.join().unwrap();
    }

    #[test]
    fn close_batch_excludes_stale_refs_and_keeps_current_refs() {
        let old = crate::daemon::HostInstanceId("old-host".into());
        let current = crate::daemon::HostInstanceId("current-host".into());
        let refs = vec![
            (
                100,
                Some(crate::daemon::SessionIdentity {
                    host_instance: old,
                    session_id: 100,
                }),
            ),
            (
                100,
                Some(crate::daemon::SessionIdentity {
                    host_instance: current.clone(),
                    session_id: 100,
                }),
            ),
        ];
        assert_eq!(
            current_batch_reference_ids(&refs, Some(&current)).unwrap(),
            vec![100]
        );
        assert!(current_batch_reference_ids(&refs, None).unwrap().is_empty());
        assert!(current_batch_reference_ids(&[(100, None)], Some(&current)).is_err());
    }

    #[test]
    fn saved_legacy_sessions_cannot_attach_by_number() {
        assert!(reference_host(100, None)
            .unwrap_err()
            .starts_with("session_identity_unknown:"));
        let old = crate::daemon::SessionIdentity {
            host_instance: crate::daemon::HostInstanceId("old-host".into()),
            session_id: 100,
        };
        let new = crate::daemon::SessionIdentity {
            host_instance: crate::daemon::HostInstanceId("new-host".into()),
            session_id: 100,
        };
        assert_ne!(
            reference_host(100, Some(old.clone())).unwrap(),
            reference_host(100, Some(new)).unwrap()
        );
        assert_eq!(
            reference_host(100, Some(old.clone())).unwrap(),
            old.host_instance
        );
        assert!(reference_host(101, Some(old)).is_err());
    }

    use super::*;
    use std::io::BufRead;
    use std::os::unix::net::UnixStream;

    fn legacy_session(title: &str, alive: bool) -> SessionInfo {
        SessionInfo {
            identity: None,
            id: 7,
            title: title.to_string(),
            cwd: None,
            cols: 80,
            rows: 24,
            alive,
            history_size: 0,
            program_status: vec![],
        }
    }

    #[test]
    fn legacy_host_list_identifies_idle_shell_and_running_agent() {
        assert_eq!(
            legacy_session_process_running(&legacy_session("zsh", true)),
            None
        );
        assert_eq!(
            legacy_session_process_running(&legacy_session("opencode", true)),
            Some(true)
        );
        assert_eq!(
            legacy_session_process_running(&legacy_session("zsh", false)),
            Some(false)
        );
        assert_eq!(
            legacy_session_process_running(&legacy_session("", true)),
            None
        );
        assert_eq!(
            legacy_session_process_running(&legacy_session("shell", true)),
            None
        );
        assert!(legacy_session_needs_mouse_modes(&legacy_session(
            "claude", true
        )));
        assert!(!legacy_session_needs_mouse_modes(&legacy_session(
            "zsh", true
        )));
    }

    #[test]
    fn legacy_idle_close_checks_status_before_force_close() {
        let close_called = std::cell::Cell::new(false);
        let active = close_legacy_session_if_idle(
            17,
            |_| Ok((true, Some("opencode".to_string()))),
            |_| {
                close_called.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(matches!(active, CloseIfIdleResult::Running(Some(label)) if label == "opencode"));
        assert!(!close_called.get());

        let idle = close_legacy_session_if_idle(
            17,
            |_| Ok((false, None)),
            |_| {
                close_called.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(matches!(idle, CloseIfIdleResult::Closed));
        assert!(close_called.get());

        close_called.set(false);
        let unknown = close_legacy_session_if_idle(
            17,
            |_| Err("legacy host returned an unknown process name".to_string()),
            |_| {
                close_called.set(true);
                Ok(())
            },
        );
        assert!(unknown.is_err());
        assert!(!close_called.get());
    }

    #[test]
    fn old_host_hello_without_capabilities_is_detected_as_legacy() {
        let response: Response =
            serde_json::from_str(r#"{"event":"hello","version":1,"fastty_version":"0.22.6"}"#)
                .unwrap();
        let Response::Hello {
            version,
            fastty_version,
            capabilities,
            host_instance,
        } = response
        else {
            panic!("expected hello response");
        };
        let info = HostInfo {
            version,
            fastty_version,
            capabilities,
            host_instance,
        };
        assert_eq!(info.fastty_version, "0.22.6");
        assert!(!info.supports("close_if_idle"));
    }

    #[test]
    fn handshake_reads_version_from_a_legacy_host() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let responder = std::thread::spawn(move || {
            let read_end = server.try_clone().unwrap();
            let mut request = String::new();
            BufReader::new(read_end).read_line(&mut request).unwrap();
            assert!(request.contains(r#""cmd":"hello""#));
            server
                .write_all(b"{\"event\":\"hello\",\"version\":1,\"fastty_version\":\"0.22.6\"}\n")
                .unwrap();
        });

        let info = probe_host(client).unwrap();
        responder.join().unwrap();
        assert_eq!(info.fastty_version, "0.22.6");
        assert!(!info.supports("close_if_idle"));
    }

    #[test]
    fn current_host_hello_advertises_safe_close_capabilities() {
        let capabilities = crate::daemon::session_host_capabilities();
        let info = HostInfo {
            version: crate::daemon::PROTOCOL_VERSION,
            fastty_version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities,
            host_instance: None,
        };
        assert!(info.supports("close_if_idle"));
        assert!(info.supports("close_if_idle_batch"));
    }

    #[test]
    fn ambiguous_spawn_error_disables_fresh_start_retry() {
        let error = SpawnAttachError::ambiguous("host disconnected".into());
        assert_eq!(error.session_id, None);
        assert!(!error.retry_safe);
        assert!(error.message.contains("session may still be running"));
    }

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
        let state = crate::server::native_snapshot::decode(&resumed.snapshot)
            .expect("reattach snapshot uses the negotiated native format");
        let snapshot: String = state
            .main
            .rows
            .iter()
            .flatten()
            .map(|cell| cell.c)
            .collect();
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
