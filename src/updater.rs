//! Auto-update system following the Tinycast pattern.
//!
//! Uses GitHub Releases as feed, stream download with progress,
//! SHA-256 verification, volume-local staging, and a detached waiter
//! process for atomic swap and relaunch.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DEFAULT_GITHUB_REPO: &str = "diegoleteliers10/fasty";

static STAGED_ARCHIVE: RwLock<Option<PathBuf>> = RwLock::new(None);

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .build()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

impl UpdateChannel {
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "beta" | "prerelease" => Self::Beta,
            _ => Self::Stable,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Beta => "Beta",
        }
    }
}

/// Channel configured via `FASTTY_UPDATE_CHANNEL` env (`stable` default,
/// `beta` opts into prereleases). Callers that own the app `Config` should
/// prefer `UpdateChannel::parse(&config.update_channel)` so the Settings UI
/// selector takes effect; this helper is the fallback for contexts without
/// config access (e.g. the startup background check).
pub fn configured_channel() -> UpdateChannel {
    std::env::var("FASTTY_UPDATE_CHANNEL")
        .map(|v| UpdateChannel::parse(&v))
        .unwrap_or(UpdateChannel::Stable)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpdateCheckState {
    pub last_checked_at: u64,
    pub latest_seen: Option<String>,
    pub dismissed_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct GitHubRelease {
    pub tag_name: String,
    pub html_url: Option<String>,
    #[allow(dead_code)]
    pub name: Option<String>,
    pub body: Option<String>,
    #[serde(default)]
    pub prerelease: bool,
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub tag_name: String,
    pub release_url: String,
    pub asset_name: String,
    pub download_url: String,
    pub release_notes: String,
    pub published_at: String,
    pub checksum_url: Option<String>,
    pub signature_url: Option<String>,
    pub self_update_blocked_reason: Option<String>,
}

pub type UpdateRelease = ReleaseInfo;

#[derive(Debug)]
pub enum UpdateError {
    Network(String),
    Serialization(String),
    Io(io::Error),
    InvalidVersion(String),
    NoMatchingAsset(String),
    ChecksumMismatch { expected: String, actual: String },
    SignatureVerificationFailed(String),
    Translocated(String),
    PermissionDenied(String),
    Cancelled,
    UnsupportedPlatform(String),
    Blocked(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(msg) => write!(f, "Network error: {msg}"),
            Self::Serialization(msg) => write!(f, "Serialization error: {msg}"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::InvalidVersion(msg) => write!(f, "Invalid version: {msg}"),
            Self::NoMatchingAsset(msg) => write!(f, "No matching release asset: {msg}"),
            Self::ChecksumMismatch { expected, actual } => {
                write!(f, "Checksum mismatch (expected {expected}, got {actual})")
            }
            Self::SignatureVerificationFailed(msg) => {
                write!(f, "Signature verification failed: {msg}")
            }
            Self::Translocated(msg) => write!(f, "App is translocated: {msg}"),
            Self::PermissionDenied(msg) => write!(f, "Permission denied: {msg}"),
            Self::Cancelled => write!(f, "Update cancelled"),
            Self::UnsupportedPlatform(msg) => write!(f, "Unsupported platform: {msg}"),
            Self::Blocked(msg) => write!(f, "Update blocked: {msg}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<io::Error> for UpdateError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

pub fn update_cache_dir() -> io::Result<PathBuf> {
    let base_cache = dirs::cache_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No cache directory found"))?;
    let dir = base_cache.join("fastty").join("updates");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn state_file_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|p| p.join("fastty").join("update-check.json"))
}

pub fn load_check_state() -> UpdateCheckState {
    let Some(path) = state_file_path() else {
        return UpdateCheckState::default();
    };
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(state) = serde_json::from_slice(&bytes) {
            return state;
        }
    }
    UpdateCheckState::default()
}

pub fn save_check_state(state: &UpdateCheckState) {
    let Some(path) = state_file_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = fs::write(path, json);
    }
}

pub fn dismiss_version(version: &str) {
    let mut state = load_check_state();
    state.dismissed_version = Some(version.to_string());
    save_check_state(&state);
}

pub fn is_version_dismissed(version: &str) -> bool {
    let state = load_check_state();
    state.dismissed_version.as_deref() == Some(version)
}

fn current_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn self_update_blocked_reason() -> Option<String> {
    if let Ok(explanation) = std::env::var("FASTTY_UPDATE_EXPLANATION") {
        if !explanation.trim().is_empty() {
            return Some(explanation);
        }
    }

    #[allow(unused_variables)]
    let exe = std::env::current_exe().ok()?;

    #[cfg(target_os = "macos")]
    {
        let exe_str = exe.to_string_lossy();
        if exe_str.contains("/AppTranslocation/") {
            return Some(
                "Fastty is running in macOS App Translocation. Move Fastty.app to /Applications before updating."
                    .to_string(),
            );
        }
    }

    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("APPIMAGE").is_some() {
            return Some(
                "Fastty is running as an AppImage. Download the latest Fastty_*.AppImage from the Releases page to update."
                    .to_string(),
            );
        }
        if exe.starts_with("/usr") {
            return Some(
                "Fastty was installed via a system package. Update it with your package manager, e.g. `sudo apt update && sudo apt upgrade fastty`."
                    .to_string(),
            );
        }
    }

    #[cfg(target_os = "windows")]
    {
        let exe_str = exe.to_string_lossy();
        for program_files_var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Ok(program_files) = std::env::var(program_files_var) {
                if !program_files.is_empty() && exe_str.starts_with(program_files.as_str()) {
                    return Some(
                        "Fastty is installed system-wide and requires administrator rights to update. Download and run the latest Fastty_*.msi installer from the Releases page."
                            .to_string(),
                    );
                }
            }
        }
    }

    None
}

pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let clean = v.trim().trim_start_matches('v');
    let mut parts = clean.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").split('-').next()?.parse().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer_version(remote: &str, current: &str) -> bool {
    let remote_clean = remote.trim().trim_start_matches('v');
    let current_clean = current.trim().trim_start_matches('v');

    if let (Ok(r), Ok(c)) = (
        semver::Version::parse(remote_clean),
        semver::Version::parse(current_clean),
    ) {
        return r > c;
    }

    match (parse_version(remote), parse_version(current)) {
        (Some(r), Some(c)) => r > c,
        _ => remote != current && !remote.is_empty(),
    }
}

pub fn match_target_asset(assets: &[ReleaseAsset]) -> Result<ReleaseAsset, UpdateError> {
    #[cfg(target_os = "macos")]
    {
        let is_arm = cfg!(target_arch = "aarch64");
        let exact_triple = if is_arm {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        };

        if let Some(asset) = assets.iter().find(|a| {
            (a.name.contains(exact_triple) || a.name.contains("universal-apple-darwin"))
                && (a.name.ends_with(".tar.gz") || a.name.ends_with(".zip"))
        }) {
            return Ok(asset.clone());
        }

        if let Some(asset) = assets
            .iter()
            .find(|a| (a.name.ends_with(".tar.gz") || a.name.ends_with(".zip")) && a.name.contains("darwin"))
        {
            return Ok(asset.clone());
        }
    }

    #[cfg(target_os = "windows")]
    {
        let exact_triple = "x86_64-pc-windows-msvc";
        if let Some(asset) = assets
            .iter()
            .find(|a| a.name.contains(exact_triple) && a.name.ends_with(".zip"))
        {
            return Ok(asset.clone());
        }
        if let Some(asset) = assets
            .iter()
            .find(|a| a.name.ends_with(".zip") && a.name.contains("windows"))
        {
            return Ok(asset.clone());
        }
    }

    #[cfg(target_os = "linux")]
    {
        let is_appimage = std::env::var_os("APPIMAGE").is_some();
        if is_appimage {
            if let Some(asset) = assets.iter().find(|a| a.name.ends_with(".AppImage")) {
                return Ok(asset.clone());
            }
        }
        let exact_triple = "x86_64-unknown-linux-gnu";
        if let Some(asset) = assets
            .iter()
            .find(|a| a.name.contains(exact_triple) && a.name.ends_with(".tar.gz"))
        {
            return Ok(asset.clone());
        }
        if let Some(asset) = assets.iter().find(|a| a.name.ends_with(".AppImage")) {
            return Ok(asset.clone());
        }
    }

    Err(UpdateError::NoMatchingAsset(
        "No matching asset found for target OS/Arch in release assets".to_owned(),
    ))
}

pub fn clean_release_notes(raw: &str) -> String {
    let mut text = raw;
    if let Some((clean, _)) = text.split_once("<!-- fastty:install -->") {
        text = clean;
    } else if let Some((clean, _)) = text.split_once("<!-- corvo:install -->") {
        text = clean;
    } else if let Some((clean, _)) = text.split_once("<!-- tinycast:install -->") {
        text = clean;
    }
    text.trim().to_string()
}

pub fn check_for_updates(
    channel: UpdateChannel,
    force: bool,
) -> Result<Option<ReleaseInfo>, UpdateError> {
    let current_ver_str = env!("CARGO_PKG_VERSION");
    let current_ver = semver::Version::parse(current_ver_str)
        .map_err(|e| UpdateError::InvalidVersion(format!("Cannot parse current version: {e}")))?;

    let now = current_timestamp_secs();
    let mut state = load_check_state();

    if !force && now.saturating_sub(state.last_checked_at) < 24 * 3600 {
        return Ok(None);
    }

    let url = format!("https://api.github.com/repos/{DEFAULT_GITHUB_REPO}/releases");
    let user_agent = format!("fastty/{current_ver_str}");

    let response = http_agent()
        .get(&url)
        .set("User-Agent", &user_agent)
        .set("Accept", "application/vnd.github.v3+json")
        .call()
        .map_err(|e| UpdateError::Network(format!("GitHub API request failed: {e}")))?;

    let releases: Vec<GitHubRelease> = response.into_json().map_err(|e| {
        UpdateError::Serialization(format!("Failed to parse GitHub releases JSON: {e}"))
    })?;

    state.last_checked_at = now;

    let mut eligible_releases: Vec<(semver::Version, GitHubRelease)> = Vec::new();
    for rel in releases {
        let tag_cleaned = rel.tag_name.trim_start_matches('v');
        let Ok(ver) = semver::Version::parse(tag_cleaned) else {
            continue;
        };

        if channel == UpdateChannel::Stable && (rel.prerelease || !ver.pre.is_empty()) {
            continue;
        }

        if ver > current_ver {
            eligible_releases.push((ver, rel));
        }
    }

    eligible_releases.sort_by(|a, b| b.0.cmp(&a.0));

    let Some((_latest_ver, latest_rel)) = eligible_releases.into_iter().next() else {
        save_check_state(&state);
        return Ok(None);
    };

    let latest_tag = latest_rel.tag_name.clone();
    state.latest_seen = Some(latest_tag.clone());
    save_check_state(&state);

    if !force && state.dismissed_version.as_deref() == Some(&latest_tag) {
        return Ok(None);
    }

    let notes = latest_rel
        .body
        .as_deref()
        .unwrap_or("No release notes provided.");
    let clean_notes = clean_release_notes(notes);

    let version = latest_tag.trim_start_matches('v').to_string();
    let release_url = latest_rel.html_url.unwrap_or_else(|| {
        format!("https://github.com/{DEFAULT_GITHUB_REPO}/releases/tag/{latest_tag}")
    });
    let published_at = latest_rel.published_at.unwrap_or_default();

    // A release without a matching asset for this OS/arch must still surface
    // the changelog modal (with a manual-download reason) instead of
    // erroring out: `check_for_update_sync` maps Err to None, which hides
    // every update affordance (modal, Skip/Update buttons, tab-bar badge).
    let target_asset = match match_target_asset(&latest_rel.assets) {
        Ok(asset) => asset,
        Err(asset_err) => {
            let reason = self_update_blocked_reason().unwrap_or_else(|| {
                format!(
                    "No automatic update package found for this OS/arch ({asset_err}). Download it manually from the Releases page."
                )
            });
            return Ok(Some(ReleaseInfo {
                version,
                tag_name: latest_tag,
                release_url,
                asset_name: String::new(),
                download_url: String::new(),
                release_notes: clean_notes,
                published_at,
                checksum_url: None,
                signature_url: None,
                self_update_blocked_reason: Some(reason),
            }));
        }
    };

    let checksum_name = format!("{}.sha256", target_asset.name);
    let checksum_url = latest_rel
        .assets
        .iter()
        .find(|a| a.name == checksum_name || a.name == "SHA256SUMS" || a.name == "checksums.txt")
        .map(|a| a.browser_download_url.clone());

    let signature_name = format!("{}.minisig", target_asset.name);
    let signature_url = latest_rel
        .assets
        .iter()
        .find(|a| a.name == signature_name)
        .map(|a| a.browser_download_url.clone());

    Ok(Some(ReleaseInfo {
        version,
        tag_name: latest_tag,
        release_url,
        asset_name: target_asset.name,
        download_url: target_asset.browser_download_url,
        release_notes: clean_notes,
        published_at,
        checksum_url,
        signature_url,
        self_update_blocked_reason: self_update_blocked_reason(),
    }))
}

pub fn check_for_update_sync() -> Option<ReleaseInfo> {
    check_for_updates(configured_channel(), false).ok().flatten()
}

/// Sync check with an explicit channel (Settings selector) and force flag.
/// `force = true` bypasses the 24h throttle and the skipped-version gate so
/// a manual "Check for updates" always reaches the network.
pub fn check_for_update_sync_with(channel: UpdateChannel, force: bool) -> Result<Option<ReleaseInfo>, UpdateError> {
    check_for_updates(channel, force)
}

pub fn download_and_verify(
    release: &ReleaseInfo,
    cancel_flag: &AtomicBool,
    on_progress: Option<&dyn Fn(u64, u64)>,
) -> Result<PathBuf, UpdateError> {
    let cache_dir = update_cache_dir()?;
    let dest_path = cache_dir.join(&release.asset_name);

    let user_agent = format!("fastty/{}", env!("CARGO_PKG_VERSION"));
    let resp = http_agent()
        .get(&release.download_url)
        .set("User-Agent", &user_agent)
        .call()
        .map_err(|e| UpdateError::Network(format!("Failed to download asset: {e}")))?;

    let total_size = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let mut reader = resp.into_reader();
    let mut file = File::create(&dest_path)?;
    let mut hasher = Sha256::new();

    let mut downloaded = 0u64;
    let mut buffer = [0u8; 16384];

    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            let _ = fs::remove_file(&dest_path);
            return Err(UpdateError::Cancelled);
        }

        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }

        file.write_all(&buffer[..bytes_read])?;
        hasher.update(&buffer[..bytes_read]);
        downloaded += bytes_read as u64;

        if let Some(cb) = on_progress {
            cb(downloaded, total_size);
        }
    }

    file.flush()?;
    drop(file);

    let computed_hash = hex::encode(hasher.finalize());

    if let Some(ref checksum_url) = release.checksum_url {
        let cs_resp = http_agent()
            .get(checksum_url)
            .set("User-Agent", &user_agent)
            .call()
            .map_err(|e| UpdateError::Network(format!("Failed to download checksum: {e}")))?;

        let mut cs_text = String::new();
        cs_resp.into_reader().read_to_string(&mut cs_text)?;

        let mut expected_hash = None;
        for line in cs_text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }
            if parts.len() == 1 && parts[0].len() == 64 {
                expected_hash = Some(parts[0].to_lowercase());
                break;
            } else if parts.len() >= 2 {
                let hash = parts[0].to_lowercase();
                let file_name = parts[1].trim_start_matches('*');
                if file_name == release.asset_name {
                    expected_hash = Some(hash);
                    break;
                }
            }
        }

        if let Some(expected) = expected_hash {
            if computed_hash != expected {
                let _ = fs::remove_file(&dest_path);
                return Err(UpdateError::ChecksumMismatch {
                    expected,
                    actual: computed_hash,
                });
            }
        }
    }

    Ok(dest_path)
}

pub fn apply_update_sync(release: &ReleaseInfo) -> anyhow::Result<()> {
    if let Some(reason) = &release.self_update_blocked_reason {
        anyhow::bail!("{reason}");
    }
    if release.download_url.trim().is_empty() {
        anyhow::bail!("No automatic update package for this OS/arch. Download it from the Releases page.");
    }

    let cancel_flag = AtomicBool::new(false);
    let downloaded_path = download_and_verify(release, &cancel_flag, None)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    *STAGED_ARCHIVE.write() = Some(downloaded_path);
    Ok(())
}

pub fn install_and_restart(archive_path: &Path) -> Result<(), UpdateError> {
    #[cfg(target_os = "macos")]
    {
        install_macos(archive_path)
    }

    #[cfg(target_os = "windows")]
    {
        install_windows(archive_path)
    }

    #[cfg(target_os = "linux")]
    {
        install_linux(archive_path)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(UpdateError::UnsupportedPlatform(
            "Self-update not implemented for this OS".into(),
        ))
    }
}

#[cfg(target_os = "macos")]
fn install_macos(archive_path: &Path) -> Result<(), UpdateError> {
    let current_exe = std::env::current_exe()?;
    let mut bundle_dir = current_exe.clone();
    while let Some(parent) = bundle_dir.parent() {
        if bundle_dir.extension().and_then(|e| e.to_str()) == Some("app") {
            break;
        }
        bundle_dir = parent.to_path_buf();
    }

    if bundle_dir.extension().and_then(|e| e.to_str()) != Some("app") {
        return Err(UpdateError::UnsupportedPlatform(
            "Fastty is not running from a macOS .app bundle".into(),
        ));
    }

    let bundle_str = bundle_dir.to_string_lossy();
    if bundle_str.contains("/AppTranslocation/") {
        return Err(UpdateError::Translocated(
            "Fastty is running in macOS App Translocation. Please move Fastty.app to /Applications before updating.".into(),
        ));
    }

    let Some(install_parent) = bundle_dir.parent() else {
        return Err(UpdateError::PermissionDenied(
            "Cannot determine parent directory of Fastty.app".into(),
        ));
    };

    let staging_app = install_parent.join("Fastty.app.staging");
    let _ = fs::remove_dir_all(&staging_app);

    let temp_staging_dir = install_parent.join(".fastty_staging_temp");
    let _ = fs::remove_dir_all(&temp_staging_dir);
    fs::create_dir_all(&temp_staging_dir)?;

    let output = Command::new("ditto")
        .arg("-x")
        .arg("-k")
        .arg(archive_path)
        .arg(&temp_staging_dir)
        .output();

    let ditto_failed = match output {
        Ok(out) => !out.status.success(),
        Err(_) => true,
    };

    if ditto_failed {
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(archive_path)
            .arg("-C")
            .arg(&temp_staging_dir)
            .status()?;
        if !status.success() {
            let _ = fs::remove_dir_all(&temp_staging_dir);
            return Err(UpdateError::Io(io::Error::other(
                "Failed to extract update bundle",
            )));
        }
    }

    let extracted_app = temp_staging_dir.join("Fastty.app");
    if !extracted_app.exists() {
        let _ = fs::remove_dir_all(&temp_staging_dir);
        return Err(UpdateError::NoMatchingAsset(
            "Fastty.app not found inside update archive".into(),
        ));
    }

    fs::rename(&extracted_app, &staging_app)?;
    let _ = fs::remove_dir_all(&temp_staging_dir);

    // Gatekeeper keys on the quarantine flag, so clearing it is what lets a
    // freshly downloaded bundle run. This must run before any signature work.
    let _ = Command::new("xattr").arg("-cr").arg(&staging_app).status();

    // A rejected update leaves a full app bundle next to the working one. Drop it
    // now, because the next attempt only cleans up on its way in.
    if let Err(e) = verify_macos_update_identity(&bundle_dir, &staging_app) {
        let _ = fs::remove_dir_all(&staging_app);
        return Err(e);
    }
    ensure_macos_signature(&staging_app);

    if !staging_app.exists() {
        return Err(UpdateError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "Staged application bundle does not exist",
        )));
    }

    let binary_in_app = bundle_dir.join("Contents/MacOS/fastty");
    if let Ok(home) = std::env::var("HOME") {
        for bin_dir in ["/usr/local/bin", &format!("{}/.local/bin", home)] {
            let symlink_path = std::path::Path::new(bin_dir).join("fastty");
            if symlink_path.exists() {
                let _ = std::fs::remove_file(&symlink_path);
                let _ = std::os::unix::fs::symlink(&binary_in_app, &symlink_path);
            }
        }
    }

    let pid = std::process::id();
    let target = bundle_dir.to_string_lossy();
    let staging = staging_app.to_string_lossy();

    let script = format!(
        "for i in $(seq 1 150); do if ! kill -0 {pid} 2>/dev/null; then break; fi; sleep 0.1; done; \
         rm -rf \"{target}.old\"; \
         mv \"{target}\" \"{target}.old\" 2>/dev/null; \
         mv \"{staging}\" \"{target}\" 2>/dev/null || true; \
         xattr -cr \"{target}\" 2>/dev/null || true; \
         cd \"$HOME\" && open -n \"{target}\"; \
         rm -rf \"{target}.old\""
    );

    Command::new("sh").arg("-c").arg(script).spawn()?;
    std::process::exit(0);
}

/// The designated requirement fastty is expected to carry once a stable
/// signing identity is configured.
///
/// The file pins the leaf hash of a self-signed certificate, which keeps the
/// requirement identical across builds. That is what stops macOS treating every
/// update as a different app and asking the user to accept it again.
#[cfg(any(target_os = "macos", test))]
const MACOS_REQUIREMENT_FILE: &str = include_str!("../assets/macos-signing-requirement.txt");

/// Bundle identifier the release workflow writes into `Info.plist`. A bundle
/// with a different identifier is not fastty, whatever its signature says.
#[cfg(any(target_os = "macos", test))]
const MACOS_BUNDLE_ID: &str = "com.diegoleteliers10.fastty";

/// True when the requirement names a real signing identity, rather than the
/// placeholder left in place before a certificate exists.
#[cfg(any(target_os = "macos", test))]
fn macos_requirement_configured_shape(requirement: &str) -> bool {
    let requirement = requirement.trim();
    !requirement.is_empty()
        && !requirement.contains("REPLACE")
        && requirement.starts_with("identifier \"")
}

/// True when the pinned requirement is still a placeholder, which is the state
/// before a signing certificate exists.
#[cfg(any(target_os = "macos", test))]
fn macos_requirement_configured() -> bool {
    macos_requirement_configured_shape(MACOS_REQUIREMENT_FILE)
}

/// Extracts the designated requirement from `codesign -dr -` output.
///
/// `codesign` writes this to stderr, commented out, like:
/// `# designated => identifier "com.example.app" and certificate leaf = H"abc..."`
/// The comment marker is part of the format, not decoration, so it has to be
/// stripped before the prefix matches.
#[cfg(any(target_os = "macos", test))]
fn parse_designated_requirement(codesign_output: &str) -> Option<String> {
    codesign_output.lines().find_map(|line| {
        line.trim()
            .strip_prefix('#')
            .unwrap_or(line.trim())
            .trim_start()
            .strip_prefix("designated => ")
            .map(|requirement| requirement.trim().to_owned())
    })
}

/// Reads the designated requirement macOS records for a bundle.
///
/// `codesign -dr -` prints the requirement on stdout and the `Executable=` line
/// on stderr, so the requirement has to be read from stdout.
#[cfg(target_os = "macos")]
fn macos_designated_requirement(app: &Path) -> Option<String> {
    let output = Command::new("codesign")
        .args(["-dr", "-"])
        .arg(app)
        .output()
        .ok()?;
    parse_designated_requirement(&String::from_utf8_lossy(&output.stdout))
}

/// Reads the ad-hoc marker out of `codesign -dv --verbose=4` output, which
/// reports the signature kind as `Signature=adhoc` on stderr.
#[cfg(any(target_os = "macos", test))]
fn parse_adhoc_flag(codesign_output: &str) -> bool {
    codesign_output
        .lines()
        .any(|line| line.trim() == "Signature=adhoc")
}

/// True when the bundle carries an ad-hoc signature, whose designated
/// requirement is the content hash. That hash changes on every build, so macOS
/// treats the next build as a different app.
#[cfg(target_os = "macos")]
fn macos_has_adhoc_signature(app: &Path) -> Option<bool> {
    let output = Command::new("codesign")
        .args(["-dv", "--verbose=4"])
        .arg(app)
        .output()
        .ok()?;
    Some(parse_adhoc_flag(&String::from_utf8_lossy(&output.stderr)))
}

/// Checks that the downloaded bundle is the app it claims to be, and that its
/// signing identity matches the one already installed.
///
/// Without a pinned requirement the check only confirms the bundle identifier,
/// so a release signed with a stable identity works and a release still signed
/// ad hoc keeps working. The identity comparison only runs once a requirement
/// is pinned, so switching to a stable identity does not lock out the builds
/// that came before it.
#[cfg(target_os = "macos")]
fn verify_macos_update_identity(current: &Path, staged: &Path) -> Result<(), UpdateError> {
    // Check the signature is internally consistent before reading anything out
    // of it. The ad-hoc fallback below re-signs a bundle that fails this, which
    // would otherwise paper over a corrupted download.
    let verified = Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(staged)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !verified {
        return Err(UpdateError::SignatureVerificationFailed(
            "The downloaded app has an invalid code signature".into(),
        ));
    }

    let bundle_id = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleIdentifier"])
        .arg(staged.join("Contents/Info.plist"))
        .output()
        .map_err(|e| {
            UpdateError::SignatureVerificationFailed(format!("Cannot read the bundle identifier: {e}"))
        })?;
    if !bundle_id.status.success()
        || String::from_utf8_lossy(&bundle_id.stdout).trim() != MACOS_BUNDLE_ID
    {
        return Err(UpdateError::SignatureVerificationFailed(
            "The downloaded app has a different bundle identifier".into(),
        ));
    }

    if !macos_requirement_configured() {
        return Ok(());
    }

    let expected = MACOS_REQUIREMENT_FILE.trim();
    let staged_requirement = macos_designated_requirement(staged).ok_or_else(|| {
        UpdateError::SignatureVerificationFailed("Cannot read the app signing identity".into())
    })?;
    if staged_requirement != expected {
        return Err(UpdateError::SignatureVerificationFailed(
            "The downloaded app does not carry the expected signing identity".into(),
        ));
    }
    // A bundle already carrying the stable identity must keep it. An ad-hoc
    // installed app is allowed to move to the stable identity once.
    if let (Some(false), Some(installed)) =
        (macos_has_adhoc_signature(current), macos_designated_requirement(current))
    {
        if installed != staged_requirement {
            return Err(UpdateError::SignatureVerificationFailed(
                "The downloaded app is signed with a different identity than the installed one".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn verify_macos_update_identity(_current: &Path, _staged: &Path) -> Result<(), UpdateError> {
    Ok(())
}

/// Signs the bundle only when it carries no usable signature.
///
/// An ad-hoc signature is the fallback for builds with no signing certificate.
/// Re-signing an already signed bundle would replace a stable identity with a
/// content hash, which is the exact state that makes macOS ask the user to
/// accept the app again after every update.
#[cfg(target_os = "macos")]
fn ensure_macos_signature(app: &Path) {
    if macos_has_adhoc_signature(app) == Some(false)
        && macos_designated_requirement(app).is_some()
    {
        return;
    }
    let _ = Command::new("codesign")
        .args(["--force", "--deep", "-s", "-", app.to_str().unwrap_or_default()])
        .status();
}

#[cfg(not(target_os = "macos"))]
fn ensure_macos_signature(_app: &Path) {}

#[cfg(target_os = "windows")]
fn install_windows(archive_path: &Path) -> Result<(), UpdateError> {
    let current_exe = std::env::current_exe()?;
    let Some(target_dir) = current_exe.parent() else {
        return Err(UpdateError::PermissionDenied(
            "Cannot determine executable directory".into(),
        ));
    };

    let temp_staging_dir = target_dir.join(".fastty_staging_temp");
    let _ = fs::remove_dir_all(&temp_staging_dir);
    fs::create_dir_all(&temp_staging_dir)?;

    let mut tar_cmd = Command::new("tar");
    tar_cmd
        .arg("-xf")
        .arg(archive_path)
        .arg("-C")
        .arg(&temp_staging_dir);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        tar_cmd.creation_flags(0x08000000);
    }
    let status = tar_cmd.status();

    if status.map(|s| !s.success()).unwrap_or(true) {
        let _ = fs::remove_dir_all(&temp_staging_dir);
        return Err(UpdateError::Io(io::Error::new(
            io::ErrorKind::Other,
            "Failed to extract Windows update archive",
        )));
    }

    let extracted_exe = temp_staging_dir.join("fastty.exe");
    let staging_exe = target_dir.join("fastty.exe.new");
    let old_exe = target_dir.join("fastty.exe.old");

    if !extracted_exe.exists() {
        let _ = fs::remove_dir_all(&temp_staging_dir);
        return Err(UpdateError::NoMatchingAsset(
            "fastty.exe not found in extracted archive".into(),
        ));
    }

    let _ = fs::remove_file(&staging_exe);
    fs::copy(&extracted_exe, &staging_exe)?;
    let _ = fs::remove_dir_all(&temp_staging_dir);

    if !staging_exe.exists() {
        return Err(UpdateError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "Staged executable does not exist",
        )));
    }

    let pid = std::process::id();
    let target_str = current_exe.to_string_lossy();
    let old_str = old_exe.to_string_lossy();
    let staging_str = staging_exe.to_string_lossy();

    let script = format!(
        ":loop\r\ntasklist /fi \"PID eq {pid}\" | find \"{pid}\" >nul\r\n\
         if not errorlevel 1 (timeout /t 1 /nobreak >nul & goto loop)\r\n\
         move /y \"{target_str}\" \"{old_str}\"\r\n\
         move /y \"{staging_str}\" \"{target_str}\"\r\n\
         start \"\" \"{target_str}\""
    );

    let mut swap_cmd = Command::new("cmd");
    swap_cmd.arg("/c").arg(script);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        swap_cmd.creation_flags(0x08000000);
    }
    swap_cmd.spawn()?;
    std::process::exit(0);
}

#[cfg(target_os = "linux")]
fn install_linux(archive_path: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt;

    let target_path = if let Ok(appimage) = std::env::var("APPIMAGE") {
        PathBuf::from(appimage)
    } else {
        std::env::current_exe()?
    };

    let Some(parent) = target_path.parent() else {
        return Err(UpdateError::PermissionDenied(
            "Cannot determine target directory".into(),
        ));
    };

    if let Ok(metadata) = fs::metadata(parent) {
        if metadata.permissions().readonly() {
            return Err(UpdateError::PermissionDenied(
                "System managed installation (/usr/bin). Please update Fastty via your package manager.".into(),
            ));
        }
    }

    let target_name = target_path.file_name().unwrap().to_string_lossy();
    let staged_file = parent.join(format!("{target_name}.new"));
    let _ = fs::remove_file(&staged_file);

    if archive_path.extension().and_then(|e| e.to_str()) == Some("AppImage") {
        fs::copy(archive_path, &staged_file)?;
    } else {
        let temp_staging_dir = parent.join(".fastty_staging_temp");
        let _ = fs::remove_dir_all(&temp_staging_dir);
        fs::create_dir_all(&temp_staging_dir)?;

        let status = Command::new("tar")
            .arg("-xzf")
            .arg(archive_path)
            .arg("-C")
            .arg(&temp_staging_dir)
            .status()?;
        if !status.success() {
            let _ = fs::remove_dir_all(&temp_staging_dir);
            return Err(UpdateError::Io(io::Error::new(
                io::ErrorKind::Other,
                "Failed to extract Linux update archive",
            )));
        }

        let extracted_bin = temp_staging_dir.join("fastty");
        if !extracted_bin.exists() {
            let _ = fs::remove_dir_all(&temp_staging_dir);
            return Err(UpdateError::NoMatchingAsset(
                "fastty binary not found in update archive".into(),
            ));
        }

        fs::copy(&extracted_bin, &staged_file)?;
        let _ = fs::remove_dir_all(&temp_staging_dir);
    }

    if !staged_file.exists() {
        return Err(UpdateError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "Staged binary does not exist",
        )));
    }

    let mut perms = fs::metadata(&staged_file)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&staged_file, perms)?;

    let pid = std::process::id();
    let target_str = target_path.to_string_lossy();
    let staged_str = staged_file.to_string_lossy();

    let script = format!(
        "for i in $(seq 1 150); do if ! kill -0 {pid} 2>/dev/null; then break; fi; sleep 0.1; done; \
         mv -f \"{staged_str}\" \"{target_str}\"; \
         chmod +x \"{target_str}\"; \
         \"{target_str}\" &"
    );

    Command::new("sh").arg("-c").arg(script).spawn()?;
    std::process::exit(0);
}

pub fn relaunch_fastty() {
    if let Some(staged) = STAGED_ARCHIVE.read().clone() {
        if staged.exists() {
            if let Ok(()) = install_and_restart(&staged) {
                return;
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut dest_app = std::path::PathBuf::from("/Applications/Fastty.app");
        if let Ok(current_exe) = std::env::current_exe() {
            if let Some(parent) = current_exe.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
                if parent.extension().and_then(|e| e.to_str()) == Some("app") {
                    dest_app = parent.to_path_buf();
                }
            }
        }
        let _ = std::process::Command::new("open").arg("-n").arg(dest_app).spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(current_exe) = std::env::current_exe() {
            let _ = std::process::Command::new(current_exe).spawn();
        }
    }
    std::process::exit(0);
}

pub fn cleanup_old_installations() {
    #[cfg(target_os = "windows")]
    {
        if let Ok(exe) = std::env::current_exe() {
            let old_exe = exe.with_extension("exe.old");
            if old_exe.exists() {
                let _ = fs::remove_file(old_exe);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(exe) = std::env::current_exe() {
            let mut bundle_dir = exe;
            while let Some(parent) = bundle_dir.parent() {
                if bundle_dir.extension().and_then(|e| e.to_str()) == Some("app") {
                    break;
                }
                bundle_dir = parent.to_path_buf();
            }
            if let Some(parent) = bundle_dir.parent() {
                let old_bundle = parent.join("Fastty.app.old");
                if old_bundle.exists() {
                    let _ = fs::remove_dir_all(old_bundle);
                }
                let staging_bundle = parent.join("Fastty.app.staging");
                if staging_bundle.exists() {
                    let _ = fs::remove_dir_all(staging_bundle);
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(exe) = std::env::current_exe() {
            let old_bin = exe.with_extension("old");
            if old_bin.exists() {
                let _ = fs::remove_file(old_bin);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `codesign -dr -` stdout, recorded from an ad-hoc probe bundle. The
    /// `# ` before `designated =>` is part of the format, not decoration.
    /// Parsing this correctly is what stops a stable signing identity from being
    /// silently replaced by an ad-hoc one on every update.
    const ADHOC_REQUIREMENT_STDOUT: &str = concat!(
        "# designated => cdhash H\"02426949f4fc198605ff2825cad7fe26ab7d17a6\"",
        " or cdhash H\"64e3561f3401a60e6da58327d33a53cb8318745e\"",
        " or cdhash H\"9efa319729aa84faa400592047cab1da1634557c\"\n",
    );

    /// `codesign -dr -` writes this line to stderr, not to the requirement
    /// stream. Reading the requirement from stderr finds nothing.
    const ADHOC_REQUIREMENT_STDERR: &str =
        "Executable=/tmp/probe.app/Contents/MacOS/Probe\n";

    /// Real `codesign -dv --verbose=4` stderr, from the same bundle. The
    /// signature kind lives here, and this stream carries no requirement.
    const ADHOC_VERBOSE_STDERR: &str = concat!(
        "Executable=/tmp/probe.app/Contents/MacOS/Probe\n",
        "Identifier=com.diegoleteliers10.fastty\n",
        "Format=app bundle with Mach-O universal (x86_64 arm64e arm64e.x1)\n",
        "CodeDirectory v=20400 size=308 flags=0x2(adhoc) hashes=3+3 location=embedded\n",
        "Signature=adhoc\n",
        "Info.plist entries=6\n",
        "TeamIdentifier=not set\n",
        "Sealed Resources=3\n",
    );

    #[test]
    fn parses_the_requirement_codesign_actually_prints() {
        let requirement = parse_designated_requirement(ADHOC_REQUIREMENT_STDOUT)
            .expect("codesign comments the requirement out with a leading #");
        assert!(
            requirement.starts_with("cdhash H\""),
            "got {requirement:?}"
        );
        assert!(!requirement.contains('#'), "the comment marker must be stripped");
    }

    /// The requirement is on stdout. The stderr stream holds only the
    /// `Executable=` line, so parsing the wrong stream silently finds nothing
    /// and the updater then re-signs every bundle it downloads.
    #[test]
    fn the_requirement_is_not_on_the_stderr_stream() {
        assert_eq!(parse_designated_requirement(ADHOC_REQUIREMENT_STDERR), None);
    }

    #[test]
    fn parses_a_requirement_without_the_comment_marker() {
        // Some tool versions print the requirement uncommented.
        let output = "designated => identifier \"com.example.app\" and anchor apple generic\n";
        assert_eq!(
            parse_designated_requirement(output).as_deref(),
            Some("identifier \"com.example.app\" and anchor apple generic")
        );
    }

    #[test]
    fn requirement_parse_fails_closed_on_unrelated_output() {
        assert_eq!(parse_designated_requirement("Executable=/tmp/x\n"), None);
        assert_eq!(parse_designated_requirement(""), None);
        // A line that is not a requirement must not be accepted as one.
        assert_eq!(
            parse_designated_requirement("# Identifier=com.example.app\n"),
            None
        );
    }

    /// The comment marker may be followed by any number of spaces, which is what
    /// the shell pattern `^#[[:space:]]*designated =>` accepts. The two parsers
    /// have to agree, or CI writes a requirement the updater then rejects.
    #[test]
    fn requirement_parse_tolerates_spacing_after_the_comment_marker() {
        for spacing in ["# ", "#  ", "#\t", "#"] {
            let output = format!("{spacing}designated => cdhash H\"aa\"\n");
            assert_eq!(
                parse_designated_requirement(&output).as_deref(),
                Some("cdhash H\"aa\""),
                "failed for {spacing:?}"
            );
        }
    }

    #[test]
    fn detects_the_adhoc_marker() {
        assert!(parse_adhoc_flag(ADHOC_VERBOSE_STDERR));
        assert!(!parse_adhoc_flag(
            "Identifier=com.diegoleteliers10.fastty\nTeamIdentifier=ABCD\n"
        ));
    }

    /// The bundle identifier lives in two places: the release workflow writes it
    /// into `Info.plist`, and the updater checks it. A change to one and not the
    /// other would make every update fail the identity check, so pin them
    /// together.
    #[test]
    fn macos_bundle_id_matches_the_release_workflow() {
        let workflow = include_str!("../.github/workflows/release.yml");
        assert!(
            workflow.contains(MACOS_BUNDLE_ID),
            "the release workflow must write the bundle id {MACOS_BUNDLE_ID} into Info.plist"
        );
        let requirement = MACOS_REQUIREMENT_FILE.trim();
        assert!(
            requirement.contains(MACOS_BUNDLE_ID),
            "the pinned signing requirement must name the bundle id {MACOS_BUNDLE_ID}"
        );
    }

    /// The shell copies of the requirement parse must strip the same comment
    /// marker. They run in CI and in the setup script, where a silent mismatch
    /// either fails the release or writes an unusable requirement file.
    #[test]
    fn shell_requirement_parsers_match_the_rust_one() {
        let requirement = parse_designated_requirement(ADHOC_REQUIREMENT_STDOUT)
            .expect("the fixture parses in Rust");
        for script in [
            include_str!("../tools/setup-signing-cert.sh"),
            include_str!("../.github/workflows/release.yml"),
        ] {
            assert!(
                script.contains("s/^#[[:space:]]*designated => //p"),
                "a shell copy is missing the # prefix in its codesign requirement parse"
            );
        }
        // The recorded output is what the shell pattern has to cope with.
        assert!(ADHOC_REQUIREMENT_STDOUT.contains("# designated => "));
        let _ = requirement;
    }

    /// Before a certificate exists the requirement file holds a placeholder, and
    /// the updater must fall back to the current ad-hoc behaviour rather than
    /// reject every update.
    #[test]
    fn unconfigured_requirement_falls_back_to_adhoc() {
        let placeholder = "identifier \"com.diegoleteliers10.fastty\" and certificate leaf = H\"REPLACE\"";
        assert!(macos_requirement_configured_shape(placeholder) == false);
        let pinned = "identifier \"com.diegoleteliers10.fastty\" and certificate leaf = H\"d2d3ed87a19d1ab0c573da9bad5aea7a12f87732\"";
        assert!(macos_requirement_configured_shape(pinned));
    }

    /// A configured requirement has the shape macOS prints for a stable
    /// identity. Anything else would make the release assertion and the updater
    /// disagree.
    #[test]
    fn configured_requirement_has_the_expected_shape() {
        if !macos_requirement_configured() {
            return;
        }
        let requirement = MACOS_REQUIREMENT_FILE.trim();
        let leaf = requirement
            .split("certificate leaf = H\"")
            .nth(1)
            .expect("a stable identity pins the certificate leaf");
        let hash = leaf.trim_end_matches('"');
        assert_eq!(hash.len(), 40, "a SHA-1 leaf hash is 40 hex characters");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "the leaf hash must be hexadecimal, got {hash}"
        );
    }
}
