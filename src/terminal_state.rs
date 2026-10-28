//! Terminal state wrapper using alacritty_terminal.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Config as AlacrittyConfig;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use parking_lot::Mutex as ParkingMutex;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::config::FontConfig;
use crate::event_listener::EventListenerProxy;

static NEXT_SOURCE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AppEvent {
    Wakeup,
    Exit {
        shell_pid: Option<u32>,
    },
    ForceExit,
    ConfigChanged,
    Bell,
    ClipboardStore(String),
    ClipboardLoad(u8),
    CwdChanged(std::path::PathBuf),
    TitleChanged(String),
    ProgramStatusChanged {
        source_id: u64,
    },
    ProgressChanged(Option<ProgressReport>),
    CommandStarted,
    CommandFinished {
        duration_ms: u128,
        exit_code: Option<i32>,
    },
    Notification {
        title: String,
        body: String,
    },
    PromptStarted {
        absolute_line: u64,
    },
    ShowToast {
        message: String,
        duration_ms: u64,
    },
    ForcePollWidgets,
    GitStatusUpdated {
        window_id: Option<usize>,
        tab_idx: usize,
        status: Option<crate::git::GitStatus>,
    },
    GitRepoChanged {
        repo_path: std::path::PathBuf,
    },
}

#[cfg(test)]
mod dws_regression_tests {
    use super::OutputHub;
    use parking_lot::Mutex;
    use std::sync::Arc;

    #[test]
    fn hosted_terminal_confirmation_tracks_foreground_process() {
        assert!(super::remote_session_needs_close_confirmation(
            Some(7),
            true
        ));
        assert!(!super::remote_session_needs_close_confirmation(
            Some(7),
            false
        ));
        assert!(!super::remote_session_needs_close_confirmation(None, true));
    }

    #[cfg(unix)]
    #[test]
    fn idle_shell_process_group_does_not_require_close_confirmation() {
        assert!(!super::foreground_process_state(41, 41, false));
        assert!(super::foreground_process_state(41, 41, true));
        assert!(super::foreground_process_state(41, 42, false));
        assert!(super::foreground_process_state(-1, 41, false));
        assert!(super::foreground_process_state(41, -1, false));
    }

    #[cfg(unix)]
    #[test]
    fn definitive_absence_allows_restore_fallback() {
        assert!(super::is_missing_host_session_error(
            "no_such_session: session ID 12 not found"
        ));
        assert!(super::is_missing_host_session_error("host_instance_changed: host replaced"));
        assert!(!super::is_missing_host_session_error("session_identity_unknown: legacy reference"));
        assert!(!super::is_missing_host_session_error("connection refused"));
        assert!(!super::is_missing_host_session_error(
            "invalid session-host response: malformed JSON"
        ));
        assert!(!super::is_missing_host_session_error(
            "spawn_failed: no_such_session"
        ));
    }

    #[test]
    fn snapshot_and_subscribe_delivers_each_boundary_chunk_once() {
        let hub = OutputHub::default();
        let terminal_state = Arc::new(Mutex::new(Vec::new()));
        let (snapshot, output) = hub.snapshot_and_subscribe(|| terminal_state.lock().clone());
        assert!(snapshot.is_empty());

        hub.publish(b"after-boundary", || {
            terminal_state.lock().extend_from_slice(b"after-boundary");
        });

        let next = output.try_recv().expect("new output follows the snapshot");
        assert_eq!(next, b"after-boundary");
        assert!(output.try_recv().is_err(), "output is delivered only once");
    }

    #[test]
    fn published_output_is_in_snapshot_and_not_replayed() {
        let hub = OutputHub::default();
        let terminal_state = Arc::new(Mutex::new(Vec::new()));
        hub.publish(b"before-boundary", || {
            terminal_state.lock().extend_from_slice(b"before-boundary");
        });

        let (snapshot, output) = hub.snapshot_and_subscribe(|| terminal_state.lock().clone());

        assert_eq!(snapshot, b"before-boundary");
        assert!(
            output.try_recv().is_err(),
            "snapshot bytes are not replayed"
        );
    }
}

/// Progress reported by a program through ConEmu's `OSC 9;4` sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressState {
    /// State 1: the percentage is meaningful.
    Normal,
    /// State 2: the task failed. The percentage is kept when present.
    Error,
    /// State 3: the task runs without a known percentage.
    Indeterminate,
    /// State 4: the task paused. The percentage is kept when present.
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressReport {
    pub state: ProgressState,
    pub percent: Option<u8>,
}

/// Events derived from raw PTY byte-stream CSI scanning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsiEvent {
    /// Plain `CSI 2J` — callers clear main-screen scrollback.
    ClearHistory,
    /// `CSI ? ... 2026 ... h` — Synchronized Output frame begins.
    SyncOutputBegin,
    /// `CSI ? ... 2026 ... l` — Synchronized Output frame ends.
    SyncOutputEnd,
    /// `CSI ? 5522 $ p` — DECRQM probe for the kitty clipboard protocol.
    QueryKittyClipboard,
    /// `CSI > flags u` — push keyboard flags.
    KittyPush(u8),
    /// `CSI < count u` — pop keyboard flags.
    KittyPop(u32),
    /// `CSI = flags ; mode u` — set keyboard flags.
    KittySet(u8, u32),
    /// `CSI ? u` — query keyboard flags.
    KittyQuery,
}

/// Byte-level watcher for the CSI sequences fastty handles outside the
/// alacritty model: scrollback wipe on plain `CSI 2J`, Synchronized
/// Output (DEC private mode 2026), the kitty clipboard probe, and the
/// kitty keyboard protocol's push/pop/set/query sequences.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CsiWatcher {
    state: CsiState,
    /// Prefix byte of the sequence: `0`, `?`, `>`, `<`, or `=`.
    prefix: u8,
    params: [u32; 8],
    param_count: usize,
    digit_acc: u32,
    intermediate: Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
enum CsiState {
    #[default]
    Ground,
    Esc,
    Params,
}

impl CsiWatcher {
    pub fn new() -> Self {
        Self {
            state: CsiState::Ground,
            prefix: 0,
            params: [0; 8],
            param_count: 0,
            digit_acc: 0,
            intermediate: None,
        }
    }

    pub fn feed(&mut self, byte: u8) -> Option<CsiEvent> {
        match self.state {
            CsiState::Ground => {
                if byte == 0x1b {
                    self.state = CsiState::Esc;
                }
                None
            }
            CsiState::Esc => {
                if byte == b'[' {
                    self.state = CsiState::Params;
                    self.prefix = 0;
                    self.params = [0; 8];
                    self.param_count = 0;
                    self.digit_acc = 0;
                    self.intermediate = None;
                    None
                } else {
                    self.state = CsiState::Ground;
                    None
                }
            }
            CsiState::Params => {
                if byte.is_ascii_digit() {
                    self.digit_acc = self
                        .digit_acc
                        .saturating_mul(10)
                        .saturating_add((byte - b'0') as u32);
                    None
                } else if matches!(byte, b'?' | b'>' | b'<' | b'=')
                    && self.param_count == 0
                    && self.digit_acc == 0
                {
                    self.prefix = byte;
                    None
                } else if byte == b';' || byte == b':' {
                    self.push_param();
                    None
                } else if (0x20..=0x2F).contains(&byte) {
                    // Intermediate byte, e.g. the `$` of a DECRQM query.
                    self.intermediate = Some(byte);
                    None
                } else {
                    self.push_param();
                    let params = &self.params[..self.param_count];
                    let event = if self.prefix == 0 && byte == b'J' && params.contains(&2) {
                        Some(CsiEvent::ClearHistory)
                    } else if self.prefix == b'?' && byte == b'h' && params.contains(&2026) {
                        Some(CsiEvent::SyncOutputBegin)
                    } else if self.prefix == b'?' && byte == b'l' && params.contains(&2026) {
                        Some(CsiEvent::SyncOutputEnd)
                    } else if self.prefix == b'?'
                        && byte == b'p'
                        && self.intermediate == Some(b'$')
                        && params.contains(&5522)
                    {
                        Some(CsiEvent::QueryKittyClipboard)
                    } else if byte == b'u' {
                        self.kitty_event()
                    } else {
                        None
                    };
                    self.state = CsiState::Ground;
                    event
                }
            }
        }
    }

    fn push_param(&mut self) {
        if self.param_count < self.params.len() {
            self.params[self.param_count] = self.digit_acc;
            self.param_count += 1;
        }
        self.digit_acc = 0;
    }

    /// Final byte `u` with a kitty prefix: push, pop, set, or query.
    /// Plain `CSI u` (no prefix) stays save-cursor and returns `None`.
    fn kitty_event(&self) -> Option<CsiEvent> {
        // The final byte always ran `push_param`, so an empty parameter
        // list reads as one zero param.
        let p0 = self.params.first().copied().unwrap_or(0);
        let p1 = self.params.get(1).copied();
        match self.prefix {
            b'>' => Some(CsiEvent::KittyPush(p0 as u8)),
            b'<' => Some(CsiEvent::KittyPop(p0.max(1))),
            b'=' => Some(CsiEvent::KittySet(p0 as u8, p1.unwrap_or(1))),
            b'?' if self.params[..self.param_count].iter().all(|&p| p == 0) => {
                Some(CsiEvent::KittyQuery)
            }
            _ => None,
        }
    }
}


#[derive(Clone, Debug)]
pub struct ImagePlacement {
    pub placement_id: u32,
    pub image_id: u32,
    pub absolute_line: u64,
    pub col: usize,
    pub cols: usize,
    pub rows: usize,
    pub z_index: i32,
    pub image: Arc<gpui::RenderImage>,
}

#[derive(Default)]
pub struct ImageStore {
    pub images: std::collections::HashMap<u32, Arc<gpui::RenderImage>>,
    pub placements: Vec<ImagePlacement>,
}

impl ImageStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_image(&mut self, id: u32, img: Arc<gpui::RenderImage>) {
        self.images.insert(id, img);
    }

    pub fn add_placement(&mut self, placement: ImagePlacement) {
        self.placements.push(placement);
    }

    pub fn delete_all(&mut self) {
        self.images.clear();
        self.placements.clear();
    }

    pub fn delete_by_id(&mut self, id: u32) {
        self.images.remove(&id);
        self.placements.retain(|p| p.image_id != id);
    }

    pub fn delete_by_placement(&mut self, p_id: u32) {
        self.placements.retain(|p| p.placement_id != p_id);
    }

    /// Prunes placements past the maximum scrollback and removes unreferenced textures.
    pub fn prune_old_placements(&mut self, min_visible_abs_line: u64) {
        let initial_len = self.placements.len();
        self.placements
            .retain(|p| (p.absolute_line + p.rows as u64) >= min_visible_abs_line);
        if self.placements.len() < initial_len {
            let active_ids: std::collections::HashSet<u32> =
                self.placements.iter().map(|p| p.image_id).collect();
            self.images.retain(|id, _| active_ids.contains(id));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
enum SequenceParseState {
    #[default]
    Normal,
    Esc,
    Osc,
    OscEsc,
    Apc,
    ApcEsc,
    Dcs,
    DcsEsc,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SideCheckpoint {
    seq_state: SequenceParseState,
    osc_buf: Vec<u8>,
    apc_buf: Vec<u8>,
    #[serde(default)]
    dcs_buf: Vec<u8>,
    #[serde(default)]
    csi_watcher: CsiWatcher,
    sync_output_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cwd: Option<std::path::PathBuf>,
}

impl Default for SideCheckpoint {
    fn default() -> Self {
        Self {
            seq_state: SequenceParseState::Normal,
            osc_buf: Vec::new(),
            apc_buf: Vec::new(),
            dcs_buf: Vec::new(),
            csi_watcher: CsiWatcher::new(),
            sync_output_active: false,
            cwd: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SequenceFrame {
    Reset,
    Osc { bytes: Vec<u8>, framing: usize },
    Apc(Vec<u8>),
    Dcs(Vec<u8>),
}

impl SideCheckpoint {
    fn feed(&mut self, byte: u8) -> Option<SequenceFrame> {
        match self.seq_state {
            SequenceParseState::Normal => {
                if byte == 0x1b {
                    self.seq_state = SequenceParseState::Esc;
                }
            }
            SequenceParseState::Esc => {
                self.seq_state = match byte {
                    b']' => {
                        self.osc_buf.clear();
                        SequenceParseState::Osc
                    }
                    b'_' => {
                        self.apc_buf.clear();
                        SequenceParseState::Apc
                    }
                    b'P' => {
                        self.dcs_buf.clear();
                        SequenceParseState::Dcs
                    }
                    b'c' => {
                        self.seq_state = SequenceParseState::Normal;
                        return Some(SequenceFrame::Reset);
                    }
                    _ => SequenceParseState::Normal,
                };
            }
            SequenceParseState::Osc | SequenceParseState::OscEsc => {
                let escaped = self.seq_state == SequenceParseState::OscEsc;
                if (escaped && byte == b'\\') || (!escaped && byte == 0x07) {
                    self.seq_state = SequenceParseState::Normal;
                    return Some(SequenceFrame::Osc {
                        bytes: std::mem::take(&mut self.osc_buf),
                        framing: if escaped { 4 } else { 3 },
                    });
                }
                if !escaped && byte == 0x1b {
                    self.seq_state = SequenceParseState::OscEsc;
                } else {
                    if escaped {
                        self.osc_buf.push(0x1b);
                    }
                    self.osc_buf.push(byte);
                    self.seq_state = if self.osc_buf.len() > 131072 {
                        SequenceParseState::Normal
                    } else {
                        SequenceParseState::Osc
                    };
                }
            }
            SequenceParseState::Apc | SequenceParseState::ApcEsc => {
                let escaped = self.seq_state == SequenceParseState::ApcEsc;
                if (escaped && byte == b'\\') || (!escaped && byte == 0x07) {
                    self.seq_state = SequenceParseState::Normal;
                    return Some(SequenceFrame::Apc(std::mem::take(&mut self.apc_buf)));
                }
                if !escaped && byte == 0x1b {
                    self.seq_state = SequenceParseState::ApcEsc;
                } else {
                    if escaped {
                        self.apc_buf.push(0x1b);
                    }
                    self.apc_buf.push(byte);
                    self.seq_state = if self.apc_buf.len() > 16 * 1024 * 1024 {
                        SequenceParseState::Normal
                    } else {
                        SequenceParseState::Apc
                    };
                }
            }
            SequenceParseState::Dcs | SequenceParseState::DcsEsc => {
                // DCS framing matches APC: payload runs to ESC \ (ST). The
                // main consumer is the sixel graphics protocol.
                let escaped = self.seq_state == SequenceParseState::DcsEsc;
                if (escaped && byte == b'\\') || (!escaped && byte == 0x07) {
                    self.seq_state = SequenceParseState::Normal;
                    return Some(SequenceFrame::Dcs(std::mem::take(&mut self.dcs_buf)));
                }
                if !escaped && byte == 0x1b {
                    self.seq_state = SequenceParseState::DcsEsc;
                } else {
                    if escaped {
                        self.dcs_buf.push(0x1b);
                    }
                    self.dcs_buf.push(byte);
                    self.seq_state = if self.dcs_buf.len() > crate::parser::sixel::MAX_SIXEL_BYTES {
                        SequenceParseState::Normal
                    } else {
                        SequenceParseState::Dcs
                    };
                }
            }
        }
        None
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.osc_buf.len() > 131074
            || self.apc_buf.len() > 16 * 1024 * 1024 + 2
            || self.dcs_buf.len() > crate::parser::sixel::MAX_SIXEL_BYTES + 2
            || self.csi_watcher.param_count > self.csi_watcher.params.len()
            || self.cwd.as_ref().is_some_and(|path| !path.is_absolute())
        {
            return Err("Invalid terminal reader continuation".into());
        }
        Ok(())
    }
}

#[derive(Default)]
struct ReaderIngress {
    processor: Processor<StdSyncHandler>,
    side: SideCheckpoint,
}

pub struct TerminalState {
    ingress: Arc<ParkingMutex<ReaderIngress>>,
    term: Arc<ParkingMutex<alacritty_terminal::term::Term<EventListenerProxy>>>,
    render_generation: Arc<AtomicU64>,
    remote_resize_sender: Option<async_channel::Sender<(usize, usize)>>,
    writer: Arc<ParkingMutex<Box<dyn Write + Send>>>,
    master: Arc<ParkingMutex<Box<dyn MasterPty + Send>>>,
    shell_pid: Option<u32>,
    remote_shell_pid: Option<u32>,
    source_id: u64,
    program_status: Arc<ParkingMutex<crate::program_status::ProgramStatusStore>>,
    program_status_pending: Arc<std::sync::atomic::AtomicBool>,
    /// Latest `OSC 9;4` progress report from the running program. `None`
    /// means the program cleared it or never reported one.
    progress: Arc<ParkingMutex<Option<ProgressReport>>>,
    /// Wall-clock milliseconds (UNIX epoch) of the last PTY output chunk.
    /// The silence watcher compares this against its threshold.
    last_output_ms: Arc<std::sync::atomic::AtomicU64>,
    /// Kitty keyboard protocol state: current flags plus the push/pop stack.
    kitty: Arc<ParkingMutex<crate::kitty_keyboard::KittyKeyboard>>,
    /// True between `OSC 133` command-start and command-end markers.
    command_running: Arc<std::sync::atomic::AtomicBool>,
    pub total_lines_pushed: Arc<AtomicU64>,
    pub prompt_lines: Arc<ParkingMutex<Vec<u64>>>,
    pub image_store: Arc<ParkingMutex<ImageStore>>,
    event_listener: EventListenerProxy,
    /// Raw PTY byte stream, fanned out to anyone subscribed via
    /// `subscribe_output` (currently just `crate::daemon`'s attach clients).
    /// Closed receivers are pruned lazily on the next write.
    output_hub: Arc<OutputHub>,
    /// Flipped to `false` by the reader thread once it sees EOF/an error on
    /// the PTY (i.e. the shell exited). Distinct from `is_process_running`,
    /// which asks whether the shell has an active *foreground child* — this
    /// asks whether the pane's shell process is there at all.
    alive: Arc<std::sync::atomic::AtomicBool>,
    has_terminated: std::sync::atomic::AtomicBool,
    remote_session_id: Option<usize>,
    remote_session_identity: Option<crate::daemon::SessionIdentity>,
    remote_control: Option<Arc<crate::session_host::RemoteControl>>,
    _cwd_observation: Option<crate::session_host::CwdObservation>,
}

#[derive(Default)]
struct OutputHub {
    boundary: ParkingMutex<()>,
    subscribers: ParkingMutex<Vec<async_channel::Sender<Vec<u8>>>>,
}

const OUTPUT_SUBSCRIBER_CAPACITY: usize = 256;

impl OutputHub {
    fn subscribe(&self) -> async_channel::Receiver<Vec<u8>> {
        let _boundary = self.boundary.lock();
        self.subscribe_at_boundary()
    }

    fn snapshot_and_subscribe<T>(
        &self,
        snapshot: impl FnOnce() -> T,
    ) -> (T, async_channel::Receiver<Vec<u8>>) {
        let _boundary = self.boundary.lock();
        let snapshot = snapshot();
        (snapshot, self.subscribe_at_boundary())
    }

    #[cfg(test)]
    fn publish(&self, bytes: &[u8], apply_to_terminal: impl FnOnce()) {
        let _boundary = self.boundary.lock();
        apply_to_terminal();
        self.publish_at_boundary(bytes);
    }

    fn publish_at_boundary(&self, bytes: &[u8]) {
        let chunk = bytes.to_vec();
        self.subscribers.lock().retain(|subscriber| {
            match subscriber.try_send(chunk.clone()) {
                Ok(()) => true,
                // A slow subscriber must not retain unbounded PTY output.
                // Drop it on either a full queue or a closed receiver.
                Err(_) => false,
            }
        });
    }

    fn subscribe_at_boundary(&self) -> async_channel::Receiver<Vec<u8>> {
        let (sender, receiver) = async_channel::bounded(OUTPUT_SUBSCRIBER_CAPACITY);
        self.subscribers.lock().push(sender);
        receiver
    }
}

fn remote_session_needs_close_confirmation(
    remote_session_id: Option<usize>,
    has_foreground_process: bool,
) -> bool {
    remote_session_id.is_some() && has_foreground_process
}

#[cfg(unix)]
fn foreground_process_state(
    shell_pgid: libc::pid_t,
    foreground_pgid: libc::pid_t,
    has_foreground_child: bool,
) -> bool {
    match (shell_pgid > 0, foreground_pgid > 0) {
        (true, true) => shell_pgid != foreground_pgid || has_foreground_child,
        _ => true,
    }
}

fn append_restored_terminal_modes(out: &mut Vec<u8>, mode: alacritty_terminal::term::TermMode) {
    use alacritty_terminal::term::TermMode;

    if mode.contains(TermMode::APP_CURSOR) {
        out.extend_from_slice(b"\x1b[?1h");
    }
    if mode.contains(TermMode::APP_KEYPAD) {
        out.extend_from_slice(b"\x1b=");
    }
    if !mode.contains(TermMode::SHOW_CURSOR) {
        out.extend_from_slice(b"\x1b[?25l");
    }
    if mode.contains(TermMode::MOUSE_MOTION) {
        out.extend_from_slice(b"\x1b[?1003h");
    } else if mode.contains(TermMode::MOUSE_DRAG) {
        out.extend_from_slice(b"\x1b[?1002h");
    } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
        out.extend_from_slice(b"\x1b[?1000h");
    }
    if mode.contains(TermMode::SGR_MOUSE) {
        out.extend_from_slice(b"\x1b[?1006h");
    }
    if mode.contains(TermMode::FOCUS_IN_OUT) {
        out.extend_from_slice(b"\x1b[?1004h");
    }
    if mode.contains(TermMode::BRACKETED_PASTE) {
        out.extend_from_slice(b"\x1b[?2004h");
    }
}

const PROMPT_TERMINAL_RESET: &[u8] =
    b"\x1b[0m\x1b[?25h\x1b[?1l\x1b>\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?2004l\x1b[?47l\x1b[?1047l\x1b[?1049l";
const IDLE_SESSION_MODE_RESET: &[u8] =
    b"\x1b[0m\x1b[?25h\x1b[?1l\x1b>\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?2004l";

#[cfg(unix)]
fn append_legacy_mouse_modes(out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b[?1003h\x1b[?1006h\x1b[?1004h\x1b[?2004h");
}

#[cfg(unix)]
fn is_missing_host_session_error(error: &str) -> bool {
    error.starts_with("no_such_session: ") || error.starts_with("host_instance_changed: ")
}

struct RemoteInputWriter {
    sender: std::sync::mpsc::SyncSender<Vec<u8>>,
    failed: Arc<std::sync::atomic::AtomicBool>,
    queued_bytes: Arc<std::sync::atomic::AtomicUsize>,
    warned_queue_full: bool,
}

const REMOTE_WRITE_QUEUE_CAPACITY: usize = 128;
const REMOTE_WRITE_QUEUE_BYTE_LIMIT: usize = 4 * 1024 * 1024;

fn remote_worker_start_error(
    remote_control: Option<&Arc<crate::session_host::RemoteControl>>,
    session_id: Option<usize>,
    spawned_here: bool,
    error: std::io::Error,
) -> anyhow::Error {
    let message = format!("could not start session worker: {error}");
    if spawned_here {
        if let (Some(remote), Some(session_id)) = (remote_control, session_id) {
            if let Err(cleanup_error) = remote.stop_with_result() {
                return anyhow::Error::new(
                    crate::session_host::SpawnAttachError::orphaned_session(
                        session_id,
                        format!("{message}; cleanup failed: {cleanup_error}"),
                    ),
                );
            }
        }
    }
    anyhow::Error::msg(message)
}

impl Write for RemoteInputWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.failed.load(Ordering::Acquire) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "session host writer stopped",
            ));
        }
        let len = bytes.len();
        let reservation =
            self.queued_bytes
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                    queued
                        .checked_add(len)
                        .filter(|total| *total <= REMOTE_WRITE_QUEUE_BYTE_LIMIT)
                });
        if reservation.is_err() {
            if !self.warned_queue_full {
                eprintln!("Fastty session input write queue is full; dropping terminal response");
                self.warned_queue_full = true;
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "session host write queue is full",
            ));
        }
        match self.sender.try_send(bytes.to_vec()) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                self.queued_bytes.fetch_sub(len, Ordering::AcqRel);
                if !self.warned_queue_full {
                    eprintln!(
                        "Fastty session input write queue is full; dropping terminal response"
                    );
                    self.warned_queue_full = true;
                }
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "session host write queue is full",
                ));
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.queued_bytes.fetch_sub(len, Ordering::AcqRel);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "session host writer stopped",
                ));
            }
        }
        self.warned_queue_full = false;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl TerminalState {
    pub fn new_headless(
        executable: &str,
        exec_args: &[String],
        cwd: Option<&str>,
        cols: usize,
        rows: usize,
    ) -> anyhow::Result<Self> {
        let cols = cols.max(10);
        let rows = rows.max(5);
        let cell_w = 9.0;
        let cell_h = 18.0;
        let viewport_w = (cols as f32) * cell_w;
        let viewport_h = (rows as f32) * cell_h;
        Self::new(
            executable,
            exec_args,
            cwd,
            10_000,
            FontConfig::default(),
            cell_w,
            cell_h,
            viewport_w,
            viewport_h,
            crate::event_listener::EventSender::None,
        )
    }

    pub fn new(
        executable: &str,
        exec_args: &[String],
        cwd: Option<&str>,
        scrollback: usize,
        _font_config: FontConfig,
        cell_width: f32,
        cell_height: f32,
        viewport_width: f32,
        viewport_height: f32,
        sender: crate::event_listener::EventSender,
    ) -> anyhow::Result<Self> {
        Self::new_internal(
            executable,
            exec_args,
            cwd,
            scrollback,
            _font_config,
            cell_width,
            cell_height,
            viewport_width,
            viewport_height,
            sender,
            None,
            false,
        )
    }

    #[cfg(unix)]
    pub fn new_hosted(
        executable: &str,
        exec_args: &[String],
        cwd: Option<&str>,
        scrollback: usize,
        font_config: FontConfig,
        cell_width: f32,
        cell_height: f32,
        viewport_width: f32,
        viewport_height: f32,
        sender: crate::event_listener::EventSender,
    ) -> anyhow::Result<Self> {
        let cols = ((viewport_width as usize) / (cell_width as usize).max(1)).max(80);
        let rows = ((viewport_height as usize) / (cell_height as usize).max(1)).max(24);
        let remote = crate::session_host::spawn_attached(executable, exec_args, cwd, cols, rows)
            .map_err(anyhow::Error::new)?;
        Self::new_internal(
            executable,
            exec_args,
            cwd,
            scrollback,
            font_config,
            cell_width,
            cell_height,
            viewport_width,
            viewport_height,
            sender,
            Some(remote),
            true,
        )
    }

    #[cfg(not(unix))]
    pub fn new_hosted(
        executable: &str,
        exec_args: &[String],
        cwd: Option<&str>,
        scrollback: usize,
        font_config: FontConfig,
        cell_width: f32,
        cell_height: f32,
        viewport_width: f32,
        viewport_height: f32,
        sender: crate::event_listener::EventSender,
    ) -> anyhow::Result<Self> {
        Self::new(
            executable,
            exec_args,
            cwd,
            scrollback,
            font_config,
            cell_width,
            cell_height,
            viewport_width,
            viewport_height,
            sender,
        )
    }

    /// Attach a GUI projection to an existing host session. `Ok(None)` means
    /// the saved ID is no longer present and the caller can start one shell.
    #[cfg(unix)]
    pub fn attach_hosted(
        session_id: usize,
        expected_identity: Option<crate::daemon::SessionIdentity>,
        executable: &str,
        exec_args: &[String],
        cwd: Option<&str>,
        scrollback: usize,
        font_config: FontConfig,
        cell_width: f32,
        cell_height: f32,
        viewport_width: f32,
        viewport_height: f32,
        sender: crate::event_listener::EventSender,
    ) -> anyhow::Result<Option<Self>> {
        let remote = match crate::session_host::attach_reference(session_id, expected_identity) {
            Ok(remote) => remote,
            Err(error) if is_missing_host_session_error(&error) => return Ok(None),
            Err(error) => return Err(anyhow::Error::msg(error)),
        };
        Self::attach_hosted_from_remote(
            remote,
            executable,
            exec_args,
            cwd,
            scrollback,
            font_config,
            cell_width,
            cell_height,
            viewport_width,
            viewport_height,
            sender,
        )
        .map(Some)
    }

    /// Build a GUI projection after the host attachment completes off thread.
    #[cfg(unix)]
    pub fn attach_hosted_from_remote(
        remote: crate::session_host::RemoteSession,
        executable: &str,
        exec_args: &[String],
        cwd: Option<&str>,
        scrollback: usize,
        font_config: FontConfig,
        cell_width: f32,
        cell_height: f32,
        viewport_width: f32,
        viewport_height: f32,
        sender: crate::event_listener::EventSender,
    ) -> anyhow::Result<Self> {
        let terminal = Self::new_internal(
            executable,
            exec_args,
            cwd,
            scrollback,
            font_config,
            cell_width,
            cell_height,
            viewport_width,
            viewport_height,
            sender,
            Some(remote),
            false,
        )?;
        if !terminal.program_status_snapshot().is_empty() {
            terminal
                .program_status_pending
                .store(true, Ordering::Relaxed);
            terminal
                .event_listener
                .send_app_event(AppEvent::ProgramStatusChanged {
                    source_id: terminal.source_id,
                });
        }
        Ok(terminal)
    }

    fn new_internal(
        executable: &str,
        exec_args: &[String],
        cwd: Option<&str>,
        scrollback: usize,
        _font_config: FontConfig,
        cell_width: f32,
        cell_height: f32,
        viewport_width: f32,
        viewport_height: f32,
        sender: crate::event_listener::EventSender,
        remote: Option<crate::session_host::RemoteSession>,
        spawned_here: bool,
    ) -> anyhow::Result<Self> {
        let remote_shell_pid = remote.as_ref().and_then(|session| session.shell_pid);
        let initial_cwd = remote
            .as_ref()
            .and_then(|session| session.cwd.as_deref())
            .or(cwd)
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_absolute());
        let restored_idle_session = remote
            .as_ref()
            .is_some_and(|remote| remote.process_running == Some(false));
        let restored_program_status = remote
            .as_ref()
            .map(|remote| remote.program_status.clone())
            .unwrap_or_default();
        let cell_w = (cell_width as usize).max(1);
        let cell_h = (cell_height as usize).max(1);
        let binary_snapshot = match remote.as_ref() {
            Some(remote) if remote.snapshot.starts_with(b"FST1") => Some(
                crate::server::binary_snapshot::decode_snapshot(&remote.snapshot).ok_or_else(
                    || anyhow::anyhow!("session host sent an invalid binary snapshot"),
                )?,
            ),
            _ => None,
        };
        if binary_snapshot
            .as_ref()
            .is_some_and(|(header, _)| header.cols == 0 || header.rows == 0)
        {
            return Err(anyhow::anyhow!(
                "session host sent an invalid binary snapshot"
            ));
        }
        let native_snapshot = match remote.as_ref() {
            Some(remote)
                if remote
                    .snapshot
                    .starts_with(crate::server::native_snapshot::MAGIC) =>
            {
                Some(
                    crate::server::native_snapshot::decode_state(&remote.snapshot)
                        .map_err(anyhow::Error::msg)?,
                )
            }
            _ => None,
        };
        let has_binary_snapshot = binary_snapshot.is_some() || native_snapshot.is_some();
        let cols = binary_snapshot
            .as_ref()
            .map(|(header, _)| header.cols as usize)
            .or_else(|| {
                native_snapshot
                    .as_ref()
                    .map(|state| state.terminal.main.columns)
            })
            .unwrap_or_else(|| ((viewport_width as usize) / cell_w).max(80));
        let rows = binary_snapshot
            .as_ref()
            .map(|(header, _)| header.rows as usize)
            .or_else(|| {
                native_snapshot
                    .as_ref()
                    .map(|state| state.terminal.main.screen_lines)
            })
            .unwrap_or_else(|| ((viewport_height as usize) / cell_h).max(24));

        let config = AlacrittyConfig {
            scrolling_history: scrollback.clamp(500, 100_000),
            ..Default::default()
        };
        let size = TermSize::new(cols, rows);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: (cols as u16).saturating_mul(cell_width as u16),
                pixel_height: (rows as u16).saturating_mul(cell_height as u16),
            })
            .expect("Failed to open PTY");

        // Determine the shell binary name (e.g. "zsh", "bash", "fish") so we can
        // choose the right CommandBuilder constructor and integration strategy.
        let _shell_name = std::path::Path::new(&executable)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // On macOS, when launched as a .app bundle from Finder/Dock/Spotlight,
        // the process inherits a minimal launchd PATH instead of the full login-shell
        // PATH. Using new_default_prog() for zsh activates portable-pty's argv0 `-`
        // trick (e.g. `-zsh`), which makes the kernel treat the process as a login
        // shell so /etc/zprofile (and path_helper) run correctly.
        //
        // Restriction: new_default_prog() panics if .arg()/.args() is called on it,
        // so we only use it for the zsh-without-exec-args branch. bash and fish still
        // use .args() for their --rcfile / -C wrappers and must stay on new().
        let source_id = NEXT_SOURCE_ID.fetch_add(1, Ordering::Relaxed);
        let mut cmd = CommandBuilder::new(executable);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "fastty");
        cmd.env("FASTTY_SESSION_TOKEN", source_id.to_string());
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        if let Ok(lang) = std::env::var("LANG") {
            cmd.env("LANG", lang);
        }
        if let Ok(path) = std::env::var("PATH") {
            #[cfg(target_os = "macos")]
            {
                let mut full_path = path;
                for p in [
                    "/opt/homebrew/bin",
                    "/opt/homebrew/sbin",
                    "/usr/local/bin",
                    "/usr/local/sbin",
                ] {
                    if !full_path.split(':').any(|segment| segment == p)
                        && std::path::Path::new(p).exists()
                    {
                        full_path = format!("{}:{}", p, full_path);
                    }
                }
                cmd.env("PATH", full_path);
            }
            #[cfg(not(target_os = "macos"))]
            {
                cmd.env("PATH", path);
            }
        }

        // Shell integration: write OSC 133 command markers so fastty can
        // detect command start/finish for duration tracking.
        //
        // The integration scripts are identical for every tab and never change
        // between releases, so materialise them at most once per process. This
        // avoids redundant filesystem writes on every `create_new_tab` call —
        // which matters at startup when a saved session is restored and many
        // tabs are created back-to-back.
        static INTEGRATION_WRITTEN: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        let integration_dir = crate::paths::get().cache_dir.join("shell_integration");
        let integration_path = integration_dir.join("fastty_shell_integration.sh");
        let integration_path_zsh = integration_dir.join("fastty_shell_integration.zsh");
        let integration_path_fish = integration_dir.join("fastty_shell_integration.fish");
        let zdotdir = integration_dir.join("fastty_zsh");
        INTEGRATION_WRITTEN.get_or_init(|| {
            let _ = std::fs::create_dir_all(&integration_dir);
            let _ = std::fs::create_dir_all(&zdotdir);

            // POSIX shell integration (bash)
            let _ = std::fs::write(
                &integration_path,
                "# fastty shell integration — OSC 133 command markers\n\
                 __fastty_cmd_start() {\n\
                 \techo -ne \"\\e]133;B\\e\\\\\"\n\
                 }\n\
                 __fastty_prompt() {\n\
                 \techo -ne \"\\e]133;D;$?\\e\\\\\"\n\
                 \techo -ne \"\\e]133;A\\e\\\\\"\n\
                 \techo -ne \"\\e[?1000l\\e[?1002l\\e[?1003l\\e[?1006l\"\n\
                 }\n\
                 PROMPT_COMMAND=\"__fastty_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"\n\
                 trap '__fastty_cmd_start' DEBUG\n",
            );

            // Zsh shell integration: precmd/preexec hooks appended via add-zsh-hook
            // so user-defined hooks in ~/.zshrc are preserved, not overwritten.
            let _ = std::fs::write(
                &integration_path_zsh,
                "# fastty shell integration — OSC 133 command markers\n\
                 autoload -Uz add-zsh-hook\n\
                 __fastty_preexec() {\n\
                 \techo -ne \"\\e]133;B\\e\\\\\"\n\
                 }\n\
                 __fastty_precmd() {\n\
                 \techo -ne \"\\e]133;D;$?\\e\\\\\"\n\
                 \techo -ne \"\\e]133;A\\e\\\\\"\n\
                 \techo -ne \"\\e[?1000l\\e[?1002l\\e[?1003l\\e[?1006l\"\n\
                 }\n\
                 add-zsh-hook preexec __fastty_preexec\n\
                 add-zsh-hook precmd __fastty_precmd\n",
            );

            // Fish shell integration
            let _ = std::fs::write(
                &integration_path_fish,
                "# fastty shell integration — OSC 133 command markers\n\
                 function __fastty_cmd_start --on-event fish_preexec\n\
                 \techo -ne \"\\e]133;B\\e\\\\\"\n\
                 end\n\
                 function __fastty_cmd_end --on-event fish_postexec\n\
                 \techo -ne \"\\e]133;D;$status\\e\\\\\"\n\
                 end\n\
                 function __fastty_prompt --on-event fish_prompt\n\
                 \techo -ne \"\\e]133;A\\e\\\\\"\n\
                 \techo -ne \"\\e[?1000l\\e[?1002l\\e[?1003l\\e[?1006l\"\n\
                 end\n",
            );

            // Bash wrapper
            let wrapper_path_bash = integration_dir.join("fastty_bashrc");
            let _ = std::fs::write(
                &wrapper_path_bash,
                format!(
                    "# Auto-generated by fastty — shell integration wrapper\n\
                     [ -f \"$HOME/.bash_profile\" ] && . \"$HOME/.bash_profile\"\n\
                     [ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"\n\
                     [ -f \"$HOME/.profile\" ] && . \"$HOME/.profile\"\n\
                     . \"{}\"\n",
                    integration_path.display(),
                ),
            );

            // Zsh wrappers
            let _ = std::fs::write(
                zdotdir.join(".zshenv"),
                "# Auto-generated by fastty\n\
                 [ -f \"$HOME/.zshenv\" ] && . \"$HOME/.zshenv\"\n",
            );
            let _ = std::fs::write(
                zdotdir.join(".zprofile"),
                "# Auto-generated by fastty\n\
                 [ -f \"$HOME/.zprofile\" ] && . \"$HOME/.zprofile\"\n",
            );
            let _ = std::fs::write(
                zdotdir.join(".zshrc"),
                format!(
                    "# Auto-generated by fastty\n\
                     [ -f \"$HOME/.zshrc\" ] && . \"$HOME/.zshrc\"\n\
                     . \"{}\"\n\
                     if [ -n \"$FASTTY_ORIG_ZDOTDIR\" ]; then\n\
                     \texport ZDOTDIR=\"$FASTTY_ORIG_ZDOTDIR\"\n\
                     else\n\
                     \tunset ZDOTDIR\n\
                     fi\n",
                    integration_path_zsh.display(),
                ),
            );
            let _ = std::fs::write(
                zdotdir.join(".zlogin"),
                "# Auto-generated by fastty\n\
                 [ -f \"$HOME/.zlogin\" ] && . \"$HOME/.zlogin\"\n",
            );
        });

        // Wrap the shell command to source the integration before launching.
        if !exec_args.is_empty() {
            cmd.args(exec_args);
        } else {
            #[cfg(unix)]
            {
                let shell_name = std::path::Path::new(&executable)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if matches!(shell_name, "zsh" | "bash" | "fish" | "sh" | "dash" | "ksh") {
                    cmd.arg("-l");
                }
            }
        }

        let no_spawn = remote.is_some() || std::env::var("FASTTY_NO_SPAWN").is_ok();
        let child = if no_spawn {
            None
        } else {
            Some(pair.slave.spawn_command(cmd)?)
        };
        let shell_pid = child.as_ref().and_then(|c| c.process_id());
        let reader_shell_pid = shell_pid;

        drop(pair.slave);

        let master: Box<dyn MasterPty + Send> = pair.master;
        let master_arc: Arc<ParkingMutex<Box<dyn MasterPty + Send>>> =
            Arc::new(ParkingMutex::new(master));
        let remote_session_id = remote.as_ref().map(|session| session.id);
        let remote_session_identity = remote.as_ref().and_then(|session| session.identity.clone());
        let remote_control = remote.as_ref().map(|session| Arc::clone(&session.control));
        #[cfg(unix)]
        let (mut reader, remote_output) = if remote.is_some() {
            let (read_end, write_end) = std::os::unix::net::UnixStream::pair()?;
            (
                Box::new(read_end) as Box<dyn std::io::Read + Send>,
                Some(write_end),
            )
        } else {
            let reader = master_arc
                .lock()
                .try_clone_reader()
                .expect("Failed to clone reader");
            (reader, None)
        };
        #[cfg(not(unix))]
        let mut reader = master_arc
            .lock()
            .try_clone_reader()
            .expect("Failed to clone reader");

        let writer_boxed: Box<dyn Write + Send> = match &remote_control {
            Some(control) => {
                let (sender, receiver) =
                    std::sync::mpsc::sync_channel::<Vec<u8>>(REMOTE_WRITE_QUEUE_CAPACITY);
                let control = Arc::clone(control);
                let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let worker_failed = Arc::clone(&failed);
                let queued_bytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let worker_queued_bytes = Arc::clone(&queued_bytes);
                let worker = std::thread::Builder::new()
                    .name("fastty-session-writer".into())
                    .spawn(move || {
                        while let Ok(bytes) = receiver.recv() {
                            let result = control.write(&bytes);
                            worker_queued_bytes.fetch_sub(bytes.len(), Ordering::AcqRel);
                            if let Err(error) = result {
                                worker_failed.store(true, Ordering::Release);
                                eprintln!("Fastty session input write failed: {error}");
                                break;
                            }
                        }
                    });
                if let Err(error) = worker {
                    return Err(remote_worker_start_error(
                        remote_control.as_ref(),
                        remote_session_id,
                        spawned_here,
                        error,
                    ));
                }
                Box::new(RemoteInputWriter {
                    sender,
                    failed,
                    queued_bytes,
                    warned_queue_full: false,
                })
            }
            None => Box::new(
                master_arc
                    .lock()
                    .take_writer()
                    .expect("Failed to take writer"),
            ),
        };
        let writer_arc: Arc<ParkingMutex<Box<dyn Write + Send>>> =
            Arc::new(ParkingMutex::new(writer_boxed));

        let output_hub = Arc::new(OutputHub::default());
        let output_hub_clone = Arc::clone(&output_hub);
        let alive: Arc<std::sync::atomic::AtomicBool> =
            Arc::new(std::sync::atomic::AtomicBool::new(true));
        let alive_clone = Arc::clone(&alive);

        let event_listener = EventListenerProxy::from_arc(writer_arc.clone());
        // Daemon-attached terminals share the child's output with the session
        // host, whose own terminal instance already answers every terminal
        // query once. Replying again from here doubles every response (cursor
        // position, OSC 10/11 colors) and breaks prompt libraries.
        event_listener.set_replies_enabled(remote_control.is_none());
        event_listener.set_event_sender(sender.clone());
        let term = Arc::new(ParkingMutex::new(alacritty_terminal::term::Term::new(
            config,
            &size,
            event_listener.clone(),
        )));
        let mut restored_kitty_flags = 0u8;
        if let Some((header, cells)) = binary_snapshot.as_ref() {
            use crate::server::binary_snapshot::SNAPSHOT_FLAG_ALT_SCREEN;
            let mut term = term.lock();
            let mut parser: Processor<StdSyncHandler> = Processor::new();
            if header.flags & SNAPSHOT_FLAG_ALT_SCREEN != 0 {
                parser.advance(&mut *term, b"\x1b[?1049h");
            }
            if !apply_snapshot_to_grid(&mut term, header, cells) {
                return Err(anyhow::anyhow!(
                    "session host sent an invalid binary snapshot"
                ));
            }
            let mode =
                alacritty_terminal::term::TermMode::from_bits_truncate(header.terminal_modes());
            let mut mode_commands = Vec::new();
            append_restored_terminal_modes(&mut mode_commands, mode);
            let kitty_flags = header.kitty_flags();
            if kitty_flags != 0 {
                // Re-arm the keyboard flags the program had at capture time,
                // so a re-attached client keeps encoding keys correctly.
                mode_commands.extend_from_slice(format!("\x1b[={};1u", kitty_flags).as_bytes());
            }
            restored_kitty_flags = kitty_flags;
            parser.advance(&mut *term, &mode_commands);
            if restored_idle_session {
                parser.advance(&mut *term, IDLE_SESSION_MODE_RESET);
            }
        }
        drop(binary_snapshot);
        let ingress = Arc::new(ParkingMutex::new(ReaderIngress::default()));
        if let Some(snapshot) = native_snapshot {
            let mut term = term.lock();
            term.restore_snapshot(snapshot.terminal)
                .map_err(anyhow::Error::new)?;
            if let Some(continuation) = snapshot.continuation {
                let mut ingress = ingress.lock();
                ingress.processor =
                    Processor::from_snapshot(continuation.parser).map_err(anyhow::Error::msg)?;
                ingress.side = continuation.side;
            }
            if restored_idle_session {
                let mut parser: Processor<StdSyncHandler> = Processor::new();
                parser.advance(&mut *term, IDLE_SESSION_MODE_RESET);
            }
        }

        {
            let mut ingress = ingress.lock();
            if ingress.side.cwd.is_none() {
                ingress.side.cwd = initial_cwd;
            }
        }

        let render_generation = Arc::new(AtomicU64::new(0));
        let render_gen_clone = Arc::clone(&render_generation);
        let term_clone = Arc::clone(&term);
        let ingress_clone = Arc::clone(&ingress);
        let event_listener_clone = event_listener.clone();
        let total_lines_pushed = Arc::new(AtomicU64::new(0));
        let total_lines_pushed_clone = Arc::clone(&total_lines_pushed);
        let prompt_lines = Arc::new(ParkingMutex::new(Vec::new()));
        let prompt_lines_clone = Arc::clone(&prompt_lines);
        let image_store = Arc::new(ParkingMutex::new(ImageStore::new()));
        let image_store_clone = Arc::clone(&image_store);
        let next_image_id = Arc::new(AtomicU64::new(1));
        let next_image_id_clone = Arc::clone(&next_image_id);
        let writer_clone = Arc::clone(&writer_arc);
        let program_status = Arc::new(ParkingMutex::new(
            crate::program_status::ProgramStatusStore::from_records(restored_program_status),
        ));
        let program_status_clone = Arc::clone(&program_status);
        let program_status_pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let program_status_pending_clone = Arc::clone(&program_status_pending);
        let progress = Arc::new(ParkingMutex::new(None));
        let progress_clone = Arc::clone(&progress);
        let last_output_ms = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let last_output_ms_clone = Arc::clone(&last_output_ms);
        let command_running = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let command_running_clone = Arc::clone(&command_running);
        let kitty = Arc::new(ParkingMutex::new(
            crate::kitty_keyboard::KittyKeyboard::default(),
        ));
        let kitty_clone = Arc::clone(&kitty);
        if restored_kitty_flags != 0 {
            kitty.lock().set(restored_kitty_flags, 1);
        }
        if !std::env::var("FASTTY_NO_READER").is_ok() {
            thread::spawn(move || {
                use std::io::Read;

                let mut buf = [0u8; 65536];
                let mut kitty_reassembler = crate::parser::kitty_graphics::KittyReassembler::new();
                let mut kitty_clip_write: Option<crate::parser::kitty_clipboard::WriteSession> =
                    None;
                let mut cmd_start_time: Option<std::time::Instant> = None;

                // CSI detection: see `CsiWatcher` (module level). The watcher
                // emits events for `CSI 2J` scrollback wipes and for Synchronized
                // Output mode 2026, which alacritty_terminal 0.24 ignores.
                let mut clear_history_flag = false;

                const SYNC_OUTPUT_TIMEOUT: std::time::Duration =
                    std::time::Duration::from_millis(150);
                let mut sync_output_active = false;
                let mut sync_started: Option<std::time::Instant> = None;

                // Debug: capture PTY bytes to log if FASTTY_PTY_DEBUG=1
                let pty_debug = std::env::var("FASTTY_PTY_DEBUG").ok().as_deref() == Some("1");
                let pty_log_path = crate::paths::get().state_dir.join("fastty_pty_debug.log");
                if pty_debug {
                    let _ = std::fs::write(&pty_log_path, ""); // truncate on start
                }

                struct PendingNotification {
                    title: String,
                    body: String,
                }
                let mut pending_notifications: std::collections::HashMap<
                    String,
                    PendingNotification,
                > = std::collections::HashMap::new();

                let mut handle_cmd = |cmd: OscCommand,
                                      cursor_line: i32,
                                      screen_lines: i32,
                                      base: u64,
                                      term: &mut alacritty_terminal::Term<EventListenerProxy>,
                                      parser: &mut Processor| {
                    match cmd {
                        OscCommand::ProgramStatus(
                            crate::program_status::ProgramStatusCommand::Query,
                        ) => {
                            let mut writer = writer_clone.lock();
                            let _ = writer.write_all(
                                b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\",
                            );
                            let _ = writer.flush();
                        }
                        OscCommand::ProgramStatus(command) => {
                            eprintln!("Fastty program status OSC parsed: {command:?}");
                            update_program_status(
                                &program_status_clone,
                                &program_status_pending_clone,
                                &event_listener_clone,
                                source_id,
                                |store| store.apply(command),
                            );
                        }
                        OscCommand::Progress(report) => {
                            *progress_clone.lock() = report;
                            event_listener_clone
                                .send_app_event(AppEvent::ProgressChanged(report));
                        }
                        OscCommand::PromptStarted => {
                            update_program_status(
                                &program_status_clone,
                                &program_status_pending_clone,
                                &event_listener_clone,
                                source_id,
                                |store| store.end_activity(),
                            );
                            // Reset terminal modes that a child may leave active before the next prompt.
                            parser.advance(term, PROMPT_TERMINAL_RESET);

                            let scrolled = (base as i64 - screen_lines as i64).max(0);
                            let absolute_line = (scrolled + cursor_line as i64).max(0) as u64;
                            {
                                let mut p = prompt_lines_clone.lock();
                                if p.last().copied() != Some(absolute_line) {
                                    p.push(absolute_line);
                                    if p.len() > 500 {
                                        let excess = p.len() - 500;
                                        p.drain(0..excess);
                                    }
                                }
                            }
                            dispatch_osc_action(
                                &OscCommand::PromptStarted,
                                &event_listener_clone,
                                cursor_line,
                                base,
                                screen_lines,
                            );
                        }
                        OscCommand::NotificationQuery { id } => {
                            let resp = format!("\x1b]99;i={}:p=OK;\x1b\\", id);
                            let mut w = writer_clone.lock();
                            let _ = w.write_all(resp.as_bytes());
                            let _ = w.flush();
                        }
                        OscCommand::NotificationFragment {
                            id,
                            p_type,
                            done,
                            payload,
                        } => {
                            if let Some(query_id) = id {
                                let entry = pending_notifications
                                    .entry(query_id.clone())
                                    .or_insert_with(|| PendingNotification {
                                        title: String::new(),
                                        body: String::new(),
                                    });
                                match p_type.as_deref() {
                                    Some("title") => entry.title.push_str(&payload),
                                    Some("body") | None => entry.body.push_str(&payload),
                                    _ => {}
                                }
                                if done {
                                    if let Some(finished) = pending_notifications.remove(&query_id)
                                    {
                                        let title = if finished.title.is_empty() {
                                            "Fastty".to_string()
                                        } else {
                                            finished.title
                                        };
                                        dispatch_notification_if_allowed(
                                            &title,
                                            &finished.body,
                                            &program_status_clone,
                                            &event_listener_clone,
                                            cursor_line,
                                            base,
                                            screen_lines,
                                        );
                                    }
                                }
                            } else {
                                dispatch_notification_if_allowed(
                                    "Fastty",
                                    &payload,
                                    &program_status_clone,
                                    &event_listener_clone,
                                    cursor_line,
                                    base,
                                    screen_lines,
                                );
                            }
                        }
                        OscCommand::KittyClipboard { metadata, payload } => {
                            use crate::parser::kitty_clipboard as kclip;
                            let Some(msg) = kclip::parse(&metadata, &payload) else {
                                return;
                            };
                            let write_resp = |resp: String| {
                                let mut w = writer_clone.lock();
                                let _ = w.write_all(resp.as_bytes());
                                let _ = w.flush();
                            };
                            match msg {
                                kclip::Message::WriteBegin { id } => {
                                    kitty_clip_write = Some(kclip::WriteSession::new(id));
                                }
                                kclip::Message::WriteData { id, mime, data } => {
                                    match (mime, kitty_clip_write.as_mut()) {
                                        (Some(mime), Some(session)) => {
                                            if session.push(mime, data).is_err() {
                                                let id = kitty_clip_write.take().and_then(|s| s.id);
                                                write_resp(kclip::response(
                                                    "write",
                                                    "EFBIG",
                                                    id.as_deref(),
                                                ));
                                            }
                                        }
                                        (None, Some(_)) => {
                                            // End of transmission: apply to the clipboard.
                                            if let Some(session) = kitty_clip_write.take() {
                                                let id = session.id.clone();
                                                let status = if session.finish().is_ok() {
                                                    "DONE"
                                                } else {
                                                    "EIO"
                                                };
                                                write_resp(kclip::response(
                                                    "write",
                                                    status,
                                                    id.as_deref(),
                                                ));
                                            }
                                        }
                                        (_, None) => {
                                            write_resp(kclip::response(
                                                "write",
                                                "EINVAL",
                                                id.as_deref(),
                                            ));
                                        }
                                    }
                                }
                                kclip::Message::Read { id, request } => {
                                    if !crate::config::clipboard_read_allowed() {
                                        write_resp(kclip::response("read", "EPERM", id.as_deref()));
                                        return;
                                    }
                                    write_resp(kclip::response("read", "OK", id.as_deref()));
                                    for (mime, data) in kclip::read(&request) {
                                        let chunks: Vec<&[u8]> = if data.is_empty() {
                                            vec![&[][..]]
                                        } else {
                                            data.chunks(kclip::MAX_DATA_CHUNK).collect()
                                        };
                                        for chunk in chunks {
                                            write_resp(kclip::response_data(
                                                &mime,
                                                chunk,
                                                id.as_deref(),
                                            ));
                                        }
                                    }
                                    write_resp(kclip::response("read", "DONE", id.as_deref()));
                                }
                            }
                        }
                        OscCommand::Notification { title, body } => {
                            dispatch_notification_if_allowed(
                                &title,
                                &body,
                                &program_status_clone,
                                &event_listener_clone,
                                cursor_line,
                                base,
                                screen_lines,
                            );
                        }
                        OscCommand::CommandStarted => {
                            command_running_clone.store(true, Ordering::Relaxed);
                            dispatch_osc_action(
                                &cmd,
                                &event_listener_clone,
                                cursor_line,
                                base,
                                screen_lines,
                            );
                        }
                        OscCommand::CommandFinished { .. } => {
                            command_running_clone.store(false, Ordering::Relaxed);
                            dispatch_osc_action(
                                &cmd,
                                &event_listener_clone,
                                cursor_line,
                                base,
                                screen_lines,
                            );
                        }
                        _ => {
                            dispatch_osc_action(
                                &cmd,
                                &event_listener_clone,
                                cursor_line,
                                base,
                                screen_lines,
                            );
                        }
                    }
                };

                let mut handle_apc = |apc_data: &[u8],
                                      cursor_line: i32,
                                      cursor_col: usize,
                                      base: u64,
                                      term_locked: &mut alacritty_terminal::Term<
                    EventListenerProxy,
                >,
                                      parser: &mut alacritty_terminal::vte::ansi::Processor|
                 -> u64 {
                    if !apc_data.starts_with(b"G") {
                        return 0;
                    }
                    let body = &apc_data[1..];
                    let semicolon_pos = body.iter().position(|&b| b == b';');
                    let (header, payload_b64) = match semicolon_pos {
                        Some(pos) => (&body[..pos], &body[pos + 1..]),
                        None => (body, &[][..]),
                    };

                    let mut added_lines = 0u64;

                    if let Some(ctrl) = crate::parser::kitty_graphics::parse_kitty_control(header) {
                        match ctrl.action {
                            crate::parser::kitty_graphics::KittyAction::Query => {
                                let id = ctrl.image_id.unwrap_or(0);
                                let resp = format!("\x1b_Gi={};OK\x1b\\", id);
                                let mut w = writer_clone.lock();
                                let _ = w.write_all(resp.as_bytes());
                                let _ = w.flush();
                            }
                            crate::parser::kitty_graphics::KittyAction::Delete => {
                                let mut store = image_store_clone.lock();
                                match ctrl.delete_action {
                                Some(crate::parser::kitty_graphics::KittyDeleteAction::All) => {
                                    store.delete_all();
                                }
                                Some(crate::parser::kitty_graphics::KittyDeleteAction::ById(id)) => {
                                    store.delete_by_id(id);
                                }
                                Some(crate::parser::kitty_graphics::KittyDeleteAction::ByPlacement(p_id)) => {
                                    store.delete_by_placement(p_id);
                                }
                                _ => {
                                    store.delete_all();
                                }
                            }
                            }
                            crate::parser::kitty_graphics::KittyAction::Put => {
                                if let Some(id) = ctrl.image_id {
                                    let mut store = image_store_clone.lock();
                                    if let Some(img) = store.images.get(&id).cloned() {
                                        let abs_line =
                                            (base as i64 + cursor_line as i64).max(0) as u64;
                                        let cols = ctrl.columns.unwrap_or(10);
                                        let rows = ctrl.rows.unwrap_or(5);
                                        store.add_placement(ImagePlacement {
                                            placement_id: ctrl.placement_id.unwrap_or(0),
                                            image_id: id,
                                            absolute_line: abs_line,
                                            col: cursor_col,
                                            cols,
                                            rows,
                                            z_index: ctrl.z_index,
                                            image: img,
                                        });
                                    }
                                }
                            }
                            crate::parser::kitty_graphics::KittyAction::TransmitAndDisplay
                            | crate::parser::kitty_graphics::KittyAction::ImmediateDisplay => {
                                if let Some((final_ctrl, mut raw_bytes)) =
                                    kitty_reassembler.push_chunk(ctrl.clone(), payload_b64)
                                {
                                    // If transmission is file-based (t=f or t=t), read bytes from filesystem path
                                    if (final_ctrl.transmission == crate::parser::kitty_graphics::KittyTransmission::File
                                    || final_ctrl.transmission == crate::parser::kitty_graphics::KittyTransmission::TempFile)
                                    && !raw_bytes.is_empty()
                                {
                                    if let Ok(path_str) = std::str::from_utf8(&raw_bytes) {
                                        let trimmed = path_str.trim();
                                        if let Ok(file_content) = std::fs::read(trimmed) {
                                            raw_bytes = file_content;
                                        }
                                    }
                                }

                                    if let Ok((render_img, img_w, img_h)) =
                                        crate::parser::kitty_graphics::decode_image_data(
                                            final_ctrl.format,
                                            final_ctrl.width_px,
                                            final_ctrl.height_px,
                                            &raw_bytes,
                                        )
                                    {
                                        let img_id = final_ctrl.image_id.unwrap_or_else(|| {
                                            next_image_id_clone.fetch_add(1, Ordering::Relaxed)
                                                as u32
                                        });
                                        let mut store = image_store_clone.lock();
                                        store.add_image(img_id, render_img.clone());

                                        let abs_line =
                                            (base as i64 + cursor_line as i64).max(0) as u64;
                                        let cw = if cell_width > 0.0 { cell_width } else { 9.0 };
                                        let ch = if cell_height > 0.0 { cell_height } else { 18.0 };
                                        let cols = final_ctrl.columns.unwrap_or_else(|| {
                                            ((img_w as f32) / cw).ceil().max(1.0) as usize
                                        });
                                        let rows = final_ctrl.rows.unwrap_or_else(|| {
                                            ((img_h as f32) / ch).ceil().max(1.0) as usize
                                        });
                                        store.add_placement(ImagePlacement {
                                            placement_id: final_ctrl.placement_id.unwrap_or(0),
                                            image_id: img_id,
                                            absolute_line: abs_line,
                                            col: cursor_col,
                                            cols,
                                            rows,
                                            z_index: final_ctrl.z_index,
                                            image: render_img,
                                        });

                                        if final_ctrl.cursor_movement && rows > 0 {
                                            parser.advance(
                                                &mut *term_locked,
                                                &vec![0x0A; rows as usize],
                                            );
                                            added_lines = rows as u64;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    added_lines
                };

                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            let _output_boundary = output_hub_clone.boundary.lock();
                            update_program_status(
                                &program_status_clone,
                                &program_status_pending_clone,
                                &event_listener_clone,
                                source_id,
                                |store| store.end_activity(),
                            );
                            alive_clone.store(false, Ordering::Relaxed);
                            event_listener_clone.send_app_event(AppEvent::Exit {
                                shell_pid: reader_shell_pid,
                            });
                            break;
                        }
                        Ok(n) => {
                            let output_boundary = output_hub_clone.boundary.lock();
                            last_output_ms_clone.store(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis() as u64)
                                    .unwrap_or(0),
                                Ordering::Relaxed,
                            );

                            let mut term_locked = term_clone.lock();
                            let mut ingress = ingress_clone.lock();
                            let mut scanner = std::mem::take(&mut ingress.side);
                            if sync_output_active != scanner.sync_output_active {
                                sync_started =
                                    scanner.sync_output_active.then(std::time::Instant::now);
                            }
                            sync_output_active = scanner.sync_output_active;
                            let mut parser = &mut ingress.processor;
                            let mut local_lines = 0;

                            // Debug: write raw PTY bytes to log file
                            if pty_debug {
                                use std::io::Write as _;
                                if let Ok(mut f) = std::fs::OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(&pty_log_path)
                                {
                                    let ts = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis();
                                    let escaped: String = buf[..n]
                                        .iter()
                                        .map(|&b| match b {
                                            0x1b => "ESC".to_string(),
                                            0x07 => "BEL".to_string(),
                                            b if b.is_ascii_graphic() || b == b' ' => {
                                                format!("{}", b as char)
                                            }
                                            b => format!("\\x{:02x}", b),
                                        })
                                        .collect();
                                    let _ = writeln!(f, "[{}ms] PTY({} bytes): {}", ts, n, escaped);
                                }
                            }

                            for &byte in buf[..n].iter() {
                                if byte == 0x0A {
                                    local_lines += 1;
                                }

                                parser.advance(&mut *term_locked, &[byte]);

                                match scanner.feed(byte) {
                                    Some(SequenceFrame::Reset) => {
                                        update_program_status(
                                            &program_status_clone,
                                            &program_status_pending_clone,
                                            &event_listener_clone,
                                            source_id,
                                            |store| store.reset(),
                                        );
                                    }
                                    Some(SequenceFrame::Osc { bytes, framing }) => {
                                        if let Some(cmd) =
                                            parse_osc_framed(&bytes, &mut cmd_start_time, framing)
                                        {
                                            let cursor_line =
                                                term_locked.grid().cursor.point.line.0 as i32;
                                            let screen_lines =
                                                term_locked.grid().screen_lines() as i32;
                                            let base = total_lines_pushed_clone
                                                .load(Ordering::Relaxed)
                                                + local_lines;
                                            if let OscCommand::Cwd(path) = &cmd {
                                                if let Some(path) = file_url_to_path(path.as_bytes()) {
                                                    if path.is_absolute() {
                                                        scanner.cwd = Some(path);
                                                    }
                                                }
                                            }
                                            handle_cmd(
                                                cmd,
                                                cursor_line,
                                                screen_lines,
                                                base,
                                                &mut term_locked,
                                                &mut parser,
                                            );
                                        }
                                    }
                                    Some(SequenceFrame::Apc(bytes)) => {
                                        let cursor_line =
                                            term_locked.grid().cursor.point.line.0 as i32;
                                        let cursor_col = term_locked.grid().cursor.point.column.0;
                                        let base = total_lines_pushed_clone.load(Ordering::Relaxed)
                                            + local_lines;
                                        local_lines += handle_apc(
                                            &bytes,
                                            cursor_line,
                                            cursor_col,
                                            base,
                                            &mut term_locked,
                                            &mut parser,
                                        );
                                    }
                                    Some(SequenceFrame::Dcs(bytes)) => {
                                        // Sixel graphics share the image
                                        // store with the kitty protocol.
                                        if crate::parser::sixel::is_sixel(&bytes) {
                                            if let Ok((render_img, img_w, img_h)) =
                                                crate::parser::sixel::decode(&bytes)
                                            {
                                                let img_id = next_image_id_clone
                                                    .fetch_add(1, Ordering::Relaxed)
                                                    as u32;
                                                let cursor_line = term_locked
                                                    .grid()
                                                    .cursor
                                                    .point
                                                    .line
                                                    .0 as i32;
                                                let cursor_col =
                                                    term_locked.grid().cursor.point.column.0;
                                                let base = total_lines_pushed_clone
                                                    .load(Ordering::Relaxed)
                                                    + local_lines;
                                                let mut store = image_store_clone.lock();
                                                store.add_image(img_id, render_img.clone());
                                                let abs_line = base
                                                    .saturating_add(cursor_line.max(0) as u64);
                                                let cw =
                                                    if cell_width > 0.0 { cell_width } else { 9.0 };
                                                let ch = if cell_height > 0.0 {
                                                    cell_height
                                                } else {
                                                    18.0
                                                };
                                                store.add_placement(ImagePlacement {
                                                    placement_id: 0,
                                                    image_id: img_id,
                                                    absolute_line: abs_line,
                                                    col: cursor_col,
                                                    cols: ((img_w as f32) / cw).ceil().max(1.0)
                                                        as usize,
                                                    rows: ((img_h as f32) / ch).ceil().max(1.0)
                                                        as usize,
                                                    z_index: 0,
                                                    image: render_img,
                                                });
                                            }
                                        }
                                    }
                                    None => {}
                                }

                                // CSI detection: dispatch watcher events for
                                // scrollback wipes and Synchronized Output.
                                match scanner.csi_watcher.feed(byte) {
                                    Some(CsiEvent::ClearHistory) => clear_history_flag = true,
                                    Some(CsiEvent::QueryKittyClipboard) => {
                                        // DECRQM for mode 5522: reply "set" —
                                        // fastty implements the kitty clipboard
                                        // protocol (reads, writes, images).
                                        let resp = b"\x1b[?5522;1$y";
                                        let mut w = writer_clone.lock();
                                        let _ = w.write_all(resp);
                                        let _ = w.flush();
                                    }
                                    Some(CsiEvent::KittyPush(flags)) => {
                                        kitty_clone.lock().push(flags);
                                    }
                                    Some(CsiEvent::KittyPop(count)) => {
                                        kitty_clone.lock().pop(count);
                                    }
                                    Some(CsiEvent::KittySet(flags, mode)) => {
                                        kitty_clone.lock().set(flags, mode);
                                    }
                                    Some(CsiEvent::KittyQuery) => {
                                        // `CSI ? u` asks for the active flags.
                                        let resp = format!("\x1b[?{}u", kitty_clone.lock().flags());
                                        let mut w = writer_clone.lock();
                                        let _ = w.write_all(resp.as_bytes());
                                        let _ = w.flush();
                                    }
                                    Some(CsiEvent::SyncOutputBegin) => {
                                        sync_output_active = true;
                                        sync_started = Some(std::time::Instant::now());
                                    }
                                    Some(CsiEvent::SyncOutputEnd) => {
                                        sync_output_active = false;
                                        sync_started = None;
                                    }
                                    None => {}
                                }
                            }
                            // After the byte loop: clear scrollback history if we detected
                            // \x1b[2J (EraseDisplay::All) on the main screen.
                            if clear_history_flag {
                                use alacritty_terminal::term::TermMode;
                                let mode = *term_locked.mode();
                                if !mode.contains(TermMode::ALT_SCREEN) {
                                    term_locked.grid_mut().clear_history();
                                    image_store_clone.lock().placements.clear();
                                }
                                clear_history_flag = false;
                            }
                            drop(term_locked);

                            total_lines_pushed_clone.fetch_add(local_lines, Ordering::Relaxed);
                            if local_lines > 0 {
                                let total_pushed = total_lines_pushed_clone.load(Ordering::Relaxed);
                                let min_line = total_pushed.saturating_sub(scrollback as u64);
                                image_store_clone.lock().prune_old_placements(min_line);
                            }

                            // Synchronized Output: if the app still holds mode 2026
                            // set and the safety deadline passed, release it so a
                            // crashed app can never freeze the screen.
                            if sync_output_active
                                && sync_started.is_some_and(|t| t.elapsed() >= SYNC_OUTPUT_TIMEOUT)
                            {
                                sync_output_active = false;
                                sync_started = None;
                            }
                            if !sync_output_active {
                                render_gen_clone.fetch_add(1, Ordering::Relaxed);
                                event_listener_clone.send_app_event(AppEvent::Wakeup);
                            }
                            scanner.sync_output_active = sync_output_active;
                            ingress.side = scanner;
                            drop(ingress);
                            output_hub_clone.publish_at_boundary(&buf[..n]);
                            drop(output_boundary);
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                            continue;
                        }
                        Err(_) => {
                            let _output_boundary = output_hub_clone.boundary.lock();
                            update_program_status(
                                &program_status_clone,
                                &program_status_pending_clone,
                                &event_listener_clone,
                                source_id,
                                |store| store.end_activity(),
                            );
                            alive_clone.store(false, Ordering::Relaxed);
                            event_listener_clone.send_app_event(AppEvent::Exit {
                                shell_pid: reader_shell_pid,
                            });
                            break;
                        }
                    }
                }
            });
        }

        #[cfg(unix)]
        if let (Some(remote), Some(mut output_sink)) = (remote, remote_output) {
            if !has_binary_snapshot {
                let mut snapshot = remote.snapshot;
                if remote.restore_legacy_mouse_modes {
                    append_legacy_mouse_modes(&mut snapshot);
                }
                let _ = output_sink.write_all(&snapshot);
            }
            thread::spawn(move || {
                let mut reader = remote.reader;
                loop {
                    match crate::session_host::read_output(&mut reader) {
                        Ok(Some(bytes)) if !bytes.is_empty() => {
                            if output_sink.write_all(&bytes).is_err() {
                                break;
                            }
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => break,
                        Err(error) => {
                            let message =
                                format!("\r\n\r\n[Fastty: terminal output stopped: {error}]\r\n");
                            let _ = output_sink.write_all(message.as_bytes());
                            break;
                        }
                    }
                }
            });
        }
        #[cfg(not(unix))]
        drop(remote);

        let remote_resize_sender = if let Some(remote) = remote_control.as_ref() {
            let worker_remote = Arc::clone(remote);
            let (sender, receiver) = async_channel::unbounded::<(usize, usize)>();
            let worker = thread::Builder::new()
                .name("fastty-session-resize".into())
                .spawn(move || {
                    while let Ok(mut size) = receiver.recv_blocking() {
                        while let Ok(latest) = receiver.try_recv() {
                            size = latest;
                        }
                        worker_remote.resize(size.0, size.1);
                    }
                });
            if let Err(error) = worker {
                return Err(remote_worker_start_error(
                    remote_control.as_ref(),
                    remote_session_id,
                    spawned_here,
                    error,
                ));
            }
            Some(sender)
        } else {
            None
        };

        let cwd_observation = remote_session_identity.clone().and_then(|identity| {
            let ingress = Arc::clone(&ingress);
            let listener = event_listener.clone();
            crate::session_host::observe_cwd(identity, move |path| {
                let changed = {
                    let mut ingress = ingress.lock();
                    if ingress.side.cwd.as_ref() == Some(&path) {
                        false
                    } else {
                        ingress.side.cwd = Some(path.clone());
                        true
                    }
                };
                if changed {
                    listener.send_app_event(AppEvent::CwdChanged(path));
                }
            })
        });

        Ok(Self {
            ingress,
            term,
            render_generation,
            remote_resize_sender,
            writer: writer_arc,
            master: master_arc,
            shell_pid,
            remote_shell_pid,
            source_id,
            program_status,
            program_status_pending,
            progress,
            last_output_ms,
            command_running,
            kitty,
            total_lines_pushed,
            prompt_lines,
            image_store,
            event_listener,
            output_hub,
            alive,
            has_terminated: std::sync::atomic::AtomicBool::new(false),
            remote_session_id,
            remote_session_identity,
            remote_control,
            _cwd_observation: cwd_observation,
        })
    }

    pub fn source_id(&self) -> u64 {
        self.source_id
    }

    pub fn program_status(&self) -> Vec<crate::program_status::ProgramRecord> {
        let store = self.program_status.lock();
        let records = store.snapshot();
        self.program_status_pending.store(false, Ordering::Relaxed);
        records
    }

    pub fn program_status_snapshot(&self) -> Vec<crate::program_status::ProgramRecord> {
        self.program_status.lock().snapshot()
    }

    /// Latest `OSC 9;4` progress report, if the program set one.
    pub fn progress(&self) -> Option<ProgressReport> {
        *self.progress.lock()
    }

    /// True between `OSC 133` command-start and command-end markers.
    pub fn is_command_running(&self) -> bool {
        self.command_running.load(Ordering::Relaxed)
    }

    /// Active kitty keyboard protocol flags for this terminal.
    pub fn kitty_flags(&self) -> u8 {
        self.kitty.lock().flags()
    }

    /// Set the kitty keyboard flags (used when restoring a snapshot).
    pub fn set_kitty_flags(&self, flags: u8) {
        self.kitty.lock().set(flags, 1);
    }

    /// Seconds since the last PTY output while a command is running.
    /// `None` when no command is running or nothing was ever printed.
    pub fn silence_seconds(&self) -> Option<u64> {
        if !self.is_command_running() {
            return None;
        }
        let last = self.last_output_ms.load(Ordering::Relaxed);
        if last == 0 {
            return None;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Some(now.saturating_sub(last) / 1000)
    }

    pub fn report_external_program_status(&self, payload: &[u8]) -> Result<(), String> {
        let command = crate::program_status::parse(payload, payload.len() + 9)
            .ok_or_else(|| "invalid program status report".to_string())?;
        let _output_boundary = self.output_hub.boundary.lock();
        update_program_status(
            &self.program_status,
            &self.program_status_pending,
            &self.event_listener,
            self.source_id,
            |store| store.apply(command),
        );
        // Mirror the report to attached clients as the OSC 7501 sequence a
        // program inside the pane would have written, so their local stores
        // and notification guards see the same transition this process
        // applied through the daemon API.
        self.output_hub
            .publish_at_boundary(&mirror_program_status_sequence(payload));
        Ok(())
    }

    pub fn shell_pid(&self) -> Option<u32> {
        self.shell_pid
    }

    pub fn remote_session_id(&self) -> Option<usize> {
        self.remote_session_id
    }

    pub fn remote_session_identity(&self) -> Option<crate::daemon::SessionIdentity> {
        self.remote_session_identity.clone()
    }

    /// Whether the pane's shell process is still there — `false` once the
    /// reader thread has seen EOF/an error on the PTY. Unlike
    /// `is_process_running`, this is about the shell itself, not whether it
    /// currently has an active foreground child.
    pub fn is_alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Subscribe to this pane's raw PTY output. Used by `crate::daemon` so an
    /// attached client sees the same bytes the GPUI renderer does, without
    /// stealing them — every subscriber gets its own copy of each chunk.
    pub fn subscribe_output(&self) -> async_channel::Receiver<Vec<u8>> {
        self.output_hub.subscribe()
    }

    pub fn snapshot_and_subscribe_output(
        &self,
    ) -> (
        Vec<u8>,
        Vec<crate::program_status::ProgramRecord>,
        bool,
        async_channel::Receiver<Vec<u8>>,
    ) {
        let ((snapshot, program_status, process_running), output) =
            self.output_hub.snapshot_and_subscribe(|| {
                (
                    self.snapshot_ansi(),
                    self.program_status_snapshot(),
                    self.is_process_running(),
                )
            });
        (snapshot, program_status, process_running, output)
    }

    pub fn snapshot_binary_and_subscribe_output(
        &self,
    ) -> (
        Vec<u8>,
        Vec<crate::program_status::ProgramRecord>,
        bool,
        async_channel::Receiver<Vec<u8>>,
    ) {
        const MAX_ATTACH_CELLS: usize = 16 * 1024 * 1024 / 16;
        let (((header, cells), program_status, process_running), output) =
            self.output_hub.snapshot_and_subscribe(|| {
                (
                    self.snapshot_binary_parts(true, Some(MAX_ATTACH_CELLS)),
                    self.program_status_snapshot(),
                    self.is_process_running(),
                )
            });
        (
            crate::server::binary_snapshot::encode_snapshot_compressed(&header, &cells),
            program_status,
            process_running,
            output,
        )
    }

    #[cfg(test)]
    fn capture_terminal_snapshot(&self) -> alacritty_terminal::snapshot::TerminalSnapshot {
        use alacritty_terminal::grid::Dimensions;
        let term = self.term.lock();
        let grid = term.grid();
        let visible_cells = grid
            .columns()
            .saturating_mul(grid.screen_lines())
            .saturating_mul(2);
        let max_history = alacritty_terminal::snapshot::MAX_SNAPSHOT_CELLS
            .saturating_sub(visible_cells)
            / grid.columns().max(1);
        term.snapshot(max_history)
    }

    fn capture_native_state(&self) -> Result<crate::server::native_snapshot::NativeState, String> {
        use alacritty_terminal::grid::Dimensions;
        let term = self.term.lock();
        let ingress = self.ingress.lock();
        let grid = term.grid();
        let visible_cells = grid
            .columns()
            .saturating_mul(grid.screen_lines())
            .saturating_mul(2);
        let max_history = alacritty_terminal::snapshot::MAX_SNAPSHOT_CELLS
            .saturating_sub(visible_cells)
            / grid.columns().max(1);
        let mut side = ingress.side.clone();
        if let Some(cwd) = self.cwd_process_id().and_then(get_process_cwd) {
            side.cwd = Some(cwd);
        }
        Ok(crate::server::native_snapshot::NativeState {
            terminal: term.snapshot(max_history),
            continuation: Some(crate::server::native_snapshot::Continuation {
                parser: ingress.processor.snapshot().map_err(str::to_string)?,
                side,
            }),
        })
    }

    pub fn snapshot_state_for_save(&self) -> Result<Vec<u8>, String> {
        let state = {
            let _boundary = self.output_hub.boundary.lock();
            self.capture_native_state()?
        };
        crate::server::native_snapshot::encode_state(
            &state,
            crate::server::native_snapshot::Encoding::Raw,
        )
    }

    pub fn snapshot_state_and_subscribe_output(
        &self,
    ) -> Result<
        (
            Vec<u8>,
            Vec<crate::program_status::ProgramRecord>,
            bool,
            async_channel::Receiver<Vec<u8>>,
        ),
        String,
    > {
        let ((state, program_status, process_running), output) =
            self.output_hub.snapshot_and_subscribe(|| {
                (
                    self.capture_native_state(),
                    self.program_status_snapshot(),
                    self.is_process_running(),
                )
            });
        match state.and_then(|state| {
            crate::server::native_snapshot::encode_state(
                &state,
                crate::server::native_snapshot::Encoding::Deflate,
            )
        }) {
            Ok(bytes) => Ok((bytes, program_status, process_running, output)),
            Err(error) => {
                output.close();
                Err(error)
            }
        }
    }

    pub fn needs_close_confirmation(&self) -> bool {
        self.close_process_status().0
    }

    pub fn close_process_status(&self) -> (bool, Option<String>) {
        if let Some(session_id) = self.remote_session_id {
            let (process_running, process_name) =
                crate::session_host::session_process_status_reference(session_id, self.remote_session_identity()).unwrap_or((true, None));
            (
                remote_session_needs_close_confirmation(Some(session_id), process_running),
                process_name,
            )
        } else {
            (
                self.is_process_running(),
                self.get_foreground_process_name(),
            )
        }
    }

    pub fn set_event_sender(&self, sender: crate::event_listener::EventSender) {
        self.event_listener.set_event_sender(sender);
        {
            let _store = self.program_status.lock();
            self.program_status_pending.store(true, Ordering::Relaxed);
        }
        self.event_listener
            .send_app_event(AppEvent::ProgramStatusChanged {
                source_id: self.source_id,
            });
    }

    fn cwd_process_id(&self) -> Option<u32> {
        self.shell_pid
            .or(self.remote_shell_pid.filter(|_| self.is_alive()))
    }

    pub fn get_current_working_directory(&self) -> Option<std::path::PathBuf> {
        self.cwd_process_id()
            .and_then(get_process_cwd)
            .or_else(|| self.ingress.lock().side.cwd.clone())
    }

    pub fn process_coordinates(&self) -> (Option<u32>, Option<u32>) {
        if self.remote_session_id.is_some() {
            return (None, None);
        }
        #[cfg(unix)]
        let foreground = self.master.lock().as_raw_fd().and_then(|fd| {
            let group = unsafe { libc::tcgetpgrp(fd) };
            (group > 0).then_some(group as u32)
        });
        #[cfg(not(unix))]
        let foreground = None;
        (self.shell_pid, foreground)
    }

    pub fn get_foreground_process_name(&self) -> Option<String> {
        let _shell_pid = self.shell_pid?;
        #[cfg(unix)]
        let pid = {
            let master = self.master.lock();
            let fd = master.as_raw_fd()?;
            let pid = unsafe { libc::tcgetpgrp(fd) };
            (pid > 0).then_some(pid as u32)?
        };
        #[cfg(target_os = "macos")]
        {
            get_process_name_macos(pid)
        }
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .ok()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            get_foreground_process_name_other(_shell_pid)
        }
    }

    pub fn get_foreground_process_command(&self) -> Option<String> {
        let _shell_pid = self.shell_pid?;
        #[cfg(unix)]
        {
            let foreground_pgid = {
                let master = self.master.lock();
                let fd = master.as_raw_fd()?;
                unsafe { libc::tcgetpgrp(fd) }
            };
            if foreground_pgid <= 0 {
                return None;
            }
            let output = std::process::Command::new("ps")
                .args(["-axo", "pid=,pgid=,command="])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let processes = std::string::String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let mut fields = line.split_whitespace();
                    let pid = fields.next()?.parse::<u32>().ok()?;
                    let pgid = fields.next()?.parse::<libc::pid_t>().ok()?;
                    let command = fields.collect::<Vec<_>>().join(" ");
                    (pgid == foreground_pgid && !command.is_empty()).then_some((pid, command))
                })
                .collect::<Vec<_>>();
            processes
                .iter()
                .find(|(pid, _)| *pid == foreground_pgid as u32)
                .map(|(_, command)| command.clone())
        }
        #[cfg(not(unix))]
        {
            let _ = _shell_pid;
            None
        }
    }

    pub fn is_process_running(&self) -> bool {
        let Some(pid) = self.shell_pid else {
            return false;
        };
        if !self.alive.load(Ordering::Relaxed) {
            return false;
        }
        #[cfg(unix)]
        {
            let master = self.master.lock();
            let Some(fd) = master.as_raw_fd() else {
                return true;
            };
            let foreground_pgid = unsafe { libc::tcgetpgrp(fd) };
            let has_foreground_child = if foreground_pgid > 0 {
                #[cfg(target_os = "macos")]
                {
                    has_active_child_macos(pid, foreground_pgid)
                }
                #[cfg(target_os = "linux")]
                {
                    has_active_child_linux(pid, foreground_pgid)
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                {
                    false
                }
            } else {
                false
            };
            foreground_process_state(
                unsafe { libc::getpgid(pid as libc::pid_t) },
                foreground_pgid,
                has_foreground_child,
            )
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            false
        }
    }

    pub fn terminate_process(&self) {
        self.terminate_process_without_writer_lock();
    }

    pub fn terminate_if_idle(&self) -> bool {
        let _writer = self.writer.lock();
        if self.is_process_running() {
            return false;
        }
        self.terminate_process_without_writer_lock();
        true
    }

    pub fn terminate_all_if_idle(terminals: &[Arc<Self>]) -> Vec<usize> {
        let mut lock_order: Vec<usize> = (0..terminals.len()).collect();
        lock_order.sort_unstable_by_key(|index| Arc::as_ptr(&terminals[*index]) as usize);
        lock_order.dedup_by_key(|index| Arc::as_ptr(&terminals[*index]) as usize);
        let _writers: Vec<_> = lock_order
            .iter()
            .map(|index| terminals[*index].writer.lock())
            .collect();

        let blocked: Vec<usize> = terminals
            .iter()
            .enumerate()
            .filter_map(|(index, terminal)| terminal.is_process_running().then_some(index))
            .collect();
        if blocked.is_empty() {
            for terminal in terminals {
                terminal.terminate_process_without_writer_lock();
            }
        }
        blocked
    }

    fn terminate_process_without_writer_lock(&self) {
        if self
            .has_terminated
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        if let Some(remote) = &self.remote_control {
            remote.stop();
            return;
        }
        if !self.is_alive() {
            return;
        }
        let Some(pid) = self.shell_pid else {
            return;
        };
        if pid <= 1 {
            return;
        }
        #[cfg(not(windows))]
        {
            let ipid = pid as i32;
            if ipid <= 1 {
                return;
            }
            std::thread::spawn(move || {
                unsafe {
                    libc::kill(-ipid, libc::SIGHUP);
                    libc::kill(-ipid, libc::SIGTERM);
                    libc::kill(ipid, libc::SIGHUP);
                    libc::kill(ipid, libc::SIGTERM);
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
                unsafe {
                    if libc::kill(ipid, 0) == 0 {
                        libc::kill(-ipid, libc::SIGKILL);
                        libc::kill(ipid, libc::SIGKILL);
                    }
                }
            });
        }
        #[cfg(windows)]
        {
            let _ = pid;
        }
    }
}

impl Drop for TerminalState {
    fn drop(&mut self) {
        if let Some(remote) = &self.remote_control {
            remote.detach();
        } else {
            self.terminate_process();
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewSpan {
    pub text: String,
    pub fg: Option<alacritty_terminal::vte::ansi::Color>,
    pub bold: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewLine {
    pub spans: Vec<PreviewSpan>,
    pub is_cursor_row: bool,
    pub cursor_col: Option<usize>,
}

impl TerminalState {
    pub fn write_to_pty(&self, bytes: &[u8]) {
        if self.has_terminated.load(Ordering::SeqCst) {
            return;
        }
        if let Some(remote) = &self.remote_control {
            if let Err(error) = remote.write(bytes) {
                eprintln!("Fastty session input write failed: {error}");
            }
        } else {
            let mut w = self.writer.lock();
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
    }

    pub fn update_scrollback(&self, scrollback: usize) {
        let mut term = self.term.lock();
        term.grid_mut()
            .update_history(scrollback.clamp(500, 100_000));
    }

    pub fn is_mouse_mode_enabled(&self) -> bool {
        let term = self.term.lock();
        let mode = *term.mode();
        mode.intersects(
            alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                | alacritty_terminal::term::TermMode::MOUSE_DRAG
                | alacritty_terminal::term::TermMode::MOUSE_MOTION,
        )
    }

    pub fn is_bracketed_paste_enabled(&self) -> bool {
        let term = self.term.lock();
        term.mode()
            .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE)
    }

    pub fn send_mouse_event(&self, button: u8, col: usize, row: usize, pressed: bool) {
        self.send_mouse_button_with_mods(button, col, row, pressed, false, false, false);
    }

    pub fn send_mouse_button_with_mods(
        &self,
        button: u8,
        col: usize,
        row: usize,
        pressed: bool,
        shift: bool,
        alt: bool,
        control: bool,
    ) {
        let term = self.term.lock();
        let mode = *term.mode();
        let is_mouse_active = mode.intersects(
            alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                | alacritty_terminal::term::TermMode::MOUSE_DRAG
                | alacritty_terminal::term::TermMode::MOUSE_MOTION,
        );
        let sgr = mode.contains(alacritty_terminal::term::TermMode::SGR_MOUSE);
        drop(term);

        if !is_mouse_active {
            return;
        }

        let mut btn_code = button;
        if shift {
            btn_code |= 4;
        }
        if alt {
            btn_code |= 8;
        }
        if control {
            btn_code |= 16;
        }

        let col_1 = col.max(1);
        let row_1 = row.max(1);

        if sgr {
            // SGR 1006 format: \x1b[<{button};{col};{row}{M/m}
            let flag = if pressed { 'M' } else { 'm' };
            let seq = format!("\x1b[<{};{};{}{}", btn_code, col_1, row_1, flag);
            self.write_to_pty(seq.as_bytes());
        } else {
            // Normal X10/1000 format: \x1b[M{btn + 32}{col + 32}{row + 32}
            let b_byte = if pressed { btn_code } else { 3 };
            let cb = b_byte.saturating_add(32);
            let cx = (col_1 as u8).saturating_add(32);
            let cy = (row_1 as u8).saturating_add(32);
            let seq = [0x1b, b'[', b'M', cb, cx, cy];
            self.write_to_pty(&seq);
        }
    }

    pub fn send_mouse_motion(
        &self,
        col: usize,
        row: usize,
        left_down: bool,
        middle_down: bool,
        right_down: bool,
        shift: bool,
        alt: bool,
        control: bool,
    ) {
        let term = self.term.lock();
        let mode = *term.mode();
        let is_mouse_active = mode.intersects(
            alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK
                | alacritty_terminal::term::TermMode::MOUSE_DRAG
                | alacritty_terminal::term::TermMode::MOUSE_MOTION,
        );
        let sgr = mode.contains(alacritty_terminal::term::TermMode::SGR_MOUSE);
        let motion_any = mode.contains(alacritty_terminal::term::TermMode::MOUSE_MOTION);
        let motion_drag = mode.contains(alacritty_terminal::term::TermMode::MOUSE_DRAG);
        drop(term);

        if !is_mouse_active {
            return;
        }

        let is_dragging = left_down || middle_down || right_down;
        if is_dragging && !motion_drag && !motion_any {
            return;
        }
        if !is_dragging && !motion_any {
            return;
        }

        let mut btn_code: u8 = if left_down {
            32
        } else if middle_down {
            1 + 32
        } else if right_down {
            2 + 32
        } else {
            3 + 32 // 35 = motion without button
        };

        if shift {
            btn_code |= 4;
        }
        if alt {
            btn_code |= 8;
        }
        if control {
            btn_code |= 16;
        }

        let col_1 = col.max(1);
        let row_1 = row.max(1);

        if sgr {
            let seq = format!("\x1b[<{};{};{}M", btn_code, col_1, row_1);
            self.write_to_pty(seq.as_bytes());
        } else {
            let cb = btn_code.saturating_add(32);
            let cx = (col_1 as u8).saturating_add(32);
            let cy = (row_1 as u8).saturating_add(32);
            let seq = [0x1b, b'[', b'M', cb, cx, cy];
            self.write_to_pty(&seq);
        }
    }

    pub fn dimensions(&self) -> (usize, usize) {
        let term = self.term.lock();
        (term.grid().columns(), term.grid().screen_lines())
    }

    pub fn resize(&self, cols: usize, rows: usize) {
        self.resize_with_pixels(cols, rows, (cols as u16) * 8, (rows as u16) * 16);
    }

    pub fn resize_with_pixels(
        &self,
        cols: usize,
        rows: usize,
        pixel_width: u16,
        pixel_height: u16,
    ) {
        if cols == 0 || rows == 0 {
            return;
        }
        let mut term = self.term.lock();
        let cur_cols = term.grid().columns();
        let cur_rows = term.grid().screen_lines();
        if cur_cols == cols && cur_rows == rows {
            return;
        }
        let size = TermSize::new(cols, rows);
        term.resize(size);
        drop(term);

        self.master
            .lock()
            .resize(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width,
                pixel_height,
            })
            .ok();
        if let Some(remote) = &self.remote_control {
            remote.resize(cols, rows);
        }

        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn try_resize_with_pixels(
        &self,
        cols: usize,
        rows: usize,
        pixel_width: u16,
        pixel_height: u16,
    ) -> bool {
        if cols == 0 || rows == 0 {
            return true;
        }
        let Some(master) = self.master.try_lock() else {
            return false;
        };
        let Some(mut term) = self.term.try_lock() else {
            return false;
        };
        let cur_cols = term.grid().columns();
        let cur_rows = term.grid().screen_lines();
        if cur_cols == cols && cur_rows == rows {
            return true;
        }
        term.resize(TermSize::new(cols, rows));
        drop(term);
        master
            .resize(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width,
                pixel_height,
            })
            .ok();
        drop(master);

        if let Some(sender) = &self.remote_resize_sender {
            let _ = sender.try_send((cols, rows));
        }
        self.render_generation.fetch_add(1, Ordering::Relaxed);
        true
    }

    pub fn scroll(&self, delta: isize) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        term.scroll_display(Scroll::Delta(delta as i32));
        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn scroll_to_offset(&self, offset: usize) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        let cur_offset = term.grid().display_offset();
        let delta = (offset as isize) - (cur_offset as isize);
        if delta != 0 {
            term.scroll_display(Scroll::Delta(delta as i32));
            self.render_generation.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn scroll_to_bottom(&self) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        term.scroll_display(Scroll::Bottom);
        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn scroll_to_top(&self) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        term.scroll_display(Scroll::Top);
        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn scroll_page(&self, pages: i32) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        if pages > 0 {
            for _ in 0..pages {
                term.scroll_display(Scroll::PageDown);
            }
        } else {
            for _ in 0..(-pages) {
                term.scroll_display(Scroll::PageUp);
            }
        }
        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn display_offset(&self) -> usize {
        let term = self.term.lock();
        term.grid().display_offset()
    }

    pub fn try_history_metrics(&self) -> Option<(usize, usize)> {
        use alacritty_terminal::grid::Dimensions;
        let term = self.term.try_lock()?;
        Some((term.history_size(), term.grid().display_offset()))
    }

    /// Cursor position in visible screen cells: (line from top, column).
    pub fn cursor_screen_pos(&self) -> (usize, usize) {
        let term = self.term.lock();
        let point = term.grid().cursor.point;
        (point.line.0.max(0) as usize, point.column.0)
    }

    pub fn scroll_to_prev_prompt(&self) {
        let cur_offset = self.display_offset() as u64;
        let total_pushed = self.total_lines_pushed.load(Ordering::Relaxed);
        let prompts = self.prompt_lines.lock();
        let mut candidates: Vec<u64> = prompts
            .iter()
            .map(|&line| total_pushed.saturating_sub(line))
            .filter(|&offset| offset > cur_offset + 1)
            .collect();
        candidates.sort_unstable();
        if let Some(&target) = candidates.first() {
            self.scroll_to_offset(target as usize);
        } else if let Some(&min_line) = prompts.first() {
            let target = total_pushed.saturating_sub(min_line);
            if target > cur_offset {
                self.scroll_to_offset(target as usize);
            }
        }
    }

    pub fn scroll_to_next_prompt(&self) {
        let cur_offset = self.display_offset() as u64;
        if cur_offset == 0 {
            return;
        }
        let total_pushed = self.total_lines_pushed.load(Ordering::Relaxed);
        let prompts = self.prompt_lines.lock();
        let mut candidates: Vec<u64> = prompts
            .iter()
            .map(|&line| total_pushed.saturating_sub(line))
            .filter(|&offset| offset + 1 < cur_offset)
            .collect();
        candidates.sort_unstable_by(|a, b| b.cmp(a));
        if let Some(&target) = candidates.first() {
            self.scroll_to_offset(target as usize);
        } else {
            self.scroll_to_bottom();
        }
    }

    pub fn history_size(&self) -> usize {
        use alacritty_terminal::grid::Dimensions;
        let term = self.term.lock();
        term.history_size()
    }

    pub fn search_matches(&self, query: &str) -> Vec<usize> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        use alacritty_terminal::index::Line;
        let query_lower = query.to_lowercase();
        let term = self.term.lock();
        let grid = term.grid();
        let history = grid.history_size();
        let screen_lines = grid.screen_lines();
        if screen_lines == 0 {
            return Vec::new();
        }
        let mut matches = Vec::new();

        for line_i in -(history as i32)..(screen_lines as i32) {
            let row = &grid[Line(line_i)];
            let mut line_str = String::new();
            for cell in row.into_iter() {
                let c = cell.c;
                if c != '\0' {
                    line_str.push(c);
                }
            }
            if line_str.to_lowercase().contains(&query_lower) {
                let offset = if line_i < 0 { (-line_i) as usize } else { 0 };
                matches.push(offset);
            }
        }
        matches.dedup();
        matches
    }

    pub fn search_snippets(&self, query: &str, max_results: usize) -> Vec<SearchSnippet> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        use alacritty_terminal::index::Line;
        let query_lower = query.to_lowercase();
        let term = self.term.lock();
        let grid = term.grid();
        let history = grid.history_size();
        let screen_lines = grid.screen_lines();
        if screen_lines == 0 {
            return Vec::new();
        }
        let mut snippets = Vec::new();

        for line_i in -(history as i32)..(screen_lines as i32) {
            let row = &grid[Line(line_i)];
            let mut line_str = String::new();
            for cell in row.into_iter() {
                let c = cell.c;
                if c != '\0' {
                    line_str.push(c);
                }
            }
            let trimmed = line_str.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let trimmed_lower = trimmed.to_lowercase();
            let is_match = if trimmed_lower.contains(&query_lower) {
                true
            } else if query_lower.len() >= 3 {
                let mut p_idx = 0;
                let q_chars: Vec<char> = query_lower.chars().collect();
                for ch in trimmed_lower.chars() {
                    if p_idx < q_chars.len() && ch == q_chars[p_idx] {
                        p_idx += 1;
                    }
                }
                p_idx == q_chars.len()
            } else {
                false
            };

            if is_match {
                let offset = if line_i < 0 { (-line_i) as usize } else { 0 };
                snippets.push(SearchSnippet {
                    offset,
                    line_index: line_i,
                    text: trimmed.to_string(),
                });
                if snippets.len() >= max_results {
                    break;
                }
            }
        }
        snippets
    }

    /// Rows the terminal shows. A preview cannot have more lines than this, so a
    /// caller sizing a preview area needs it to know the real upper bound.
    pub fn screen_line_count(&self) -> usize {
        self.term.lock().grid().screen_lines()
    }

    pub fn get_screen_preview(&self, max_lines: usize) -> Vec<PreviewLine> {
        use alacritty_terminal::index::{Column, Line};
        use alacritty_terminal::term::TermMode;
        use alacritty_terminal::term::cell::Flags;

        let term = self.term.lock();
        let grid = term.grid();
        let screen_lines = grid.screen_lines();
        let cols = grid.columns();
        if screen_lines == 0 || cols == 0 || max_lines == 0 {
            return Vec::new();
        }

        let is_alt = term.mode().contains(TermMode::ALT_SCREEN);
        let cursor_point = grid.cursor.point;
        let cursor_line = cursor_point.line.0.max(0) as usize;
        let cursor_col = cursor_point.column.0.min(cols.saturating_sub(1));

        let (start_row, end_row) = if is_alt {
            let count = screen_lines.min(max_lines);
            (0, count.saturating_sub(1))
        } else {
            let mut last_non_empty = 0;
            for line_i in (0..screen_lines).rev() {
                let row = &grid[Line(line_i as i32)];
                let has_content = (0..cols).any(|c| {
                    let cell = &row[Column(c)];
                    cell.c != ' ' && cell.c != '\0' && cell.c != '\t'
                });
                if has_content {
                    last_non_empty = line_i;
                    break;
                }
            }
            let anchor = cursor_line.max(last_non_empty).min(screen_lines - 1);
            if anchor < max_lines {
                (0, screen_lines.min(max_lines).saturating_sub(1))
            } else {
                let start = (anchor + 1).saturating_sub(max_lines);
                (start, anchor)
            }
        };

        let mut result = Vec::with_capacity(end_row.saturating_sub(start_row) + 1);

        for line_i in start_row..=end_row {
            let row = &grid[Line(line_i as i32)];
            let is_cursor_row = line_i == cursor_line;

            let mut last_used_col = 0;
            let mut row_has_chars = false;
            for c in (0..cols).rev() {
                let cell = &row[Column(c)];
                if cell.c != ' ' && cell.c != '\0' && cell.c != '\t' {
                    last_used_col = c;
                    row_has_chars = true;
                    break;
                }
            }

            let mut spans: Vec<PreviewSpan> = Vec::new();
            if row_has_chars || is_cursor_row {
                let limit_col = if row_has_chars {
                    if is_cursor_row {
                        last_used_col.max(cursor_col)
                    } else {
                        last_used_col
                    }
                } else {
                    cursor_col
                };

                let mut cur_text = String::new();
                let mut cur_fg: Option<alacritty_terminal::vte::ansi::Color> = None;
                let mut cur_bold = false;

                for col_i in 0..=limit_col.min(cols.saturating_sub(1)) {
                    let cell = &row[Column(col_i)];
                    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                        continue;
                    }

                    let effective_fg = if cell.flags.contains(Flags::INVERSE) {
                        cell.bg
                    } else {
                        cell.fg
                    };
                    let effective_bold = cell.flags.contains(Flags::BOLD);
                    let c = if cell.c == '\t' || cell.c == '\0' {
                        ' '
                    } else {
                        cell.c
                    };

                    let cell_fg_opt = match effective_fg {
                        alacritty_terminal::vte::ansi::Color::Named(
                            alacritty_terminal::vte::ansi::NamedColor::Foreground,
                        ) => None,
                        other => Some(other),
                    };

                    if cell_fg_opt != cur_fg || effective_bold != cur_bold {
                        if !cur_text.is_empty() {
                            spans.push(PreviewSpan {
                                text: std::mem::take(&mut cur_text),
                                fg: cur_fg,
                                bold: cur_bold,
                            });
                        }
                        cur_fg = cell_fg_opt;
                        cur_bold = effective_bold;
                    }
                    cur_text.push(c);
                }

                if !cur_text.is_empty() {
                    spans.push(PreviewSpan {
                        text: cur_text,
                        fg: cur_fg,
                        bold: cur_bold,
                    });
                }
            }

            result.push(PreviewLine {
                spans,
                is_cursor_row,
                cursor_col: if is_cursor_row {
                    Some(cursor_col)
                } else {
                    None
                },
            });
        }

        result
    }

    pub fn get_screen_preview_lines(&self, max_lines: usize) -> Vec<String> {
        self.get_screen_preview(max_lines)
            .into_iter()
            .map(|line| {
                let joined = line
                    .spans
                    .into_iter()
                    .map(|s| s.text)
                    .collect::<Vec<_>>()
                    .join("");
                joined.trim_end().to_string()
            })
            .collect()
    }

    /// Render the current screen as ANSI/SGR bytes. Capture calls use this
    /// screen-only form. Colors/attributes that depend on the
    /// active GPUI theme (`NamedColor::Foreground/Background/Cursor/Dim*`)
    /// fall back to the terminal's default rather than being resolved here --
    /// this method has no theme to resolve them against, and a client-side
    /// default is a reasonable approximation for a one-shot snapshot.
    pub fn snapshot_ansi(&self) -> Vec<u8> {
        self.snapshot_ansi_bounded(usize::MAX).0
    }

    /// Render the current screen into at most `max_bytes` ANSI bytes.
    ///
    /// The renderer stops when the next complete ANSI command or character
    /// does not fit. This bounds both the returned buffer and cell traversal.
    pub fn snapshot_ansi_bounded(&self, max_bytes: usize) -> (Vec<u8>, bool) {
        self.render_ansi_snapshot(max_bytes)
    }

    fn render_ansi_snapshot(&self, max_bytes: usize) -> (Vec<u8>, bool) {
        use alacritty_terminal::index::Line;
        use alacritty_terminal::term::cell::Flags;
        use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor};

        struct BoundedSnapshot {
            bytes: Vec<u8>,
            max_bytes: usize,
            truncated: bool,
        }

        impl BoundedSnapshot {
            fn new(max_bytes: usize) -> Self {
                Self {
                    bytes: Vec::with_capacity(max_bytes.min(4096)),
                    max_bytes,
                    truncated: false,
                }
            }

            fn extend_from_slice(&mut self, chunk: &[u8]) {
                if self.truncated {
                    return;
                }
                if chunk.len() > self.max_bytes.saturating_sub(self.bytes.len()) {
                    self.truncated = true;
                    return;
                }
                self.bytes.extend_from_slice(chunk);
            }

            fn extend(&mut self, bytes: impl IntoIterator<Item = u8>) {
                let bytes = bytes.into_iter().collect::<Vec<_>>();
                self.extend_from_slice(&bytes);
            }
        }

        fn push_color(out: &mut BoundedSnapshot, color: AnsiColor, is_bg: bool) {
            match color {
                AnsiColor::Named(n) => {
                    let code = match n {
                        NamedColor::Black => 0,
                        NamedColor::Red => 1,
                        NamedColor::Green => 2,
                        NamedColor::Yellow => 3,
                        NamedColor::Blue => 4,
                        NamedColor::Magenta => 5,
                        NamedColor::Cyan => 6,
                        NamedColor::White => 7,
                        NamedColor::BrightBlack => 8,
                        NamedColor::BrightRed => 9,
                        NamedColor::BrightGreen => 10,
                        NamedColor::BrightYellow => 11,
                        NamedColor::BrightBlue => 12,
                        NamedColor::BrightMagenta => 13,
                        NamedColor::BrightCyan => 14,
                        NamedColor::BrightWhite => 15,
                        NamedColor::Foreground => {
                            if !is_bg {
                                out.extend_from_slice(b"\x1b[39m");
                            }
                            return;
                        }
                        NamedColor::Background => {
                            if is_bg {
                                out.extend_from_slice(b"\x1b[49m");
                            }
                            return;
                        }
                        _ => {
                            if is_bg {
                                out.extend_from_slice(b"\x1b[49m");
                            } else {
                                out.extend_from_slice(b"\x1b[39m");
                            }
                            return;
                        }
                    };
                    if code < 8 {
                        let base = if is_bg { 40 } else { 30 };
                        out.extend(format!("\x1b[{}m", base + code).bytes());
                    } else {
                        let base = if is_bg { 100 } else { 90 };
                        out.extend(format!("\x1b[{}m", base + (code - 8)).bytes());
                    }
                }
                AnsiColor::Indexed(i) => {
                    out.extend(format!("\x1b[{};5;{}m", if is_bg { 48 } else { 38 }, i).bytes());
                }
                AnsiColor::Spec(rgb) => {
                    out.extend(
                        format!(
                            "\x1b[{};2;{};{};{}m",
                            if is_bg { 48 } else { 38 },
                            rgb.r,
                            rgb.g,
                            rgb.b
                        )
                        .bytes(),
                    );
                }
            }
        }

        let term = self.term.lock();
        let grid = term.grid();
        let screen_lines = grid.screen_lines();
        let cols = grid.columns();
        let mode = *term.mode();

        let mut out = BoundedSnapshot::new(max_bytes);
        if mode.contains(alacritty_terminal::term::TermMode::ALT_SCREEN) {
            out.extend_from_slice(b"\x1b[?1049h");
        }
        out.extend_from_slice(b"\x1b[0m\x1b[2J\x1b[H");

        let mut cur_fg: Option<AnsiColor> = None;
        let mut cur_bg: Option<AnsiColor> = None;
        let mut cur_flags = Flags::empty();

        for line_i in 0..screen_lines {
            if out.truncated {
                break;
            }
            if line_i > 0 {
                out.extend_from_slice(b"\r\n");
            }
            let row = &grid[Line(line_i as i32)];
            for col in 0..cols {
                if out.truncated {
                    break;
                }
                let cell = &row[alacritty_terminal::index::Column(col)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                if Some(cell.fg) != cur_fg {
                    push_color(&mut out, cell.fg, false);
                    cur_fg = Some(cell.fg);
                }
                if Some(cell.bg) != cur_bg {
                    push_color(&mut out, cell.bg, true);
                    cur_bg = Some(cell.bg);
                }
                if cell.flags != cur_flags {
                    out.extend_from_slice(b"\x1b[22;23;24;27;28;29m");
                    if cell.flags.contains(Flags::BOLD) {
                        out.extend_from_slice(b"\x1b[1m");
                    }
                    if cell.flags.contains(Flags::DIM) {
                        out.extend_from_slice(b"\x1b[2m");
                    }
                    if cell.flags.contains(Flags::ITALIC) {
                        out.extend_from_slice(b"\x1b[3m");
                    }
                    if cell.flags.intersects(Flags::ALL_UNDERLINES) {
                        out.extend_from_slice(b"\x1b[4m");
                    }
                    if cell.flags.contains(Flags::INVERSE) {
                        out.extend_from_slice(b"\x1b[7m");
                    }
                    if cell.flags.contains(Flags::HIDDEN) {
                        out.extend_from_slice(b"\x1b[8m");
                    }
                    if cell.flags.contains(Flags::STRIKEOUT) {
                        out.extend_from_slice(b"\x1b[9m");
                    }
                    push_color(&mut out, cell.fg, false);
                    push_color(&mut out, cell.bg, true);
                    cur_flags = cell.flags;
                }
                let c = if cell.c == '\0' { ' ' } else { cell.c };
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        if out.truncated {
            return (out.bytes, true);
        }
        out.extend_from_slice(b"\x1b[0m");

        let cursor_line = grid.cursor.point.line.0;
        let cursor_col = grid.cursor.point.column.0;
        out.extend(format!("\x1b[{};{}H", (cursor_line + 1).max(1), cursor_col + 1).bytes());

        let mut restored_modes = Vec::new();
        append_restored_terminal_modes(&mut restored_modes, mode);
        out.extend_from_slice(&restored_modes);
        (out.bytes, out.truncated)
    }

    /// Take a full binary terminal state snapshot in microseconds.
    ///
    /// Directly extracts cells into a flat `FasttyPackedCell` contiguous vector with a 32-byte
    /// `FasttyBinarySnapshotHeader`. Restorable directly in WASM or native clients with zero VT
    /// parsing overhead.
    fn snapshot_binary_parts(
        &self,
        include_history: bool,
        max_cells: Option<usize>,
    ) -> (
        crate::server::binary_snapshot::FasttyBinarySnapshotHeader,
        Vec<crate::server::binary_snapshot::FasttyPackedCell>,
    ) {
        use crate::server::binary_snapshot::{
            BINARY_SNAPSHOT_MAGIC, BINARY_SNAPSHOT_VERSION, CELL_FLAG_BOLD, CELL_FLAG_DEFAULT_BG,
            CELL_FLAG_DEFAULT_FG, CELL_FLAG_DIM, CELL_FLAG_HIDDEN, CELL_FLAG_INVERSE,
            CELL_FLAG_ITALIC, CELL_FLAG_STRIKETHROUGH, CELL_FLAG_UNDERLINE, CELL_FLAG_WIDE_CHAR,
            CELL_FLAG_WIDE_CHAR_SPACER, FasttyBinarySnapshotHeader, FasttyPackedCell,
            SNAPSHOT_FLAG_ALT_SCREEN, SNAPSHOT_FLAG_CURSOR_VISIBLE,
        };
        use alacritty_terminal::index::Line;
        use alacritty_terminal::term::cell::Flags;
        use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor};

        // Named and 0-15 indexed colors resolve against the ACTIVE theme —
        // snapshots used to hardcode Catppuccin Mocha, feeding web clients
        // wrong colors under any other theme. Built once per snapshot.
        let theme_name = crate::config::ACTIVE_THEME.read().clone();
        let theme = crate::ui::theme::Theme::from_name(&theme_name);
        let hsla_to_rgb = |c: gpui::Hsla| -> u32 {
            let rgba = gpui::Rgba::from(c);
            let ch = |v: f32| ((v.clamp(0.0, 1.0)) * 255.0).round() as u32;
            (ch(rgba.r) << 16) | (ch(rgba.g) << 8) | ch(rgba.b)
        };
        let palette: [u32; 16] = [
            hsla_to_rgb(theme.black),
            hsla_to_rgb(theme.red),
            hsla_to_rgb(theme.green),
            hsla_to_rgb(theme.yellow),
            hsla_to_rgb(theme.blue),
            hsla_to_rgb(theme.magenta),
            hsla_to_rgb(theme.cyan),
            hsla_to_rgb(theme.white),
            hsla_to_rgb(theme.bright_black),
            hsla_to_rgb(theme.bright_red),
            hsla_to_rgb(theme.bright_green),
            hsla_to_rgb(theme.bright_yellow),
            hsla_to_rgb(theme.bright_blue),
            hsla_to_rgb(theme.bright_magenta),
            hsla_to_rgb(theme.bright_cyan),
            hsla_to_rgb(theme.bright_white),
        ];
        let default_fg = hsla_to_rgb(theme.foreground);
        let default_bg = hsla_to_rgb(theme.background);
        let named_to_rgb = |named: NamedColor| -> u32 {
            match named {
                NamedColor::Black => palette[0],
                NamedColor::Red => palette[1],
                NamedColor::Green => palette[2],
                NamedColor::Yellow => palette[3],
                NamedColor::Blue => palette[4],
                NamedColor::Magenta => palette[5],
                NamedColor::Cyan => palette[6],
                NamedColor::White => palette[7],
                NamedColor::BrightBlack => palette[8],
                NamedColor::BrightRed => palette[9],
                NamedColor::BrightGreen => palette[10],
                NamedColor::BrightYellow => palette[11],
                NamedColor::BrightBlue => palette[12],
                NamedColor::BrightMagenta => palette[13],
                NamedColor::BrightCyan => palette[14],
                NamedColor::BrightWhite => palette[15],
                _ => 0,
            }
        };

        fn color_to_rgb(
            color: AnsiColor,
            palette: &[u32; 16],
            default: u32,
            named_to_rgb: &dyn Fn(NamedColor) -> u32,
        ) -> u32 {
            match color {
                AnsiColor::Named(n) => {
                    let mapped = named_to_rgb(n);
                    // Foreground/Background/Dim resolve per channel via the
                    // caller's default (theme fg for fg, theme bg for bg).
                    if mapped == 0 { default } else { mapped }
                }
                AnsiColor::Indexed(i) => {
                    if i < 16 {
                        palette[i as usize]
                    } else if i >= 232 {
                        let gray = ((i - 232) as u32) * 10 + 8;
                        (gray << 16) | (gray << 8) | gray
                    } else {
                        let idx = i - 16;
                        let r = (idx / 36) % 6;
                        let g = (idx / 6) % 6;
                        let b = idx % 6;
                        let r_val = if r > 0 { (r * 40 + 55) as u32 } else { 0 };
                        let g_val = if g > 0 { (g * 40 + 55) as u32 } else { 0 };
                        let b_val = if b > 0 { (b * 40 + 55) as u32 } else { 0 };
                        (r_val << 16) | (g_val << 8) | b_val
                    }
                }
                AnsiColor::Spec(rgb) => {
                    ((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | (rgb.b as u32)
                }
            }
        }

        let cell_to_packed = |cell: &alacritty_terminal::term::cell::Cell| -> FasttyPackedCell {
            let mut flags = 0u16;
            if cell.fg == AnsiColor::Named(NamedColor::Foreground) {
                flags |= CELL_FLAG_DEFAULT_FG;
            }
            if cell.bg == AnsiColor::Named(NamedColor::Background) {
                flags |= CELL_FLAG_DEFAULT_BG;
            }
            if cell.flags.contains(Flags::BOLD) {
                flags |= CELL_FLAG_BOLD;
            }
            if cell.flags.contains(Flags::DIM) {
                flags |= CELL_FLAG_DIM;
            }
            if cell.flags.contains(Flags::ITALIC) {
                flags |= CELL_FLAG_ITALIC;
            }
            if cell.flags.intersects(Flags::ALL_UNDERLINES) {
                flags |= CELL_FLAG_UNDERLINE;
            }
            if cell.flags.contains(Flags::INVERSE) {
                flags |= CELL_FLAG_INVERSE;
            }
            if cell.flags.contains(Flags::HIDDEN) {
                flags |= CELL_FLAG_HIDDEN;
            }
            if cell.flags.contains(Flags::STRIKEOUT) {
                flags |= CELL_FLAG_STRIKETHROUGH;
            }
            if cell.flags.contains(Flags::WIDE_CHAR) {
                flags |= CELL_FLAG_WIDE_CHAR;
            }
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                flags |= CELL_FLAG_WIDE_CHAR_SPACER;
            }

            let c = if cell.c == '\0' {
                ' ' as u32
            } else {
                cell.c as u32
            };
            FasttyPackedCell {
                c,
                fg: color_to_rgb(cell.fg, &palette, default_fg, &named_to_rgb),
                bg: color_to_rgb(cell.bg, &palette, default_bg, &named_to_rgb),
                flags,
                _reserved: 0,
            }
        };

        let term = self.term.lock();
        let grid = term.grid();
        let screen_lines = grid.screen_lines();
        let cols = grid.columns();
        let snapshot_is_alt = term
            .mode()
            .contains(alacritty_terminal::term::TermMode::ALT_SCREEN);
        // Alt-screen apps don't have history; the alternate grid replaces it.
        let history_rows = if snapshot_is_alt || !include_history {
            0
        } else {
            let max_history_rows = max_cells
                .map(|max| max.saturating_sub(screen_lines.saturating_mul(cols)) / cols.max(1))
                .unwrap_or(usize::MAX);
            grid.total_lines()
                .saturating_sub(screen_lines)
                .min(max_history_rows)
        };

        let total_cells = (history_rows + screen_lines) * cols;
        let mut cells = Vec::with_capacity(total_cells);

        // History rows, oldest first, then the visible screen top to bottom.
        for i in 0..history_rows {
            let line = Line(-((history_rows - i) as i32));
            let row = &grid[line];
            for col in 0..cols {
                cells.push(cell_to_packed(&row[alacritty_terminal::index::Column(col)]));
            }
        }
        for line_i in 0..screen_lines {
            let row = &grid[Line(line_i as i32)];
            for col in 0..cols {
                cells.push(cell_to_packed(&row[alacritty_terminal::index::Column(col)]));
            }
        }

        let mut snapshot_flags = 0u16;
        let mode = *term.mode();
        if mode.contains(alacritty_terminal::term::TermMode::ALT_SCREEN) {
            snapshot_flags |= SNAPSHOT_FLAG_ALT_SCREEN;
        }
        if mode.contains(alacritty_terminal::term::TermMode::SHOW_CURSOR) {
            snapshot_flags |= SNAPSHOT_FLAG_CURSOR_VISIBLE;
        }

        let cursor_line = grid.cursor.point.line.0.max(0) as u16;
        let cursor_col = grid.cursor.point.column.0 as u16;

        let mut header = FasttyBinarySnapshotHeader {
            magic: BINARY_SNAPSHOT_MAGIC,
            version: BINARY_SNAPSHOT_VERSION,
            flags: snapshot_flags,
            cols: cols as u16,
            rows: screen_lines as u16,
            cursor_col,
            cursor_row: cursor_line,
            cursor_style: 0,
            _reserved1: 0,
            cell_count: cells.len() as u32,
            _reserved2: [0; 10],
        };
        header.set_history_rows(history_rows as u32);
        header.set_terminal_modes(mode.bits());
        header.set_kitty_flags(self.kitty_flags());

        (header, cells)
    }

    /// Take a full binary terminal state snapshot in microseconds.
    pub fn snapshot_binary(&self) -> Vec<u8> {
        let (header, cells) = self.snapshot_binary_parts(true, None);
        crate::server::binary_snapshot::encode_snapshot(&header, &cells)
    }

    /// Take a full binary terminal state snapshot compressed with Deflate.
    pub fn snapshot_binary_compressed(&self) -> Vec<u8> {
        const MAX_SAVED_CELLS: usize = 16 * 1024 * 1024 / 16;
        let (header, cells) = self.snapshot_binary_parts(true, Some(MAX_SAVED_CELLS));
        crate::server::binary_snapshot::encode_snapshot_compressed(&header, &cells)
    }

    /// Capture a bounded snapshot without compression for a background save.
    pub fn snapshot_binary_for_save(&self) -> Vec<u8> {
        const MAX_SAVED_CELLS: usize = 16 * 1024 * 1024 / 16;
        let (header, cells) = self.snapshot_binary_parts(true, Some(MAX_SAVED_CELLS));
        crate::server::binary_snapshot::encode_snapshot(&header, &cells)
    }

    pub fn snapshot_binary_compressed_with_history(&self, include_history: bool) -> Vec<u8> {
        let (header, cells) = self.snapshot_binary_parts(include_history, None);
        crate::server::binary_snapshot::encode_snapshot_compressed(&header, &cells)
    }

    /// Rebuilds the terminal from a binary snapshot (FST1 v2): scrollback
    /// history, screen cells, cursor position and alt-screen state. The
    /// running shell keeps going and prints at the restored cursor — the
    /// same semantics as tmux's `respawn-pane`. Returns false on malformed
    /// input, leaving the terminal untouched.
    pub fn restore_binary_snapshot(&self, data: &[u8]) -> bool {
        self.restore_binary_snapshot_internal(data, true)
    }

    fn restore_binary_snapshot_internal(&self, data: &[u8], resume_parser: bool) -> bool {
        use crate::server::binary_snapshot::decode_snapshot;
        use alacritty_terminal::term::TermMode;

        if data.starts_with(crate::server::native_snapshot::MAGIC) {
            let Ok(snapshot) = crate::server::native_snapshot::decode_state(data) else {
                return false;
            };
            let (cols, rows) = (
                snapshot.terminal.main.columns,
                snapshot.terminal.main.screen_lines,
            );
            let restored_ingress = match snapshot.continuation.filter(|_| resume_parser) {
                Some(continuation) => match Processor::from_snapshot(continuation.parser) {
                    Ok(processor) => ReaderIngress {
                        processor,
                        side: continuation.side,
                    },
                    Err(_) => return false,
                },
                None => ReaderIngress::default(),
            };
            let boundary = self.output_hub.boundary.lock();
            let mut term = self.term.lock();
            let mut ingress = self.ingress.lock();
            if term.restore_snapshot(snapshot.terminal).is_err() {
                return false;
            }
            *ingress = restored_ingress;
            if !resume_parser {
                ingress.processor.advance(&mut *term, PROMPT_TERMINAL_RESET);
            }
            drop(ingress);
            drop(term);
            drop(boundary);
            self.master
                .lock()
                .resize(PtySize {
                    rows: rows as u16,
                    cols: cols as u16,
                    pixel_width: (cols as u16).saturating_mul(8),
                    pixel_height: (rows as u16).saturating_mul(16),
                })
                .ok();
            if let Some(remote) = &self.remote_control {
                remote.resize(cols, rows);
            }
            self.render_generation.fetch_add(1, Ordering::Relaxed);
            return true;
        }

        let Some((header, cells)) = decode_snapshot(data) else {
            return false;
        };
        let cols = header.cols as usize;
        let rows = header.rows as usize;
        if cols == 0 || rows == 0 {
            return false;
        }
        let Some(screen_cells) = cols.checked_mul(rows) else {
            return false;
        };
        if cells.len() < screen_cells {
            return false;
        }
        let history_cells = cells.len() - screen_cells;
        if history_cells % cols != 0 {
            return false;
        }

        // Match the snapshot's size; the PTY resize signals the shell so it
        // redraws at the restored dimensions.
        self.resize(cols, rows);

        let mut term = self.term.lock();
        // The replay targets the main grid (that's where scrollback lives).
        // A snapshot taken inside an alt-screen app restores its visible
        // content into the main grid — visually identical, and the next
        // full-screen app takes over the alt grid as usual.
        if term.mode().contains(TermMode::ALT_SCREEN) {
            term.swap_alt();
        }
        let ok = apply_snapshot_to_grid(&mut term, &header, &cells);
        drop(term);
        if ok {
            self.render_generation.fetch_add(1, Ordering::Relaxed);
        }
        ok
    }

    pub fn restore_saved_snapshot(&self, data: &[u8]) -> bool {
        if data.starts_with(crate::server::native_snapshot::MAGIC) {
            return self.restore_binary_snapshot_internal(data, false);
        }
        if !self.restore_binary_snapshot(data) {
            return false;
        }
        let _boundary = self.output_hub.boundary.lock();
        let mut term = self.term.lock();
        let mut ingress = self.ingress.lock();
        *ingress = ReaderIngress::default();
        ingress.processor.advance(&mut *term, PROMPT_TERMINAL_RESET);
        self.render_generation.fetch_add(1, Ordering::Relaxed);
        true
    }

    pub fn render_generation(&self) -> u64 {
        self.render_generation.load(Ordering::Relaxed)
    }

    pub fn term(&self) -> &Arc<ParkingMutex<alacritty_terminal::term::Term<EventListenerProxy>>> {
        &self.term
    }
}

#[derive(Debug, Clone)]
pub struct SearchSnippet {
    pub offset: usize,
    pub line_index: i32,
    pub text: String,
}

/// Replays a decoded FST1 snapshot into an alacritty `Term`: resets the
/// active grid and feeds the snapshot's scrollback + screen cells through
/// the VT parser as ANSI, so history grows through alacritty's own scroll
/// machinery and the cursor lands where the snapshot says. Free function
/// so tests can drive a bare `Term` without a PTY.
fn apply_snapshot_to_grid(
    term: &mut alacritty_terminal::term::Term<EventListenerProxy>,
    header: &crate::server::binary_snapshot::FasttyBinarySnapshotHeader,
    cells: &[crate::server::binary_snapshot::FasttyPackedCell],
) -> bool {
    use crate::server::binary_snapshot::{
        CELL_FLAG_BOLD, CELL_FLAG_DEFAULT_BG, CELL_FLAG_DEFAULT_FG, CELL_FLAG_DIM,
        CELL_FLAG_HIDDEN, CELL_FLAG_INVERSE, CELL_FLAG_ITALIC, CELL_FLAG_STRIKETHROUGH,
        CELL_FLAG_UNDERLINE, CELL_FLAG_WIDE_CHAR_SPACER, SNAPSHOT_FLAG_CURSOR_VISIBLE,
    };
    use alacritty_terminal::vte::ansi::Processor;

    let cols = header.cols as usize;
    let rows = header.rows as usize;
    let screen_cells = cols * rows;
    if cols == 0 || rows == 0 || cells.len() < screen_cells {
        return false;
    }
    let history_cells = cells.len() - screen_cells;

    // Clear history, screen, cursor, display offset and scroll region of
    // the active grid; leave alt-screen to the caller.
    term.grid_mut().reset();

    let mut out = Vec::with_capacity(cells.len() * 4 + 64);
    out.extend_from_slice(b"\x1b[r");

    // One SGR block per style run, matching the packed cell's colors.
    let mut cur_fg = u32::MAX;
    let mut cur_bg = u32::MAX;
    let mut cur_flags = u16::MAX;
    let emit_sgr = |out: &mut Vec<u8>, fg: u32, bg: u32, flags: u16| {
        out.extend_from_slice(b"\x1b[0m");
        if flags & CELL_FLAG_BOLD != 0 {
            out.extend_from_slice(b"\x1b[1m");
        }
        if flags & CELL_FLAG_DIM != 0 {
            out.extend_from_slice(b"\x1b[2m");
        }
        if flags & CELL_FLAG_ITALIC != 0 {
            out.extend_from_slice(b"\x1b[3m");
        }
        if flags & CELL_FLAG_UNDERLINE != 0 {
            out.extend_from_slice(b"\x1b[4m");
        }
        if flags & CELL_FLAG_INVERSE != 0 {
            out.extend_from_slice(b"\x1b[7m");
        }
        if flags & CELL_FLAG_HIDDEN != 0 {
            out.extend_from_slice(b"\x1b[8m");
        }
        if flags & CELL_FLAG_STRIKETHROUGH != 0 {
            out.extend_from_slice(b"\x1b[9m");
        }
        if flags & CELL_FLAG_DEFAULT_FG != 0 {
            out.extend_from_slice(b"\x1b[39m");
        } else {
            out.extend_from_slice(
                format!(
                    "\x1b[38;2;{};{};{}m",
                    (fg >> 16) & 0xff,
                    (fg >> 8) & 0xff,
                    fg & 0xff
                )
                .as_bytes(),
            );
        }
        if flags & CELL_FLAG_DEFAULT_BG != 0 {
            out.extend_from_slice(b"\x1b[49m");
        } else {
            out.extend_from_slice(
                format!(
                    "\x1b[48;2;{};{};{}m",
                    (bg >> 16) & 0xff,
                    (bg >> 8) & 0xff,
                    bg & 0xff
                )
                .as_bytes(),
            );
        }
    };

    let mut emit_row = |out: &mut Vec<u8>,
                        row_cells: &[crate::server::binary_snapshot::FasttyPackedCell],
                        newline: bool| {
        for pc in row_cells {
            // Wide-char spacers are drawn by the parser's own width handling.
            if pc.flags & CELL_FLAG_WIDE_CHAR_SPACER != 0 {
                continue;
            }
            if pc.fg != cur_fg || pc.bg != cur_bg || pc.flags != cur_flags {
                emit_sgr(out, pc.fg, pc.bg, pc.flags);
                cur_fg = pc.fg;
                cur_bg = pc.bg;
                cur_flags = pc.flags;
            }
            let ch = char::from_u32(pc.c).unwrap_or(' ');
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
        if newline {
            // Blank the run state at line boundaries: simpler and safe.
            cur_fg = u32::MAX;
            cur_bg = u32::MAX;
            cur_flags = u16::MAX;
            out.extend_from_slice(b"\r\n");
        }
    };

    // Scrollback rows first: each newline scrolls one line into history.
    for r in 0..history_cells / cols {
        emit_row(&mut out, &cells[r * cols..(r + 1) * cols], true);
    }
    // Screen rows, no trailing newline on the last one.
    let screen_start = history_cells;
    for r in 0..rows {
        let last = r + 1 == rows;
        emit_row(
            &mut out,
            &cells[screen_start + r * cols..screen_start + (r + 1) * cols],
            !last,
        );
    }

    // Restore cursor position and visibility.
    let cursor_row = (header.cursor_row as usize).min(rows - 1) + 1;
    let cursor_col = (header.cursor_col as usize).min(cols - 1) + 1;
    out.extend_from_slice(format!("\x1b[{cursor_row};{cursor_col}H").as_bytes());
    if header.flags & SNAPSHOT_FLAG_CURSOR_VISIBLE != 0 {
        out.extend_from_slice(b"\x1b[?25h");
    } else {
        out.extend_from_slice(b"\x1b[?25l");
    }

    let mut parser: Processor<StdSyncHandler> = Processor::new();
    parser.advance(term, &out);
    true
}

/// Decode a `file://` URL or plain directory path to a local filesystem path.
/// Accepts `file:///abs/path`, `file://hostname/abs/path`, and raw `/path` or `C:\path` forms.
fn file_url_to_path(buf: &[u8]) -> Option<std::path::PathBuf> {
    let s = std::str::from_utf8(buf).ok()?;
    if let Some(path) = s.strip_prefix("file://") {
        let path = if let Some(slash_idx) = path.find('/') {
            &path[slash_idx..]
        } else {
            path
        };
        let decoded = percent_decode(path);
        Some(std::path::PathBuf::from(decoded))
    } else {
        if !s.is_empty()
            && (s.starts_with('/')
                || s.starts_with('~')
                || (s.len() >= 2 && s.chars().nth(1) == Some(':')))
        {
            Some(std::path::PathBuf::from(s))
        } else {
            None
        }
    }
}

/// Minimal percent-decode for file:// paths.
fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut table = [0u8; 256];
    for (i, &c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .iter()
        .enumerate()
    {
        table[c as usize] = i as u8;
    }

    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;

    for &b in s.as_bytes() {
        if b == b'=' {
            break;
        }
        let val = table[b as usize];
        if val == 0 && b != b'A' {
            continue; // Skip whitespace/invalid chars
        }
        buffer = (buffer << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
enum OscCommand {
    ProgramStatus(crate::program_status::ProgramStatusCommand),
    Cwd(String),
    SetTitle(String),
    CommandStarted,
    CommandFinished {
        duration_ms: u128,
        exit_code: Option<i32>,
    },
    PromptStarted,
    Notification {
        title: String,
        body: String,
    },
    NotificationQuery {
        id: String,
    },
    NotificationFragment {
        id: Option<String>,
        p_type: Option<String>,
        done: bool,
        payload: String,
    },
    ResetPalette,
    ResetColor {
        code: u8,
    },
    /// Kitty clipboard protocol (OSC 5522): metadata + raw payload halves.
    KittyClipboard {
        metadata: String,
        payload: Vec<u8>,
    },
    /// ConEmu `OSC 9;4` progress. `None` clears the indicator.
    Progress(Option<ProgressReport>),
}

/// Decodes the OSC sequences fastty acts on itself.
///
/// Color and palette queries (`OSC 4;n;?`, `OSC 10;?`, `OSC 11;?`, `OSC 12;?`)
/// are deliberately absent. `alacritty_terminal` already answers them through
/// `Event::ColorRequest`, and a second reply leaves the cursor position report
/// (`CSI 6 n`) unclaimed in the app's input stream. Go prompt libraries (`gh`,
/// `survey`) then print the report as literal text and abort the prompt.
#[cfg(test)]
fn parse_osc(buf: &[u8], cmd_start_time: &mut Option<std::time::Instant>) -> Option<OscCommand> {
    parse_osc_framed(buf, cmd_start_time, 4)
}

fn parse_osc_framed(
    buf: &[u8],
    cmd_start_time: &mut Option<std::time::Instant>,
    delimiter_bytes: usize,
) -> Option<OscCommand> {
    let semicolon = buf.iter().position(|&b| b == b';')?;
    let code = &buf[..semicolon];
    let payload = &buf[semicolon + 1..];

    match code {
        b"7501" => crate::program_status::parse(payload, buf.len() + delimiter_bytes)
            .map(OscCommand::ProgramStatus),
        b"0" | b"1" | b"2" => {
            if let Ok(title) = std::str::from_utf8(payload) {
                Some(OscCommand::SetTitle(title.to_string()))
            } else {
                None
            }
        }
        b"6" | b"7" | b"176" => {
            if let Ok(s) = std::str::from_utf8(payload) {
                Some(OscCommand::Cwd(s.to_string()))
            } else {
                None
            }
        }
        b"9" => {
            if let Some(rest) = payload.strip_prefix(b"4;") {
                return parse_progress(rest);
            }
            if let Ok(msg) = std::str::from_utf8(payload) {
                let (title, body) = if let Some(idx) = msg.find(';') {
                    let t = msg[..idx].trim();
                    let b = msg[idx + 1..].trim();
                    if t.is_empty() { ("Fastty", b) } else { (t, b) }
                } else {
                    ("Fastty", msg.trim())
                };
                Some(OscCommand::Notification {
                    title: title.to_string(),
                    body: body.to_string(),
                })
            } else {
                None
            }
        }
        b"5522" => {
            // Kitty clipboard protocol: the payload is itself
            // "metadata;payload" of the 5522 packet format.
            let (meta, rest) = match payload.iter().position(|&b| b == b';') {
                Some(pos) => (&payload[..pos], &payload[pos + 1..]),
                None => (payload, &[][..]),
            };
            Some(OscCommand::KittyClipboard {
                metadata: String::from_utf8_lossy(meta).into_owned(),
                payload: rest.to_vec(),
            })
        }
        b"99" => {
            let (metadata_bytes, payload_bytes) =
                if let Some(idx) = payload.iter().position(|&b| b == b';') {
                    (&payload[..idx], &payload[idx + 1..])
                } else {
                    (payload, &[][..])
                };

            let metadata = std::str::from_utf8(metadata_bytes).ok()?;
            let actual_payload = std::str::from_utf8(payload_bytes).ok()?;

            let mut id = None;
            let mut p_type = None;
            let mut done = true;
            let mut is_base64 = false;

            for part in metadata.split(':') {
                if let Some(eq) = part.find('=') {
                    let key = &part[..eq];
                    let val = &part[eq + 1..];
                    match key {
                        "i" => id = Some(val.to_string()),
                        "p" => p_type = Some(val.to_string()),
                        "d" => done = val == "1" || val != "0",
                        "e" => is_base64 = val == "1",
                        _ => {}
                    }
                }
            }

            let decoded_payload = if is_base64 {
                if let Some(bytes) = base64_decode(actual_payload) {
                    String::from_utf8(bytes).unwrap_or_else(|_| actual_payload.to_string())
                } else {
                    actual_payload.to_string()
                }
            } else {
                actual_payload.to_string()
            };

            if p_type.as_deref() == Some("?") {
                if let Some(query_id) = id {
                    return Some(OscCommand::NotificationQuery { id: query_id });
                }
            }

            Some(OscCommand::NotificationFragment {
                id,
                p_type,
                done,
                payload: decoded_payload,
            })
        }
        b"104" => Some(OscCommand::ResetPalette),
        b"110" | b"111" | b"112" => {
            let code_num = match code {
                b"110" => 10,
                b"111" => 11,
                b"112" => 12,
                _ => 10,
            };
            Some(OscCommand::ResetColor { code: code_num })
        }
        b"777" => {
            if payload.starts_with(b"notify;") {
                let parts: Vec<&[u8]> = payload[7..].split(|&b| b == b';').collect();
                if parts.len() >= 2 {
                    let title = std::str::from_utf8(parts[0])
                        .unwrap_or("Fastty")
                        .to_string();
                    let body = parts[1..]
                        .iter()
                        .map(|p| std::str::from_utf8(p).unwrap_or(""))
                        .collect::<Vec<&str>>()
                        .join(";");
                    Some(OscCommand::Notification { title, body })
                } else if parts.len() == 1 {
                    let body = std::str::from_utf8(parts[0]).unwrap_or("").to_string();
                    Some(OscCommand::Notification {
                        title: "Fastty".to_string(),
                        body,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        }
        b"133" => match payload {
            b"A" => Some(OscCommand::PromptStarted),
            b"B" => {
                *cmd_start_time = Some(std::time::Instant::now());
                Some(OscCommand::CommandStarted)
            }
            b"C" => {
                if cmd_start_time.is_none() {
                    *cmd_start_time = Some(std::time::Instant::now());
                }
                Some(OscCommand::CommandStarted)
            }
            b"D" => {
                if let Some(start) = cmd_start_time.take() {
                    let duration_ms = start.elapsed().as_millis();
                    Some(OscCommand::CommandFinished {
                        duration_ms,
                        exit_code: None,
                    })
                } else {
                    None
                }
            }
            _ => {
                if payload.starts_with(b"D;") {
                    let exit_str = std::str::from_utf8(&payload[2..]).unwrap_or("0");
                    let exit_code: Option<i32> = exit_str.parse().ok();
                    if let Some(start) = cmd_start_time.take() {
                        let duration_ms = start.elapsed().as_millis();
                        Some(OscCommand::CommandFinished {
                            duration_ms,
                            exit_code,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        },
        b"633" => match payload {
            b"A" => Some(OscCommand::PromptStarted),
            b"B" => {
                *cmd_start_time = Some(std::time::Instant::now());
                Some(OscCommand::CommandStarted)
            }
            b"C" => {
                if cmd_start_time.is_none() {
                    *cmd_start_time = Some(std::time::Instant::now());
                }
                Some(OscCommand::CommandStarted)
            }
            b"D" => {
                if let Some(start) = cmd_start_time.take() {
                    let duration_ms = start.elapsed().as_millis();
                    Some(OscCommand::CommandFinished {
                        duration_ms,
                        exit_code: None,
                    })
                } else {
                    None
                }
            }
            _ => {
                if payload.starts_with(b"D;") {
                    let exit_str = std::str::from_utf8(&payload[2..]).unwrap_or("0");
                    let exit_code: Option<i32> = exit_str.parse().ok();
                    if let Some(start) = cmd_start_time.take() {
                        let duration_ms = start.elapsed().as_millis();
                        Some(OscCommand::CommandFinished {
                            duration_ms,
                            exit_code,
                        })
                    } else {
                        None
                    }
                } else if payload.starts_with(b"P;Cwd=") {
                    let cwd_str = std::str::from_utf8(&payload[6..]).ok()?;
                    Some(OscCommand::Cwd(cwd_str.to_string()))
                } else {
                    None
                }
            }
        },
        b"1337" => {
            if let Ok(s) = std::str::from_utf8(payload) {
                s.strip_prefix("CurrentDir=")
                    .map(|dir| OscCommand::Cwd(dir.to_string()))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Parse the `state[;percent]` tail of ConEmu's `OSC 9;4`.
///
/// States: 0 hide, 1 normal, 2 error, 3 indeterminate, 4 paused.
/// A missing or out-of-range percentage keeps the report valid.
fn parse_progress(payload: &[u8]) -> Option<OscCommand> {
    let mut parts = payload.split(|byte| *byte == b';');
    let state = parse_progress_number(parts.next()?)?;
    let percent = match parts.next() {
        Some(raw) if !raw.is_empty() => {
            let value = parse_progress_number(raw)?;
            if value > 100 {
                return None;
            }
            Some(value as u8)
        }
        _ => None,
    };
    let report = match state {
        0 => return Some(OscCommand::Progress(None)),
        1 => ProgressReport {
            state: ProgressState::Normal,
            percent,
        },
        2 => ProgressReport {
            state: ProgressState::Error,
            percent,
        },
        3 => ProgressReport {
            state: ProgressState::Indeterminate,
            percent: None,
        },
        4 => ProgressReport {
            state: ProgressState::Paused,
            percent,
        },
        _ => return None,
    };
    Some(OscCommand::Progress(Some(report)))
}

fn parse_progress_number(value: &[u8]) -> Option<u32> {
    if value.is_empty() || value.len() > 3 || !value.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(value).ok()?.parse().ok()
}

fn mirror_program_status_sequence(payload: &[u8]) -> Vec<u8> {
    let mut sequence = Vec::with_capacity(payload.len() + 10);
    sequence.extend_from_slice(b"\x1b]7501;");
    sequence.extend_from_slice(payload);
    sequence.extend_from_slice(b"\x1b\\");
    sequence
}

fn update_program_status(
    store: &ParkingMutex<crate::program_status::ProgramStatusStore>,
    pending: &std::sync::atomic::AtomicBool,
    listener: &EventListenerProxy,
    source_id: u64,
    update: impl FnOnce(&mut crate::program_status::ProgramStatusStore),
) {
    let mut store = store.lock();
    let before = store.snapshot();
    update(&mut store);
    let changed = store.snapshot() != before;
    let send_update = changed && !pending.swap(true, Ordering::Relaxed);
    drop(store);
    if send_update {
        listener.send_app_event(AppEvent::ProgramStatusChanged { source_id });
    }
}

fn dispatch_osc_action(
    cmd: &OscCommand,
    sender: &crate::event_listener::EventListenerProxy,
    cursor_line: i32,
    absolute_base: u64,
    screen_lines: i32,
) {
    match cmd {
        // The kitty clipboard protocol is fully handled in the reader
        // thread (`handle_cmd`): it needs the PTY writer to reply.
        OscCommand::KittyClipboard { .. } | OscCommand::ProgramStatus(_) => {}
        OscCommand::Progress(_) => {}
        OscCommand::Cwd(path) => {
            if let Some(p) = file_url_to_path(path.as_bytes()) {
                sender.send_app_event(AppEvent::CwdChanged(p));
            }
        }
        OscCommand::SetTitle(title) => {
            sender.send_app_event(AppEvent::TitleChanged(title.clone()));
        }
        OscCommand::CommandStarted => {
            sender.send_app_event(AppEvent::CommandStarted);
        }
        OscCommand::CommandFinished {
            duration_ms,
            exit_code,
        } => {
            sender.send_app_event(AppEvent::CommandFinished {
                duration_ms: *duration_ms,
                exit_code: *exit_code,
            });
        }
        OscCommand::PromptStarted => {
            let scrolled = (absolute_base as i32 - screen_lines).max(0);
            let absolute_line = scrolled + cursor_line;
            sender.send_app_event(AppEvent::PromptStarted {
                absolute_line: absolute_line.max(0) as u64,
            });
        }
        OscCommand::Notification { title, body } => {
            sender.send_app_event(AppEvent::Notification {
                title: title.clone(),
                body: body.clone(),
            });
        }
        OscCommand::NotificationQuery { .. }
        | OscCommand::NotificationFragment { .. }
        | OscCommand::ResetPalette
        | OscCommand::ResetColor { .. } => {}
    }
}

fn dispatch_notification_if_allowed(
    title: &str,
    body: &str,
    program_status: &ParkingMutex<crate::program_status::ProgramStatusStore>,
    sender: &EventListenerProxy,
    cursor_line: i32,
    absolute_base: u64,
    screen_lines: i32,
) {
    if !program_status.lock().should_deliver_native_notification() {
        return;
    }
    dispatch_osc_action(
        &OscCommand::Notification {
            title: title.to_owned(),
            body: body.to_owned(),
        },
        sender,
        cursor_line,
        absolute_base,
        screen_lines,
    );
}

#[cfg(target_os = "macos")]
fn get_process_cwd(pid: u32) -> Option<std::path::PathBuf> {
    use std::ffi::CStr;
    use std::os::raw::{c_int, c_void};

    #[repr(C)]
    struct VnodeInfoPath {
        _vip_vi: [u8; 152],
        vip_path: [libc::c_char; 1024],
    }

    #[repr(C)]
    struct ProcVnodePathInfo {
        pvi_cdir: VnodeInfoPath,
        pvi_rdir: VnodeInfoPath,
    }

    extern "C" {
        fn proc_pidinfo(
            pid: c_int,
            flavor: c_int,
            arg: u64,
            buffer: *mut c_void,
            buffersize: c_int,
        ) -> c_int;
    }

    const PROC_PIDVNODEPATHINFO: c_int = 9;

    let mut path_info = std::mem::MaybeUninit::<ProcVnodePathInfo>::uninit();
    let size = std::mem::size_of::<ProcVnodePathInfo>() as c_int;
    let res = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDVNODEPATHINFO,
            0,
            path_info.as_mut_ptr() as *mut c_void,
            size,
        )
    };

    if res == size {
        let path_info = unsafe { path_info.assume_init() };
        let c_str = unsafe { CStr::from_ptr(path_info.pvi_cdir.vip_path.as_ptr()) };
        if let Ok(path_str) = c_str.to_str() {
            if !path_str.is_empty() {
                return Some(std::path::PathBuf::from(path_str));
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn get_process_name_macos(pid: u32) -> Option<String> {
    use std::os::raw::{c_int, c_void};
    extern "C" {
        fn proc_name(pid: c_int, buffer: *mut c_void, buffersize: u32) -> c_int;
    }
    let mut name_buf = [0u8; 256];
    let res = unsafe {
        proc_name(
            pid as c_int,
            name_buf.as_mut_ptr() as *mut c_void,
            name_buf.len() as u32,
        )
    };
    if res > 0 {
        let name_bytes = &name_buf[..res as usize];
        // Strip trailing null bytes if present
        let clean_len = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(name_bytes.len());
        let name = String::from_utf8_lossy(&name_bytes[..clean_len])
            .trim()
            .to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn has_active_child_macos(shell_pid: u32, foreground_pgid: libc::pid_t) -> bool {
    use std::os::raw::{c_int, c_uint, c_void};

    extern "C" {
        fn proc_listpids(
            proc_type: c_uint,
            proc_type_arg: c_uint,
            buffer: *mut c_void,
            buffersize: c_int,
        ) -> c_int;
    }
    const PROC_PPID_ONLY: c_uint = 6;

    let mut pids = [0i32; 64];
    let res = unsafe {
        proc_listpids(
            PROC_PPID_ONLY,
            shell_pid as c_uint,
            pids.as_mut_ptr() as *mut c_void,
            std::mem::size_of_val(&pids) as c_int,
        )
    };
    if res > 0 {
        let count = (res as usize) / std::mem::size_of::<i32>();
        for &p in &pids[..count] {
            if p > 0 && p != (shell_pid as i32) {
                let child_pgid = unsafe { libc::getpgid(p) };
                if child_pgid == foreground_pgid {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(target_os = "linux")]
fn get_process_cwd(pid: u32) -> Option<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{}/cwd", pid)).ok()
}

#[cfg(target_os = "linux")]
fn has_active_child_linux(shell_pid: u32, foreground_pgid: libc::pid_t) -> bool {
    if let Ok(content) =
        std::fs::read_to_string(format!("/proc/{}/task/{}/children", shell_pid, shell_pid))
    {
        return content
            .split_whitespace()
            .filter_map(|value| value.parse::<libc::pid_t>().ok())
            .any(|child_pid| unsafe { libc::getpgid(child_pid) } == foreground_pgid);
    }
    false
}

#[cfg(target_os = "windows")]
fn get_process_cwd(_pid: u32) -> Option<std::path::PathBuf> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn get_foreground_process_name_other(_shell_pid: u32) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_program_status_wraps_payload_in_osc_7501() {
        assert_eq!(
            mirror_program_status_sequence(b"state=done:id=agent:app=opencode"),
            b"\x1b]7501;state=done:id=agent:app=opencode\x1b\\".to_vec()
        );
    }
    use super::CsiEvent::{self, *};
    use super::CsiWatcher;
    use super::*;

    fn run(bytes: &[u8]) -> Vec<CsiEvent> {
        let mut w = CsiWatcher::new();
        bytes.iter().filter_map(|&b| w.feed(b)).collect()
    }

    #[test]
    fn kitty_keyboard_sequences_parse() {
        use crate::kitty_keyboard::FLAG_DISAMBIGUATE;

        assert_eq!(run(b"\x1b[>1u"), vec![KittyPush(FLAG_DISAMBIGUATE)]);
        assert_eq!(run(b"\x1b[>u"), vec![KittyPush(0)]);
        assert_eq!(run(b"\x1b[<u"), vec![KittyPop(1)]);
        assert_eq!(run(b"\x1b[<3u"), vec![KittyPop(3)]);
        assert_eq!(run(b"\x1b[=1;1u"), vec![KittySet(1, 1)]);
        assert_eq!(run(b"\x1b[=;2u"), vec![KittySet(0, 2)]);
        assert_eq!(run(b"\x1b[?u"), vec![KittyQuery]);

        // Plain CSI u stays save-cursor: no kitty event.
        assert!(run(b"\x1b[u").is_empty());
        // Mode-set sequences with other final bytes are untouched.
        assert!(run(b"\x1b[?1049h").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn terminal_query_replies_are_complete_and_terminate() {
        // Mirrors the gh/termenv startup pattern: the child sends OSC 11,
        // OSC 10, DA1, and the kitty keyboard query, then reads its stdin
        // raw. survey then reads anything that arrives LATE and chokes on
        // `ESC ]` (crash class: `could not prompt: unexpected escape
        // sequence from terminal: ['\x1b' ']']`). This test pins the wire:
        // exactly one complete reply per query, and silence afterwards.
        let probe = r#"
import os, sys, termios, time, select, tty
fd = 0
old = termios.tcgetattr(fd)
tty.setraw(fd)
sys.stdout.write("\x1b]11;?\x1b\\\x1b]10;?\x1b\\\x1b[c\x1b[?u")
sys.stdout.flush()
def drain(seconds):
    buf = b""
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            buf += os.read(fd, 4096)
    return buf
first = drain(1.5)
late = drain(1.0)
sys.stderr.write("REPLY:" + first.hex() + "\n")
sys.stderr.write("LATE:" + late.hex() + "\n")
termios.tcsetattr(fd, termios.TCSADRAIN, old)
"#;
        let terminal = TerminalState::new_headless(
            "/usr/bin/python3",
            &["-c".to_string(), probe.to_string()],
            None,
            80,
            24,
        )
        .expect("create headless terminal for query probe");
        let output = terminal.subscribe_output();
        let mut reply = String::new();
        let mut late = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline && (reply.is_empty() || late.is_empty()) {
            match output.try_recv() {
                Ok(chunk) => {
                    let text = String::from_utf8_lossy(&chunk).to_string();
                    if let Some(rest) = text.strip_prefix("REPLY:") {
                        reply = rest.trim().to_string();
                    }
                    if let Some(rest) = text.strip_prefix("LATE:") {
                        late = rest.trim().to_string();
                    }
                }
                Err(async_channel::TryRecvError::Empty) => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(async_channel::TryRecvError::Closed) => break,
            }
        }
        eprintln!("reply={reply} late={late}");

        // Exactly one complete reply per query: OSC 11, OSC 10, DA1, kitty.
        // `reply` is the hex of the wire bytes: 1b5d = ESC ], 1b5b = ESC [.
        assert_eq!(reply.matches("1b5d").count(), 2, "OSC replies: {reply}");
        assert_eq!(reply.matches("1b5b").count(), 2, "CSI replies: {reply}");
        assert!(reply.contains("7267623a"), "rgb: payload present: {reply}"); // "rgb:"
        // Nothing may arrive after the replies: survey reads next and any
        // stray byte is a fatal `unexpected escape sequence`.
        assert!(late.is_empty(), "stray input after replies: {late}");
    }

    #[cfg(unix)]
    #[test]
    fn external_program_status_mirrors_osc_7501_to_output_subscribers() {
        let terminal = TerminalState::new_headless("/bin/sh", &[], None, 80, 24)
            .expect("create terminal for status mirror test");
        let output = terminal.subscribe_output();
        terminal
            .report_external_program_status(b"state=done:id=agent:app=opencode")
            .expect("external status report is accepted");
        let chunk = loop {
            match output.recv_blocking() {
                Ok(chunk) => break chunk,
                Err(_) => panic!("mirrored status should reach output subscribers"),
            }
        };
        assert_eq!(
            chunk,
            b"\x1b]7501;state=done:id=agent:app=opencode\x1b\\".to_vec()
        );
    }

    #[cfg(unix)]
    #[test]
    fn try_resize_returns_without_waiting_for_the_terminal_lock() {
        let terminal = TerminalState::new_headless("/bin/sh", &[], None, 80, 24)
            .expect("create terminal for resize test");
        let term = terminal.term();
        let guard = term.lock();
        let started = std::time::Instant::now();
        assert!(!terminal.try_resize_with_pixels(100, 30, 800, 480));
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
        drop(guard);

        assert!(terminal.try_resize_with_pixels(100, 30, 800, 480));
        assert_eq!(terminal.dimensions(), (100, 30));
    }

    #[cfg(unix)]
    #[test]
    fn try_history_metrics_returns_without_waiting_for_the_terminal_lock() {
        let terminal = TerminalState::new_headless("/bin/sh", &[], None, 80, 24)
            .expect("create terminal for history metrics test");
        let term = terminal.term();
        let guard = term.lock();
        let started = std::time::Instant::now();
        assert_eq!(terminal.try_history_metrics(), None);
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
        drop(guard);

        assert_eq!(terminal.try_history_metrics(), Some((0, 0)));
    }

    #[test]
    fn remote_input_queue_rejects_writes_without_blocking_when_full() {
        let (sender, _receiver) = std::sync::mpsc::sync_channel(0);
        let queued_bytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut writer = RemoteInputWriter {
            sender,
            failed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            queued_bytes: Arc::clone(&queued_bytes),
            warned_queue_full: false,
        };

        let started = std::time::Instant::now();
        let error = writer.write(b"response").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert_eq!(queued_bytes.load(Ordering::Acquire), 0);
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
    }

    #[test]
    fn test_child_process_detection() {
        let mut child = std::process::Command::new("sleep")
            .arg("2")
            .spawn()
            .expect("failed to spawn sleep");
        let parent_pid = std::process::id();
        let foreground_pgid = unsafe { libc::getpgid(child.id() as libc::pid_t) };
        #[cfg(target_os = "macos")]
        assert!(has_active_child_macos(parent_pid, foreground_pgid));
        #[cfg(target_os = "linux")]
        assert!(has_active_child_linux(parent_pid, foreground_pgid));
        let _ = child.kill();
    }

    #[test]
    fn detects_synchronized_output_set_and_reset() {
        assert_eq!(run(b"\x1b[?2026h"), vec![SyncOutputBegin]);
        assert_eq!(run(b"\x1b[?2026l"), vec![SyncOutputEnd]);
    }

    #[test]
    fn detects_2026_combined_with_other_private_modes() {
        assert_eq!(run(b"\x1b[?1049;2026h"), vec![SyncOutputBegin]);
        assert_eq!(run(b"\x1b[?2026;1049l"), vec![SyncOutputEnd]);
    }

    #[test]
    fn detects_2026_across_split_chunks() {
        let mut w = CsiWatcher::new();
        assert_eq!(w.feed(0x1b), None);
        assert_eq!(w.feed(b'['), None);
        assert_eq!(w.feed(b'?'), None);
        assert_eq!(w.feed(b'2'), None);
        assert_eq!(w.feed(b'0'), None);
        assert_eq!(w.feed(b'2'), None);
        assert_eq!(w.feed(b'6'), None);
        assert_eq!(w.feed(b'h'), Some(SyncOutputBegin));
    }

    #[test]
    fn plain_2j_clears_history_but_private_2j_does_not() {
        assert_eq!(run(b"\x1b[2J"), vec![ClearHistory]);
        assert_eq!(run(b"\x1b[?2J"), Vec::<CsiEvent>::new());
        assert_eq!(run(b"\x1b[3J"), Vec::<CsiEvent>::new());
    }

    #[test]
    fn other_private_modes_are_ignored() {
        assert_eq!(
            run(b"\x1b[?1049h\x1b[?25l\x1b[?1000;1006h"),
            Vec::<CsiEvent>::new()
        );
    }

    #[test]
    fn plain_text_emits_nothing() {
        assert_eq!(run(b"hello \x1b]0;title\x07 world"), Vec::<CsiEvent>::new());
    }

    #[test]
    fn test_osc_parsing() {
        use super::parse_osc;
        let mut cmd_start = None;

        // OSC 0/1/2 - Title
        let cmd = parse_osc(b"0;My Terminal", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::SetTitle(t) if t == "My Terminal"));

        // OSC 6, 7, 176 - CWD
        let cmd = parse_osc(b"7;file:///Users/foo/project", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::Cwd(p) if p == "file:///Users/foo/project"));

        let cmd = parse_osc(b"6;file:///tmp", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::Cwd(p) if p == "file:///tmp"));

        let cmd = parse_osc(b"176;file:///var", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::Cwd(p) if p == "file:///var"));

        // OSC 1337 CurrentDir
        let cmd = parse_osc(b"1337;CurrentDir=/home/user", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::Cwd(p) if p == "/home/user"));

        // OSC 133 - Shell Integration
        let cmd = parse_osc(b"133;A", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::PromptStarted));

        let cmd = parse_osc(b"133;B", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::CommandStarted));
        assert!(cmd_start.is_some());

        let cmd = parse_osc(b"133;D;0", &mut cmd_start).unwrap();
        assert!(matches!(
            cmd,
            super::OscCommand::CommandFinished {
                exit_code: Some(0),
                ..
            }
        ));

        // OSC 633 - VS Code Shell Integration
        let cmd = parse_osc(b"633;A", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::PromptStarted));

        let cmd = parse_osc(b"633;P;Cwd=/opt/app", &mut cmd_start).unwrap();
        assert!(matches!(cmd, super::OscCommand::Cwd(p) if p == "/opt/app"));

        // OSC 10/11/12 (color) and OSC 4 (palette) queries are answered by
        // alacritty_terminal through `Event::ColorRequest`. fastty's own
        // scanner must stay silent for them: a second reply leaves the cursor
        // position report in the app's input stream, and Go prompt libraries
        // (`gh`, `survey`) then print it and abort the prompt.
        assert!(parse_osc(b"10;?", &mut cmd_start).is_none());
        assert!(parse_osc(b"11;?", &mut cmd_start).is_none());
        assert!(parse_osc(b"12;?", &mut cmd_start).is_none());
        assert!(parse_osc(b"4;2;?", &mut cmd_start).is_none());
        assert!(parse_osc(b"4;?;2", &mut cmd_start).is_none());
    }

    #[test]
    fn notification_protocols_parse_to_the_filtered_command_path() {
        let mut cmd_start = None;
        for sequence in [b"9;hello".as_slice(), b"777;notify;4;3".as_slice()] {
            assert!(matches!(
                parse_osc(sequence, &mut cmd_start),
                Some(OscCommand::Notification { .. })
            ));
        }
        assert!(matches!(
            parse_osc(b"99;i=job:p=body;3", &mut cmd_start),
            Some(OscCommand::NotificationFragment { .. })
        ));
    }

    #[test]
    fn osc_94_progress_parses_by_state() {
        let mut cmd_start = None;
        let mut parse = |sequence: &[u8]| parse_osc(sequence, &mut cmd_start);

        // State 0 clears the indicator.
        assert_eq!(parse(b"9;4;0"), Some(OscCommand::Progress(None)));

        // State 1 with and without a percentage.
        assert_eq!(
            parse(b"9;4;1;42"),
            Some(OscCommand::Progress(Some(ProgressReport {
                state: ProgressState::Normal,
                percent: Some(42),
            })))
        );
        assert_eq!(
            parse(b"9;4;1"),
            Some(OscCommand::Progress(Some(ProgressReport {
                state: ProgressState::Normal,
                percent: None,
            })))
        );

        // State 3 is indeterminate: the percentage is dropped.
        assert_eq!(
            parse(b"9;4;3"),
            Some(OscCommand::Progress(Some(ProgressReport {
                state: ProgressState::Indeterminate,
                percent: None,
            })))
        );
        assert_eq!(
            parse(b"9;4;2;75"),
            Some(OscCommand::Progress(Some(ProgressReport {
                state: ProgressState::Error,
                percent: Some(75),
            })))
        );
        assert_eq!(
            parse(b"9;4;4;10"),
            Some(OscCommand::Progress(Some(ProgressReport {
                state: ProgressState::Paused,
                percent: Some(10),
            })))
        );

        // Out-of-range and malformed payloads are ignored, not shown as
        // notifications.
        assert_eq!(parse(b"9;4;1;250"), None);
        assert_eq!(parse(b"9;4;x;5"), None);
        assert_eq!(parse(b"9;4;7"), None);
    }

    #[test]
    fn native_notifications_are_suppressed_until_program_completion() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let writer: Box<dyn Write + Send> = Box::new(std::io::sink());
        let sender = EventListenerProxy::from_arc(Arc::new(ParkingMutex::new(writer)));
        sender.set_event_sender(crate::event_listener::EventSender::Callback(Arc::new(
            move |event| captured_events.lock().unwrap().push(event),
        )));
        let program_status =
            ParkingMutex::new(crate::program_status::ProgramStatusStore::default());
        program_status
            .lock()
            .apply(crate::program_status::ProgramStatusCommand::Report(
                crate::program_status::ProgramRecord {
                    id: "pi".to_owned(),
                    state: crate::program_status::ProgramState::Working,
                    app: Some("pi".to_owned()),
                    title: None,
                    msg: None,
                    progress: None,
                    kind: None,
                },
            ));

        dispatch_notification_if_allowed("4", "3", &program_status, &sender, 0, 0, 24);
        assert!(events.lock().unwrap().is_empty());

        program_status
            .lock()
            .apply(crate::program_status::ProgramStatusCommand::Report(
                crate::program_status::ProgramRecord {
                    id: "pi".to_owned(),
                    state: crate::program_status::ProgramState::Done,
                    app: Some("pi".to_owned()),
                    title: None,
                    msg: None,
                    progress: None,
                    kind: None,
                },
            ));
        dispatch_notification_if_allowed("Pi", "Done", &program_status, &sender, 0, 0, 24);

        let events = events.lock().unwrap();
        assert!(matches!(
            events.as_slice(),
            [AppEvent::Notification { title, body }] if title == "Pi" && body == "Done"
        ));
    }

    /// A color query must produce exactly one reply.
    ///
    /// `gh auth login` and other Go prompt libraries send `OSC 11;?` followed by
    /// `CSI 6 n`, then read both replies. Two replies to the color query leave
    /// the cursor position report in the input stream: the prompt echoes it as
    /// literal text and aborts with "unexpected escape sequence from terminal".
    #[test]
    fn color_query_gets_exactly_one_reply() {
        use alacritty_terminal::term::test::TermSize;
        use alacritty_terminal::term::{Config, Term};
        use alacritty_terminal::vte::ansi::Processor;
        use parking_lot::Mutex;
        use std::io::Write;
        use std::sync::Arc;

        /// Collects every byte written back to the PTY.
        #[derive(Clone, Default)]
        struct CapturedPty(Arc<Mutex<Vec<u8>>>);

        impl Write for CapturedPty {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        // The reversed palette form (`OSC 4;?;n`) is not answered by
        // alacritty_terminal, so it only has to stay silent on fastty's side.
        assert!(parse_osc(b"4;?;2", &mut None).is_none());

        for (name, query) in [
            ("OSC 10 BEL", &b"\x1b]10;?\x07"[..]),
            ("OSC 10 ST", &b"\x1b]10;?\x1b\\"[..]),
            ("OSC 11 BEL", &b"\x1b]11;?\x07"[..]),
            ("OSC 11 ST", &b"\x1b]11;?\x1b\\"[..]),
            ("OSC 12 BEL", &b"\x1b]12;?\x07"[..]),
            ("OSC 12 ST", &b"\x1b]12;?\x1b\\"[..]),
            // The palette query and the multi-index form a colour-reporting
            // reader sends. The multi-index form is the one most likely to
            // desynchronise a prompt's reply reader.
            ("OSC 4 n;? BEL", &b"\x1b]4;2;?\x07"[..]),
            ("OSC 4 n;? ST", &b"\x1b]4;2;?\x1b\\"[..]),
            ("OSC 4 multi", &b"\x1b]4;1;?;2;?;3;?\x1b\\"[..]),
        ] {
            // fastty's own scanner must not claim these codes. This is the
            // assertion that guards the fix: alacritty_terminal is the single
            // answerer, so any second reply path has to start here.
            let body = query
                .strip_prefix(b"\x1b]")
                .and_then(|rest| {
                    rest.strip_suffix(b"\x07")
                        .or_else(|| rest.strip_suffix(b"\x1b\\"))
                })
                .unwrap_or(query);
            assert!(
                parse_osc(body, &mut None).is_none(),
                "fastty's OSC scanner must not answer {name}"
            );

            // alacritty_terminal answers through the event listener. One reply
            // per queried colour: a multi-index query reports each index once.
            let captured = CapturedPty::default();
            let writer: Box<dyn Write + Send> = Box::new(captured.clone());
            let proxy = EventListenerProxy::from_arc(Arc::new(Mutex::new(writer)));
            let size = TermSize::new(80, 24);
            let mut term = Term::new(Config::default(), &size, proxy);
            let mut parser: Processor = Processor::new();
            parser.advance(&mut term, query);

            let written = captured.0.lock().clone();
            // Split into replies. A reply ends at BEL or at the ESC of an
            // ST terminator, so an ST-terminated reply keeps a trailing ESC
            // and must be trimmed before the text is checked.
            let replies: Vec<String> = written
                .split(|b| *b == 0x07)
                .flat_map(|bell_terminated| {
                    bell_terminated
                        .split(|b| *b == b'\\')
                        .map(|st_terminated| String::from_utf8_lossy(st_terminated).to_string())
                })
                .filter(|chunk| chunk.starts_with("\x1b]"))
                .collect();

            // One reply per queried colour: the multi-index `OSC 4` form asks
            // for several, and each needs its own answer.
            let expected = if name.starts_with("OSC 4") {
                String::from_utf8_lossy(body)
                    .split(';')
                    .filter(|part| *part == "?")
                    .count()
            } else {
                1
            };
            assert_eq!(
                replies.len(),
                expected,
                "{name} produced {} replies, expected {expected}: {written:?}",
                replies.len()
            );

            // Each reply must be a well-formed colour response, not just any
            // `OSC` write: the program parses it before it trusts the value.
            let query_code = String::from_utf8_lossy(body)
                .split(';')
                .next()
                .unwrap_or("")
                .to_string();
            for reply in &replies {
                let trimmed = reply.trim_end_matches('\x1b');
                let expected_prefix = format!("\x1b]{query_code};");
                assert!(
                    trimmed.starts_with(&expected_prefix),
                    "{name} reply {trimmed:?} does not answer code {query_code}"
                );
                // `OSC 4` carries its index between the code and the colour, as
                // in `\x1b]4;2;rgb:...`. `OSC 10|11|12` does not.
                let value = if query_code == "4" {
                    trimmed.splitn(3, ';').nth(2).unwrap_or_default()
                } else {
                    trimmed
                        .split_once(';')
                        .map(|(_, rest)| rest)
                        .unwrap_or_default()
                };
                assert!(
                    value.starts_with("rgb:"),
                    "{name} reply {trimmed:?} is not in rgb form"
                );
                let components: Vec<&str> = value["rgb:".len()..].split('/').collect();
                assert_eq!(
                    components.len(),
                    3,
                    "{name} reply {trimmed:?} needs three colour components"
                );
                for component in components {
                    assert!(
                        component.len() == 4 && component.chars().all(|c| c.is_ascii_hexdigit()),
                        "{name} reply {trimmed:?} has a malformed component {component:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_mouse_reporting_reset_sequence() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::term::test::TermSize;
        use alacritty_terminal::term::{Config, Term, TermMode};
        use alacritty_terminal::vte::ansi::Processor;

        let size = TermSize::new(80, 24);
        let mut term: Term<VoidListener> = Term::new(Config::default(), &size, VoidListener);
        let mut parser: Processor = Processor::new();

        // Enable mouse tracking
        parser.advance(
            &mut term,
            b"\x1b[?1049h\x1b[?1h\x1b=\x1b[?25l\x1b[?1000h\x1b[?1006h\x1b[?1004h\x1b[?2004h",
        );
        assert!(term.mode().contains(TermMode::MOUSE_REPORT_CLICK));
        assert!(term.mode().contains(TermMode::SGR_MOUSE));
        assert!(term.mode().contains(TermMode::APP_CURSOR));
        assert!(term.mode().contains(TermMode::APP_KEYPAD));
        assert!(term.mode().contains(TermMode::FOCUS_IN_OUT));
        assert!(term.mode().contains(TermMode::BRACKETED_PASTE));
        assert!(term.mode().contains(TermMode::ALT_SCREEN));
        assert!(!term.mode().contains(TermMode::SHOW_CURSOR));

        let mut restored: Term<VoidListener> = Term::new(Config::default(), &size, VoidListener);
        let mut snapshot_modes = Vec::new();
        snapshot_modes.extend_from_slice(b"\x1b[?1049h");
        append_restored_terminal_modes(&mut snapshot_modes, *term.mode());
        let mut restored_parser: Processor = Processor::new();
        restored_parser.advance(&mut restored, &snapshot_modes);
        assert!(restored.mode().contains(TermMode::MOUSE_REPORT_CLICK));
        assert!(restored.mode().contains(TermMode::SGR_MOUSE));
        assert!(restored.mode().contains(TermMode::APP_CURSOR));
        assert!(restored.mode().contains(TermMode::APP_KEYPAD));
        assert!(restored.mode().contains(TermMode::FOCUS_IN_OUT));
        assert!(restored.mode().contains(TermMode::BRACKETED_PASTE));
        assert!(restored.mode().contains(TermMode::ALT_SCREEN));
        assert!(!restored.mode().contains(TermMode::SHOW_CURSOR));

        // Reset modes that a child can leave active.
        parser.advance(&mut term, PROMPT_TERMINAL_RESET);
        assert!(!term.mode().contains(TermMode::MOUSE_REPORT_CLICK));
        assert!(!term.mode().contains(TermMode::SGR_MOUSE));
        assert!(!term.mode().contains(TermMode::MOUSE_MOTION));
        assert!(!term.mode().contains(TermMode::MOUSE_DRAG));
        assert!(!term.mode().contains(TermMode::APP_CURSOR));
        assert!(!term.mode().contains(TermMode::APP_KEYPAD));
        assert!(!term.mode().contains(TermMode::FOCUS_IN_OUT));
        assert!(!term.mode().contains(TermMode::BRACKETED_PASTE));
        assert!(!term.mode().contains(TermMode::ALT_SCREEN));
        assert!(term.mode().contains(TermMode::SHOW_CURSOR));

        parser.advance(
            &mut term,
            b"\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?1004h\x1b[?2004h",
        );
        parser.advance(&mut term, IDLE_SESSION_MODE_RESET);
        assert!(term.mode().contains(TermMode::ALT_SCREEN));
        assert!(!term.mode().contains(TermMode::MOUSE_REPORT_CLICK));
        assert!(!term.mode().contains(TermMode::SGR_MOUSE));
        assert!(!term.mode().contains(TermMode::FOCUS_IN_OUT));
        assert!(!term.mode().contains(TermMode::BRACKETED_PASTE));
    }

    #[cfg(unix)]
    #[test]
    fn legacy_active_session_seed_restores_local_mouse_and_focus_modes() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::term::test::TermSize;
        use alacritty_terminal::term::{Config, Term, TermMode};
        use alacritty_terminal::vte::ansi::Processor;

        let mut terminal: Term<VoidListener> =
            Term::new(Config::default(), &TermSize::new(80, 24), VoidListener);
        let mut parser: Processor = Processor::new();
        let mut modes = Vec::new();
        append_legacy_mouse_modes(&mut modes);
        parser.advance(&mut terminal, &modes);

        assert!(terminal.mode().contains(TermMode::MOUSE_MOTION));
        assert!(terminal.mode().contains(TermMode::SGR_MOUSE));
        assert!(terminal.mode().contains(TermMode::FOCUS_IN_OUT));
        assert!(terminal.mode().contains(TermMode::BRACKETED_PASTE));
    }

    #[test]
    fn test_search_snippets_and_preview() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::term::test::TermSize;
        use alacritty_terminal::term::{Config, Term};
        use alacritty_terminal::vte::ansi::Processor;

        let size = TermSize::new(80, 24);
        let mut term: Term<VoidListener> = Term::new(Config::default(), &size, VoidListener);
        let mut parser: Processor = Processor::new();

        let sample = b"Line 1: cargo build\r\nLine 2: error in file.rs\r\nLine 3: success\r\n";
        parser.advance(&mut term, sample);

        // Test preview lines from grid
        let grid = term.grid();
        let screen_lines = grid.screen_lines();
        assert!(screen_lines >= 3);

        use alacritty_terminal::index::Line;
        let mut preview = Vec::new();
        for i in 0..3 {
            let row = &grid[Line(i as i32)];
            let mut s = String::new();
            for cell in row.into_iter() {
                if cell.c != '\0' {
                    s.push(cell.c);
                }
            }
            preview.push(s.trim_end().to_string());
        }
        assert_eq!(preview[0], "Line 1: cargo build");
        assert_eq!(preview[1], "Line 2: error in file.rs");
        assert_eq!(preview[2], "Line 3: success");
    }

    #[test]
    fn test_image_store_prune_old_placements() {
        let mut store = ImageStore::new();
        let frame1 = image::Frame::new(image::RgbaImage::new(1, 1));
        let frame2 = image::Frame::new(image::RgbaImage::new(1, 1));
        let p1 = ImagePlacement {
            image_id: 1,
            placement_id: 10,
            absolute_line: 100,
            col: 0,
            rows: 10,
            cols: 20,
            z_index: 0,
            image: Arc::new(gpui::RenderImage::new(smallvec::smallvec![frame1])),
        };
        let p2 = ImagePlacement {
            image_id: 2,
            placement_id: 20,
            absolute_line: 500,
            col: 0,
            rows: 10,
            cols: 20,
            z_index: 0,
            image: Arc::new(gpui::RenderImage::new(smallvec::smallvec![frame2])),
        };
        store.add_image(1, p1.image.clone());
        store.add_image(2, p2.image.clone());
        store.add_placement(p1);
        store.add_placement(p2);

        assert_eq!(store.placements.len(), 2);
        assert_eq!(store.images.len(), 2);

        // Prune anything that ended before line 200 (p1: 100+10 = 110 < 200)
        store.prune_old_placements(200);

        assert_eq!(store.placements.len(), 1);
        assert_eq!(store.placements[0].image_id, 2);
        assert_eq!(store.images.len(), 1);
        assert!(store.images.contains_key(&2));
        assert!(!store.images.contains_key(&1));
    }

    #[test]
    fn test_get_screen_preview_anchoring() {
        if let Ok(term) = TerminalState::new_headless("/bin/sh", &[], None, 80, 24) {
            let preview = term.get_screen_preview(10);
            assert!(preview.len() <= 10);
            let lines = term.get_screen_preview_lines(10);
            assert_eq!(lines.len(), preview.len());
            // Lines should correspond to rendered preview spans
            for (line_str, prev_line) in lines.iter().zip(preview.iter()) {
                let joined = prev_line
                    .spans
                    .iter()
                    .map(|s| s.text.as_str())
                    .collect::<String>();
                assert_eq!(line_str, joined.trim_end());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn binary_snapshot_roundtrip_preserves_default_and_explicit_colors() {
        use alacritty_terminal::index::{Column, Line};
        use alacritty_terminal::term::cell::Flags;
        use alacritty_terminal::term::{Config, Term};
        use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

        let source = TerminalState::new_headless("/bin/cat", &[], None, 10, 5)
            .expect("create snapshot source");
        let initial = source.snapshot_binary();
        let (_, initial_cells) = crate::server::binary_snapshot::decode_snapshot(&initial)
            .expect("decode initial snapshot");
        let background = initial_cells[0].bg;
        let explicit_bg = Rgb {
            r: (background >> 16) as u8,
            g: (background >> 8) as u8,
            b: background as u8,
        };
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(
            &mut *source.term.lock(),
            b"\x1b[?1049h\x1b[48;2;23;23;23m\x1b[2Jopencode",
        );
        parser.advance(&mut *source.term.lock(), PROMPT_TERMINAL_RESET);
        let contents = format!(
            "a\x1b[38;2;200;100;50m\x1b[48;2;{};{};{}mb\x1b[0mc\x1b[7md\x1b[0m",
            explicit_bg.r, explicit_bg.g, explicit_bg.b,
        );
        parser.advance(&mut *source.term.lock(), contents.as_bytes());
        let encoded = source.snapshot_binary();
        assert_eq!(source.term.lock().grid()[Line(0)][Column(1)].c, 'b');
        source.terminate_process();
        assert!(source.restore_binary_snapshot(&encoded));
        let encoded = source.snapshot_binary();
        let (header, cells) =
            crate::server::binary_snapshot::decode_snapshot(&encoded).expect("decode snapshot");
        let proxy = EventListenerProxy::from_arc(Arc::new(ParkingMutex::new(
            Box::new(std::io::sink()) as Box<dyn Write + Send>,
        )));
        let mut restored = Term::new(
            Config::default(),
            &TermSize::new(header.cols as usize, header.rows as usize),
            proxy,
        );
        assert!(apply_snapshot_to_grid(&mut restored, &header, &cells));
        let grid = restored.grid();
        assert_eq!(grid[Line(0)][Column(1)].c, 'b');
        for column in [0, 2, 9] {
            assert_eq!(
                grid[Line(0)][Column(column)].bg,
                Color::Named(NamedColor::Background)
            );
            assert_eq!(
                grid[Line(0)][Column(column)].fg,
                Color::Named(NamedColor::Foreground)
            );
        }
        assert_eq!(grid[Line(0)][Column(1)].bg, Color::Spec(explicit_bg));
        assert_eq!(
            grid[Line(0)][Column(1)].fg,
            Color::Spec(Rgb {
                r: 200,
                g: 100,
                b: 50
            })
        );
        assert!(grid[Line(0)][Column(3)].flags.contains(Flags::INVERSE));
        assert_eq!(
            grid[Line(0)][Column(3)].bg,
            Color::Named(NamedColor::Background)
        );

        let mut legacy_cells = cells.clone();
        for cell in &mut legacy_cells {
            cell.flags &= !(crate::server::binary_snapshot::CELL_FLAG_DEFAULT_FG
                | crate::server::binary_snapshot::CELL_FLAG_DEFAULT_BG);
        }
        assert!(apply_snapshot_to_grid(
            &mut restored,
            &header,
            &legacy_cells
        ));
        assert_eq!(
            restored.grid()[Line(0)][Column(0)].bg,
            Color::Spec(explicit_bg)
        );

        parser.advance(&mut restored, b"\x1b[?1049h");
        assert!(apply_snapshot_to_grid(&mut restored, &header, &cells));
        assert!(
            restored
                .mode()
                .contains(alacritty_terminal::term::TermMode::ALT_SCREEN)
        );
        assert_eq!(
            restored.grid()[Line(0)][Column(1)].bg,
            Color::Spec(explicit_bg)
        );
        assert_eq!(
            restored.grid()[Line(0)][Column(0)].bg,
            Color::Named(NamedColor::Background)
        );
    }

    #[test]
    fn test_apply_snapshot_to_grid_restores_history_and_cursor() {
        use crate::server::binary_snapshot::{
            CELL_FLAG_DEFAULT_BG, CELL_FLAG_DEFAULT_FG, FasttyBinarySnapshotHeader,
            FasttyPackedCell, encode_snapshot,
        };
        use alacritty_terminal::term::{Config as AlacrittyTestConfig, Term as TestTerm};
        use parking_lot::Mutex as ParkingMutex;

        struct NullWriter;
        impl std::io::Write for NullWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let writer: Box<dyn Write + Send> = Box::new(NullWriter);
        let proxy = EventListenerProxy::from_arc(Arc::new(ParkingMutex::new(writer)));
        let size = TermSize::new(4, 2);
        let mut term = TestTerm::new(AlacrittyTestConfig::default(), &size, proxy);

        // Snapshot: 3 history rows ('a','b','c') + 2 screen rows ('x','y'),
        // 4 cols each, cursor at screen row 1, col 2.
        let cell = |ch: char| FasttyPackedCell {
            c: ch as u32,
            fg: 0x00FFFFFF,
            bg: 0x00000000,
            flags: CELL_FLAG_DEFAULT_BG | CELL_FLAG_DEFAULT_FG,
            _reserved: 0,
        };
        let mut cells = Vec::new();
        for ch in ['a', 'b', 'c'] {
            for _ in 0..4 {
                cells.push(cell(ch));
            }
        }
        for ch in ['x', 'y'] {
            for _ in 0..4 {
                cells.push(cell(ch));
            }
        }
        let mut header = FasttyBinarySnapshotHeader::default();
        header.cols = 4;
        header.rows = 2;
        header.cell_count = cells.len() as u32;
        header.set_history_rows(3);
        header.cursor_col = 2;
        header.cursor_row = 1;

        // The encoded form round-trips through the real decoder path.
        let encoded = encode_snapshot(&header, &cells);
        let (decoded_header, decoded_cells) =
            crate::server::binary_snapshot::decode_snapshot(&encoded).unwrap();
        assert_eq!(decoded_header.history_rows(), 3);

        assert!(apply_snapshot_to_grid(
            &mut term,
            &decoded_header,
            &decoded_cells
        ));

        let grid = term.grid();
        // Newest history row sits directly above the screen.
        assert_eq!(
            grid[alacritty_terminal::index::Line(-1)][alacritty_terminal::index::Column(0)].c,
            'c'
        );
        assert_eq!(
            grid[alacritty_terminal::index::Line(-3)][alacritty_terminal::index::Column(0)].c,
            'a'
        );
        // Screen rows land top to bottom.
        assert_eq!(
            grid[alacritty_terminal::index::Line(0)][alacritty_terminal::index::Column(0)].c,
            'x'
        );
        assert_eq!(
            grid[alacritty_terminal::index::Line(1)][alacritty_terminal::index::Column(0)].c,
            'y'
        );
        // Cursor restored.
        assert_eq!(grid.cursor.point.line.0, 1);
        assert_eq!(grid.cursor.point.column.0, 2);
        assert_eq!(
            grid[alacritty_terminal::index::Line(-1)][alacritty_terminal::index::Column(0)].bg,
            alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Background,
            ),
        );
    }

    #[test]
    fn bounded_ansi_snapshot_stops_at_limit() {
        let terminal = TerminalState::new_headless("/bin/sh", &[], None, 80, 24)
            .expect("create headless terminal");
        let max_bytes = 64;

        let (snapshot, truncated) = terminal.snapshot_ansi_bounded(max_bytes);

        assert!(truncated);
        assert!(snapshot.len() <= max_bytes);
        assert!(snapshot.len() < terminal.snapshot_ansi().len());
    }

    #[test]
    fn bounded_ansi_snapshot_marks_too_small_limit() {
        let terminal = TerminalState::new_headless("/bin/sh", &[], None, 80, 24)
            .expect("create headless terminal");

        let (snapshot, truncated) = terminal.snapshot_ansi_bounded(1);

        assert!(truncated);
        assert!(snapshot.is_empty());
    }
}

#[cfg(all(test, unix))]
#[path = "terminal_restore_tests.rs"]
mod restore_tests;
