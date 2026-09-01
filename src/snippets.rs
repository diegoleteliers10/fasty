//! User-customizable text snippets for shell command expansion.
//!
//! Triggers are short identifiers (e.g. `gst`, `gcm`) that the user types
//! at the prompt; pressing `Tab` expands the snippet in place. The expansion
//! uses VSCode-style placeholders:
//!
//!   - `$1`, `$2`, ... jump markers.
//!   - `${1:default}` for a placeholder with default text.
//!   - `$0` marks the final cursor position (only one per snippet).
//!
//! File: `~/.config/fastty/snippets.toml`. On first run we write a bundled
//! set of defaults. User edits to that file are preserved across launches
//! (we only seed it when it does not yet exist). The file is watched with
//! `notify` for live reload — same mechanism as `config.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use parking_lot::RwLock;
use serde::Deserialize;

const WATCH_DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SnippetFile {
    #[serde(default)]
    pub snippet: HashMap<String, String>,
}

static SNIPPETS: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();

fn snippets_cell() -> &'static RwLock<HashMap<String, String>> {
    SNIPPETS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn snippets_path() -> PathBuf {
    crate::paths::get().config_dir.join("snippets.toml")
}

const BUNDLED: &str = include_str!("../snippets.bundled.toml");

fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    std::fs::write(&tmp, contents)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

fn seed_defaults_if_missing() {
    let path = snippets_path();
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = atomic_write(&path, BUNDLED.as_bytes());
}

pub fn load() {
    seed_defaults_if_missing();
    let path = snippets_path();
    let mut map: HashMap<String, String> = HashMap::new();
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(file) = toml_edit::de::from_str::<SnippetFile>(&content) {
            for (k, v) in file.snippet {
                if !k.is_empty() && !v.is_empty() {
                    map.insert(k, v);
                }
            }
        }
    }
    *snippets_cell().write() = map;
}

/// Find the longest trigger in the current map that is a suffix of `prefix`.
/// Returns the trigger length if found.
pub fn match_trigger(prefix: &str) -> Option<usize> {
    let map = snippets_cell().read();
    let mut best: Option<usize> = None;
    for trigger in map.keys() {
        if trigger.is_empty() { continue; }
        if prefix.len() >= trigger.len() && prefix.ends_with(trigger.as_str()) {
            match best {
                Some(n) if n >= trigger.len() => {}
                _ => best = Some(trigger.len()),
            }
        }
    }
    best
}

pub fn get_expansion(trigger: &str) -> Option<String> {
    snippets_cell().read().get(trigger).cloned()
}

/// Expand placeholders in `body`. Replaces `$0` with empty (final cursor
/// position marker) and `$1`, `$2`, ... with empty (or `default` for
/// `${1:default}`). Returns the resulting string and the byte offset of
/// the final cursor position (the first `$0` occurrence), or `body.len()`
/// if no `$0` is present.
///
/// Placeholders are numbered; `${1:foo}` jumps to position 1 and fills
/// the default text `foo` until the user types over it. We do not yet
/// implement interactive placeholder jumping — that would require keeping
/// state per-snippet-in-flight. For v1, placeholders are stripped down to
/// their default text (or empty) and `$0` becomes a single cursor marker.
pub fn expand(body: &str) -> (String, Option<usize>) {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut cursor_pos: Option<usize> = None;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c != '$' {
            out.push(c);
            i += 1;
            continue;
        }
        if i + 1 >= bytes.len() {
            out.push('$');
            i += 1;
            continue;
        }
        let next = bytes[i + 1] as char;
        if next == '$' {
            out.push('$');
            i += 2;
            continue;
        }
        if next == '0' {
            if cursor_pos.is_none() {
                cursor_pos = Some(out.len());
            }
            i += 2;
            continue;
        }
        if next.is_ascii_digit() {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && (bytes[j] as char).is_ascii_digit() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b':' && j + 1 < bytes.len() && bytes[j + 1] == b'}' {
                let close = find_matching_brace(body, j);
                if let Some(end) = close {
                    let default = &body[j + 2..end];
                    out.push_str(default);
                    i = end + 1;
                    continue;
                }
            }
            i = j;
            continue;
        }
        if next == '{' {
            if let Some(close) = find_matching_brace(body, i + 1) {
                let inner = &body[i + 2..close];
                if let Some((_, default)) = inner.split_once(':') {
                    out.push_str(default);
                }
                i = close + 1;
                continue;
            }
        }
        out.push('$');
        i += 1;
    }
    (out, cursor_pos)
}

fn find_matching_brace(s: &str, open_idx: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.get(open_idx) != Some(&b'{') { return None; }
    let mut depth = 1i32;
    for (k, &byte) in bytes.iter().enumerate().skip(open_idx + 1) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 { return Some(k); }
            }
            _ => {}
        }
    }
    None
}

pub fn start_watcher<F>(on_change: F) -> anyhow::Result<()>
where
    F: Fn() + Send + 'static,
{
    use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};

    let path = snippets_path();
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    if !parent.exists() {
        std::fs::create_dir_all(&parent)?;
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(WATCH_DEBOUNCE, tx)?;
    debouncer.watcher().watch(&parent, RecursiveMode::NonRecursive)?;

    let watched_file = path.clone();
    let watched_name = watched_file.file_name().map(|n| n.to_owned());
    std::thread::Builder::new()
        .name("fastty-snippets-watch".into())
        .spawn(move || {
            let _debouncer = debouncer;
            for batch in rx {
                let events = match batch {
                    Ok(ev) => ev,
                    Err(_) => continue,
                };
                let touches_file = events.iter().any(|e| e.path.file_name() == watched_name.as_deref());
                if !touches_file { continue; }
                on_change();
            }
        })?;

    Ok(())
}
