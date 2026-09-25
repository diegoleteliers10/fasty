# Changelog

Notable changes per Fastty release. The newest section ships inside the app and
appears in the "What's new" dialog after an update.

## 0.14.0 - 2026-09-25

- Editable keyboard shortcuts in Settings: click any shortcut and press the
  keys. All actions across Tabs, Panes, Search, Tools, and Application are
  customizable on macOS, Linux, and Windows, with per-preset defaults
  (Default, Ghostty, tmux, iTerm2) applied initially.
- Shortcut conflicts ask before reassigning, with per-action reset, unbind,
  and reset-all back to the preset defaults.
- Upgraded terminal engine to alacritty_terminal 0.26 with rustix 1.1,
  notify 8, notify-debouncer-mini 0.7, and criterion 0.8. The unified
  dependency tree drops the old rustix 0.38 and vte 0.13 copies.
- Release pipeline actions updated: checkout v7, upload-artifact v7,
  download-artifact v8, action-gh-release v3.

## 0.13.2 - 2026-09-25

- Update checks and downloads no longer depend on the system `curl`. The
  updater now uses the built-in HTTP client, so the update button appears
  reliably on Windows, including behind `HTTP_PROXY`/`HTTPS_PROXY` proxies.
- The "What's new" dialog appears after an upgrade from any version older
  than 0.13.0, not only after a previous 0.13.x run recorded itself. This
  fixes the dialog staying silent on macOS upgrades that skipped releases.

## 0.13.1 - 2026-09-22

- Fixed the web client failing to restore the terminal screen after a
  reconnect.
- Upgraded the WebAssembly terminal engine to vte 0.15 and wasm-bindgen
  0.2.128.
- Optimized the WebAssembly binary with bulk-memory operations.
- The release pipeline now rebuilds the web client before every release
  build, so the gateway always serves current assets.
- Dependabot keeps dependencies current with weekly update PRs.

## 0.13.0 - 2026-09-22

- "What's new" dialog after each update, with per-version notes and a link to
  the full changelog. Shows once per version on macOS, Windows, and Linux.
- Settings window opens instantly: font enumeration and Ollama model detection
  now run in the background, with the font list cached per process.
- Font combobox and Ollama model list fill in when background discovery lands.

## 0.12.0 - 2026-09-21

- Performance: cached git status, subprocess timeouts, slow-poll backoff, and
  reduced network traffic for status widgets.
- Settings show a stale badge when the config file changes outside the app.
- Command palette live preview for themes, font size, and layouts.
- Pane zoom with a context-menu entry.
- Nerd Font auto-fallback when the configured font lacks glyphs.
- Snippet picker and pull-request picker with `gh` checkout, approve, and merge.
- Config-error banner and missing-tool hints.

## 0.11.1 - 2026-09-15

- Split pane routing: new panes open where the active pane sits.
- Font combobox with search in Settings.
- Token usage hovercard in the AI sidebar.

## 0.11.0 - 2026-09-13

- Agent edit review: inspect and approve file edits before they apply.
- New selection engine for more precise text selection.
- Panel polish across the AI sidebar and status bar.

## 0.10.1 - 2026-09-12

- Agent `run_command` tool now inherits the user environment.

## 0.10.0 - 2026-09-10

- Multimodal images: paste images into the AI chat.
- PDF support for the AI assistant.
- Markdown rendering for AI responses.
- Daemon release notes overlay on the web dashboard.

## 0.9.0 - 2026-09-07

- AI agent panel with tool use.
- `fastty ask` CLI for one-shot questions.
- Editor-grade file editing for the agent.
