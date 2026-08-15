//! fastty library - GPUI terminal emulator

pub mod config;
pub mod event_listener;
pub mod git;
pub mod keybindings;
pub mod paths;
pub mod selection_classifier;
pub mod session;
pub mod snippets;
pub mod ssh;
pub mod parser;
pub mod terminal_state;
pub mod ui;
pub mod widgets;
#[cfg(target_os = "macos")]
pub mod font_discovery_macos;