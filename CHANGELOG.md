# Changelog

Notable changes per Fastty release. The newest section ships inside the app and
appears in the "What's new" dialog after an update.

## 0.15.0 - 2026-09-30

### Terminal

- Fix `gh auth login` rendering `\;1R` with an empty menu: fastty answered OSC 11 twice, once from its own scanner and once from the terminal library. The manual path is gone and one answerer remains.

### Mission Control

- Give each card its own icon: Copy, Paste, the four split directions and Duplicate Tab shared a `+` sign, which carried no information next to each other.
- Fill the window edge to edge with a small margin, enlarge the cells, and recompute the grid on resize.
- Fix arrow navigation on Windows and everywhere else: Left and Right moved through a flat list and wrapped. They now move in two dimensions and stop at the edges.

### Status bar

- Model icons as SVG instead of emoji glyphs, so the branch marker, the pull request counts and every status indicator draw the same on macOS, Windows and Linux.
- Replace the remaining status glyphs (review pending, run in progress, ahead, behind, staged) with icons.
- Remove the tab count badge from the vertical tab sidebar.

### AI panel

- Move the `@` file mention menu with the arrow keys. Up and Down wrap, Home and End jump, Page Up and Page Down stop at the ends, and Enter or Tab commits the highlighted row.
- Attach a file dropped on the AI panel instead of pasting its path into the shell. The panel highlights itself while a file is over it.
- The paperclip opens the system file dialog.

### Settings

- Redesign the hotkey display after corvo: each key is its own keycap, and non-macOS shows `Ctrl`, `Alt` and `Super` instead of the macOS symbols.

### macOS updates

- Keep a stable signing identity across releases so an update does not look like a different app. Run `tools/setup-signing-cert.sh` once, then add the three `FASTTY_CERTIFICATE_*` repository secrets.
- Stop re-signing ad hoc in the install script and the Homebrew cask, which discarded the release signature on every install.

## 0.14.9 - 2026-09-28

- Fix update dialog never appearing: releases without a matching OS/arch asset now show the changelog modal with a manual download option instead of failing silently.
- Update modal opens automatically when a new release is found, with Later, Skip this version, and Update Now actions (or Open download page when self-update is blocked).
- Add manual "Check for Updates" command in the command palette and a Check for updates button with Stable/Beta channel selector in Settings General.
- Add Skip this version support so dismissed releases stop being offered on all three OSes.
- Fix console window flashing on Windows when opening Settings: font detection, Ollama detection, and update staging now run with CREATE_NO_WINDOW.

## 0.14.8 - 2026-09-28

- Organize tabs in a grid layout with a maximum of 4 tabs per row in Mission Control.
- Adjust preview card size to 300px × 210px for balanced screen distribution.
- Replace folder and git branch emojis with vector icons in preview card footers.
- Normalize tab characters to spaces in screen previews to preserve terminal column alignment.
- Update vertical keyboard navigation to step by row column count.

## 0.14.7 - 2026-09-28

- Show changelog modal with release notes when an update is ready before restart.
- Add user choice to apply the update immediately or dismiss with "Later".
- Retain existing user configuration and themes across updates via lenient document recovery.
- Prioritize platform standard configuration paths over legacy paths and isolate macOS launch environment.
- Save active sessions and user configuration before update reboot.

## 0.14.6 - 2026-09-28

- Cross-platform atomic auto-updater following the Tinycast pattern.
- Channel support (Stable and Beta) with 24-hour freshness cache to respect GitHub API rate limits.
- Background streaming download to user cache directory with incremental SHA-256 verification.
- Safe volume-local staging and detached waiter process swap after parent process terminates, preventing file-lock errors and in-place crashes.
- Automatic startup cleanup of leftover `.old` and `.staging` files.
- macOS App Translocation detection, Windows ProgramFiles detection, and Linux package-manager detection.

## 0.14.5 - 2026-09-27

- Single-cell isolation and centered rendering for keyboard modifiers (⌘, ⌥, ⌃, ⇧) and arrows to prevent character overlap across all monospace fonts.
- Restored 100% native font size for keyboard symbols and arrows with dynamic slot fitting for wide glyph fonts.
- Multi-click paragraph selection (4 clicks) and soft-wrapped logical line selection (3 clicks).
- Tab alignment normalization: tabs expanded to column-aligned spaces in grid spans and clipboard copy.

## 0.14.4 - 2026-09-27

- Terminal context menu opens strictly on right-click (and trackpad two-finger tap).
- Double-click and triple-click now only select words and lines without opening the context menu.

## 0.14.3 - 2026-09-26

- File Path Picker keybinding is now exclusively `Ctrl+Cmd+,` (`⌃⌘,` / `Ctrl+Super+,`).
- Shortcut capture in Settings rejects bare keys without modifiers (such as bare Return or Backspace) and cancels capture on bare Return or Escape.
- Rebinding an action now replaces existing combinations cleanly instead of accumulating multiple shortcuts.

## 0.14.2 - 2026-09-26

- Keybindings cleanup: each action now has a single deterministic shortcut
  across all presets (Default, Ghostty, Tmux, ITerm2) to prevent overlapping
  and shortcut conflicts.
- File Path Picker shortcut is standardized to `Ctrl+Shift+,` (`⌃⇧,` on macOS).
  Removed duplicate `Ctrl+Alt+,` keybinding.
- Removed duplicate base shortcuts for copy, paste, font adjustments, and tab navigation.

## 0.14.1 - 2026-09-25

- File Path Picker (`Ctrl+Shift+,`): search workspace files and insert the
  path straight into the terminal input line. Bounded index (20k files,
  150 ms) skips `node_modules`, `target`, `.git` and friends; fuzzy matching
  ranks filename and segment hits first.
- The picker is a dropdown anchored at the cursor line: it opens below the
  line and flips above it near the screen edge, like the context menu.
- Shift + punctuation shortcuts now resolve on macOS (GPUI folds Shift into
  the key). This also fixes `⌘⇧]` / `⌘⇧[` tab switching.
- Layout-proof alias `Ctrl+Alt+,` for the picker on non-US keyboards, where
  Shift+comma produces a different character.
- Importing a terminal config (Ghostty, Warp, ...) now applies and persists
  immediately; imported presets no longer leak into later settings saves.

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
