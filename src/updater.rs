use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: String,
    pub tag_name: String,
    pub release_url: String,
    pub asset_name: String,
    pub download_url: String,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: Option<String>,
    assets: Option<Vec<GitHubAsset>>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
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
    match (parse_version(remote), parse_version(current)) {
        (Some(r), Some(c)) => r > c,
        _ => remote != current && !remote.is_empty(),
    }
}

pub fn get_target_asset_name() -> Option<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Some("fastty-aarch64-apple-darwin.tar.gz")
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Some("fastty-x86_64-apple-darwin.tar.gz")
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Some("fastty-x86_64-unknown-linux-gnu.tar.gz")
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Some("fastty-x86_64-pc-windows-msvc.zip")
    }
    #[cfg(not(any(
        all(target_os = "macos", any(target_arch = "aarch64", target_arch = "x86_64")),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        None
    }
}

pub fn check_for_update_sync() -> Option<ReleaseInfo> {
    let current_version = env!("CARGO_PKG_VERSION");
    let api_url = "https://api.github.com/repos/diegoleteliers10/fasty/releases/latest";

    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-sSfL", "--max-time", "6", "-H", "User-Agent: fastty", api_url]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let output = cmd.output().ok()?;

    if !output.status.success() {
        return None;
    }

    let release: GitHubRelease = serde_json::from_slice(&output.stdout).ok()?;
    let tag = release.tag_name.trim();
    let version = tag.trim_start_matches('v').to_string();

    if !is_newer_version(&version, current_version) {
        return None;
    }

    let target_asset_name = get_target_asset_name()?;
    let mut download_url = format!(
        "https://github.com/diegoleteliers10/fasty/releases/download/{}/{}",
        tag, target_asset_name
    );

    if let Some(assets) = release.assets {
        for asset in assets {
            if asset.name == target_asset_name {
                download_url = asset.browser_download_url;
                break;
            }
        }
    }

    Some(ReleaseInfo {
        version,
        tag_name: tag.to_string(),
        release_url: release
            .html_url
            .unwrap_or_else(|| format!("https://github.com/diegoleteliers10/fasty/releases/tag/{}", tag)),
        asset_name: target_asset_name.to_string(),
        download_url,
    })
}

pub fn apply_update_sync(release: &ReleaseInfo) -> anyhow::Result<()> {
    let temp_dir = std::env::temp_dir().join(format!("fastty-update-{}", release.version));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir)?;

    let archive_path = temp_dir.join(&release.asset_name);

    // 1. Download archive
    let mut curl_cmd = std::process::Command::new("curl");
    curl_cmd.args(["-sSfL", "-o", archive_path.to_str().unwrap(), &release.download_url]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        curl_cmd.creation_flags(0x08000000);
    }
    let status = curl_cmd.status()?;

    if !status.success() {
        anyhow::bail!("Failed to download update asset from {}", release.download_url);
    }

    // 2. Extract and Overwrite per platform
    #[cfg(target_os = "macos")]
    {
        let ext_status = std::process::Command::new("tar")
            .args(["-xzf", archive_path.to_str().unwrap(), "-C", temp_dir.to_str().unwrap()])
            .status()?;
        if !ext_status.success() {
            anyhow::bail!("Failed to extract archive");
        }

        let new_app = temp_dir.join("Fastty.app");
        if !new_app.exists() {
            anyhow::bail!("Fastty.app not found in release archive");
        }

        let mut dest_app = std::path::PathBuf::from("/Applications/Fastty.app");
        if let Ok(current_exe) = std::env::current_exe() {
            if let Some(parent) = current_exe.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
                if parent.extension().and_then(|e| e.to_str()) == Some("app") {
                    dest_app = parent.to_path_buf();
                }
            }
        }

        let backup_app = temp_dir.join("Fastty.old.app");
        let _ = std::fs::remove_dir_all(&backup_app);
        if dest_app.exists() {
            let _ = std::fs::rename(&dest_app, &backup_app);
            let _ = std::fs::remove_dir_all(&dest_app);
        }

        let cp_status = std::process::Command::new("cp")
            .args(["-R", new_app.to_str().unwrap(), dest_app.to_str().unwrap()])
            .status()?;
        if !cp_status.success() {
            if backup_app.exists() {
                let _ = std::fs::rename(&backup_app, &dest_app);
            }
            anyhow::bail!("Failed to copy updated Fastty.app to {}", dest_app.display());
        }

        let _ = std::fs::remove_dir_all(&backup_app);
        let _ = std::fs::remove_dir_all(&temp_dir);

        let binary_in_app = dest_app.join("Contents/MacOS/fastty");
        if let Ok(home) = std::env::var("HOME") {
            for bin_dir in ["/usr/local/bin", &format!("{}/.local/bin", home)] {
                let symlink_path = std::path::Path::new(bin_dir).join("fastty");
                if symlink_path.exists() {
                    let _ = std::fs::remove_file(&symlink_path);
                    let _ = std::os::unix::fs::symlink(&binary_in_app, &symlink_path);
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let ext_status = std::process::Command::new("tar")
            .args(["-xzf", archive_path.to_str().unwrap(), "-C", temp_dir.to_str().unwrap()])
            .status()?;
        if !ext_status.success() {
            anyhow::bail!("Failed to extract archive");
        }

        let new_bin = temp_dir.join("fastty");
        if !new_bin.exists() {
            anyhow::bail!("fastty binary not found in archive");
        }

        let current_exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("/usr/local/bin/fastty"));
        let old_bin = current_exe.with_extension("old");
        let _ = std::fs::remove_file(&old_bin);
        let _ = std::fs::rename(&current_exe, &old_bin);
        std::fs::copy(&new_bin, &current_exe)?;

        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&current_exe, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_file(&old_bin);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut tar_cmd = std::process::Command::new("tar");
        tar_cmd.args(["-xf", archive_path.to_str().unwrap(), "-C", temp_dir.to_str().unwrap()]);
        tar_cmd.creation_flags(0x08000000);
        let ext_status = tar_cmd.status().or_else(|_| {
            let mut ps_cmd = std::process::Command::new("powershell");
            ps_cmd.args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                    archive_path.display(),
                    temp_dir.display()
                ),
            ]);
            ps_cmd.creation_flags(0x08000000);
            ps_cmd.status()
        })?;

        if !ext_status.success() {
            anyhow::bail!("Failed to extract zip archive");
        }

        let new_bin = temp_dir.join("fastty.exe");
        if !new_bin.exists() {
            anyhow::bail!("fastty.exe not found in archive");
        }

        if let Ok(current_exe) = std::env::current_exe() {
            let old_bin = current_exe.with_extension("old.exe");
            let _ = std::fs::remove_file(&old_bin);
            let _ = std::fs::rename(&current_exe, &old_bin);
            std::fs::copy(&new_bin, &current_exe)?;
            let _ = std::fs::remove_dir_all(&temp_dir);
        }
    }

    Ok(())
}
