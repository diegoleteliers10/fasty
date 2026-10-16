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
    if matches!(provider, "opencode" | "opencode-v2" | "pi" | "hermes") {
        let files = plugin_files(provider, &path)?;
        let registration = opencode_cli_registration(provider, &path, true)?;
        for (target, contents) in &files {
            if target.exists() {
                let existing = fs::read(target).map_err(|error| error.to_string())?;
                if existing != *contents && !is_previous_plugin(provider, &existing) {
                    return Err(format!(
                        "{} differs from the Fastty adapter. It was not changed.",
                        target.display()
                    ));
                }
            }
        }
        for (target, contents) in files {
            if target.exists() {
                if fs::read(&target).map_err(|error| error.to_string())? != contents {
                    write_preserving_backup(&target, &contents)?;
                }
            } else {
                write_new_file(&target, &contents)?;
            }
        }
        if let Some((config_path, document)) = registration {
            write_json(&config_path, &document)?;
        }
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
    if matches!(provider, "opencode" | "opencode-v2" | "pi" | "hermes") {
        let files = plugin_files(provider, &path)?;
        let registration = opencode_cli_registration(provider, &path, false)?;
        for (target, expected) in &files {
            if target.exists() {
                let actual = fs::read(target).map_err(|error| error.to_string())?;
                if actual != *expected && !is_previous_plugin(provider, &actual) {
                    return Err(format!(
                        "{} differs from the Fastty adapter. It was not removed.",
                        target.display()
                    ));
                }
            }
        }
        if let Some((config_path, document)) = registration {
            write_json(&config_path, &document)?;
        }
        for (target, _) in files {
            if target.exists() {
                fs::remove_file(&target).map_err(|error| error.to_string())?;
            }
        }
        if provider == "hermes" {
            if let Some(parent) = path.parent() {
                let _ = fs::remove_dir(parent);
            }
        }
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
    if provider == "hermes" {
        if scope == InstallScope::Project {
            return Err("Hermes uses profile plugins. Use --user. Set HERMES_HOME to select a profile.".into());
        }
        let home = match std::env::var_os("HERMES_HOME").filter(|value| !value.is_empty()) {
            Some(home) => PathBuf::from(home),
            None => PathBuf::from(std::env::var_os("HOME")
                .ok_or_else(|| "HOME is not set".to_string())?).join(".hermes"),
        };
        if !home.is_absolute() {
            return Err("HERMES_HOME must be an absolute path".into());
        }
        return Ok(home.join("plugins/fastty-status/__init__.py"));
    }
    if matches!(provider, "opencode" | "opencode-v2") {
        let root = match scope {
            InstallScope::Project => std::env::current_dir()
                .map_err(|error| error.to_string())?.join(".opencode"),
            InstallScope::User => {
                if let Some(root) = std::env::var_os("OPENCODE_CONFIG_DIR").filter(|value| !value.is_empty()) {
                    PathBuf::from(root)
                } else if let Some(root) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
                    PathBuf::from(root).join("opencode")
                } else {
                    PathBuf::from(std::env::var_os("HOME")
                        .ok_or_else(|| "HOME is not set".to_string())?).join(".config/opencode")
                }
            }
        };
        if !root.is_absolute() {
            return Err("The OpenCode config directory must be an absolute path".into());
        }
        return Ok(root.join(if provider == "opencode-v2" {
            "fastty/fastty-status-v2.js"
        } else {
            "plugins/fastty-status.js"
        }));
    }
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
        "opencode-v2" => include_bytes!("../integrations/opencode/fastty-v2.js").as_slice(),
        "pi" => include_bytes!("../integrations/pi/fastty-extension.js").as_slice(),
        "hermes" => include_bytes!("../integrations/hermes/__init__.py").as_slice(),
        _ => return Err(format!("unsupported extension provider: {provider}")),
    };
    Ok(contents.to_vec())
}

fn plugin_files(provider: &str, path: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let mut files = vec![(path.to_path_buf(), plugin_contents(provider)?)];
    if provider == "hermes" {
        files.push((path.with_file_name("plugin.yaml"),
            include_bytes!("../integrations/hermes/plugin.yaml").to_vec()));
    }
    Ok(files)
}

fn is_previous_plugin(provider: &str, contents: &[u8]) -> bool {
    provider == "opencode"
        && contents == include_bytes!("../integrations/opencode/legacy/fastty.js")
}

fn opencode_cli_registration(
    provider: &str,
    plugin: &Path,
    install: bool,
) -> Result<Option<(PathBuf, Value)>, String> {
    if provider != "opencode-v2" {
        return Ok(None);
    }
    let root = plugin.parent().and_then(Path::parent)
        .ok_or_else(|| "The OpenCode plugin has no config directory".to_string())?;
    let path = root.join("cli.json");
    if root.join("cli.jsonc").exists() {
        return Err("OpenCode uses cli.jsonc. Add or remove the Fastty CLI plugin in that file manually.".into());
    }
    if !install && !path.exists() {
        return Ok(None);
    }
    let mut document = read_json_object(&path)?;
    let original = document.clone();
    let plugin = plugin.to_str()
        .ok_or_else(|| "The OpenCode plugin path must use UTF-8".to_string())?;
    if install {
        let entries = document.as_object_mut().unwrap()
            .entry("plugins").or_insert_with(|| json!([]))
            .as_array_mut().ok_or_else(|| "OpenCode cli.json: plugins must be an array".to_string())?;
        if !entries.iter().any(|entry| entry.as_str() == Some(plugin)) {
            entries.push(json!(plugin));
        }
    } else if let Some(entries) = document.get_mut("plugins") {
        let entries = entries.as_array_mut()
            .ok_or_else(|| "OpenCode cli.json: plugins must be an array".to_string())?;
        entries.retain(|entry| entry.as_str() != Some(plugin));
        if entries.is_empty() {
            document.as_object_mut().unwrap().remove("plugins");
        }
    }
    Ok((document != original).then_some((path, document)))
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
