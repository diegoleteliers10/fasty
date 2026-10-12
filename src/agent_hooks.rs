use serde_json::{json, Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallScope {
    Project,
    User,
}

pub fn install(provider: &str, scope: InstallScope) -> Result<PathBuf, String> {
    let path = provider_path(provider, scope)?;
    if matches!(provider, "opencode" | "pi") {
        let contents = plugin_contents(provider)?;
        if path.exists() {
            let existing = fs::read(&path).map_err(|error| error.to_string())?;
            return if existing == contents {
                Ok(path)
            } else {
                Err(format!(
                    "{} already exists and differs from the Fastty adapter. It was not changed.",
                    path.display()
                ))
            };
        }
        write_new_file(&path, &contents)?;
        return Ok(path);
    }

    let mut document = read_json_object(&path)?;
    let original = document.clone();
    let hooks = document
        .as_object_mut()
        .ok_or_else(|| format!("{}: expected a JSON object", path.display()))?
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| format!("{}: hooks must be a JSON object", path.display()))?;
    for (event, matcher) in provider_events(provider)? {
        let entries = hooks
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| format!("{}: hooks.{event} must be an array", path.display()))?;
        let hook = hook_entry(provider, matcher);
        if !entries.contains(&hook) {
            entries.push(hook);
        }
    }
    if document == original {
        return Ok(path);
    }
    write_json(&path, &document)?;
    Ok(path)
}

pub fn uninstall(provider: &str, scope: InstallScope) -> Result<PathBuf, String> {
    let path = provider_path(provider, scope)?;
    if matches!(provider, "opencode" | "pi") {
        if !path.exists() {
            return Ok(path);
        }
        let expected = plugin_contents(provider)?;
        let actual = fs::read(&path).map_err(|error| error.to_string())?;
        if actual != expected {
            return Err(format!(
                "{} differs from the Fastty adapter. It was not removed.",
                path.display()
            ));
        }
        fs::remove_file(&path).map_err(|error| error.to_string())?;
        return Ok(path);
    }

    if !path.exists() {
        return Ok(path);
    }
    let mut document = read_json_object(&path)?;
    let original = document.clone();
    let Some(hooks_value) = document.get_mut("hooks") else {
        return Ok(path);
    };
    let hooks = hooks_value
        .as_object_mut()
        .ok_or_else(|| format!("{}: hooks must be a JSON object", path.display()))?;
    for (event, matcher) in provider_events(provider)? {
        let Some(entries) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
            continue;
        };
        let owned = hook_entry(provider, matcher);
        entries.retain(|entry| entry != &owned);
        if entries.is_empty() {
            hooks.remove(event);
        }
    }
    if hooks.is_empty() {
        document.as_object_mut().unwrap().remove("hooks");
    }
    if document == original {
        return Ok(path);
    }
    write_json(&path, &document)?;
    Ok(path)
}

fn provider_path(provider: &str, scope: InstallScope) -> Result<PathBuf, String> {
    let root = match scope {
        InstallScope::Project => std::env::current_dir().map_err(|error| error.to_string())?,
        InstallScope::User => {
            PathBuf::from(std::env::var_os("HOME").ok_or_else(|| "HOME is not set".to_string())?)
        }
    };
    let path = match (provider, scope) {
        ("claude", InstallScope::Project) => root.join(".claude/settings.local.json"),
        ("claude", InstallScope::User) => root.join(".claude/settings.json"),
        ("gemini", _) => root.join(".gemini/settings.json"),
        ("codex", _) => root.join(".codex/hooks.json"),
        ("opencode", InstallScope::Project) => root.join(".opencode/plugins/fastty-status.js"),
        ("opencode", InstallScope::User) => root.join(".config/opencode/plugins/fastty-status.js"),
        ("pi", InstallScope::Project) => root.join(".pi/extensions/fastty-status.js"),
        ("pi", InstallScope::User) => root.join(".pi/agent/extensions/fastty-status.js"),
        _ => return Err(format!("unsupported agent provider: {provider}")),
    };
    Ok(path)
}

fn provider_events(provider: &str) -> Result<Vec<(&'static str, Option<&'static str>)>, String> {
    match provider {
        "claude" => Ok(vec![
            ("SessionStart", None),
            ("UserPromptSubmit", None),
            ("Stop", None),
            ("StopFailure", None),
            ("Notification", None),
            ("SessionEnd", None),
        ]),
        "gemini" => Ok(vec![
            ("SessionStart", None),
            ("BeforeAgent", None),
            ("AfterAgent", None),
            ("SessionEnd", None),
            ("Notification", Some("ToolPermission")),
        ]),
        "codex" => Ok(vec![
            ("SessionStart", None),
            ("UserPromptSubmit", None),
            ("Stop", None),
            ("Interrupt", None),
            ("SessionEnd", None),
            ("PermissionRequest", None),
        ]),
        _ => Err(format!("{provider} uses an extension file, not hook JSON")),
    }
}

fn hook_entry(provider: &str, matcher: Option<&str>) -> Value {
    let mut hook = json!({
        "type": "command",
        "command": format!("fastty agent-hook {provider}")
    });
    if provider == "codex" {
        hook["timeout"] = json!(3);
    }
    if provider == "gemini" {
        hook["name"] = json!("fastty-status");
        let mut entry = json!({ "hooks": [hook] });
        if let Some(matcher) = matcher {
            entry["matcher"] = json!(matcher);
        }
        entry
    } else {
        json!({ "hooks": [hook] })
    }
}

fn plugin_contents(provider: &str) -> Result<Vec<u8>, String> {
    let contents = match provider {
        "opencode" => include_bytes!("../integrations/opencode/fastty.js").as_slice(),
        "pi" => include_bytes!("../integrations/pi/fastty-extension.js").as_slice(),
        _ => return Err(format!("unsupported extension provider: {provider}")),
    };
    Ok(contents.to_vec())
}

fn read_json_object(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{}: invalid JSON: {error}", path.display()))?;
    if !value.is_object() {
        return Err(format!("{}: expected a JSON object", path.display()));
    }
    Ok(value)
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    write_preserving_backup(path, &bytes)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "path has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(bytes).map_err(|error| error.to_string())
}

fn write_preserving_backup(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "path has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    if path.exists() {
        let backup = path.with_extension(format!("fastty-{}.bak", unix_timestamp()));
        fs::copy(path, &backup).map_err(|error| format!("backup failed: {error}"))?;
    }
    let temp = path.with_extension("fastty-tmp");
    let mut file = fs::File::create(&temp).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temp, path).map_err(|error| error.to_string())
}

fn unix_timestamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_hook_entry_keeps_its_matcher_and_command() {
        let entry = hook_entry("gemini", Some("ToolPermission"));
        assert_eq!(entry["matcher"], "ToolPermission");
        assert_eq!(entry["hooks"][0]["command"], "fastty agent-hook gemini");
    }

    #[test]
    fn gemini_lifecycle_hook_has_no_wildcard_matcher() {
        let entry = hook_entry("gemini", None);
        assert!(entry.get("matcher").is_none());
    }

    #[test]
    fn uninstall_removes_only_exact_owned_hooks() {
        let mut doc = json!({"hooks": {"Stop": [
            hook_entry("claude", None),
            {"hooks": [{"type":"command", "command":"user-hook"}]}
        ]}});
        let hooks = doc["hooks"].as_object_mut().unwrap();
        let entries = hooks.get_mut("Stop").unwrap().as_array_mut().unwrap();
        let owned = hook_entry("claude", None);
        entries.retain(|entry| entry != &owned);
        assert_eq!(doc["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(doc["hooks"]["Stop"][0]["hooks"][0]["command"], "user-hook");
    }
}
