//! Stable output and exit codes for the automation subcommands.
//!
//! Scripts depend on two things that must not drift: the exit code, and the
//! JSON document on stdout. Both are defined here so every command answers the
//! same way, and a future command cannot invent a new convention.
//!
//! The contract is documented in `docs/cli-automation.md`.

use serde_json::{json, Value};

/// Bumped only on a breaking change to the JSON shape or the exit codes.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliExit {
    Ok,
    /// Bad flags or arguments. Nothing was sent to the daemon.
    Usage,
    /// The daemon was unreachable, the protocol failed, or it answered with an
    /// error that is not a missing session.
    Failed,
    /// The session id does not exist, or it disappeared before the request.
    NotFound,
    /// `capture` returned a partial screen because the output hit the cap.
    Truncated,
    /// `wait` reached its timeout without seeing the pattern.
    Timeout,
}

impl CliExit {
    pub fn code(self) -> i32 {
        match self {
            CliExit::Ok => 0,
            CliExit::Usage => 1,
            CliExit::Failed => 2,
            CliExit::NotFound => 3,
            CliExit::Truncated => 4,
            CliExit::Timeout => 5,
        }
    }
}

/// Map a daemon error code to the exit code a script branches on.
pub fn classify(daemon_code: &str) -> CliExit {
    match daemon_code {
        "no_such_session" => CliExit::NotFound,
        _ => CliExit::Failed,
    }
}

/// One JSON document for a command that succeeded. `fields` are merged into the
/// envelope, so a command only names its own payload keys.
pub fn ok_envelope(command: &str, fields: Value) -> Value {
    let mut envelope = json!({
        "schema_version": SCHEMA_VERSION,
        "command": command,
        "ok": true,
    });
    if let (Some(envelope), Some(extra)) = (envelope.as_object_mut(), fields.as_object()) {
        for (key, value) in extra {
            envelope.insert(key.clone(), value.clone());
        }
    }
    envelope
}

/// One JSON document for a command that failed. `exit_code` repeats the process
/// exit code inside the document so a script can branch without a second read.
pub fn err_envelope(command: &str, exit: CliExit, code: &str, message: &str) -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "command": command,
        "ok": false,
        "exit_code": exit.code(),
        "error": { "code": code, "message": message },
    })
}

/// Print one document on a single line: a script reads one line per invocation.
pub fn emit(value: &Value) {
    println!("{value}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_the_documented_numbers() {
        assert_eq!(CliExit::Ok.code(), 0);
        assert_eq!(CliExit::Usage.code(), 1);
        assert_eq!(CliExit::Failed.code(), 2);
        assert_eq!(CliExit::NotFound.code(), 3);
        assert_eq!(CliExit::Truncated.code(), 4);
        assert_eq!(CliExit::Timeout.code(), 5);
    }

    #[test]
    fn a_missing_session_is_distinguishable_from_other_failures() {
        assert_eq!(classify("no_such_session"), CliExit::NotFound);
        assert_eq!(classify("access_denied"), CliExit::Failed);
        assert_eq!(classify("bad_request"), CliExit::Failed);
        assert_eq!(classify("not_attached"), CliExit::Failed);
    }

    #[test]
    fn success_envelope_carries_the_schema_and_payload() {
        let envelope = ok_envelope("send", json!({ "session": 4, "bytes": 12 }));
        assert_eq!(envelope["schema_version"], json!(SCHEMA_VERSION));
        assert_eq!(envelope["command"], json!("send"));
        assert_eq!(envelope["ok"], json!(true));
        assert_eq!(envelope["session"], json!(4));
        assert_eq!(envelope["bytes"], json!(12));
        assert!(envelope.get("error").is_none());
    }

    #[test]
    fn error_envelope_repeats_the_exit_code() {
        let envelope = err_envelope("capture", CliExit::NotFound, "no_such_session", "gone");
        assert_eq!(envelope["ok"], json!(false));
        assert_eq!(envelope["exit_code"], json!(3));
        assert_eq!(envelope["error"]["code"], json!("no_such_session"));
        assert_eq!(envelope["error"]["message"], json!("gone"));
    }

    #[test]
    fn envelope_stays_on_one_line() {
        let envelope = ok_envelope("capture", json!({ "data": "line one\nline two" }));
        let rendered = envelope.to_string();
        assert!(!rendered.contains('\n'), "a JSON document must not span lines");
    }
}
