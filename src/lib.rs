//! fastty library - GPUI terminal emulator
// The `#[test]` attribute expands recursively, once per test in a module. A
// module with many tests needs more headroom than the default limit of 128.
#![recursion_limit = "512"]

pub mod ai;
pub mod cli;

pub mod config;
pub mod daemon;
pub mod daemon_client;
pub mod event_listener;
pub mod file_search;
#[cfg(target_os = "macos")]
pub mod font_discovery_macos;
pub mod gateway;
pub mod git;
pub mod importer;
pub mod keybindings;
pub mod mcp;
pub mod pane_tree;
pub mod parser;
pub mod paste;
pub mod paths;
pub mod selection_classifier;
pub mod session;
pub mod session_manager;
pub mod snippets;
pub mod ssh;
pub mod terminal_state;
pub mod ui;
pub mod universal_picker;
pub mod updater;
pub mod whats_new;
pub mod widgets;
pub mod server;
