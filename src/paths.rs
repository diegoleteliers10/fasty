use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context, anyhow};

pub struct FasttyDirs {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
}

pub static FASTTY_DIRS: OnceLock<FasttyDirs> = OnceLock::new();

impl FasttyDirs {
    pub fn new() -> anyhow::Result<Self> {
        let (config_dir, data_dir, state_dir, cache_dir) = if cfg!(target_os = "windows") {
            let app_data = dirs::config_dir()
                .ok_or_else(|| anyhow!("dirs::config_dir returned None (unsupported platform)"))?;
            let local_app_data = dirs::data_local_dir()
                .ok_or_else(|| anyhow!("dirs::data_local_dir returned None (unsupported platform)"))?;
            (
                app_data.join("fastty").join("config"),
                app_data.join("fastty").join("data"),
                local_app_data.join("fastty").join("state"),
                local_app_data.join("fastty").join("cache"),
            )
        } else if cfg!(target_os = "macos") {
            let app_support = dirs::config_dir()
                .ok_or_else(|| anyhow!("dirs::config_dir returned None (unsupported platform)"))?;
            let caches = dirs::cache_dir()
                .ok_or_else(|| anyhow!("dirs::cache_dir returned None (unsupported platform)"))?;
            (
                app_support.join("fastty"),
                app_support.join("fastty"),
                app_support.join("fastty"),
                caches.join("fastty"),
            )
        } else {
            // Linux and other Unix-like OS
            let config = dirs::config_dir()
                .ok_or_else(|| anyhow!("dirs::config_dir returned None (unsupported platform)"))?;
            let data = dirs::data_dir()
                .ok_or_else(|| anyhow!("dirs::data_dir returned None (unsupported platform)"))?;
            let state = dirs::state_dir()
                .unwrap_or_else(|| {
                    // Fall back if state_dir is None, usually ~/.local/state
                    dirs::data_dir()
                        .map(|d| d.parent().map(|p| p.join("state")).unwrap_or_else(|| d.clone()))
                        .unwrap_or_else(|| config.clone())
                });
            let cache = dirs::cache_dir()
                .ok_or_else(|| anyhow!("dirs::cache_dir returned None (unsupported platform)"))?;
            (
                config.join("fastty"),
                data.join("fastty"),
                state.join("fastty"),
                cache.join("fastty"),
            )
        };

        let dirs = Self {
            config_dir,
            data_dir,
            state_dir,
            cache_dir,
        };

        fs::create_dir_all(&dirs.config_dir)
            .with_context(|| format!("creating {}", dirs.config_dir.display()))?;
        fs::create_dir_all(&dirs.data_dir)
            .with_context(|| format!("creating {}", dirs.data_dir.display()))?;
        fs::create_dir_all(&dirs.state_dir)
            .with_context(|| format!("creating {}", dirs.state_dir.display()))?;
        fs::create_dir_all(&dirs.cache_dir)
            .with_context(|| format!("creating {}", dirs.cache_dir.display()))?;

        Ok(dirs)
    }
}

pub fn init() -> anyhow::Result<&'static FasttyDirs> {
    let dirs = FasttyDirs::new()?;
    Ok(FASTTY_DIRS.get_or_init(|| dirs))
}

pub fn get() -> &'static FasttyDirs {
    FASTTY_DIRS.get_or_init(|| FasttyDirs::new().expect("failed to initialize fastty paths"))
}

pub fn default_system_shell() -> String {
    #[cfg(target_os = "windows")]
    {
        if let Ok(shell) = std::env::var("SHELL") {
            if !shell.is_empty() && (std::path::Path::new(&shell).exists() || shell.ends_with(".exe")) {
                return shell;
            }
        }
        // Prefer PowerShell Core 7 (pwsh.exe) which starts significantly faster than Windows PowerShell 5.1
        if let Ok(path_var) = std::env::var("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let pwsh = dir.join("pwsh.exe");
                if pwsh.exists() {
                    return pwsh.to_string_lossy().into_owned();
                }
            }
        }
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            let pwsh = std::path::PathBuf::from(program_files)
                .join("PowerShell")
                .join("7")
                .join("pwsh.exe");
            if pwsh.exists() {
                return pwsh.to_string_lossy().into_owned();
            }
        }
        if let Ok(comspec) = std::env::var("COMSPEC") {
            if std::path::Path::new(&comspec).exists() {
                // If COMSPEC points to cmd.exe, prefer powershell if available, otherwise comspec
                if let Ok(sys_root) = std::env::var("SystemRoot") {
                    let ps_path = std::path::PathBuf::from(sys_root)
                        .join("System32")
                        .join("WindowsPowerShell")
                        .join("v1.0")
                        .join("powershell.exe");
                    if ps_path.exists() {
                        return ps_path.to_string_lossy().into_owned();
                    }
                }
                return comspec;
            }
        }
        "powershell.exe".to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(shell) = std::env::var("SHELL") {
            if !shell.is_empty() && std::path::Path::new(&shell).exists() {
                return shell;
            }
        }
        #[cfg(target_os = "macos")]
        {
            if std::path::Path::new("/bin/zsh").exists() {
                return "/bin/zsh".to_string();
            }
            if std::path::Path::new("/bin/bash").exists() {
                return "/bin/bash".to_string();
            }
            "/bin/sh".to_string()
        }
        #[cfg(not(target_os = "macos"))]
        {
            if std::path::Path::new("/bin/bash").exists() {
                return "/bin/bash".to_string();
            }
            if std::path::Path::new("/usr/bin/bash").exists() {
                return "/usr/bin/bash".to_string();
            }
            if std::path::Path::new("/bin/zsh").exists() {
                return "/bin/zsh".to_string();
            }
            if std::path::Path::new("/usr/bin/zsh").exists() {
                return "/usr/bin/zsh".to_string();
            }
            if std::path::Path::new("/bin/fish").exists() {
                return "/bin/fish".to_string();
            }
            if std::path::Path::new("/usr/bin/fish").exists() {
                return "/usr/bin/fish".to_string();
            }
            if std::path::Path::new("/bin/sh").exists() {
                return "/bin/sh".to_string();
            }
            "sh".to_string()
        }
    }
}
