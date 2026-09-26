//! fastty library - GPUI terminal emulator
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
pub mod updater;
pub mod whats_new;
pub mod widgets;
pub mod server;
