use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context, anyhow};

pub struct FastyDirs {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
}

pub static FASTY_DIRS: OnceLock<FastyDirs> = OnceLock::new();

impl FastyDirs {
    pub fn new() -> anyhow::Result<Self> {
        let (config_dir, data_dir, state_dir, cache_dir) = if cfg!(target_os = "windows") {
            let app_data = dirs::config_dir()
                .ok_or_else(|| anyhow!("dirs::config_dir returned None (unsupported platform)"))?;
            let local_app_data = dirs::data_local_dir()
                .ok_or_else(|| anyhow!("dirs::data_local_dir returned None (unsupported platform)"))?;
            (
                app_data.join("fasty").join("config"),
                app_data.join("fasty").join("data"),
                local_app_data.join("fasty").join("state"),
                local_app_data.join("fasty").join("cache"),
            )
        } else if cfg!(target_os = "macos") {
            let app_support = dirs::config_dir()
                .ok_or_else(|| anyhow!("dirs::config_dir returned None (unsupported platform)"))?;
            let caches = dirs::cache_dir()
                .ok_or_else(|| anyhow!("dirs::cache_dir returned None (unsupported platform)"))?;
            (
                app_support.join("fasty"),
                app_support.join("fasty"),
                app_support.join("fasty"),
                caches.join("fasty"),
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
                config.join("fasty"),
                data.join("fasty"),
                state.join("fasty"),
                cache.join("fasty"),
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

pub fn init() -> anyhow::Result<&'static FastyDirs> {
    let dirs = FastyDirs::new()?;
    Ok(FASTY_DIRS.get_or_init(|| dirs))
}

pub fn get() -> &'static FastyDirs {
    FASTY_DIRS.get().expect("FASTY_DIRS not initialized; call paths::init() first")
}
