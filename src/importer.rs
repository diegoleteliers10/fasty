//! Auto-detect and import configurations from other terminal emulators and muxers.

use std::path::{Path, PathBuf};
use crate::config::Config;
use crate::keybindings::KeybindingPreset;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalApp {
    Tmux,
    Ghostty,
    Alacritty,
    Kitty,
    WezTerm,
}

impl std::fmt::Display for ExternalApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tmux => write!(f, "tmux"),
            Self::Ghostty => write!(f, "Ghostty"),
            Self::Alacritty => write!(f, "Alacritty"),
            Self::Kitty => write!(f, "Kitty"),
            Self::WezTerm => write!(f, "WezTerm"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DetectedConfig {
    pub app: ExternalApp,
    pub path: PathBuf,
    pub label: String,
    pub details: String,
}

/// Scan standard system locations for third-party terminal and muxer configs.
pub fn detect_all_external_configs() -> Vec<DetectedConfig> {
    let mut detected = Vec::new();
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return detected,
    };

    // 1. Ghostty
    let ghostty_paths = [
        home.join(".config/ghostty/config"),
        home.join("Library/Application Support/com.mitchellh.ghostty/config"),
    ];
    for p in ghostty_paths {
        if p.exists() {
            detected.push(DetectedConfig {
                app: ExternalApp::Ghostty,
                path: p,
                label: "Ghostty Config".to_string(),
                details: "Theme, font, scrollback, and Ghostty keybinding preset".to_string(),
            });
            break;
        }
    }

    // 2. Alacritty
    let alacritty_paths = [
        home.join(".config/alacritty/alacritty.toml"),
        home.join(".config/alacritty/alacritty.yml"),
        home.join(".alacritty.toml"),
        home.join(".alacritty.yml"),
    ];
    for p in alacritty_paths {
        if p.exists() {
            detected.push(DetectedConfig {
                app: ExternalApp::Alacritty,
                path: p,
                label: "Alacritty Config".to_string(),
                details: "Font family, font size, and scrolling history".to_string(),
            });
            break;
        }
    }

    // 3. Kitty
    let kitty_paths = [
        home.join(".config/kitty/kitty.conf"),
        home.join(".kitty.conf"),
    ];
    for p in kitty_paths {
        if p.exists() {
            detected.push(DetectedConfig {
                app: ExternalApp::Kitty,
                path: p,
                label: "Kitty Config".to_string(),
                details: "Font family, font size, scrollback, and opacity".to_string(),
            });
            break;
        }
    }

    // 4. Tmux
    let tmux_paths = [
        home.join(".tmux.conf"),
        home.join(".config/tmux/tmux.conf"),
    ];
    for p in tmux_paths {
        if p.exists() {
            detected.push(DetectedConfig {
                app: ExternalApp::Tmux,
                path: p,
                label: "tmux Config".to_string(),
                details: "Adopt tmux keybinding preset and split shortcuts".to_string(),
            });
            break;
        }
    }

    // 5. WezTerm
    let wezterm_paths = [
        home.join(".config/wezterm/wezterm.lua"),
        home.join(".wezterm.lua"),
    ];
    for p in wezterm_paths {
        if p.exists() {
            detected.push(DetectedConfig {
                app: ExternalApp::WezTerm,
                path: p,
                label: "WezTerm Config".to_string(),
                details: "Font family, font size, and opacity".to_string(),
            });
            break;
        }
    }

    detected
}

/// Import settings from the specified detected config and update fastty config.
pub fn import_external_config(app: ExternalApp, path: &Path, cfg: &mut Config) -> anyhow::Result<String> {
    let content = std::fs::read_to_string(path)?;
    let mut imported_items = Vec::new();

    match app {
        ExternalApp::Ghostty => {
            cfg.keybinding_preset = Some(KeybindingPreset::Ghostty);
            imported_items.push("Preset: Ghostty keybindings".to_string());

            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('#') || trimmed.is_empty() {
                    continue;
                }
                if let Some((k, v)) = trimmed.split_once('=') {
                    let key = k.trim();
                    let val = v.trim().trim_matches('"').trim_matches('\'');
                    match key {
                        "font-family" => {
                            if !val.is_empty() {
                                cfg.font.family = val.to_string();
                                imported_items.push(format!("Font family: {val}"));
                            }
                        }
                        "font-size" => {
                            if let Ok(size) = val.parse::<f32>() {
                                cfg.font.size = size.clamp(6.0, 72.0);
                                imported_items.push(format!("Font size: {size}"));
                            }
                        }
                        "theme" => {
                            if !val.is_empty() {
                                let mapped_theme = match val.to_lowercase().as_str() {
                                    s if s.contains("catppuccin") => "catppuccin",
                                    s if s.contains("one dark") || s.contains("onedark") => "one-dark",
                                    s if s.contains("solarized") => "solarized-dark",
                                    _ => "default",
                                };
                                cfg.theme = Some(mapped_theme.to_string());
                                imported_items.push(format!("Theme: {mapped_theme}"));
                            }
                        }
                        "background-opacity" => {
                            if let Ok(op) = val.parse::<f32>() {
                                cfg.opacity = op.clamp(0.1, 1.0);
                                imported_items.push(format!("Opacity: {op}"));
                            }
                        }
                        "mouse-scroll-multiplier" | "scrollback-limit" => {
                            if let Ok(lim) = val.parse::<usize>() {
                                cfg.scrollback = lim.clamp(500, 100_000);
                                imported_items.push(format!("Scrollback: {lim} lines"));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        ExternalApp::Tmux => {
            cfg.keybinding_preset = Some(KeybindingPreset::Tmux);
            imported_items.push("Preset: tmux keybindings".to_string());
        }
        ExternalApp::Alacritty => {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('#') || trimmed.is_empty() {
                    continue;
                }
                if let Some((k, v)) = trimmed.split_once('=') {
                    let key = k.trim();
                    let val = v.trim().trim_matches('"').trim_matches('\'');
                    if key == "family" && !val.is_empty() {
                        cfg.font.family = val.to_string();
                        imported_items.push(format!("Font family: {val}"));
                    } else if key == "size" {
                        if let Ok(size) = val.parse::<f32>() {
                            cfg.font.size = size.clamp(6.0, 72.0);
                            imported_items.push(format!("Font size: {size}"));
                        }
                    } else if key == "history" || key == "scrolling.history" {
                        if let Ok(hist) = val.parse::<usize>() {
                            cfg.scrollback = hist.clamp(500, 100_000);
                            imported_items.push(format!("Scrollback: {hist} lines"));
                        }
                    }
                }
            }
        }
        ExternalApp::Kitty => {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('#') || trimmed.is_empty() {
                    continue;
                }
                if let Some((k, v)) = trimmed.split_once(char::is_whitespace) {
                    let key = k.trim();
                    let val = v.trim().trim_matches('"').trim_matches('\'');
                    match key {
                        "font_family" => {
                            if !val.is_empty() {
                                cfg.font.family = val.to_string();
                                imported_items.push(format!("Font family: {val}"));
                            }
                        }
                        "font_size" => {
                            if let Ok(size) = val.parse::<f32>() {
                                cfg.font.size = size.clamp(6.0, 72.0);
                                imported_items.push(format!("Font size: {size}"));
                            }
                        }
                        "background_opacity" => {
                            if let Ok(op) = val.parse::<f32>() {
                                cfg.opacity = op.clamp(0.1, 1.0);
                                imported_items.push(format!("Opacity: {op}"));
                            }
                        }
                        "scrollback_lines" => {
                            if let Ok(lines) = val.parse::<usize>() {
                                cfg.scrollback = lines.clamp(500, 100_000);
                                imported_items.push(format!("Scrollback: {lines} lines"));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        ExternalApp::WezTerm => {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.contains("font_size") {
                    if let Some((_, v)) = trimmed.split_once('=') {
                        let val = v.trim().trim_matches(',').trim();
                        if let Ok(size) = val.parse::<f32>() {
                            cfg.font.size = size.clamp(6.0, 72.0);
                            imported_items.push(format!("Font size: {size}"));
                        }
                    }
                }
                if trimmed.contains("window_background_opacity") {
                    if let Some((_, v)) = trimmed.split_once('=') {
                        let val = v.trim().trim_matches(',').trim();
                        if let Ok(op) = val.parse::<f32>() {
                            cfg.opacity = op.clamp(0.1, 1.0);
                            imported_items.push(format!("Opacity: {op}"));
                        }
                    }
                }
            }
        }
    }

    // Persist changes
    cfg.save_default()?;

    if imported_items.is_empty() {
        Ok(format!("Imported default preset for {app}"))
    } else {
        Ok(format!("Imported from {app}: {}", imported_items.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ghostty_import() {
        let mut cfg = Config::default();
        let ghostty_sample = r#"
# Ghostty config
font-family = "JetBrains Mono"
font-size = 16
theme = "Catppuccin Mocha"
background-opacity = 0.92
scrollback-limit = 25000
"#;
        let tmp = std::env::temp_dir().join("test_ghostty_config");
        std::fs::write(&tmp, ghostty_sample).unwrap();

        let res = import_external_config(ExternalApp::Ghostty, &tmp, &mut cfg);
        assert!(res.is_ok());
        assert_eq!(cfg.font.family, "JetBrains Mono");
        assert_eq!(cfg.font.size, 16.0);
        assert_eq!(cfg.theme.as_deref(), Some("catppuccin"));
        assert_eq!(cfg.opacity, 0.92);
        assert_eq!(cfg.scrollback, 25000);
        assert_eq!(cfg.keybinding_preset, Some(KeybindingPreset::Ghostty));

        let _ = std::fs::remove_file(&tmp);
    }
}
