//! Learned auto-allow for AI tool permissions.
//!
//! "You either die a yolo mode or live long enough to become auto allow."
//! Every manual approval of a `run_command` call counts toward its command
//! family (`git status`, `cargo build`, …). Once a family crosses
//! [`APPROVAL_THRESHOLD`], the confirmation card offers to auto-allow it
//! from then on. Learning only ever *suggests*; the user flips the switch.
//! The hardcoded danger layer in `permissions` still runs first, so a
//! learned rule can never green-light the hard-deny cases.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Manual approvals of the same command family before a suggestion appears.
pub const APPROVAL_THRESHOLD: u32 = 3;

/// Master switch, mirrored from `[ai] learned_allow` in fastty.toml.
static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Programs whose "first token" says nothing about safety — shells,
/// interpreters, and anything that executes arbitrary arguments. They are
/// counted but never suggested or matched.
const NEVER_LEARN: &[&str] = &[
    "sh", "bash", "zsh", "fish", "dash", "ksh", "csh", "tcsh", "nu",
    "python", "python3", "python2", "node", "deno", "bun", "ruby", "perl",
    "php", "lua", "luajit", "osascript", "sudo", "su", "doas", "pkexec",
    "xargs", "env", "nohup", "time", "watch", "strace", "eval", "source",
    "exec", "awk", "sed", "make", "just", "nix", "nix-shell",
];

/// Tools that route their behavior through a subcommand: for these the
/// family key includes the subcommand (`cargo build`, not `cargo`), which
/// keeps `git status` from ever auto-allowing `git push --force`.
const SUBCOMMAND_TOOLS: &[&str] = &[
    "git", "cargo", "npm", "pnpm", "yarn", "bun", "deno", "docker",
    "podman", "kubectl", "helm", "gh", "brew", "go", "uv", "pip", "pip3",
    "poetry", "flutter", "rustup", "cargo-nextest",
];

/// The command family a tool call belongs to: `None` for anything that is
/// not a `run_command` with a parseable command (never learned).
pub fn program_key(tool_name: &str, input_summary: &str) -> Option<String> {
    if tool_name != "run_command" {
        return None;
    }
    let args: serde_json::Value = serde_json::from_str(input_summary).ok()?;
    let command = args.get("command")?.as_str()?;
    let mut tokens = command.split_whitespace();
    let program = tokens.next()?;
    // `/usr/bin/git` and `git` are the same family.
    let program = program.rsplit('/').next().unwrap_or(program);
    if NEVER_LEARN.contains(&program) {
        return None;
    }
    if SUBCOMMAND_TOOLS.contains(&program) {
        if let Some(sub) = tokens.next() {
            // Long-option subcommands are noise; keep plain words.
            if !sub.starts_with('-') {
                return Some(format!("{program} {sub}"));
            }
        }
    }
    Some(program.to_string())
}

/// Persisted learning state. Pure core; the static wrapper does the I/O.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LearnedState {
    /// Family -> approval count so far (before any accept/decline).
    pub counts: HashMap<String, u32>,
    /// Families the user accepted: auto-allowed forever after.
    pub rules: HashMap<String, bool>,
    /// Families the user declined the suggestion for: stop asking.
    pub declined: HashMap<String, bool>,
}

impl LearnedState {
    /// Whether a rule already covers this family.
    pub fn is_learned(&self, key: &str) -> bool {
        self.rules.get(key).copied().unwrap_or(false)
    }

    /// Records one manual approval. Returns the family key the moment its
    /// count crosses the threshold (first crossing only) so callers can
    /// celebrate with a suggestion.
    pub fn record_approval(&mut self, key: &str) -> Option<String> {
        let count = self.counts.entry(key.to_string()).or_insert(0);
        *count += 1;
        if *count == APPROVAL_THRESHOLD && !self.rules.contains_key(key) {
            return Some(key.to_string());
        }
        None
    }

    /// The family to suggest auto-allowing for this call, if any. The
    /// returned slice borrows from `key`.
    pub fn suggestion<'k>(&self, key: &'k str) -> Option<&'k str> {
        let count = self.counts.get(key).copied().unwrap_or(0);
        if count >= APPROVAL_THRESHOLD
            && !self.rules.contains_key(key)
            && !self.declined.contains_key(key)
        {
            Some(key)
        } else {
            None
        }
    }

    pub fn accept(&mut self, key: &str) {
        self.rules.insert(key.to_string(), true);
    }

    pub fn decline(&mut self, key: &str) {
        self.declined.insert(key.to_string(), true);
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }
}

static STATE: OnceLock<Mutex<LearnedState>> = OnceLock::new();

fn with_state<R>(f: impl FnOnce(&mut LearnedState) -> R) -> Option<R> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    let state = STATE.get_or_init(|| {
        let path = crate::paths::get().state_dir.join("ai_learned_allow.json");
        Mutex::new(LearnedState::load(&path))
    });
    state.lock().ok().map(|mut guard| f(&mut guard))
}

fn persist(state: &LearnedState) {
    let path = crate::paths::get().state_dir.join("ai_learned_allow.json");
    state.save(&path);
}

/// True when a learned rule already auto-allows this tool call.
pub fn is_learned(tool_name: &str, input_summary: &str) -> bool {
    let Some(key) = program_key(tool_name, input_summary) else {
        return false;
    };
    with_state(|s| s.is_learned(&key)).unwrap_or(false)
}

/// Counts one manual approval; returns the family key on threshold
/// crossing (first time only), for callers that surface a hint.
pub fn record_approval(tool_name: &str, input_summary: &str) -> Option<String> {
    let key = program_key(tool_name, input_summary)?;
    let crossed = with_state(|s| {
        let crossed = s.record_approval(&key);
        if crossed.is_some() {
            persist(s);
        }
        crossed
    })?;
    crossed
}

/// The auto-allow suggestion for the confirmation card, if the user has
/// approved this family enough times and neither accepted nor declined.
pub fn suggestion_for(tool_name: &str, input_summary: &str) -> Option<String> {
    let key = program_key(tool_name, input_summary)?;
    with_state(|s| s.suggestion(&key).map(str::to_string)).flatten()
}

/// Accepts the suggestion: this family is auto-allowed from now on.
pub fn accept_suggestion(key: &str) {
    if let Some(()) = with_state(|s| {
        s.accept(key);
        persist(s);
    }) {}
}

/// Declines the suggestion: stop asking for this family.
pub fn decline_suggestion(key: &str) {
    if let Some(()) = with_state(|s| {
        s.decline(key);
        persist(s);
    }) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(summary: &str) -> String {
        format!("{{\"command\": {summary}}}")
    }

    #[test]
    fn program_key_extracts_family_and_subcommand() {
        assert_eq!(program_key("run_command", &cmd("\"git status\"")).as_deref(), Some("git status"));
        assert_eq!(program_key("run_command", &cmd("\"cargo build --lib\"")).as_deref(), Some("cargo build"));
        assert_eq!(program_key("run_command", &cmd("\"ls -la /tmp\"")).as_deref(), Some("ls"));
        // Absolute paths collapse to the program name.
        assert_eq!(program_key("run_command", &cmd("\"/usr/bin/git push\"")).as_deref(), Some("git push"));
        // Only run_command learns.
        assert_eq!(program_key("edit_file", &cmd("\"git status\"")), None);
        // Flags are not subcommands.
        assert_eq!(program_key("run_command", &cmd("\"git --no-pager log\"")).as_deref(), Some("git"));
    }

    #[test]
    fn never_learn_programs_yield_no_key() {
        for dangerous in ["bash -c rm", "sudo reboot", "sh run.sh", "xargs rm", "python -c x"] {
            assert_eq!(program_key("run_command", &cmd(&format!("\"{dangerous}\""))), None);
        }
    }

    #[test]
    fn threshold_crossing_fires_once() {
        let mut s = LearnedState::default();
        assert!(s.record_approval("cargo test").is_none());
        assert!(s.record_approval("cargo test").is_none());
        assert_eq!(s.record_approval("cargo test"), Some("cargo test".to_string()));
        // Fourth approval: no repeat suggestion.
        assert!(s.record_approval("cargo test").is_none());
        assert_eq!(s.suggestion("cargo test"), Some("cargo test"));
    }

    #[test]
    fn accept_and_decline_clear_the_suggestion() {
        let mut s = LearnedState::default();
        for _ in 0..APPROVAL_THRESHOLD {
            let _ = s.record_approval("gh pr checkout");
        }
        assert!(s.suggestion("gh pr checkout").is_some());
        s.decline("gh pr checkout");
        assert!(s.suggestion("gh pr checkout").is_none());
        assert!(!s.is_learned("gh pr checkout"));

        let mut s2 = LearnedState::default();
        for _ in 0..APPROVAL_THRESHOLD {
            let _ = s2.record_approval("gh pr checkout");
        }
        s2.accept("gh pr checkout");
        assert!(s2.is_learned("gh pr checkout"));
        assert!(s2.suggestion("gh pr checkout").is_none());
    }

    #[test]
    fn unrelated_families_do_not_leak() {
        let mut s = LearnedState::default();
        for _ in 0..APPROVAL_THRESHOLD {
            let _ = s.record_approval("cargo build");
        }
        s.accept("cargo build");
        assert!(!s.is_learned("cargo publish"));
        assert!(!s.is_learned("cargo"));
    }

    #[test]
    fn state_roundtrips_through_json() {
        let mut s = LearnedState::default();
        for _ in 0..APPROVAL_THRESHOLD {
            let _ = s.record_approval("git diff");
        }
        s.accept("git diff");
        let dir = std::env::temp_dir().join(format!("fastty_learned_{}", std::process::id()));
        let path = dir.join("state.json");
        s.save(&path);
        let loaded = LearnedState::load(&path);
        assert!(loaded.is_learned("git diff"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
