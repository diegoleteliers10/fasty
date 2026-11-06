use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;

static INSTALL_LOCK: Mutex<()> = Mutex::new(());
static LOGIN_SHELL_PATHS: OnceLock<Vec<PathBuf>> = OnceLock::new();
const REGISTRY: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const PATH_MARKER_START: &str = "__FASTTY_PATH_START__";
const PATH_MARKER_END: &str = "__FASTTY_PATH_END__";
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Serialize, Deserialize)]
struct Launch {
    program: PathBuf,
    args: Vec<String>,
    node: bool,
}

pub(crate) fn prepare(command: &OsStr, args: &[String], cwd: &Path) -> anyhow::Result<Command> {
    let paths = search_paths();
    let path = std::env::join_paths(&paths).context("Cannot prepare the ACP PATH")?;
    let configured = Path::new(command);
    let configured = if !configured.is_absolute() && configured.components().count() > 1 {
        cwd.join(configured)
    } else {
        configured.to_path_buf()
    };
    if let Some(program) = resolve(configured.as_os_str(), &paths) {
        let mut result = Command::new(program);
        result.args(args).env("PATH", path);
        return Ok(result);
    }
    let id = match command.to_str() {
        Some("claude-agent-acp") => "claude-acp",
        Some("agy_acp_server" | "agy_acp_server.par") => "antigravity-acp",
        _ => bail!(
            "ACP executable '{}' was not found. Install it or set its full path.",
            command.to_string_lossy()
        ),
    };
    let _guard = INSTALL_LOCK
        .lock()
        .map_err(|_| anyhow!("The ACP install lock is unavailable"))?;
    let root = crate::paths::get().cache_dir.join("acp_agents").join(id);
    fs::create_dir_all(&root)?;
    let launch = read_launch(&root);
    let launch = match launch {
        Some(launch) => launch,
        None => install(id, &root, &paths, &path).with_context(|| {
            format!(
                "Cannot install missing ACP executable '{}'",
                command.to_string_lossy()
            )
        })?,
    };
    let program = if launch.node {
        node(&paths, &path, &root)?
    } else {
        launch.program.clone()
    };
    let mut result = Command::new(program);
    if launch.node {
        result.arg(&launch.program);
    }
    result.args(&launch.args).args(args).env("PATH", path);
    Ok(result)
}

fn search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    push_unique(
        &mut paths,
        std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .unwrap_or_default(),
    );
    // A Finder launch starts with a bare PATH, so ask the login shell where
    // the user actually installs tools. This finds OpenCode in ~/.opencode/bin
    // and version-manager shims (nvm, asdf, mise, fnm) without extra config.
    push_unique(&mut paths, login_shell_paths().iter().cloned());
    if let Some(home) = dirs::home_dir() {
        push_unique(
            &mut paths,
            [
                ".local/bin",
                ".opencode/bin",
                ".bun/bin",
                ".npm-global/bin",
                ".cargo/bin",
                ".volta/bin",
            ]
            .into_iter()
            .map(|dir| home.join(dir)),
        );
        #[cfg(windows)]
        push_unique(&mut paths, [home.join("AppData/Roaming/npm")]);
    }
    push_unique(
        &mut paths,
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from),
    );
    #[cfg(windows)]
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        push_unique(&mut paths, [PathBuf::from(program_files).join("nodejs")]);
    }
    paths
}

fn push_unique(paths: &mut Vec<PathBuf>, candidates: impl IntoIterator<Item = PathBuf>) {
    for candidate in candidates {
        if !candidate.as_os_str().is_empty() && !paths.contains(&candidate) {
            paths.push(candidate);
        }
    }
}

#[cfg(unix)]
fn login_shell_paths() -> &'static [PathBuf] {
    LOGIN_SHELL_PATHS.get_or_init(|| login_shell_path().unwrap_or_default())
}

#[cfg(not(unix))]
fn login_shell_paths() -> &'static [PathBuf] {
    &[]
}

/// Reads PATH from the user's interactive login shell once per process. The
/// probe writes markers around the value, so prompt or rc noise cannot leak
/// into the result. It runs on a worker thread with a hard deadline and a
/// discarded stderr.
#[cfg(unix)]
fn login_shell_path() -> Option<Vec<PathBuf>> {
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| OsString::from("/bin/zsh"));
    let script = login_shell_script(&shell);
    let mut child = Command::new(&shell)
        .args(["-ilc", script.as_str()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("acp-login-path".into())
        .spawn(move || {
            let mut output = String::new();
            let _ = std::io::BufReader::new(stdout)
                .take(64 * 1024)
                .read_to_string(&mut output);
            let _ = sender.send(output);
        })
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < LOGIN_SHELL_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = receiver.recv_timeout(Duration::from_secs(1)).ok()?;
    parse_login_shell_path(&output)
}

/// Fish keeps PATH as a list, so a plain `"$PATH"` expands with spaces. Join
/// it with colons the way the exported variable looks.
#[cfg(unix)]
fn login_shell_script(shell: &OsStr) -> String {
    let name = Path::new(shell)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let value = if name.contains("fish") {
        "(string join : $PATH)"
    } else {
        "\"$PATH\""
    };
    format!("printf '\\n{PATH_MARKER_START}%s{PATH_MARKER_END}\\n' {value}")
}

fn parse_login_shell_path(output: &str) -> Option<Vec<PathBuf>> {
    let start = output.find(PATH_MARKER_START)? + PATH_MARKER_START.len();
    let rest = &output[start..];
    let end = rest.find(PATH_MARKER_END)?;
    let paths: Vec<PathBuf> = std::env::split_paths(&rest[..end]).collect();
    (!paths.is_empty()).then_some(paths)
}

fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(windows)]
    {
        return path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| {
                ["exe", "com", "cmd", "bat"]
                    .iter()
                    .any(|allowed| extension.eq_ignore_ascii_case(allowed))
            });
    }
    #[cfg(not(any(unix, windows)))]
    true
}

fn resolve(command: &OsStr, paths: &[PathBuf]) -> Option<PathBuf> {
    let command = Path::new(command);
    if command.components().count() > 1 || command.is_absolute() {
        return executable(command).then(|| command.to_path_buf());
    }
    for dir in paths {
        let candidate = dir.join(command);
        if executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        for extension in ["exe", "cmd", "bat"] {
            let candidate = candidate.with_extension(extension);
            if executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn read_launch(root: &Path) -> Option<Launch> {
    let launch: Launch = serde_json::from_slice(&fs::read(root.join("launch.json")).ok()?).ok()?;
    let canonical = launch.program.canonicalize().ok()?;
    if canonical.starts_with(root.canonicalize().ok()?)
        && if launch.node {
            canonical.is_file()
        } else {
            executable(&canonical)
        }
    {
        Some(launch)
    } else {
        None
    }
}

fn install(id: &str, root: &Path, paths: &[PathBuf], path: &OsStr) -> anyhow::Result<Launch> {
    let client = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(120))
        .timeout_write(Duration::from_secs(30))
        .build();
    let mut registry_bytes = Vec::new();
    client
        .get(REGISTRY)
        .call()?
        .into_reader()
        .take(8 * 1024 * 1024)
        .read_to_end(&mut registry_bytes)?;
    let registry: Value = serde_json::from_slice(&registry_bytes)?;
    let agent = registry["agents"]
        .as_array()
        .and_then(|agents| agents.iter().find(|agent| agent["id"] == id))
        .ok_or_else(|| anyhow!("The official ACP registry has no '{id}' adapter"))?;
    let version = agent["version"]
        .as_str()
        .ok_or_else(|| anyhow!("The ACP registry has no adapter version"))?;
    if version.is_empty()
        || !version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
    {
        bail!("The ACP registry has an invalid adapter version");
    }
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let staging = root.join(format!(".stage-{}-{nonce}", std::process::id()));
    fs::create_dir(&staging)?;
    let mut installed_dir = None;
    let result = (|| {
        let mut launch = if id == "claude-acp" {
            install_claude(agent, &staging, paths, path)?
        } else {
            install_agy(agent, &staging, paths, path, &client)?
        };
        let relative = launch.program.strip_prefix(&staging)?.to_path_buf();
        let destination = root.join(format!("{version}-{nonce}"));
        fs::rename(&staging, &destination)?;
        installed_dir = Some(destination.clone());
        launch.program = destination.join(relative);
        let metadata = root.join(format!(".launch-{nonce}.json"));
        fs::write(&metadata, serde_json::to_vec(&launch)?)?;
        #[cfg(windows)]
        if root.join("launch.json").exists() {
            fs::remove_file(root.join("launch.json"))?;
        }
        fs::rename(metadata, root.join("launch.json"))?;
        Ok(launch)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
        if let Some(destination) = installed_dir {
            let _ = fs::remove_dir_all(destination);
        }
        let _ = fs::remove_file(root.join(format!(".launch-{nonce}.json")));
    }
    result
}

fn node(paths: &[PathBuf], path: &OsStr, root: &Path) -> anyhow::Result<PathBuf> {
    let program = resolve(OsStr::new("node"), paths).ok_or_else(|| {
        anyhow!("Missing prerequisite 'node'. Install Node.js 22 or later for claude-agent-acp.")
    })?;
    let mut command = Command::new(&program);
    command.arg("--version").env("PATH", path);
    let output = run(command, root, Duration::from_secs(10))?;
    let major = output
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|value| value.parse::<u32>().ok());
    if major.is_none_or(|version| version < 22) {
        bail!(
            "claude-agent-acp requires Node.js 22 or later. Found '{}'.",
            output.trim()
        );
    }
    Ok(program)
}

fn install_claude(
    agent: &Value,
    staging: &Path,
    paths: &[PathBuf],
    path: &OsStr,
) -> anyhow::Result<Launch> {
    node(paths, path, staging)?;
    let npm = resolve(OsStr::new("npm"), paths)
        .ok_or_else(|| anyhow!("Missing prerequisite 'npm'. Install npm for claude-agent-acp."))?;
    let package = agent["distribution"]["npx"]["package"]
        .as_str()
        .ok_or_else(|| anyhow!("The ACP registry has no Claude npm package"))?;
    let (name, version) = package
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("The Claude npm package is not pinned"))?;
    if name != "@agentclientprotocol/claude-agent-acp"
        || version.is_empty()
        || !version.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        bail!("The ACP registry has an unexpected Claude npm package: {package}");
    }
    fs::write(staging.join("package.json"), b"{\"private\":true}")?;
    let mut command = Command::new(npm);
    command
        .args([
            "install",
            "--no-audit",
            "--no-fund",
            "--save-exact",
            package,
        ])
        .current_dir(staging)
        .env("PATH", path);
    run(command, staging, Duration::from_secs(300))?;
    let package_root = staging.join("node_modules").join(name);
    let manifest: Value = serde_json::from_slice(&fs::read(package_root.join("package.json"))?)?;
    let bin = manifest["bin"]
        .as_str()
        .or_else(|| manifest["bin"]["claude-agent-acp"].as_str())
        .ok_or_else(|| anyhow!("The Claude npm package has no claude-agent-acp executable"))?;
    let program = package_root.join(safe_relative(bin)?);
    if !program.is_file()
        || !program
            .canonicalize()?
            .starts_with(package_root.canonicalize()?)
    {
        bail!("The Claude npm executable is missing or outside its package");
    }
    Ok(Launch {
        program,
        args: vec![],
        node: true,
    })
}

fn install_agy(
    agent: &Value,
    staging: &Path,
    paths: &[PathBuf],
    path: &OsStr,
    client: &ureq::Agent,
) -> anyhow::Result<Launch> {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    };
    let platform = format!("{os}-{}", std::env::consts::ARCH);
    let binary = &agent["distribution"]["binary"][&platform];
    let archive = binary["archive"]
        .as_str()
        .ok_or_else(|| anyhow!("Antigravity ACP has no adapter for {platform}"))?;
    if !archive.starts_with("https://dl.google.com/agy-extensions/releases/")
        || !archive.ends_with(".zip")
    {
        bail!("The ACP registry has an unexpected Antigravity download URL");
    }
    let cmd = safe_relative(
        binary["cmd"]
            .as_str()
            .ok_or_else(|| anyhow!("The Antigravity adapter has no command"))?,
    )?;
    let args = binary
        .get("args")
        .map(|args| serde_json::from_value::<Vec<String>>(args.clone()))
        .transpose()?
        .unwrap_or_default();
    let mut bytes = Vec::new();
    client
        .get(archive)
        .call()?
        .into_reader()
        .take(MAX_ARCHIVE + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ARCHIVE {
        bail!("The Antigravity adapter archive exceeds 512 MiB");
    }
    validate_zip(&bytes)?;
    let zip = staging.join("adapter.zip");
    File::create(&zip)?.write_all(&bytes)?;
    #[cfg(not(windows))]
    let mut extraction = {
        let unzip = resolve(OsStr::new("unzip"), paths).ok_or_else(|| {
            anyhow!("Missing prerequisite 'unzip'. Install unzip for agy_acp_server.")
        })?;
        let mut command = Command::new(unzip);
        command.arg("-q").arg(&zip).arg("-d").arg(staging);
        command
    };
    #[cfg(windows)]
    let mut extraction = {
        let powershell = resolve(OsStr::new("powershell"), paths).ok_or_else(|| {
            anyhow!("Missing prerequisite 'powershell'. Install PowerShell for agy_acp_server.")
        })?;
        let mut command = Command::new(powershell);
        command.args(["-NoProfile", "-NonInteractive", "-Command", "Expand-Archive -LiteralPath $env:FASTTY_ACP_ZIP -DestinationPath $env:FASTTY_ACP_DEST -Force"])
            .env("FASTTY_ACP_ZIP", &zip).env("FASTTY_ACP_DEST", staging);
        command
    };
    extraction.env("PATH", path);
    run(extraction, staging, Duration::from_secs(120))?;
    fs::remove_file(zip)?;
    let program = staging.join(cmd);
    if !program.is_file() {
        bail!(
            "The Antigravity archive has no '{}' executable",
            program.display()
        );
    }
    #[cfg(unix)]
    make_executable(staging)?;
    Ok(Launch {
        program,
        args,
        node: false,
    })
}

fn safe_relative(value: &str) -> anyhow::Result<PathBuf> {
    if value.is_empty() || value.contains('\\') || value.contains(':') || value.starts_with('/') {
        bail!("The ACP adapter has an unsafe path: {value}");
    }
    let path = PathBuf::from(value);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        bail!("The ACP adapter has an unsafe path: {value}");
    }
    Ok(path)
}

fn validate_zip(bytes: &[u8]) -> anyhow::Result<()> {
    let end = bytes
        .len()
        .checked_sub(22)
        .ok_or_else(|| anyhow!("The ACP ZIP archive is incomplete"))?;
    let eocd = (end.saturating_sub(65535)..=end)
        .rev()
        .find(|&i| bytes.get(i..i + 4) == Some(b"PK\x05\x06"))
        .ok_or_else(|| anyhow!("The ACP ZIP archive has no directory"))?;
    let u16_at = |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
    let u32_at = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    if u16_at(eocd + 4) != 0 || u16_at(eocd + 6) != 0 || u16_at(eocd + 8) != u16_at(eocd + 10) {
        bail!("The ACP ZIP archive uses multiple disks");
    }
    let count = u16_at(eocd + 10);
    let mut offset = u32_at(eocd + 16) as usize;
    let directory_end = offset
        .checked_add(u32_at(eocd + 12) as usize)
        .ok_or_else(|| anyhow!("Invalid ACP ZIP directory"))?;
    if count == u16::MAX || directory_end != eocd {
        bail!("The ACP ZIP directory is invalid or uses ZIP64");
    }
    let mut expanded = 0u64;
    for _ in 0..count {
        if offset.checked_add(46).is_none_or(|end| end > directory_end)
            || bytes.get(offset..offset + 4) != Some(b"PK\x01\x02")
        {
            bail!("Invalid ACP ZIP directory entry");
        }
        let name_len = u16_at(offset + 28) as usize;
        let next =
            offset + 46 + name_len + u16_at(offset + 30) as usize + u16_at(offset + 32) as usize;
        if next > directory_end {
            bail!("Invalid ACP ZIP entry length");
        }
        let name = std::str::from_utf8(&bytes[offset + 46..offset + 46 + name_len])?;
        safe_relative(name)?;
        let mode = u32_at(offset + 38) >> 16;
        if mode & 0o170000 == 0o120000 || name.contains('\0') {
            bail!("The ACP ZIP archive contains an unsafe link or name");
        }
        expanded += u32_at(offset + 24) as u64;
        if expanded > 2 * 1024 * 1024 * 1024 {
            bail!("The expanded ACP ZIP archive exceeds 2 GiB");
        }
        offset = next;
    }
    if offset != directory_end {
        bail!("Invalid ACP ZIP directory size");
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(root: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            make_executable(&entry.path())?;
        } else if entry.file_type()?.is_file() {
            let mut permissions = entry.metadata()?.permissions();
            permissions.set_mode(permissions.mode() | 0o100);
            fs::set_permissions(entry.path(), permissions)?;
        }
    }
    Ok(())
}

fn run(mut command: Command, root: &Path, timeout: Duration) -> anyhow::Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let log_path = root.join("install.log");
    let log = File::create(&log_path)?;
    command
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    let name: OsString = command.get_program().to_os_string();
    let mut child = command
        .spawn()
        .with_context(|| format!("Cannot start prerequisite '{}'", name.to_string_lossy()))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let mut output = String::new();
            File::open(&log_path)?
                .take(64 * 1024)
                .read_to_string(&mut output)?;
            if !status.success() {
                bail!(
                    "ACP prerequisite '{}' failed ({status}): {}",
                    name.to_string_lossy(),
                    output.trim()
                );
            }
            return Ok(output);
        }
        if start.elapsed() >= timeout {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
            }
            #[cfg(windows)]
            let _ = Command::new("taskkill")
                .args(["/F", "/T", "/PID", &child.id().to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "ACP prerequisite '{}' exceeded {} seconds",
                name.to_string_lossy(),
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_marked_path_and_ignores_shell_noise() {
        let output = format!(
            "welcome to zsh\n{PATH_MARKER_START}/usr/bin:/opt/homebrew/bin{PATH_MARKER_END}\nbye\n"
        );
        assert_eq!(
            parse_login_shell_path(&output),
            Some(vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/opt/homebrew/bin")
            ])
        );
    }

    #[test]
    fn rejects_unmarked_shell_output() {
        assert_eq!(
            parse_login_shell_path("zsh: command not found: brew\n"),
            None
        );
        assert_eq!(parse_login_shell_path(""), None);
    }

    #[test]
    fn fish_joins_the_path_list_with_colons() {
        assert!(login_shell_script(OsStr::new("/opt/homebrew/bin/fish"))
            .contains("string join : $PATH"));
        assert!(login_shell_script(OsStr::new("/bin/zsh")).contains("\"$PATH\""));
    }

    #[test]
    fn push_unique_keeps_order_and_drops_duplicates() {
        let mut paths = vec![PathBuf::from("/usr/bin")];
        push_unique(
            &mut paths,
            [
                PathBuf::from("/usr/bin"),
                PathBuf::from(""),
                PathBuf::from("/opt/homebrew/bin"),
            ],
        );
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/opt/homebrew/bin")
            ]
        );
    }
}
