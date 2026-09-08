use std::collections::HashSet;
use std::sync::RwLock;
use crate::ai::config::PermissionMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Confirm,
    Deny,
}

pub struct PermissionChecker {
    mode: RwLock<PermissionMode>,
    session_allowlist: RwLock<HashSet<String>>,
}

impl PermissionChecker {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode: RwLock::new(mode),
            session_allowlist: RwLock::new(HashSet::new()),
        }
    }

    pub fn set_mode(&self, mode: PermissionMode) {
        if let Ok(mut lock) = self.mode.write() {
            *lock = mode;
        }
    }

    pub fn allow_always(&self, key: &str) {
        if let Ok(mut lock) = self.session_allowlist.write() {
            lock.insert(key.to_string());
        }
    }

    pub fn allow_always_tool(&self, tool_name: &str, input_summary: &str) {
        let cache_key = format!("{}:{}", tool_name, Self::scope_for(tool_name, input_summary));
        self.allow_always(&cache_key);
    }

    /// Stable per-target scope so "Allow Always" covers future calls against
    /// the same path or command, not just byte-identical repeats.
    fn scope_for(tool_name: &str, input_summary: &str) -> String {
        let _ = tool_name;
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(input_summary) {
            if let Some(path) = val.get("path").and_then(|v| v.as_str()) {
                return path.to_string();
            }
            if let Some(cmd) = val.get("command").and_then(|v| v.as_str()) {
                return cmd.to_string();
            }
        }
        input_summary.to_string()
    }

    pub fn is_always_allowed(&self, key: &str) -> bool {
        if let Ok(lock) = self.session_allowlist.read() {
            lock.contains(key)
        } else {
            false
        }
    }

    pub fn check_permission(
        &self,
        tool_name: &str,
        input_summary: &str,
    ) -> PermissionDecision {
        // 1. Hardcoded critical security rules
        if tool_name == "run_command" {
            let lower = input_summary.to_lowercase();
            let is_hard_deny = lower.contains("rm -rf /")
                || lower.contains("rm -fr /")
                || lower.contains("rm -rf ~")
                || lower.contains("rm -fr ~")
                || lower.contains(":(){ :|:& };:")
                || lower.contains("mkfs")
                || lower.contains("> /dev/sd")
                || lower.contains("dd if=/dev");

            if is_hard_deny {
                return PermissionDecision::Deny;
            }
        }

        // 2. Session allowlist check
        let cache_key = format!("{}:{}", tool_name, Self::scope_for(tool_name, input_summary));
        if self.is_always_allowed(&cache_key) {
            return PermissionDecision::Allow;
        }

        // 3. Permission mode rules
        let mode = self
            .mode
            .read()
            .map(|m| *m)
            .unwrap_or(PermissionMode::ConfirmWrites);

        match mode {
            PermissionMode::Yolo => PermissionDecision::Allow,
            PermissionMode::ConfirmAll => PermissionDecision::Confirm,
            PermissionMode::ConfirmWrites => {
                match tool_name {
                    "read_file" | "search" | "list_dir" => PermissionDecision::Allow,
                    _ => PermissionDecision::Confirm,
                }
            }
        }
    }
}
