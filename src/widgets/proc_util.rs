//! Shared helpers for widgets that shell out to external processes.
//!
//! Goals: never block the UI thread on a child process, never leak a hung
//! child, back off when the outside world keeps failing, and skip network
//! work while the GitHub CLI is unauthenticated or unreachable.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Timeout for local subprocesses (`git`, `kubectl config`, ...).
pub const LOCAL_PROC_TIMEOUT: Duration = Duration::from_secs(5);
/// Timeout for network subprocesses (`gh`, `aws sts`, ...).
pub const NETWORK_PROC_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for user-configured commands (command widget).
pub const USER_CMD_TIMEOUT: Duration = Duration::from_secs(30);

/// Run `cmd` to completion, killing the child if it exceeds `timeout`.
///
/// Returns `None` on spawn failure or timeout. Never blocks longer than
/// `timeout` plus a small scheduling margin. stdout is drained on a side
/// thread so a chatty child can never deadlock against a full pipe.
pub fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Option<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut pipe = child.stdout.take();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
                return Some(Output {
                    status,
                    stdout,
                    stderr: Vec::new(),
                });
            }
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    }
}

/// Consecutive-failure backoff for polling widgets.
///
/// The effective interval grows as `base * 2^failures` (capped at 30 min),
/// so a persistently failing `gh`/`kubectl`/`aws` stops burning CPU and API
/// quota, and recovers to the base cadence on the first success.
#[derive(Debug, Clone, Default)]
pub struct FailureBackoff {
    consecutive: u32,
}

impl FailureBackoff {
    pub fn record(&mut self, ok: bool) {
        if ok {
            self.consecutive = 0;
        } else {
            self.consecutive = self.consecutive.saturating_add(1);
        }
    }

    pub fn failures(&self) -> u32 {
        self.consecutive
    }

    pub fn effective_interval(&self, base: Duration) -> Duration {
        if self.consecutive == 0 {
            return base;
        }
        let factor = 1u32 << self.consecutive.min(5); // cap x32
        base.saturating_mul(factor).min(Duration::from_secs(1800))
    }
}

const GH_PROBE_TTL: Duration = Duration::from_secs(120);

fn gh_probe_cache() -> &'static Mutex<(Option<Instant>, bool)> {
    static CACHE: OnceLock<Mutex<(Option<Instant>, bool)>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new((None, false)))
}

/// Cached `gh auth status` probe: true when the GitHub CLI is installed and
/// authenticated. The result is cached for 2 minutes and the probe itself
/// runs with a short timeout, so repeated polls stay cheap.
///
/// This doubles as the offline skip: without network (or without `gh`) the
/// probe fails fast and network widgets keep their cached state instead of
/// launching doomed requests. Call only from background threads.
pub fn gh_authenticated() -> bool {
    if let Ok(guard) = gh_probe_cache().lock() {
        if let Some(at) = guard.0 {
            if at.elapsed() < GH_PROBE_TTL {
                return guard.1;
            }
        }
    }
    let mut cmd = Command::new("gh");
    cmd.args(["auth", "status"]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let ok = run_with_timeout(&mut cmd, Duration::from_secs(5))
        .map(|o| o.status.success())
        .unwrap_or(false);
    if let Ok(mut guard) = gh_probe_cache().lock() {
        *guard = (Some(Instant::now()), ok);
    }
    ok
}

/// True when `name` resolves to a file on `PATH`. Used on the failure path
/// to tell "tool not installed" (stay silent) apart from "tool hung"
/// (report + back off). Only runs after a spawn already failed, so the
/// extra stats are rare and never on the hot path.
pub fn binary_on_path(name: &str) -> bool {
    let exts: &[&str] = if cfg!(target_os = "windows") {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                exts.iter()
                    .any(|ext| dir.join(format!("{name}{ext}")).is_file())
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_grows_and_resets() {
        let mut b = FailureBackoff::default();
        let base = Duration::from_secs(60);
        assert_eq!(b.effective_interval(base), base);
        b.record(false);
        assert_eq!(b.effective_interval(base), Duration::from_secs(120));
        b.record(false);
        assert_eq!(b.effective_interval(base), Duration::from_secs(240));
        b.record(true);
        assert_eq!(b.effective_interval(base), base);
    }

    #[test]
    fn test_backoff_caps_at_30min() {
        let mut b = FailureBackoff::default();
        for _ in 0..20 {
            b.record(false);
        }
        assert_eq!(
            b.effective_interval(Duration::from_secs(60)),
            Duration::from_secs(1800)
        );
    }

    #[test]
    fn test_run_with_timeout_captures_output() {
        #[cfg(not(target_os = "windows"))]
        {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "echo hello"]);
            let out = run_with_timeout(&mut cmd, Duration::from_secs(5)).expect("echo works");
            assert!(out.status.success());
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
        }
    }

    #[test]
    fn test_run_with_timeout_kills_hung_child() {
        #[cfg(not(target_os = "windows"))]
        {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "sleep 30"]);
            let start = Instant::now();
            assert!(run_with_timeout(&mut cmd, Duration::from_millis(300)).is_none());
            assert!(start.elapsed() < Duration::from_secs(10));
        }
    }
}
