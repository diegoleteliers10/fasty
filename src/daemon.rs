//! Local IPC daemon: lets other processes (a Neovim plugin, a Zed extension,
//! a future web bridge) list fastty's live tabs and attach to one — seeing
//! the same output the GPUI window renders and optionally sending input —
//! without taking the tab away from the GUI.
//!
//! - One Unix domain socket per user, at `<state_dir>/fasttyd.sock`.
//! - Newline-delimited JSON control messages (see `docs/daemon-protocol.md`).
//! - The registry is in-memory only and owned by the local session-host
//!   process. It survives GUI exit, and ends when the host or computer stops.
//! - Windows has no Unix domain sockets in the same shape; `start()` is a
//!   no-op there for now rather than half-supporting named pipes.
//!
//! The host owns PTYs. A GUI pane attaches to a host session and renders its
//! output through a local terminal projection. Closing the GUI detaches that
//! projection. It does not stop the session. Use the explicit close-session
//! request to stop a host session.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::pane_tree::PaneId;
use crate::process_state::{ProcessObservation, ProcessTable, ProcessUnavailableReason, SessionProcessState};
use crate::terminal_state::TerminalState;

/// Protocol generation used by session hosts and clients. The session-host
/// handshake uses capabilities for command support across generations.
pub const PROTOCOL_VERSION: u32 = 2;
pub const EVENT_API_VERSION: u16 = 1;
const MAX_CAPTURE_BYTES: usize = 1_048_576;
const EVENT_JOURNAL_CAPACITY: usize = 4096;

pub(crate) fn session_host_capabilities() -> Vec<String> {
    [
        "close_if_idle",
        "close_if_idle_batch",
        "binary_snapshot_attach",
        "attached_process_running",
        "spawn_request_status",
        "terminal_state_snapshot_v1",
        "api_discovery_v1",
        "session_identity_v1",
        "host_status_v1",
        "safe_host_restart_v1",
        "process_events_v1",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct HostInstanceId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionIdentity {
    pub host_instance: HostInstanceId,
    pub session_id: PaneId,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HostStatus {
    pub host_instance: HostInstanceId,
    pub protocol_version: u32,
    pub fastty_version: String,
    pub capabilities: Vec<String>,
    pub sessions: usize,
    pub connections: usize,
    pub restart_available: bool,
}

struct HostLifecycle {
    instance: HostInstanceId,
    next_id: usize,
    reserved_until: usize,
    connections: usize,
    stopping: bool,
}

fn instance_identity() -> &'static HostInstanceId {
    static INSTANCE: OnceLock<HostInstanceId> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("host identity requires system randomness");
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let hex = hex::encode(bytes);
        HostInstanceId(format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]))
    })
}

impl HostLifecycle {
    fn restart_if_empty(&mut self, expected: &HostInstanceId, sessions: usize) -> Result<(), &'static str> {
        if expected != &self.instance { return Err("host_instance_changed"); }
        if sessions != 0 || self.connections > 1 || self.stopping { return Err("host_restart_blocked"); }
        self.stopping = true;
        Ok(())
    }
}

fn host_lifecycle() -> &'static Mutex<HostLifecycle> {
    static HOST: OnceLock<Mutex<HostLifecycle>> = OnceLock::new();
    HOST.get_or_init(|| {
        Mutex::new(HostLifecycle {
            instance: instance_identity().clone(),
            next_id: 0,
            reserved_until: 0,
            connections: 0,
            stopping: false,
        })
    })
}

pub fn host_instance_id() -> HostInstanceId {
    instance_identity().clone()
}

fn reserve_session_ids(host: &mut HostLifecycle) -> Result<(), String> {
    reserve_session_ids_in(host, &crate::paths::get().state_dir)
}

fn reserve_session_ids_in(host: &mut HostLifecycle, directory: &std::path::Path) -> Result<(), String> {
    use std::io::{Read, Write};
    let path = directory.join("host-session-high-water");
    let high_water = match std::fs::File::open(&path) {
        Ok(file) => {
            let mut value = String::new();
            file.take(33).read_to_string(&mut value).map_err(|error| error.to_string())?;
            if value.len() > 32 { return Err("invalid host session high-water mark".into()); }
            value.trim().parse::<usize>().map_err(|_| "invalid host session high-water mark".to_string())?
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 100,
        Err(error) => return Err(error.to_string()),
    };
    let start = high_water.max(100);
    let end = start.checked_add(1024).ok_or_else(|| "host session IDs exhausted".to_string())?;
    let pending = directory.join("host-session-high-water.pending");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&pending).map_err(|error| error.to_string())?;
    writeln!(file, "{end}").map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    std::fs::rename(&pending, &path).map_err(|error| error.to_string())?;
    std::fs::File::open(directory).and_then(|file| file.sync_all()).map_err(|error| error.to_string())?;
    host.next_id = start;
    host.reserved_until = end;
    Ok(())
}

fn allocate_session_id(host: &mut HostLifecycle) -> Result<PaneId, String> {
    if host.next_id == host.reserved_until { reserve_session_ids(host)?; }
    let id = host.next_id;
    host.next_id += 1;
    Ok(id)
}

const MAX_EVENT_PROGRAM_STATUSES: usize = 32;
const MAX_EVENT_TEXT_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Gui,
    Headless,
}

#[derive(Clone)]
pub struct SessionEntry {
    pub terminal: Arc<TerminalState>,
    pub kind: SessionKind,
    output_receivers: Arc<Mutex<Vec<async_channel::Receiver<Vec<u8>>>>>,
}

type Registry = Mutex<HashMap<PaneId, SessionEntry>>;

struct EventJournal {
    sequence: u64,
    events: VecDeque<EventEnvelope>,
    latest_sessions: HashMap<PaneId, SessionInfo>,
    latest_processes: HashMap<PaneId, SessionProcessState>,
}

struct EventBatch {
    oldest_sequence: Option<u64>,
    events: Vec<EventEnvelope>,
}

impl EventJournal {
    fn new() -> Self {
        Self {
            sequence: 0,
            events: VecDeque::with_capacity(EVENT_JOURNAL_CAPACITY),
            latest_sessions: HashMap::new(),
            latest_processes: HashMap::new(),
        }
    }

    fn publish(&mut self, session_id: Option<PaneId>, kind: SessionEvent) -> EventEnvelope {
        self.sequence = self.sequence.saturating_add(1);
        let event = EventEnvelope {
            version: EVENT_API_VERSION,
            sequence: self.sequence,
            timestamp_unix_ms: unix_time_millis(),
            session_id,
            kind,
        };
        self.events.push_back(event.clone());
        if self.events.len() > EVENT_JOURNAL_CAPACITY {
            self.events.pop_front();
        }
        event
    }

    fn added(&mut self, session: SessionInfo) {
        self.latest_sessions.insert(session.id, session.clone());
        self.latest_processes.insert(session.id, SessionProcessState {
            session: SessionIdentity { host_instance: host_instance_id(), session_id: session.id },
            generation: 0,
            observation: ProcessObservation::Unavailable { reason: ProcessUnavailableReason::PendingObservation },
        });
        self.publish(Some(session.id), SessionEvent::SessionAdded { session });
    }

    fn removed(&mut self, id: PaneId) {
        self.latest_sessions.remove(&id);
        self.latest_processes.remove(&id);
        self.publish(Some(id), SessionEvent::SessionRemoved { id });
    }

    fn update_if_changed(&mut self, session: SessionInfo) {
        let Some(previous) = self.latest_sessions.get(&session.id) else {
            return;
        };
        if previous == &session {
            return;
        }
        let terminal_changed = previous.title != session.title
            || previous.cwd != session.cwd
            || previous.alive != session.alive
            || previous.program_status != session.program_status;
        self.latest_sessions.insert(session.id, session.clone());
        self.publish(
            Some(session.id),
            SessionEvent::SessionUpdated {
                session,
                terminal_state_changed: terminal_changed,
            },
        );
    }

    fn seed(&mut self, sessions: &[SessionInfo]) {
        self.latest_sessions = sessions
            .iter()
            .cloned()
            .map(|session| (session.id, session))
            .collect();
    }

    fn batch_after(&self, sequence: u64) -> EventBatch {
        EventBatch {
            oldest_sequence: self.events.front().map(|event| event.sequence),
            events: self
                .events
                .iter()
                .filter(|event| event.sequence > sequence)
                .cloned()
                .collect(),
        }
    }

    fn process_changed(&mut self, id: PaneId, observation: ProcessObservation) {
        let Some(previous) = self.latest_processes.get_mut(&id) else { return; };
        if previous.observation == observation { return; }
        previous.generation = previous.generation.saturating_add(1);
        previous.observation = observation;
        let state = previous.clone();
        self.publish(Some(id), SessionEvent::ProcessChanged { state });
    }

    fn process_snapshot(&self) -> Vec<SessionProcessState> {
        let mut states: Vec<_> = self.latest_processes.values().cloned().collect();
        states.sort_by_key(|state| state.session.session_id);
        states
    }

    fn snapshot(&self) -> Vec<SessionInfo> {
        self.latest_sessions.values().cloned().collect()
    }
}

fn event_is_visible(event: &SessionEvent, process_events: bool) -> bool {
    process_events || !matches!(event, SessionEvent::ProcessChanged { .. })
}

fn event_journal() -> &'static Mutex<EventJournal> {
    static JOURNAL: OnceLock<Mutex<EventJournal>> = OnceLock::new();
    JOURNAL.get_or_init(|| Mutex::new(EventJournal::new()))
}

fn start_process_sampler() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::spawn(|| {
            let mut table = ProcessTable::new();
            loop {
                if host_lifecycle().lock().stopping { return; }
                let terminals: Vec<_> = registry().lock().iter()
                    .map(|(id, entry)| (*id, Arc::clone(&entry.terminal))).collect();
                if !terminals.is_empty() {
                    table.refresh();
                    for (id, terminal) in terminals {
                        let (shell, foreground) = terminal.process_coordinates();
                        let observation = table.observe(shell, foreground);
                        let mut journal = event_journal().lock();
                        let still_registered = registry().lock().get(&id)
                            .is_some_and(|entry| Arc::ptr_eq(&entry.terminal, &terminal));
                        if still_registered { journal.process_changed(id, observation); }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(350));
            }
        });
    });
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn spawn_requests() -> &'static Mutex<HashMap<String, PaneId>> {
    static REQUESTS: OnceLock<Mutex<HashMap<String, PaneId>>> = OnceLock::new();
    REQUESTS.get_or_init(Default::default)
}

/// Called when a pane's `TerminalState` is created, so it becomes visible to
/// `fastty sessions` / `fastty attach`.
pub fn register(id: PaneId, terminal: Arc<TerminalState>) {
    let mut journal = event_journal().lock();
    let previous = registry().lock().insert(
        id,
        SessionEntry {
            terminal: Arc::clone(&terminal),
            kind: SessionKind::Gui,
            output_receivers: Arc::new(Mutex::new(Vec::new())),
        },
    );
    if let Some(previous) = previous {
        close_output_receivers(&previous);
    }
    journal.added(session_info(id, &terminal));
}

/// Called when a headless session is created via IPC `spawn`.
pub fn register_headless(id: PaneId, terminal: Arc<TerminalState>) {
    let mut journal = event_journal().lock();
    let previous = registry().lock().insert(
        id,
        SessionEntry {
            terminal: Arc::clone(&terminal),
            kind: SessionKind::Headless,
            output_receivers: Arc::new(Mutex::new(Vec::new())),
        },
    );
    if let Some(previous) = previous {
        close_output_receivers(&previous);
    }
    journal.added(session_info(id, &terminal));
}

/// Called when a pane is closed in the GUI or terminated, so external clients stop seeing
/// it in `fastty sessions`.
pub fn unregister(id: PaneId) {
    let mut journal = event_journal().lock();
    if let Some(entry) = registry().lock().remove(&id) {
        close_output_receivers(&entry);
        journal.removed(id);
    }
}

fn close_output_receivers(entry: &SessionEntry) {
    for receiver in entry.output_receivers.lock().drain(..) {
        receiver.close();
    }
}

fn is_registered(pane_id: PaneId) -> bool {
    registry().lock().contains_key(&pane_id)
}

/// Look up a registered terminal by its id, used by the GUI to access
/// headless sessions spawned via IPC.
pub fn get_session(id: PaneId) -> Option<Arc<TerminalState>> {
    registry().lock().get(&id).map(|e| Arc::clone(&e.terminal))
}

/// List all active session IDs currently registered in the daemon.
pub fn list_session_ids() -> Vec<PaneId> {
    registry().lock().keys().copied().collect()
}

/// A request from the daemon to the in-process GUI. `OpenSession` is
/// fire-and-forget; the pane-control requests carry a reply channel so the
/// daemon's client can get the result (new pane id, layout snapshot, ...).
pub enum GuiRequest {
    OpenSession {
        id: PaneId,
        terminal: Arc<TerminalState>,
    },
    SplitPane {
        target: PaneId,
        direction: crate::pane_tree::Direction,
        command: Option<String>,
        args: Vec<String>,
        cwd: Option<String>,
        reply: std::sync::mpsc::Sender<Result<PaneId, String>>,
    },
    ResizePane {
        target: PaneId,
        direction: crate::pane_tree::Direction,
        delta: f32,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    FocusPane {
        target: PaneId,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    Layout {
        reply: std::sync::mpsc::Sender<Result<Vec<TabLayout>, String>>,
    },
    ResizeWindow {
        cols: usize,
        rows: usize,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    ExternalApproval {
        request_id: u64,
        session_id: PaneId,
        action: ExternalAction,
        reply: std::sync::mpsc::Sender<bool>,
    },
    CancelExternalApproval {
        request_id: u64,
    },
}

type GuiSender = async_channel::Sender<GuiRequest>;

static GUI_SENDER: OnceLock<Mutex<Option<GuiSender>>> = OnceLock::new();
static NEXT_APPROVAL_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);
const EXTERNAL_APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Called by the GUI (`RootView`) at startup so the daemon can ask it to
/// surface sessions; passing `None` detaches (e.g. when the view goes away).
/// Sends after detach fail silently, which is the desired behavior: with no
/// GUI there is nothing to show and the session stays headless.
pub fn set_gui_sender(sender: Option<GuiSender>) {
    *GUI_SENDER.get_or_init(Default::default).lock() = sender;
}

fn notify_gui_open(id: PaneId, terminal: Arc<TerminalState>) -> bool {
    if let Some(sender) = GUI_SENDER.get_or_init(Default::default).lock().as_ref() {
        sender
            .send_blocking(GuiRequest::OpenSession { id, terminal })
            .is_ok()
    } else {
        false
    }
}

/// Send a pane-control request to the GUI and block for its reply.
/// Connection handlers run on their own thread, so waiting here (bounded)
/// is safe. Fails fast when no GUI is attached to this daemon.
fn call_gui<T>(
    build: impl FnOnce(std::sync::mpsc::Sender<Result<T, String>>) -> GuiRequest,
) -> Result<T, String> {
    let Some(sender) = GUI_SENDER.get_or_init(Default::default).lock().clone() else {
        return Err("no fastty window is running".to_string());
    };
    let (reply_tx, reply_rx) = std::sync::mpsc::channel::<Result<T, String>>();
    sender
        .send_blocking(build(reply_tx))
        .map_err(|_| "fastty window is not responding".to_string())?;
    match reply_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(message)) => Err(message),
        Err(_) => Err("timed out waiting for the fastty window".to_string()),
    }
}

fn request_external_approval(
    session_id: PaneId,
    action: ExternalAction,
) -> Result<(), String> {
    request_external_approval_with_sender(
        GUI_SENDER
        .get_or_init(Default::default)
        .lock()
        .clone(),
        session_id,
        action,
        EXTERNAL_APPROVAL_TIMEOUT,
    )
}

fn request_external_approval_with_sender(
    sender: Option<GuiSender>,
    session_id: PaneId,
    action: ExternalAction,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let sender = sender
        .ok_or_else(|| "approval_unavailable: no Fastty window is running".to_string())?;
    let request_id = NEXT_APPROVAL_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    sender
        .send_blocking(GuiRequest::ExternalApproval {
            request_id,
            session_id,
            action,
            reply: reply_tx,
        })
        .map_err(|_| "approval_unavailable: Fastty window is not responding".to_string())?;
    match reply_rx.recv_timeout(timeout) {
        Ok(true) => Ok(()),
        Ok(false) => Err("approval_denied: the user denied this action".to_string()),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            let _ = sender.send_blocking(GuiRequest::CancelExternalApproval { request_id });
            Err("approval_timeout: no decision arrived within 60 seconds".to_string())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let _ = sender.send_blocking(GuiRequest::CancelExternalApproval { request_id });
            Err("approval_unavailable: the approval window closed".to_string())
        }
    }
}

/// Error code for a failed GUI-side pane operation, from `call_gui`'s
/// error text (kept intentionally coarse: the message carries the detail).
fn gui_error_code(message: &str) -> &'static str {
    if message.contains("no fastty window") {
        "no_gui"
    } else if message.contains("timed out") || message.contains("not responding") {
        "gui_timeout"
    } else {
        "gui_error"
    }
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
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Hello {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        read_only: bool,
        /// Protocol version this client speaks. Absent means the client
        /// predates version negotiation, so the host answers with the oldest
        /// shape it still supports.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        protocol_version: Option<u32>,
    },
    List,
    ApiList,
    ApiDescribe { method: String },
    HostStatus,
    RestartHost { expected_host_instance: HostInstanceId },
    /// Check whether a session ID is registered without creating a default session.
    SessionExists {
        id: PaneId,
    },
    /// Get the live process state for a registered terminal session.
    SessionStatus {
        id: PaneId,
    },
    /// Resolve a possibly lost `spawn` response by its client request ID.
    SpawnStatus {
        request_id: String,
    },
    CloseIfIdle {
        id: PaneId,
    },
    CloseIfIdleBatch {
        ids: Vec<PaneId>,
    },
    /// Push-updated session list: an initial `sessions` event with everything
    /// currently registered, followed by `session_added` / `session_removed`
    /// / `session_updated` as the registry changes, for as long as this
    /// connection stays open. No explicit unsubscribe in v1 -- a client that
    /// wants this typically dedicates one connection to it.
    SubscribeSessions,
    /// Start a typed JSONL event stream with a session snapshot and deltas.
    SubscribeEvents {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        process_events: bool,
    },
    /// Capture the visible terminal screen as ANSI text.
    Capture {
        id: PaneId,
    },
    /// Lower this connection's access scope for its remaining lifetime.
    SetAccessScope {
        scope: AccessScope,
    },
    Attach {
        id: PaneId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_host_instance: Option<HostInstanceId>,
        #[serde(default)]
        mode: AttachMode,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        binary_snapshot: bool,
        #[serde(default, skip_serializing_if = "SnapshotFormat::is_legacy")]
        snapshot_format: SnapshotFormat,
    },
    Detach {
        id: PaneId,
    },
    Write {
        id: PaneId,
        data: String,
        /// Opt-in ack: when true the daemon replies `done` after the write
        /// lands (or `error`). Older clients omit it and keep the silent
        /// streaming behavior.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        ack: bool,
    },
    /// Ask the visible Fastty window to approve an external session action.
    /// This command fails closed when no GUI can receive the request.
    ExternalAction {
        id: PaneId,
        action: ExternalAction,
    },
    Resize {
        id: PaneId,
        cols: usize,
        rows: usize,
        /// Opt-in ack, same semantics as on `write`.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        ack: bool,
    },
    Spawn {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
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
        /// Open the new session as a visible tab in the running fastty GUI
        /// (silently ignored when no GUI is attached to this daemon).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        open: bool,
    },
    Close {
        id: PaneId,
        /// Also close GUI-owned tabs: their running process is killed, the
        /// same as closing the tab in the window. Without `force` a GUI
        /// session answers `not_closable` — the tab belongs to the user.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        force: bool,
    },
    /// Split a visible GUI pane: create a new pane next to it (same tab)
    /// running a shell (or `command`), and return the new pane's session id.
    SplitPane {
        id: PaneId,
        /// `"left" | "right" | "top" | "down"` — which side of the target.
        direction: String,
        #[serde(default)]
        command: Option<String>,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
    },
    /// Move the divider on one side of a visible GUI pane, growing or
    /// shrinking it inside its tab.
    ResizePane {
        id: PaneId,
        direction: String,
        /// Fraction of the split axis to move the divider by (e.g. 0.05).
        #[serde(default = "default_resize_delta")]
        delta: f32,
    },
    /// Bring a pane's tab to the front and make the pane active.
    FocusPane {
        id: PaneId,
    },
    /// Full GUI layout: every tab with its panes, so a client can see which
    /// sessions share a tab before splitting/focusing. Headless sessions
    /// (spawned without a tab) are not part of any tab — see `list`.
    Layout,
    /// Best-effort resize of the fastty window so the (single-pane) active
    /// tab renders at `cols` x `rows`. With splits the result is
    /// approximate; use `resize_pane` to move dividers.
    ResizeWindow {
        cols: usize,
        rows: usize,
    },
    ReportProgramStatus {
        source_id: u64,
        payload: String,
    },
    /// Fast microsecond binary snapshot of terminal state (magic b"FST1").
    BinarySnapshot {
        id: PaneId,
        #[serde(default)]
        screen_only: bool,
    },
}

impl Request {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "hello",
            Self::List => "list",
            Self::ApiList => "api_list",
            Self::ApiDescribe { .. } => "api_describe",
            Self::HostStatus => "host_status",
            Self::RestartHost { .. } => "restart_host",
            Self::SessionExists { .. } => "session_exists",
            Self::SessionStatus { .. } => "session_status",
            Self::SpawnStatus { .. } => "spawn_status",
            Self::CloseIfIdle { .. } => "close_if_idle",
            Self::CloseIfIdleBatch { .. } => "close_if_idle_batch",
            Self::SubscribeSessions => "subscribe_sessions",
            Self::SubscribeEvents { .. } => "subscribe_events",
            Self::Capture { .. } => "capture",
            Self::SetAccessScope { .. } => "set_access_scope",
            Self::Attach { .. } => "attach",
            Self::Detach { .. } => "detach",
            Self::Write { .. } => "write",
            Self::ExternalAction { .. } => "external_action",
            Self::Resize { .. } => "resize",
            Self::Spawn { .. } => "spawn",
            Self::Close { .. } => "close",
            Self::SplitPane { .. } => "split_pane",
            Self::ResizePane { .. } => "resize_pane",
            Self::FocusPane { .. } => "focus_pane",
            Self::Layout => "layout",
            Self::ResizeWindow { .. } => "resize_window",
            Self::ReportProgramStatus { .. } => "report_program_status",
            Self::BinarySnapshot { .. } => "binary_snapshot",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ExternalAction {
    Send { text: String },
    Close { force: bool },
}

impl ExternalAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Send { .. } => "Send input",
            Self::Close { .. } => "Close session",
        }
    }

    pub fn display_detail(&self) -> String {
        match self {
            Self::Send { text } => format!("Input: {:?}", text),
            Self::Close { .. } => "Terminate the process and close this session".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotFormat {
    #[default]
    Legacy,
    TerminalState,
}

impl SnapshotFormat {
    fn is_legacy(&self) -> bool {
        *self == Self::Legacy
    }
}

/// Whether an `attach` is allowed to `write` to the session it attached to,
/// on the connection that attached. Omitting `mode` in an `attach` request
/// defaults to `read_write`, matching the original (mode-less) behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttachMode {
    #[default]
    ReadWrite,
    ReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessScope {
    Observe,
    Interact,
    Manage,
}

impl AccessScope {
    fn permits(self, required: Self) -> bool {
        self.rank() >= required.rank()
    }

    fn rank(self) -> u8 {
        match self {
            Self::Observe => 0,
            Self::Interact => 1,
            Self::Manage => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionInfo {
    pub id: PaneId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<SessionIdentity>,
    pub title: String,
    pub cwd: Option<String>,
    pub cols: usize,
    pub rows: usize,
    pub alive: bool,
    #[serde(default)]
    pub history_size: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub program_status: Vec<ProgramStatusInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProgramStatusInfo {
    pub id: String,
    pub state: crate::program_status::ProgramState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<crate::program_status::BlockedKind>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EventEnvelope {
    pub version: u16,
    pub sequence: u64,
    pub timestamp_unix_ms: u64,
    pub session_id: Option<PaneId>,
    pub kind: SessionEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    ProcessChanged { state: SessionProcessState },
    SessionAdded { session: SessionInfo },
    SessionRemoved { id: PaneId },
    SessionUpdated {
        session: SessionInfo,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        terminal_state_changed: bool,
    },
    /// A session event this build does not know. Never sent.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

impl SessionEvent {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::ProcessChanged { .. } => "process_changed",
            Self::SessionAdded { .. } => "session_added",
            Self::SessionRemoved { .. } => "session_removed",
            Self::SessionUpdated { .. } => "session_updated",
            Self::Unknown => "unknown",
        }
    }
}

/// One pane inside a tab, as reported by `layout`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneLayoutInfo {
    pub id: PaneId,
    pub title: String,
    pub cwd: Option<String>,
    pub active: bool,
    pub cols: usize,
    pub rows: usize,
}

/// One tab of the GUI layout, as reported by `layout`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TabLayout {
    pub tab_id: usize,
    pub title: String,
    pub active: bool,
    pub active_pane: PaneId,
    pub zoomed_pane: Option<PaneId>,
    pub panes: Vec<PaneLayoutInfo>,
}

/// Parses the protocol's direction strings ("left"/"right"/"top"/"down").
fn parse_direction(s: &str) -> Option<crate::pane_tree::Direction> {
    match s {
        "left" => Some(crate::pane_tree::Direction::Left),
        "right" => Some(crate::pane_tree::Direction::Right),
        "top" => Some(crate::pane_tree::Direction::Top),
        "down" => Some(crate::pane_tree::Direction::Down),
        _ => None,
    }
}

fn default_resize_delta() -> f32 {
    0.05
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BlockedSession {
    pub id: PaneId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_process_label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Response {
    ApiList { api: crate::server_api::ApiCatalog },
    ApiDescribe { api: crate::server_api::ApiMethodDescription },
    HostStatus { status: HostStatus },
    HostRestarting,
    Hello {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host_instance: Option<HostInstanceId>,
        version: u32,
        fastty_version: String,
        #[serde(default)]
        capabilities: Vec<String>,
    },
    /// Full session list -- the reply to `list`, and also the first thing
    /// sent after `subscribe_sessions` (as a baseline before any
    /// `session_added`/`session_removed`/`session_updated` deltas).
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    ProcessStates { states: Vec<SessionProcessState> },
    EventStreamStarted {
        version: u16,
        sequence: u64,
        sessions: Vec<SessionInfo>,
    },
    /// The bounded event journal no longer contains every event after the
    /// subscriber's cursor. Replace local state with this snapshot, then
    /// resume at `latest_sequence`.
    EventStreamGap {
        version: u16,
        after_sequence: u64,
        oldest_sequence: u64,
        latest_sequence: u64,
        sessions: Vec<SessionInfo>,
    },
    StreamEvent {
        envelope: EventEnvelope,
    },
    Captured {
        id: PaneId,
        data: String,
        truncated: bool,
    },
    SessionExists {
        exists: bool,
    },
    SessionStatus {
        alive: bool,
        process_running: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        foreground_process_label: Option<String>,
    },
    CloseBlocked {
        id: PaneId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        foreground_process_label: Option<String>,
    },
    CloseBatch {
        closed_ids: Vec<PaneId>,
        blocked: Vec<BlockedSession>,
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
        #[serde(default)]
        mode: AttachMode,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        program_status: Vec<crate::program_status::ProgramRecord>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_running: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell_pid: Option<u32>,
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
    /// Raw microsecond binary snapshot (base64-encoded bytes with magic b"FST1").
    BinarySnapshot {
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
        /// Whether `open: true` reached a running GUI (false when no GUI is
        /// attached to this daemon, or when the spawn didn't ask to open).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        opened: bool,
    },
    SpawnStatus {
        id: Option<PaneId>,
    },
    /// The reply to `split_pane`: the new pane's session id.
    PaneSplit {
        id: PaneId,
    },
    /// The reply to `layout`.
    Layout {
        tabs: Vec<TabLayout>,
    },
    /// Ack for `write` / `resize` requests that opted in with `ack: true`.
    /// `cmd` echoes the request name (`"write"` / `"resize"`). Clients that
    /// never set `ack` never receive this event.
    Done {
        cmd: String,
    },
    Error {
        /// Stable, machine-matchable reason: `"bad_request"`,
        /// `"no_such_session"`, `"invalid_base64"`, `"not_attached"`,
        /// `"read_only"`, `"spawn_failed"`, `"not_closable"`, or
        /// `"unsupported"`. New codes may be added over
        /// time; treat an unrecognized one the same as a generic failure.
        code: String,
        message: String,
    },
    /// An event this build does not know. Parsed from a newer host or client,
    /// never sent. Callers must ignore it and keep reading.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

impl Response {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::ApiList { .. } => "api_list",
            Self::ApiDescribe { .. } => "api_describe",
            Self::HostStatus { .. } => "host_status",
            Self::HostRestarting => "host_restarting",
            Self::Hello { .. } => "hello",
            Self::Sessions { .. } => "sessions",
            Self::ProcessStates { .. } => "process_states",
            Self::EventStreamStarted { .. } => "event_stream_started",
            Self::EventStreamGap { .. } => "event_stream_gap",
            Self::StreamEvent { .. } => "stream_event",
            Self::Captured { .. } => "captured",
            Self::SessionExists { .. } => "session_exists",
            Self::SessionStatus { .. } => "session_status",
            Self::CloseBlocked { .. } => "close_blocked",
            Self::CloseBatch { .. } => "close_batch",
            Self::SessionAdded { .. } => "session_added",
            Self::SessionRemoved { .. } => "session_removed",
            Self::SessionUpdated { .. } => "session_updated",
            Self::Attached { .. } => "attached",
            Self::Snapshot { .. } => "snapshot",
            Self::BinarySnapshot { .. } => "binary_snapshot",
            Self::Detached { .. } => "detached",
            Self::Closed { .. } => "closed",
            Self::Output { .. } => "output",
            Self::Spawned { .. } => "spawned",
            Self::SpawnStatus { .. } => "spawn_status",
            Self::PaneSplit { .. } => "pane_split",
            Self::Layout { .. } => "layout",
            Self::Done { .. } => "done",
            Self::Error { .. } => "error",
            Self::Unknown => "unknown",
        }
    }
}

pub fn spawn_headless_session(
    cmd: Option<&str>,
    args: &[String],
    cwd: Option<&str>,
    cols: Option<usize>,
    rows: Option<usize>,
) -> Result<PaneId, String> {
    let mut lifecycle = host_lifecycle().lock();
    if lifecycle.stopping { return Err("host is stopping".into()); }
    let mut id = allocate_session_id(&mut lifecycle)?;
    while is_registered(id) { id = allocate_session_id(&mut lifecycle)?; }

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

    register_headless(id, Arc::new(terminal));
    drop(lifecycle);
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
        identity: Some(SessionIdentity { host_instance: host_instance_id(), session_id: id }),
        title: bounded_event_text(
            &terminal
                .get_foreground_process_name()
                .unwrap_or_else(|| "shell".to_string()),
        ),
        cwd: terminal.get_current_working_directory().map(|path| {
            bounded_event_text(&path.to_string_lossy())
        }),
        cols,
        rows,
        alive: terminal.is_alive(),
        history_size: terminal.history_size(),
        program_status: terminal
            .program_status_snapshot()
            .into_iter()
            .take(MAX_EVENT_PROGRAM_STATUSES)
            .map(|record| ProgramStatusInfo {
                id: bounded_event_text(&record.id),
                state: record.state,
                app: record.app.map(|app| bounded_event_text(&app)),
                progress: record.progress,
                kind: record.kind,
            })
            .collect(),
    }
}

fn bounded_event_text(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_EVENT_TEXT_BYTES)
        .collect();
    sanitized
}

fn unix_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
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

/// Run the local host accept loop in this process.
///
/// Unlike `start`, this returns if another process already owns the socket.
pub fn run_host() {
    #[cfg(unix)]
    unix_impl::run();
}

#[cfg(unix)]
mod unix_impl {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::Duration;

    pub fn run() {
        let path = socket_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Some(_instance_lock) = acquire_instance_lock() else {
            return;
        };
        if UnixStream::connect(&path).is_ok() {
            return;
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            use std::os::unix::fs::FileTypeExt;
            if !metadata.file_type().is_socket() || std::fs::remove_file(&path).is_err() {
                return;
            }
        }
        if reserve_session_ids(&mut host_lifecycle().lock()).is_err() { return; }
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

        if listener.set_nonblocking(true).is_err() { return; }
        start_process_sampler();
        loop {
            let mut lifecycle = host_lifecycle().lock();
            if lifecycle.stopping { break; }
            match listener.accept() {
                Ok((stream, _)) => {
                    if stream.set_nonblocking(false).is_err() { continue; }
                    lifecycle.connections += 1;
                    std::thread::spawn(move || {
                        handle_connection(stream);
                        host_lifecycle().lock().connections -= 1;
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
            drop(lifecycle);
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_file(&path);
    }

    fn acquire_instance_lock() -> Option<std::fs::File> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::PermissionsExt;

        let lock_path = crate::paths::get().state_dir.join("fasttyd.lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(lock_path)
            .ok()?;
        let mut permissions = lock.metadata().ok()?.permissions();
        permissions.set_mode(0o600);
        lock.set_permissions(permissions).ok()?;
        let result = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        (result == 0).then_some(lock)
    }

    /// One currently-attached session on a connection: its stop-flag (so
    /// `Detach{id}` can stop just this attach thread) and whether `write` to
    /// this `id` is allowed on this connection.
    pub(super) struct Attachment {
        pub(super) alive: Arc<AtomicBool>,
        pub(super) output: async_channel::Receiver<Vec<u8>>,
        pub(super) read_only: bool,
    }

    pub(super) struct Conn {
        pub(super) out: Arc<Mutex<PeerStream>>,
        pub(super) conn_alive: Arc<AtomicBool>,
        pub(super) read_only: bool,
        pub(super) access_scope: AccessScope,
        pub(super) attachments: HashMap<PaneId, Attachment>,
    }

    /// The write half of a connection plus the protocol version its peer
    /// announced in `hello`. Threads cloned for `attach` keep the version.
    pub(super) struct PeerStream {
        stream: UnixStream,
        protocol: AtomicU32,
    }

    impl PeerStream {
        pub(super) fn new(stream: UnixStream) -> Self {
            Self {
                stream,
                protocol: AtomicU32::new(0),
            }
        }

        /// `None` until the peer sends `hello`.
        pub(super) fn announced(&self) -> Option<u32> {
            match self.protocol.load(Ordering::Relaxed) {
                0 => None,
                version => Some(version),
            }
        }

        fn announce(&self, version: Option<u32>) {
            let version = version.unwrap_or(1).clamp(1, PROTOCOL_VERSION);
            self.protocol.store(version, Ordering::Relaxed);
        }

        fn encode(&self, response: &Response) -> Option<String> {
            crate::protocol_compat::encode_response(response, self.announced())
        }
    }

    fn handle_connection(stream: UnixStream) {
        let write_half = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut conn = Conn {
            out: Arc::new(Mutex::new(PeerStream::new(write_half))),
            conn_alive: Arc::new(AtomicBool::new(true)),
            read_only: false,
            access_scope: AccessScope::Manage,
            attachments: HashMap::new(),
        };

        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let peer = conn.out.lock().announced();
            let request = match crate::protocol_compat::parse_request(&line, peer) {
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
            if host_lifecycle().lock().stopping { break; }
        }
        conn.conn_alive.store(false, Ordering::Relaxed);
        for attachment in conn.attachments.values() {
            attachment.alive.store(false, Ordering::Relaxed);
            attachment.output.close();
        }
    }

    pub(super) fn handle_request(request: Request, conn: &mut Conn) {
        match request {
            Request::ApiList => { match crate::server_api::api_list() {
                Ok(api) => send(&conn.out, &Response::ApiList { api }),
                Err(message) => send(&conn.out, &Response::Error { code: "api_schema_error".into(), message }),
            }; },
            Request::ApiDescribe { method } => { match crate::server_api::api_describe(&method) {
                Ok(api) => send(&conn.out, &Response::ApiDescribe { api }),
                Err(message) => send(&conn.out, &Response::Error { code: "unknown_method".into(), message }),
            }; },
            Request::Hello { read_only, protocol_version } => {
                conn.out.lock().announce(protocol_version);
                if read_only {
                    conn.read_only = true;
                    conn.access_scope = AccessScope::Observe;
                }
                send(
                    &conn.out,
                    &Response::Hello {
                        host_instance: Some(host_instance_id()),
                        version: PROTOCOL_VERSION,
                        fastty_version: env!("CARGO_PKG_VERSION").to_string(),
                        capabilities: session_host_capabilities(),
                    },
                );
            }
            Request::HostStatus => {
                let lifecycle = host_lifecycle().lock();
                let sessions = registry().lock().len();
                send(&conn.out, &Response::HostStatus { status: HostStatus {
                    host_instance: lifecycle.instance.clone(),
                    protocol_version: PROTOCOL_VERSION,
                    fastty_version: env!("CARGO_PKG_VERSION").into(),
                    capabilities: session_host_capabilities(),
                    sessions,
                    connections: lifecycle.connections.saturating_sub(1),
                    restart_available: sessions == 0 && lifecycle.connections <= 1 && !lifecycle.stopping,
                }});
            }
            Request::RestartHost { expected_host_instance } => {
                if reject_access_scope(conn, AccessScope::Manage) { return; }
                let mut lifecycle = host_lifecycle().lock();
                let sessions = registry().lock().len();
                match lifecycle.restart_if_empty(&expected_host_instance, sessions) {
                    Ok(()) => send(&conn.out, &Response::HostRestarting),
                    Err(code) => send(&conn.out, &Response::Error {
                        code: code.into(),
                        message: if code == "host_restart_blocked" { "host has sessions or other connections" } else { "host instance changed" }.into(),
                    }),
                };
            }

            Request::SetAccessScope { scope } => {
                if scope.permits(conn.access_scope) && scope != conn.access_scope {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "permission_escalation".to_string(),
                            message: "a connection can only lower its access scope".to_string(),
                        },
                    );
                    return;
                }
                conn.access_scope = scope;
                send(
                    &conn.out,
                    &Response::Done {
                        cmd: "set_access_scope".to_string(),
                    },
                );
            }
            Request::List => {
                let _ = ensure_default_session();
                let sessions = registry()
                    .lock()
                    .iter()
                    .map(|(id, entry)| session_info(*id, &entry.terminal))
                    .collect();
                send(&conn.out, &Response::Sessions { sessions });
            }
            Request::SessionExists { id } => {
                let exists = registry().lock().contains_key(&id);
                send(&conn.out, &Response::SessionExists { exists });
            }
            Request::SessionStatus { id } => {
                let terminal = registry()
                    .lock()
                    .get(&id)
                    .map(|entry| Arc::clone(&entry.terminal));
                match terminal {
                    Some(terminal) => {
                        let alive = terminal.is_alive();
                        let process_running = terminal.is_process_running();
                        let foreground_process_label = if process_running {
                            terminal
                                .get_foreground_process_command()
                                .and_then(foreground_command_label)
                                .or_else(|| terminal.get_foreground_process_name())
                        } else {
                            None
                        };
                        send(
                            &conn.out,
                            &Response::SessionStatus {
                                alive,
                                process_running,
                                foreground_process_label,
                            },
                        );
                    }
                    None => {
                        send(
                            &conn.out,
                            &Response::Error {
                                code: "no_such_session".to_string(),
                                message: "no such session".to_string(),
                            },
                        );
                    }
                }
            }
            Request::SpawnStatus { request_id } => {
                let id = spawn_requests()
                    .lock()
                    .get(&request_id)
                    .copied()
                    .filter(|id| is_registered(*id));
                send(&conn.out, &Response::SpawnStatus { id });
            }
            Request::CloseIfIdle { id } => {
                if reject_access_scope(conn, AccessScope::Manage) {
                    return;
                }
                let entry = registry().lock().get(&id).cloned();
                match entry {
                    Some(entry) if entry.terminal.terminate_if_idle() => {
                        unregister(id);
                        send(&conn.out, &Response::Closed { id });
                    }
                    Some(entry) => {
                        let foreground_process_label = entry
                            .terminal
                            .get_foreground_process_command()
                            .and_then(foreground_command_label)
                            .or_else(|| entry.terminal.get_foreground_process_name());
                        send(
                            &conn.out,
                            &Response::CloseBlocked {
                                id,
                                foreground_process_label,
                            },
                        );
                    }
                    None => {
                        send(
                            &conn.out,
                            &Response::Error {
                                code: "no_such_session".to_string(),
                                message: "no such session".to_string(),
                            },
                        );
                    }
                }
            }
            Request::CloseIfIdleBatch { ids } => {
                if reject_access_scope(conn, AccessScope::Manage) {
                    return;
                }
                let ids: Vec<PaneId> = ids
                    .into_iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let entries: Vec<(PaneId, Arc<TerminalState>)> = {
                    let sessions = registry().lock();
                    ids.iter()
                        .filter_map(|id| {
                            sessions
                                .get(id)
                                .map(|entry| (*id, Arc::clone(&entry.terminal)))
                        })
                        .collect()
                };
                let terminals: Vec<Arc<TerminalState>> = entries
                    .iter()
                    .map(|(_, terminal)| Arc::clone(terminal))
                    .collect();
                let blocked_indices = TerminalState::terminate_all_if_idle(&terminals);
                if !blocked_indices.is_empty() {
                    let blocked = blocked_indices
                        .into_iter()
                        .filter_map(|index| entries.get(index))
                        .map(|(id, terminal)| BlockedSession {
                            id: *id,
                            foreground_process_label: terminal
                                .get_foreground_process_command()
                                .and_then(foreground_command_label)
                                .or_else(|| terminal.get_foreground_process_name()),
                        })
                        .collect();
                    send(
                        &conn.out,
                        &Response::CloseBatch {
                            closed_ids: Vec::new(),
                            blocked,
                        },
                    );
                    return;
                }
                for id in &ids {
                    unregister(*id);
                }
                send(
                    &conn.out,
                    &Response::CloseBatch {
                        closed_ids: ids,
                        blocked: Vec::new(),
                    },
                );
            }
            Request::Capture { id } => {
                let terminal = registry()
                    .lock()
                    .get(&id)
                    .map(|entry| Arc::clone(&entry.terminal));
                match terminal {
                    Some(terminal) => {
                        let (snapshot, truncated) =
                            terminal.snapshot_ansi_bounded(MAX_CAPTURE_BYTES);
                        send(
                            &conn.out,
                            &Response::Captured {
                                id,
                                data: String::from_utf8_lossy(&snapshot).into_owned(),
                                truncated,
                            },
                        );
                    }
                    None => {
                        send(
                            &conn.out,
                            &Response::Error {
                                code: "no_such_session".to_string(),
                                message: format!("no such session {id}"),
                            },
                        );
                    }
                }
            }
            Request::Attach {
                id,
                mode,
                binary_snapshot,
                snapshot_format,
                expected_host_instance,
            } => {
                if expected_host_instance.as_ref().is_some_and(|expected| expected != &host_instance_id()) {
                    send(&conn.out, &Response::Error { code: "host_instance_changed".into(), message: "the saved session belongs to another host instance".into() });
                    return;
                }
                let terminal = registry().lock().get(&id).map(|e| Arc::clone(&e.terminal));
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
                let effective_mode =
                    if conn.read_only || !conn.access_scope.permits(AccessScope::Interact) {
                    AttachMode::ReadOnly
                } else {
                    mode
                };
                let (cols, rows) = terminal.dimensions();
                let (snapshot, program_status, process_running, rx) = if snapshot_format == SnapshotFormat::TerminalState {
                    match terminal.snapshot_state_and_subscribe_output() {
                        Ok(snapshot) => snapshot,
                        Err(message) => {
                            send(&conn.out, &Response::Error {
                                code: "invalid_snapshot".into(),
                                message,
                            });
                            return;
                        }
                    }
                } else if binary_snapshot {
                    terminal.snapshot_binary_and_subscribe_output()
                } else {
                    terminal.snapshot_and_subscribe_output()
                };
                let registered = {
                    let sessions = registry().lock();
                    sessions.get(&id).is_some_and(|entry| {
                        if !Arc::ptr_eq(&entry.terminal, &terminal) {
                            return false;
                        }
                        let mut receivers = entry.output_receivers.lock();
                        receivers.retain(|receiver| !receiver.is_closed());
                        receivers.push(rx.clone());
                        true
                    })
                };
                if !registered {
                    rx.close();
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "no_such_session".to_string(),
                            message: "no such session".to_string(),
                        },
                    );
                    return;
                }
                send(
                    &conn.out,
                    &Response::Attached {
                        id,
                        cols,
                        rows,
                        mode: effective_mode,
                        program_status,
                        process_running: Some(process_running),
                        cwd: terminal
                            .get_current_working_directory()
                            .map(|path| path.to_string_lossy().into_owned()),
                        shell_pid: terminal.shell_pid(),
                    },
                );
                send(
                    &conn.out,
                    &Response::Snapshot {
                        id,
                        data: base64_encode(&snapshot),
                    },
                );

                // Replacing a stale entry (re-attaching to the same id on the
                // same connection) stops the old thread cleanly first.
                if let Some(old) = conn.attachments.remove(&id) {
                    old.alive.store(false, Ordering::Relaxed);
                    old.output.close();
                }
                let alive = Arc::new(AtomicBool::new(true));
                conn.attachments.insert(
                    id,
                    Attachment {
                        alive: Arc::clone(&alive),
                        output: rx.clone(),
                        read_only: effective_mode == AttachMode::ReadOnly,
                    },
                );

                let out = Arc::clone(&conn.out);
                let conn_alive = Arc::clone(&conn.conn_alive);
                std::thread::spawn(move || {
                    let should_run =
                        || conn_alive.load(Ordering::Relaxed) && alive.load(Ordering::Relaxed);
                    while should_run() {
                        match rx.recv_blocking() {
                            Ok(mut bytes) => {
                                if !should_run() {
                                    break;
                                }
                                // Coalesce consecutive waiting chunks to avoid NDJSON
                                // framing & IPC context-switch overhead during high throughput.
                                while let Ok(next) = rx.try_recv() {
                                    bytes.extend_from_slice(&next);
                                    if bytes.len() >= 65536 {
                                        break;
                                    }
                                }
                                if !should_run() {
                                    break;
                                }
                                let data = base64_encode(&bytes);
                                if !send(&out, &Response::Output { id, data }) {
                                    break;
                                }
                            }
                            Err(_) => {
                                if should_run() {
                                    if is_registered(id) {
                                        send(
                                            &out,
                                            &Response::Error {
                                                code: "output_overflow".to_string(),
                                                message: "terminal output stopped because the attached client could not keep up".to_string(),
                                            },
                                        );
                                    } else {
                                        send(&out, &Response::Closed { id });
                                    }
                                }
                                break;
                            }
                        }
                    }
                    rx.close();
                });
            }
            Request::Detach { id } => {
                if let Some(attachment) = conn.attachments.remove(&id) {
                    attachment.alive.store(false, Ordering::Relaxed);
                    attachment.output.close();
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
            Request::Write { id, data, ack } => {
                if reject_access_scope(conn, AccessScope::Interact) {
                    return;
                }
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
                if let Some(entry) = registry().lock().get(&id) {
                    entry.terminal.write_to_pty(&bytes);
                    if ack {
                        send(
                            &conn.out,
                            &Response::Done {
                                cmd: "write".to_string(),
                            },
                        );
                    }
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
            Request::ExternalAction { id, action } => {
                let required_scope = match &action {
                    ExternalAction::Send { .. } => AccessScope::Interact,
                    ExternalAction::Close { .. } => AccessScope::Manage,
                };
                if reject_access_scope(conn, required_scope) {
                    return;
                }
                let Some(entry) = registry().lock().get(&id).cloned() else {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "no_such_session".to_string(),
                            message: format!("no such session {id}"),
                        },
                    );
                    return;
                };
                if let ExternalAction::Send { text } = &action {
                    if text.len() > 65_536 {
                        send(
                            &conn.out,
                            &Response::Error {
                                code: "input_too_large".to_string(),
                                message: "approved input cannot exceed 65536 bytes".to_string(),
                            },
                        );
                        return;
                    }
                }
                if let Err(error) = request_external_approval(id, action.clone()) {
                    let (code, message) = error
                        .split_once(": ")
                        .unwrap_or(("approval_unavailable", error.as_str()));
                    send(
                        &conn.out,
                        &Response::Error {
                            code: code.to_string(),
                            message: message.to_string(),
                        },
                    );
                    return;
                }
                if !conn.conn_alive.load(Ordering::Relaxed) {
                    return;
                }
                let active_entry = registry().lock().get(&id).cloned();
                let Some(active_entry) = active_entry.filter(|active| {
                    Arc::ptr_eq(&active.terminal, &entry.terminal)
                }) else {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "session_changed".to_string(),
                            message: "session ended or changed before approval completed".to_string(),
                        },
                    );
                    return;
                };
                match action {
                    ExternalAction::Send { text } if active_entry.terminal.is_alive() => {
                        active_entry.terminal.write_to_pty(text.as_bytes());
                        send(
                            &conn.out,
                            &Response::Done {
                                cmd: "external_action".to_string(),
                            },
                        );
                    }
                    ExternalAction::Send { .. } => {
                        send(
                            &conn.out,
                            &Response::Error {
                                code: "session_ended".to_string(),
                                message: "session ended before the approved input was sent".to_string(),
                            },
                        );
                    }
                    ExternalAction::Close { force } => {
                        if active_entry.kind == SessionKind::Gui && !force {
                            send(
                                &conn.out,
                                &Response::Error {
                                    code: "not_closable".to_string(),
                                    message: "GUI session cannot be closed without force: true".to_string(),
                                },
                            );
                            return;
                        }
                        active_entry.terminal.terminate_process();
                        unregister(id);
                        send(&conn.out, &Response::Closed { id });
                    }
                }
            }
            Request::SubscribeSessions => {
                let _ = ensure_default_session();
                let initial: Vec<SessionInfo> = registry()
                    .lock()
                    .iter()
                    .map(|(id, entry)| session_info(*id, &entry.terminal))
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
                            .map(|(id, entry)| (*id, session_info(*id, &entry.terminal)))
                            .collect();

                        for id in last.keys() {
                            if !current.contains_key(id)
                                && !send(&out, &Response::SessionRemoved { id: *id })
                            {
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
            Request::SubscribeEvents { process_events } => {
                let _ = ensure_default_session();
                let (initial, sequence, process_states) = {
                    let mut journal = event_journal().lock();
                    let initial: Vec<SessionInfo> = registry()
                        .lock()
                        .iter()
                        .map(|(id, entry)| session_info(*id, &entry.terminal))
                        .collect();
                    journal.seed(&initial);
                    (initial, journal.sequence, journal.process_snapshot())
                };
                send(
                    &conn.out,
                    &Response::EventStreamStarted {
                        version: EVENT_API_VERSION,
                        sequence,
                        sessions: initial.clone(),
                    },
                );
                if process_events && !send(&conn.out, &Response::ProcessStates { states: process_states }) { return; }

                let out = Arc::clone(&conn.out);
                let conn_alive = Arc::clone(&conn.conn_alive);
                std::thread::spawn(move || {
                    let mut sequence = sequence;
                    let mut last: HashMap<PaneId, SessionInfo> = initial
                        .into_iter()
                        .map(|session| (session.id, session))
                        .collect();
                    while conn_alive.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                        if !conn_alive.load(Ordering::Relaxed) {
                            return;
                        }
                        let current: HashMap<PaneId, SessionInfo> = registry()
                            .lock()
                            .iter()
                            .map(|(id, entry)| (*id, session_info(*id, &entry.terminal)))
                            .collect();

                        for (id, session) in &current {
                            if last.get(id).is_some_and(|previous| previous != session) {
                                event_journal().lock().update_if_changed(session.clone());
                            }
                        }
                        last = current;

                        let batch = event_journal().lock().batch_after(sequence);
                        let is_gap = batch
                            .oldest_sequence
                            .is_some_and(|oldest| sequence.saturating_add(1) < oldest);
                        if is_gap {
                            let journal = event_journal().lock();
                            let oldest_sequence = journal
                                .events
                                .front()
                                .map(|event| event.sequence)
                                .unwrap_or_else(|| journal.sequence.saturating_add(1));
                            let latest_sequence = journal.sequence;
                            let sessions = journal.snapshot();
                            let process_states = journal.process_snapshot();
                            drop(journal);
                            if !send(
                                &out,
                                &Response::EventStreamGap {
                                    version: EVENT_API_VERSION,
                                    after_sequence: sequence,
                                    oldest_sequence,
                                    latest_sequence,
                                    sessions,
                                },
                            ) {
                                return;
                            }
                            if process_events && !send(&out, &Response::ProcessStates { states: process_states }) { return; }
                            sequence = latest_sequence;
                            continue;
                        }

                        for event in batch.events {
                            sequence = event.sequence;
                            if !event_is_visible(&event.kind, process_events) { continue; }
                            if !send(&out, &Response::StreamEvent { envelope: event }) {
                                return;
                            }
                        }
                    }
                });
            }
            Request::Resize {
                id,
                cols,
                rows,
                ack,
            } => {
                if reject_access_scope(conn, AccessScope::Interact) {
                    return;
                }
                if let Some(entry) = registry().lock().get(&id) {
                    entry.terminal.resize(cols, rows);
                    if ack {
                        send(
                            &conn.out,
                            &Response::Done {
                                cmd: "resize".to_string(),
                            },
                        );
                    }
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
                request_id,
                command,
                args,
                cwd,
                cols,
                rows,
                open,
            } => {
                if reject_access_scope(conn, AccessScope::Manage) {
                    return;
                }
                let mut requests = spawn_requests().lock();
                requests.retain(|_, id| is_registered(*id));
                if let Some(id) = request_id
                    .as_ref()
                    .and_then(|request_id| requests.get(request_id).copied())
                {
                    let opened = if open {
                        get_session(id)
                            .map(|terminal| notify_gui_open(id, terminal))
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    send(&conn.out, &Response::Spawned { id, opened });
                    return;
                }
                match spawn_headless_session(command.as_deref(), &args, cwd.as_deref(), cols, rows) {
                    Ok(id) => {
                        if let Some(request_id) = request_id {
                            requests.insert(request_id, id);
                        }
                        let opened = if open {
                            get_session(id)
                                .map(|terminal| notify_gui_open(id, terminal))
                                .unwrap_or(false)
                        } else {
                            false
                        };
                        send(&conn.out, &Response::Spawned { id, opened });
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
            Request::Close { id, force } => {
                if reject_access_scope(conn, AccessScope::Manage) {
                    return;
                }
                let entry = registry().lock().get(&id).cloned();
                match entry {
                    Some(entry) => {
                        if entry.kind == SessionKind::Gui && !force {
                            send(
                                &conn.out,
                                &Response::Error {
                                    code: "not_closable".to_string(),
                                    message: "GUI session cannot be closed via daemon; retry with force: true to kill it".to_string(),
                                },
                            );
                            return;
                        }
                        // For GUI sessions this kills the process group; the
                        // GUI's own exit handling then closes the tab (its
                        // unregister of this id is a no-op at that point).
                        entry.terminal.terminate_process();
                        unregister(id);
                        send(&conn.out, &Response::Closed { id });
                    }
                    None => {
                        send(
                            &conn.out,
                            &Response::Error {
                                code: "no_such_session".to_string(),
                                message: "no such session".to_string(),
                            },
                        );
                    }
                };
            }
            Request::SplitPane {
                id,
                direction,
                command,
                args,
                cwd,
            } => {
                if reject_access_scope(conn, AccessScope::Manage) {
                    return;
                }
                let Some(dir) = parse_direction(&direction) else {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "bad_request".to_string(),
                            message: format!(
                                "invalid direction {direction:?}: use left/right/top/down"
                            ),
                        },
                    );
                    return;
                };
                match call_gui(move |reply| GuiRequest::SplitPane {
                    target: id,
                    direction: dir,
                    command,
                    args,
                    cwd,
                    reply,
                }) {
                    Ok(new_id) => send(&conn.out, &Response::PaneSplit { id: new_id }),
                    Err(message) => send(
                        &conn.out,
                        &Response::Error {
                            code: gui_error_code(&message).to_string(),
                            message: format!("split_pane {id}: {message}"),
                        },
                    ),
                };
            }
            Request::ResizePane {
                id,
                direction,
                delta,
            } => {
                if reject_access_scope(conn, AccessScope::Manage) {
                    return;
                }
                let Some(dir) = parse_direction(&direction) else {
                    send(
                        &conn.out,
                        &Response::Error {
                            code: "bad_request".to_string(),
                            message: format!(
                                "invalid direction {direction:?}: use left/right/top/down"
                            ),
                        },
                    );
                    return;
                };
                let delta = delta.clamp(-0.5, 0.5);
                match call_gui(move |reply| GuiRequest::ResizePane {
                    target: id,
                    direction: dir,
                    delta,
                    reply,
                }) {
                    Ok(()) => send(
                        &conn.out,
                        &Response::Done {
                            cmd: "resize_pane".to_string(),
                        },
                    ),
                    Err(message) => send(
                        &conn.out,
                        &Response::Error {
                            code: gui_error_code(&message).to_string(),
                            message: format!("resize_pane {id}: {message}"),
                        },
                    ),
                };
            }
            Request::FocusPane { id } => {
                if reject_access_scope(conn, AccessScope::Manage) {
                    return;
                }
                match call_gui(|reply| GuiRequest::FocusPane { target: id, reply }) {
                    Ok(()) => send(
                        &conn.out,
                        &Response::Done {
                            cmd: "focus_pane".to_string(),
                        },
                    ),
                    Err(message) => send(
                        &conn.out,
                        &Response::Error {
                            code: gui_error_code(&message).to_string(),
                            message: format!("focus_pane {id}: {message}"),
                        },
                    ),
                };
            }
            Request::Layout => {
                match call_gui(|reply| GuiRequest::Layout { reply }) {
                    Ok(tabs) => send(&conn.out, &Response::Layout { tabs }),
                    Err(message) => send(
                        &conn.out,
                        &Response::Error {
                            code: gui_error_code(&message).to_string(),
                            message,
                        },
                    ),
                };
            }
            Request::ResizeWindow { cols, rows } => {
                if reject_access_scope(conn, AccessScope::Manage) {
                    return;
                }
                let cols = cols.max(20);
                let rows = rows.max(5);
                match call_gui(move |reply| GuiRequest::ResizeWindow { cols, rows, reply }) {
                    Ok(()) => send(
                        &conn.out,
                        &Response::Done {
                            cmd: "resize_window".to_string(),
                        },
                    ),
                    Err(message) => send(
                        &conn.out,
                        &Response::Error {
                            code: gui_error_code(&message).to_string(),
                            message: format!("resize_window: {message}"),
                        },
                    ),
                };
            }
            Request::BinarySnapshot { id, screen_only } => {
                if let Some(entry) = registry().lock().get(&id) {
                    let bin = entry
                        .terminal
                        .snapshot_binary_compressed_with_history(!screen_only);
                    send(
                        &conn.out,
                        &Response::BinarySnapshot {
                            id,
                            data: base64_encode(&bin),
                        },
                    );
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
            Request::ReportProgramStatus { source_id, payload } => {
                if reject_access_scope(conn, AccessScope::Interact) {
                    return;
                }
                let entry = registry()
                    .lock()
                    .values()
                    .find(|entry| entry.terminal.source_id() == source_id)
                    .cloned();
                let _ = match entry {
                    Some(entry) => match entry
                        .terminal
                        .report_external_program_status(payload.as_bytes())
                    {
                        Ok(()) => send(
                            &conn.out,
                            &Response::Done {
                                cmd: "report_program_status".to_string(),
                            },
                        ),
                        Err(message) => send(
                            &conn.out,
                            &Response::Error {
                                code: "invalid_status".to_string(),
                                message,
                            },
                        ),
                    },
                    None => send(
                        &conn.out,
                        &Response::Error {
                            code: "no_such_session".to_string(),
                            message: "the Fastty session token is no longer active".to_string(),
                        },
                    ),
                };
            }
        }
    }

    fn send(out: &Arc<Mutex<PeerStream>>, response: &Response) -> bool {
        let mut peer = out.lock();
        let Some(mut line) = peer.encode(response) else {
            return true;
        };
        line.push('\n');
        peer.stream.write_all(line.as_bytes()).is_ok()
    }

    fn reject_access_scope(conn: &Conn, required: AccessScope) -> bool {
        if conn.access_scope.permits(required) && !conn.read_only {
            return false;
        }
        let code = if conn.access_scope == AccessScope::Observe || conn.read_only {
            "read_only"
        } else {
            "access_denied"
        };
        let role = match required {
            AccessScope::Observe => "observe",
            AccessScope::Interact => "interact",
            AccessScope::Manage => "manage",
        };
        send(
            &conn.out,
            &Response::Error {
                code: code.to_string(),
                message: format!("{role} access scope is required for this request"),
            },
        );
        true
    }
}

fn foreground_command_label(command: String) -> Option<String> {
    let known_cli = ["claude", "opencode", "agy", "gemini", "codex", "pi"];
    command.split_whitespace().find_map(|argument| {
        std::path::Path::new(argument)
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .find_map(|name| {
                known_cli
                    .iter()
                    .find(|known| name == **known || name.starts_with(&format!("{known}-")))
                    .map(|known| (*known).to_string())
            })
    })
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

    fn test_lifecycle() -> HostLifecycle {
        HostLifecycle { instance: HostInstanceId("test-host".into()), next_id: 0, reserved_until: 0, connections: 1, stopping: false }
    }

    #[test]
    fn durable_host_session_reservations_survive_restart() {
        let directory = std::env::temp_dir().join(format!("fastty-ids-{}-{}", std::process::id(), host_instance_id().0));
        std::fs::create_dir_all(&directory).unwrap();
        let mut first = test_lifecycle();
        reserve_session_ids_in(&mut first, &directory).unwrap();
        let first_id = first.next_id;
        let reserved_until = first.reserved_until;
        let mut restarted = test_lifecycle();
        reserve_session_ids_in(&mut restarted, &directory).unwrap();
        assert!(restarted.next_id >= reserved_until);
        assert!(restarted.next_id > first_id);
        std::fs::write(directory.join("host-session-high-water"), "x".repeat(33)).unwrap();
        assert!(reserve_session_ids_in(&mut restarted, &directory).is_err());
        std::fs::write(directory.join("host-session-high-water"), usize::MAX.to_string()).unwrap();
        assert!(reserve_session_ids_in(&mut restarted, &directory).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn host_restart_preserves_sessions_and_connections() {
        let mut lifecycle = test_lifecycle();
        let instance = lifecycle.instance.clone();
        assert_eq!(lifecycle.restart_if_empty(&instance, 1), Err("host_restart_blocked"));
        assert!(!lifecycle.stopping);
        lifecycle.connections = 2;
        assert_eq!(lifecycle.restart_if_empty(&instance, 0), Err("host_restart_blocked"));
        lifecycle.connections = 1;
        assert_eq!(lifecycle.restart_if_empty(&HostInstanceId("old-host".into()), 0), Err("host_instance_changed"));
        assert_eq!(lifecycle.restart_if_empty(&instance, 0), Ok(()));
        assert!(lifecycle.stopping);
    }

    #[cfg(unix)]
    #[test]
    fn attach_rejects_another_host_before_session_lookup() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixStream;
        use std::sync::atomic::AtomicBool;
        use unix_impl::{handle_request, Conn, PeerStream};
        let (client, server) = UnixStream::pair().unwrap();
        let mut conn = Conn { out: Arc::new(Mutex::new(PeerStream::new(server))), conn_alive: Arc::new(AtomicBool::new(true)), read_only: false, access_scope: AccessScope::Manage, attachments: HashMap::new() };
        handle_request(Request::Attach { id: usize::MAX, expected_host_instance: Some(HostInstanceId("previous-host".into())), mode: AttachMode::ReadWrite, binary_snapshot: false, snapshot_format: SnapshotFormat::Legacy }, &mut conn);
        let mut line = String::new();
        BufReader::new(client).read_line(&mut line).unwrap();
        assert!(line.contains("host_instance_changed"));
        assert!(!line.contains("no_such_session"));
        assert!(conn.attachments.is_empty());
    }


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
            Request::Hello { read_only: false, protocol_version: Some(PROTOCOL_VERSION) },
            Request::Hello { read_only: true, protocol_version: Some(PROTOCOL_VERSION) },
            Request::List,
            Request::SessionStatus { id: 42 },
            Request::SubscribeSessions,
            Request::SubscribeEvents { process_events: false },
            Request::SubscribeEvents { process_events: true },
            Request::Capture { id: 42 },
            Request::SetAccessScope {
                scope: AccessScope::Interact,
            },
            Request::Attach {
                expected_host_instance: None,
                id: 42,
                mode: AttachMode::ReadWrite,
                binary_snapshot: false,
                snapshot_format: crate::daemon::SnapshotFormat::Legacy,
            },
            Request::Attach {
                expected_host_instance: None,
                id: 42,
                mode: AttachMode::ReadOnly,
                binary_snapshot: true,
                snapshot_format: crate::daemon::SnapshotFormat::Legacy,
            },
            Request::Attach {
                expected_host_instance: None,
                id: 42,
                mode: AttachMode::ReadWrite,
                binary_snapshot: true,
                snapshot_format: SnapshotFormat::TerminalState,
            },
            Request::Detach { id: 42 },
            Request::Write {
                id: 42,
                data: "aGVsbG8=".to_string(),
                ack: false,
            },
            Request::Resize {
                id: 42,
                cols: 80,
                rows: 24,
                ack: false,
            },
            Request::Spawn {
                request_id: None,
                command: Some("bash".to_string()),
                args: vec!["-l".to_string()],
                cwd: Some("/tmp".to_string()),
                cols: Some(100),
                rows: Some(30),
                open: true,
            },
            Request::Close {
                id: 42,
                force: true,
            },
            Request::SplitPane {
                id: 7,
                direction: "right".to_string(),
                command: Some("zsh".to_string()),
                args: vec!["-l".to_string()],
                cwd: Some("/tmp".to_string()),
            },
            Request::ResizePane {
                id: 7,
                direction: "left".to_string(),
                delta: 0.1,
            },
            Request::FocusPane { id: 7 },
            Request::Layout,
            Request::ResizeWindow {
                cols: 120,
                rows: 40,
            },
        ];

        for req in reqs {
            let json = serde_json::to_string(&req).expect("serialize");
            let parsed: Request = serde_json::from_str(&json).expect("deserialize");
            match (&req, &parsed) {
                (Request::Hello { read_only: a, .. }, Request::Hello { read_only: b, .. }) => {
                    assert_eq!(a, b);
                }
                (Request::List, Request::List) => {}
                (Request::SessionStatus { id: a }, Request::SessionStatus { id: b }) => {
                    assert_eq!(a, b);
                }
                (Request::SubscribeSessions, Request::SubscribeSessions) => {}
                (Request::SubscribeEvents { process_events: a }, Request::SubscribeEvents { process_events: b }) => assert_eq!(a, b),
                (Request::Capture { id: a }, Request::Capture { id: b }) => assert_eq!(a, b),
                (Request::SetAccessScope { scope: a }, Request::SetAccessScope { scope: b }) => {
                    assert_eq!(a, b)
                }
                (
                    Request::Attach { id: a, mode: m1, binary_snapshot: s1, snapshot_format: f1, .. },
                    Request::Attach { id: b, mode: m2, binary_snapshot: s2, snapshot_format: f2, .. },
                ) => {
                    assert_eq!(a, b);
                    assert_eq!(m1, m2);
                    assert_eq!(s1, s2);
                    assert_eq!(f1, f2);
                }
                (Request::Detach { id: a }, Request::Detach { id: b }) => assert_eq!(a, b),
                (
                    Request::Write {
                        id: a, data: d1, ..
                    },
                    Request::Write {
                        id: b, data: d2, ..
                    },
                ) => {
                    assert_eq!(a, b);
                    assert_eq!(d1, d2);
                }
                (
                    Request::Resize {
                        id: a,
                        cols: c1,
                        rows: r1,
                        ..
                    },
                    Request::Resize {
                        id: b,
                        cols: c2,
                        rows: r2,
                        ..
                    },
                ) => {
                    assert_eq!(a, b);
                    assert_eq!(c1, c2);
                    assert_eq!(r1, r2);
                }
                (
                    Request::Spawn {
                        request_id: i1,
                        command: c1,
                        args: a1,
                        cwd: cw1,
                        cols: co1,
                        rows: r1,
                        open: o1,
                    },
                    Request::Spawn {
                        request_id: i2,
                        command: c2,
                        args: a2,
                        cwd: cw2,
                        cols: co2,
                        rows: r2,
                        open: o2,
                    },
                ) => {
                    assert_eq!(i1, i2);
                    assert_eq!(c1, c2);
                    assert_eq!(a1, a2);
                    assert_eq!(cw1, cw2);
                    assert_eq!(co1, co2);
                    assert_eq!(r1, r2);
                    assert_eq!(o1, o2);
                }
                (Request::Close { id: a, force: f1 }, Request::Close { id: b, force: f2 }) => {
                    assert_eq!(a, b);
                    assert_eq!(f1, f2);
                }
                (
                    Request::SplitPane {
                        id: a,
                        direction: d1,
                        command: c1,
                        args: a1,
                        cwd: cw1,
                    },
                    Request::SplitPane {
                        id: b,
                        direction: d2,
                        command: c2,
                        args: a2,
                        cwd: cw2,
                    },
                ) => {
                    assert_eq!(a, b);
                    assert_eq!(d1, d2);
                    assert_eq!(c1, c2);
                    assert_eq!(a1, a2);
                    assert_eq!(cw1, cw2);
                }
                (
                    Request::ResizePane {
                        id: a,
                        direction: d1,
                        delta: dl1,
                    },
                    Request::ResizePane {
                        id: b,
                        direction: d2,
                        delta: dl2,
                    },
                ) => {
                    assert_eq!(a, b);
                    assert_eq!(d1, d2);
                    assert_eq!(dl1, dl2);
                }
                (Request::FocusPane { id: a }, Request::FocusPane { id: b }) => assert_eq!(a, b),
                (Request::Layout, Request::Layout) => {}
                (
                    Request::ResizeWindow { cols: c1, rows: r1 },
                    Request::ResizeWindow { cols: c2, rows: r2 },
                ) => {
                    assert_eq!(c1, c2);
                    assert_eq!(r1, r2);
                }
                _ => panic!("mismatched variant: {req:?} vs {parsed:?}"),
            }
        }
    }

    #[test]
    fn test_attach_request_defaults_to_read_write() {
        let json = r#"{"cmd":"attach","id":1}"#;
        let req: Request = serde_json::from_str(json).expect("deserialize");
        match req {
            Request::Attach { id, mode, binary_snapshot, snapshot_format, .. } => {
                assert_eq!(id, 1);
                assert_eq!(mode, AttachMode::ReadWrite);
                assert!(!binary_snapshot);
                assert_eq!(snapshot_format, SnapshotFormat::Legacy);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_response_serialization() {
        let status = Response::SessionStatus {
            alive: true,
            process_running: false,
            foreground_process_label: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["event"], "session_status");
        assert_eq!(json["process_running"], false);

        let res = Response::Attached {
            id: 1,
            cols: 80,
            rows: 24,
            mode: AttachMode::ReadOnly,
            program_status: vec![],
            process_running: None,
            cwd: None,
            shell_pid: None,
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

    #[test]
    fn event_envelopes_have_stable_versioned_fields() {
        let event = EventEnvelope {
            version: EVENT_API_VERSION,
            sequence: 7,
            timestamp_unix_ms: 1234,
            session_id: Some(42),
            kind: SessionEvent::SessionRemoved { id: 42 },
        };
        let json = serde_json::to_value(event).expect("serialize event");
        assert_eq!(json["version"], 1);
        assert_eq!(json["sequence"], 7);
        assert_eq!(json["session_id"], 42);
        assert_eq!(json["kind"]["type"], "session_removed");
        assert_eq!(json["kind"]["id"], 42);
    }

    #[test]
    fn test_hello_backward_compatibility() {
        let json_no_flag = r#"{"cmd":"hello"}"#;
        let req1: Request = serde_json::from_str(json_no_flag).expect("deserialize without flag");
        match req1 {
            Request::Hello { read_only, .. } => assert!(!read_only),
            _ => panic!("wrong variant"),
        }

        let json_with_flag = r#"{"cmd":"hello","read_only":true}"#;
        let req2: Request = serde_json::from_str(json_with_flag).expect("deserialize with flag");
        match req2 {
            Request::Hello { read_only, .. } => assert!(read_only),
            _ => panic!("wrong variant"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_daemon_read_only_sticky_enforcement() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixStream;
        use std::sync::atomic::AtomicBool;
        use unix_impl::{handle_request, Conn, PeerStream};

        let (client, server) = UnixStream::pair().unwrap();
        let mut conn = Conn {
            out: Arc::new(Mutex::new(PeerStream::new(server))),
            conn_alive: Arc::new(AtomicBool::new(true)),
            read_only: false,
            access_scope: AccessScope::Manage,
            attachments: HashMap::new(),
        };

        let mut client_reader = BufReader::new(client);

        // 1. Send Hello with read_only: true
        handle_request(Request::Hello { read_only: true, protocol_version: Some(PROTOCOL_VERSION) }, &mut conn);
        assert!(conn.read_only);

        let mut line = String::new();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""event":"hello""#));
        let hello: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert!(hello["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "close_if_idle"));

        // 2. Try to disable read_only with Hello { read_only: false } -> should remain sticky true
        handle_request(Request::Hello { read_only: false, protocol_version: Some(PROTOCOL_VERSION) }, &mut conn);
        assert!(conn.read_only);
        line.clear();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""event":"hello""#));

        handle_request(
            Request::SetAccessScope {
                scope: AccessScope::Manage,
            },
            &mut conn,
        );
        line.clear();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"permission_escalation""#));

        // 3. Write command rejected with code "read_only"
        handle_request(
            Request::Write {
                id: 1,
                data: base64_encode(b"test"),
                ack: false,
            },
            &mut conn,
        );
        line.clear();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"read_only""#));

        // 4. Spawn command rejected with code "read_only"
        handle_request(
            Request::Spawn {
                request_id: None,
                command: None,
                args: vec![],
                cwd: None,
                cols: None,
                rows: None,
                open: false,
            },
            &mut conn,
        );
        line.clear();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"read_only""#));

        // 5. Close command rejected with code "read_only"
        handle_request(Request::Close { id: 1, force: true }, &mut conn);
        line.clear();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"read_only""#));

        // Resize and focus also change session or window state.
        handle_request(
            Request::Resize {
                id: 1,
                cols: 80,
                rows: 24,
                ack: true,
            },
            &mut conn,
        );
        line.clear();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"read_only""#));

        handle_request(Request::FocusPane { id: 1 }, &mut conn);
        line.clear();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"read_only""#));
    }

    #[cfg(unix)]
    fn hello_response(protocol_version: Option<u32>) -> serde_json::Value {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixStream;
        use std::sync::atomic::AtomicBool;
        use unix_impl::{handle_request, Conn, PeerStream};

        let (client, server) = UnixStream::pair().unwrap();
        let mut conn = Conn {
            out: Arc::new(Mutex::new(PeerStream::new(server))),
            conn_alive: Arc::new(AtomicBool::new(true)),
            read_only: false,
            access_scope: AccessScope::Manage,
            attachments: HashMap::new(),
        };
        handle_request(
            Request::Hello {
                read_only: false,
                protocol_version,
            },
            &mut conn,
        );
        let mut line = String::new();
        BufReader::new(client).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn hello_without_a_version_receives_the_oldest_supported_shape() {
        let hello = hello_response(None);
        assert!(hello.get("capabilities").is_none());
        assert!(hello.get("host_instance").is_none());
        assert!(hello.get("version").is_some());
        assert!(hello.get("fastty_version").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn hello_with_the_native_version_receives_the_native_shape() {
        let hello = hello_response(Some(PROTOCOL_VERSION));
        assert!(hello["capabilities"].is_array());
        assert!(hello.get("host_instance").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn hello_with_a_future_version_is_capped_to_the_native_shape() {
        let hello = hello_response(Some(PROTOCOL_VERSION + 5));
        assert!(hello["capabilities"].is_array());
    }

    #[cfg(unix)]
    #[test]
    fn test_close_gui_vs_headless_session() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixStream;
        use std::sync::atomic::AtomicBool;
        use unix_impl::{handle_request, Conn, PeerStream};

        let (client, server) = UnixStream::pair().unwrap();
        let mut conn = Conn {
            out: Arc::new(Mutex::new(PeerStream::new(server))),
            conn_alive: Arc::new(AtomicBool::new(true)),
            read_only: false,
            access_scope: AccessScope::Manage,
            attachments: HashMap::new(),
        };
        let mut client_reader = BufReader::new(client);

        // 1. Non-existent session
        handle_request(
            Request::Close {
                id: 999_999,
                force: false,
            },
            &mut conn,
        );
        let mut line = String::new();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"no_such_session""#));

        // 2. GUI session without force answers not_closable and stays
        if let Ok(gui_term) = TerminalState::new_headless("/bin/sh", &[], None, 80, 24) {
            let gui_id = 999_001;
            register(gui_id, Arc::new(gui_term));
            handle_request(
                Request::Close {
                    id: gui_id,
                    force: false,
                },
                &mut conn,
            );
            line.clear();
            client_reader.read_line(&mut line).unwrap();
            assert!(line.contains(r#""code":"not_closable""#));
            assert!(is_registered(gui_id));

            // 3. The same session with force closes and unregisters
            handle_request(
                Request::Close {
                    id: gui_id,
                    force: true,
                },
                &mut conn,
            );
            line.clear();
            client_reader.read_line(&mut line).unwrap();
            assert!(line.contains(r#""event":"closed""#));
            assert!(!is_registered(gui_id));
        }

        // 4. Headless session closes without force
        if let Ok(headless_term) = TerminalState::new_headless("/bin/sh", &[], None, 80, 24) {
            let headless_id = 999_002;
            register_headless(headless_id, Arc::new(headless_term));
            assert!(is_registered(headless_id));
            handle_request(
                Request::Close {
                    id: headless_id,
                    force: false,
                },
                &mut conn,
            );
            line.clear();
            client_reader.read_line(&mut line).unwrap();
            assert!(line.contains(r#""event":"closed""#));
            assert!(!is_registered(headless_id));
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_pane_control_without_gui_answers_no_gui() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixStream;
        use std::sync::atomic::AtomicBool;
        use unix_impl::{handle_request, Conn, PeerStream};

        let (client, server) = UnixStream::pair().unwrap();
        let mut conn = Conn {
            out: Arc::new(Mutex::new(PeerStream::new(server))),
            conn_alive: Arc::new(AtomicBool::new(true)),
            read_only: false,
            access_scope: AccessScope::Manage,
            attachments: HashMap::new(),
        };
        let mut client_reader = BufReader::new(client);

        // No GUI attached to this daemon process: pane ops fail fast.
        handle_request(Request::Layout, &mut conn);
        let mut line = String::new();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"no_gui""#), "{line}");

        handle_request(
            Request::SplitPane {
                id: 1,
                direction: "right".to_string(),
                command: None,
                args: vec![],
                cwd: None,
            },
            &mut conn,
        );
        line.clear();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"no_gui""#), "{line}");

        handle_request(Request::FocusPane { id: 1 }, &mut conn);
        line.clear();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"no_gui""#), "{line}");

        // An invalid direction is rejected before the GUI is involved.
        handle_request(
            Request::ResizePane {
                id: 1,
                direction: "sideways".to_string(),
                delta: 0.05,
            },
            &mut conn,
        );
        line.clear();
        client_reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"bad_request""#), "{line}");
    }

    #[test]
    fn test_session_info_history_size() {
        let json = r#"{"id":42,"title":"zsh","cwd":"/tmp","cols":80,"rows":24,"alive":true,"history_size":120}"#;
        let info: SessionInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, 42);
        assert_eq!(info.history_size, 120);

        // Backward compatibility: missing history_size defaults to 0
        let old_json = r#"{"id":42,"title":"zsh","cwd":"/tmp","cols":80,"rows":24,"alive":true}"#;
        let old_info: SessionInfo = serde_json::from_str(old_json).unwrap();
        assert_eq!(old_info.history_size, 0);
        assert!(old_info.program_status.is_empty());

        let status_json = r#"{"id":42,"title":"claude","cwd":"/tmp","cols":80,"rows":24,"alive":true,"history_size":0,"program_status":[{"id":"assistant","state":"blocked","app":"claude","progress":45,"kind":"permission","msg":"private text"}]}"#;
        let info: SessionInfo = serde_json::from_str(status_json).unwrap();
        assert_eq!(
            info.program_status[0].state,
            crate::program_status::ProgramState::Blocked
        );
        let safe_json = serde_json::to_value(info).unwrap();
        assert_eq!(safe_json["program_status"][0]["app"], "claude");
        assert!(safe_json["program_status"][0].get("msg").is_none());
    }

    fn test_session(id: PaneId) -> SessionInfo {
        SessionInfo {
            id,
            identity: None,
            title: "shell".to_string(),
            cwd: Some("/tmp".to_string()),
            cols: 80,
            rows: 24,
            alive: true,
            history_size: 0,
            program_status: vec![],
        }
    }

    #[test]
    fn event_journal_preserves_add_remove_pairs_between_polls() {
        let mut journal = EventJournal::new();
        journal.added(test_session(7));
        journal.removed(7);

        let batch = journal.batch_after(0);
        assert_eq!(batch.events.len(), 2);
        assert_eq!(batch.events[0].sequence, 1);
        assert!(matches!(
            batch.events[0].kind,
            SessionEvent::SessionAdded { .. }
        ));
        assert_eq!(batch.events[1].sequence, 2);
        assert!(matches!(
            batch.events[1].kind,
            SessionEvent::SessionRemoved { id: 7 }
        ));
        assert!(journal.snapshot().is_empty());
    }

    #[test]
    fn legacy_event_subscriptions_do_not_receive_process_events() {
        let request: Request = serde_json::from_str(r#"{"cmd":"subscribe_events"}"#).unwrap();
        assert!(matches!(request, Request::SubscribeEvents { process_events: false }));
        assert_eq!(serde_json::to_string(&request).unwrap(), r#"{"cmd":"subscribe_events"}"#);
        let mut journal = EventJournal::new();
        journal.added(test_session(7));
        journal.process_changed(7, ProcessObservation::Unavailable { reason: ProcessUnavailableReason::ShellUnavailable });
        journal.removed(7);
        let batch = journal.batch_after(0);
        let visible: Vec<_> = batch.events.iter().filter(|event| event_is_visible(&event.kind, false)).collect();
        assert_eq!(visible.len(), 2);
        assert!(matches!(visible[0].kind, SessionEvent::SessionAdded { .. }));
        assert!(matches!(visible[1].kind, SessionEvent::SessionRemoved { .. }));
        assert_eq!(batch.events.last().unwrap().sequence, 3);
        assert!(batch.events.iter().all(|event| event_is_visible(&event.kind, true)));
    }

    #[test]
    fn process_journal_publishes_changes_once_and_keeps_a_baseline() {
        let mut journal = EventJournal::new();
        journal.added(test_session(7));
        let observation = ProcessObservation::Unavailable { reason: ProcessUnavailableReason::ShellUnavailable };
        journal.process_changed(7, observation.clone());
        journal.process_changed(7, observation.clone());
        assert_eq!(journal.sequence, 2);
        let states = journal.process_snapshot();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].generation, 1);
        assert_eq!(states[0].session.session_id, 7);
        assert_eq!(states[0].observation, observation);
        assert!(matches!(journal.batch_after(1).events[0].kind, SessionEvent::ProcessChanged { .. }));
        journal.process_changed(7, ProcessObservation::Unavailable { reason: ProcessUnavailableReason::ForegroundUnavailable });
        assert_eq!(journal.process_snapshot()[0].generation, 2);
        journal.removed(7);
        journal.process_changed(7, observation);
        assert!(journal.process_snapshot().is_empty());
        assert_eq!(journal.sequence, 4);
    }

    #[test]
    fn event_journal_reports_gap_after_bounded_history_overflows() {
        let mut journal = EventJournal::new();
        for id in 0..=EVENT_JOURNAL_CAPACITY {
            journal.added(test_session(id));
        }

        let batch = journal.batch_after(0);
        assert_eq!(batch.oldest_sequence, Some(2));
        assert!(0_u64.saturating_add(1) < batch.oldest_sequence.unwrap());
        assert_eq!(batch.events.len(), EVENT_JOURNAL_CAPACITY);
        assert_eq!(journal.snapshot().len(), EVENT_JOURNAL_CAPACITY + 1);
    }

    #[cfg(unix)]
    #[test]
    fn access_scope_can_only_decrease_and_blocks_manage_requests() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixStream;
        use std::sync::atomic::AtomicBool;
        use unix_impl::{handle_request, Conn, PeerStream};

        let (client, server) = UnixStream::pair().unwrap();
        let mut conn = Conn {
            out: Arc::new(Mutex::new(PeerStream::new(server))),
            conn_alive: Arc::new(AtomicBool::new(true)),
            read_only: false,
            access_scope: AccessScope::Manage,
            attachments: HashMap::new(),
        };
        let mut reader = BufReader::new(client);
        let mut line = String::new();

        handle_request(
            Request::SetAccessScope {
                scope: AccessScope::Interact,
            },
            &mut conn,
        );
        reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""cmd":"set_access_scope""#));

        line.clear();
        handle_request(Request::Close { id: 1, force: true }, &mut conn);
        reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"access_denied""#));

        line.clear();
        handle_request(
            Request::SetAccessScope {
                scope: AccessScope::Manage,
            },
            &mut conn,
        );
        reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""code":"permission_escalation""#));
    }

    #[test]
    fn external_action_serializes_as_an_additive_request() {
        let request = Request::ExternalAction {
            id: 42,
            action: ExternalAction::Send {
                text: "hello\n".to_string(),
            },
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["cmd"], "external_action");
        assert_eq!(value["id"], 42);
        assert_eq!(value["action"]["action"], "send");
        assert_eq!(value["action"]["text"], "hello\n");
    }

    #[test]
    fn external_action_fails_closed_without_a_gui() {
        let result = request_external_approval_with_sender(
            None,
            42,
            ExternalAction::Send {
                text: "hello".to_string(),
            },
            EXTERNAL_APPROVAL_TIMEOUT,
        );
        assert!(result
            .unwrap_err()
            .starts_with("approval_unavailable: no Fastty window"));
    }

    #[test]
    fn external_action_returns_the_gui_decision() {
        let (sender, receiver) = async_channel::unbounded();
        let worker = std::thread::spawn(move || {
            request_external_approval_with_sender(
                Some(sender),
                42,
                ExternalAction::Send {
                    text: "hello".to_string(),
                },
                EXTERNAL_APPROVAL_TIMEOUT,
            )
        });
        let request = receiver.recv_blocking().unwrap();
        match request {
            GuiRequest::ExternalApproval {
                session_id,
                action,
                reply,
                ..
            } => {
                assert_eq!(session_id, 42);
                assert_eq!(action, ExternalAction::Send {
                    text: "hello".to_string(),
                });
                reply.send(true).unwrap();
            }
            _ => panic!("unexpected GUI request"),
        }
        assert!(worker.join().unwrap().is_ok());
    }

    #[test]
    fn external_action_timeout_cancels_the_ui_prompt() {
        let (sender, receiver) = async_channel::unbounded();
        let worker = std::thread::spawn(move || {
            request_external_approval_with_sender(
                Some(sender),
                42,
                ExternalAction::Close { force: true },
                std::time::Duration::from_millis(1),
            )
        });
        let (request_id, _reply) = match receiver.recv_blocking().unwrap() {
            GuiRequest::ExternalApproval {
                request_id, reply, ..
            } => (request_id, reply),
            _ => panic!("expected an approval request"),
        };
        assert!(worker
            .join()
            .unwrap()
            .unwrap_err()
            .starts_with("approval_timeout:"));
        assert!(matches!(
            receiver.recv_blocking().unwrap(),
            GuiRequest::CancelExternalApproval { request_id: canceled_id }
                if canceled_id == request_id
        ));
    }
}
