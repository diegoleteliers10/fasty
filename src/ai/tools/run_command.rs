use std::io::Read;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::ai::model::CancelToken;
use crate::ai::tool::{Tool, ToolCtx, ToolOutput};

const MAX_OUTPUT_BYTES: usize = 128 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RunCommandInput {
    /// The shell command line string to execute
    pub command: String,
}

pub struct RunCommandTool;

impl RunCommandTool {
    pub fn new() -> Self {
        Self
    }
}

pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if let Some(&next_ch) = chars.peek() {
                if next_ch == '[' {
                    chars.next();
                    while let Some(&c) = chars.peek() {
                        chars.next();
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                    continue;
                } else if next_ch == ']' {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                    continue;
                }
            }
        } else if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                continue;
            }
            out.push('\n');
            continue;
        }
        out.push(ch);
    }
    out
}

impl Tool for RunCommandTool {
    fn name(&self) -> &'static str {
        "run_command"
    }

    fn description(&self) -> String {
        "Execute a shell command in an isolated PTY environment. Captures up to 128KB of output with a 120s timeout.".to_string()
    }

    fn schema(&self) -> Value {
        let schema = schemars::schema_for!(RunCommandInput);
        serde_json::to_value(&schema).unwrap_or_default()
    }

    fn run(
        &self,
        input: Value,
        ctx: &ToolCtx,
        cancel: &CancelToken,
    ) -> Result<ToolOutput, ToolOutput> {
        let parsed: RunCommandInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return Err(ToolOutput::error(format!("Invalid arguments: {}", e))),
        };

        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(p) => p,
            Err(e) => return Err(ToolOutput::error(format!("Failed to open PTY: {}", e))),
        };

        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = CommandBuilder::new("powershell.exe");
            c.args(["-NoProfile", "-Command", &parsed.command]);
            c
        };

        #[cfg(not(target_os = "windows"))]
        let mut cmd = {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            let mut c = CommandBuilder::new(shell);
            c.args(["-c", &parsed.command]);
            c
        };

        cmd.cwd(&ctx.cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let mut child = match pair.slave.spawn_command(cmd) {
            Ok(c) => c,
            Err(e) => return Err(ToolOutput::error(format!("Failed to spawn command: {}", e))),
        };

        // Drop slave so EOF can be detected when child exits
        drop(pair.slave);

        let mut reader = match pair.master.try_clone_reader() {
            Ok(r) => r,
            Err(e) => return Err(ToolOutput::error(format!("Failed to clone PTY reader: {}", e))),
        };

        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("ai-pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .ok();

        let start = Instant::now();
        let mut raw_output = Vec::new();
        let mut truncated = false;

        loop {
            if cancel.is_cancelled() {
                let _ = child.kill();
                return Err(ToolOutput::error("Command cancelled by user."));
            }

            if start.elapsed() > TIMEOUT {
                let _ = child.kill();
                return Err(ToolOutput::error(format!(
                    "Command timed out after {} seconds.",
                    TIMEOUT.as_secs()
                )));
            }

            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => {
                    raw_output.extend_from_slice(&chunk);
                    if raw_output.len() > MAX_OUTPUT_BYTES {
                        let excess = raw_output.len() - MAX_OUTPUT_BYTES;
                        raw_output.drain(0..excess);
                        truncated = true;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Check if child exited
                    if let Ok(Some(_)) = child.try_wait() {
                        // Drain any remaining bytes
                        while let Ok(chunk) = rx.try_recv() {
                            raw_output.extend_from_slice(&chunk);
                            if raw_output.len() > MAX_OUTPUT_BYTES {
                                let excess = raw_output.len() - MAX_OUTPUT_BYTES;
                                raw_output.drain(0..excess);
                                truncated = true;
                            }
                        }
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }

        let status = child.wait().ok();
        let exit_code = status.map(|s| s.exit_code());

        let output_str = String::from_utf8_lossy(&raw_output);
        let clean = strip_ansi(&output_str);

        let mut final_content = if truncated {
            format!("[Output truncated to 128KB]\n{}", clean)
        } else {
            clean
        };

        if final_content.is_empty() {
            final_content = "(No output)".to_string();
        }

        match exit_code {
            Some(0) => Ok(ToolOutput::success(final_content)),
            Some(code) => Err(ToolOutput::error(format!(
                "Command exited with code {}:\n{}",
                code, final_content
            ))),
            None => Ok(ToolOutput::success(final_content)),
        }
    }
}
