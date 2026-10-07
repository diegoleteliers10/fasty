# Changelog

Notable changes per Fastty release. The newest section ships inside the app and
appears in the "What's new" dialog after an update.

## 0.18.0 - 2026-10-07

### OpenCode ACP

- Add OpenCode as a native AI provider over ACP. Fastty starts and manages the OpenCode agent, discovers models and reasoning variants, and streams agent activity and permission requests in the AI panel.
- Keep an OpenCode session for each AI conversation. Restore its session when you return to that conversation.
- Show discovered OpenCode skills and commands in the AI panel's `@` menu.

### AI conversations

- Save AI conversations and context-window settings per tab. Switch between saved conversations, start a new conversation with `+`, clear the active conversation with the reset button, and delete conversations from the history menu.
- Show an empty-state message in the history menu when the tab has no conversations.
- Update the context window display when its setting changes.

### Menus

- Close menus when you click outside them or press Escape.
- Draw AI menus and dropdowns above panel content.

## 0.17.0 - 2026-10-03

### Agent-Driven Layout Control (MCP)

- Five new daemon operations, exposed as MCP tools, that let an agent drive the fastty window itself: `fastty_layout` (the full window structure — every tab with its panes, ids, titles, cwds and sizes), `fastty_split_pane` (create a new pane beside any visible pane in the same tab, running the shell or a custom command; returns its session id), `fastty_resize_pane` (move a pane divider by a fraction of the split axis), `fastty_focus_pane` (bring the pane's tab to the front and activate it, so the user sees what the agent touches), and `fastty_resize_window` (resize the window to approximately cols × rows). CLI: `fastty split <pane-id> --direction right`. Requests are answered over a correlated daemon→GUI channel with a 5-second timeout; with no window running they fail fast with `no_gui` instead of hanging.
- Sessions spawned through the daemon now open as visible tabs. `fastty_spawn_session` defaults to `open: true` (and `fastty spawn --open` on the CLI), so a session an agent creates appears in the fastty window: the pane adopts the same `TerminalState` — same id — meaning the agent types into the tab the user is watching. Event wiring is re-attached on adoption, so title/cwd changes and shell exit close the tab exactly like a hand-opened one. `fastty_run_command` stays headless.
- `close { force: true }` (and `fastty_close_session` with `force`, `fastty close <id> --force`) closes GUI tabs too: the process group is killed exactly as when closing the tab in the window, and the tab disappears. Without `force`, GUI sessions still answer `not_closable` — those tabs belong to the user — but the error now says what to do.
- `spawned` responses report `opened: bool` (and `fastty spawn --json` prints it), so callers can tell a visible tab from a headless session when no window is running.
- Session-resize semantics clarified: `resize` only affects headless grids — GUI panes always follow the window; dividers are `resize_pane`'s job and the window itself is `resize_window`'s.

### Contrast Correction Fixes

- Fixed invisible text under selection. Selected cells now correct explicit program colors against the accent selection tint actually painted under the glyphs (shared `SELECTION_OVERLAY_ALPHA`), not the plain cell background — dim TUI grays (Claude Code, opencode) that read on the background used to sink to ~1.6:1 under the tint and vanish; they now re-correct in place.
- Fixed the Oklab bisection converging to the fully-corrected extreme: it brightened every failing gray to near-white (or near-black on light themes), flattening dim/bright hierarchy in TUIs. Corrections are now minimal — the smallest lightness move that restores the target contrast — so dim text stays dim.
- The selection's per-line column math is shared between the painter and the span builder (`selection_columns_on_line`), so both always agree on what "selected" means.

## 0.16.0 - 2026-10-03

### MCP Server

- New `fastty mcp` subcommand: a local MCP (Model Context Protocol) server over stdio that connects AI agents (Claude Code, Codex, Cursor, …) to fastty's daemon. Seven tools: list/spawn/write/read/resize/close sessions plus `fastty_run_command`, which runs a command in a throwaway headless session, waits for exit, and returns the terminal's real rendered output (last 300 lines of scrollback + screen). When fastty's GUI is open the agents share its live sessions; when it isn't, the MCP process embeds its own daemon — agents work headless with no app running. Registration one-liners in `docs/mcp.md` (`claude mcp add fastty -- fastty mcp`); `fastty --help` lists the subcommand. Unix only (daemon sockets).

### Binary Snapshots (FST1 v2)

- The binary snapshot format grows to v2: it now carries the full terminal state — scrollback history (oldest first), screen cells, cursor position, and cursor visibility — compressed with Deflate. Measured on a 2000-row × 200-col grid with realistic shell output: 6.2MB of raw cells compress to ~35KB (~184x), sub-100ms encode/decode even in debug builds. The web client now restores your scrollback on reconnect, not just the visible screen.
- New native `restore_binary_snapshot` on TerminalState: replays a decoded snapshot into the alacritty grid through the VT parser (history grows through alacritty's own scroll machinery, cursor lands where the snapshot says, cursor visibility restored). This powers session content restore below and is the groundwork for persistent sessions.
- v1 payloads keep decoding everywhere (the history field rides in previously-reserved header bytes that v1 left zeroed); wide-char and wide-spacer cells are now preserved exactly.
- Fix: binary snapshots hardcoded Catppuccin Mocha's palette for program-requested named/indexed colors — web clients got wrong colors under any other theme. Named and 0-15 indexed colors now resolve against the active theme at snapshot time.

### Session Content Restore

- "Session: Save Workspace" now freezes each pane's terminal content (scrollback + screen + cursor) as a compressed binary snapshot alongside the layout and cwd; "Session: Restore Workspace" respawns the shells and replays the saved content — output comes back, not just the directory. New shells print at the restored cursor, tmux `respawn-pane` style. Old session JSON files (without snapshots) keep loading untouched.

### Themes

- The built-in suite grows from 5 to 20 themes: Catppuccin Frappé/Macchiato/Latte, Dracula, Nord, Tokyo Night (+ Storm), Gruvbox Dark/Light, Rosé Pine (+ Moon/Dawn), One Light, Solarized Light, and Kanagawa Wave — including six light themes, fastty's first. Palette families live as compact `ThemeSpec`s and chrome (surfaces, borders, hover, selection) is derived from the background's lightness, so light themes get light chrome; this also fixes user JSON themes with light palettes, which previously inherited the dark default chrome.
- One registry (`THEME_REGISTRY`) now feeds the command palette, palette live preview, Settings theme cards, and labels — switching themes via `⌘P` previews each of the 20 entries live.
- Settings → Appearance swaps the wrapping grid of theme cards for a dropdown: the trigger shows the active theme's badge (color dots + label), and the list offers every theme as the same badge row — checkmark on the active one, keyboard navigation (↑/↓ to move, Enter to apply, Esc to close), hover-to-highlight, and scroll for the full 20. The list renders as a window-level overlay anchored to the trigger (flipping above it when there's more room), so it floats above every section and can't be clipped by the group cards.
- The config importer maps theme names from all new families (e.g. a Ghostty `theme = tokyo-night-storm` now lands on the matching fastty theme instead of the default).

### AI Permissions

- Learned auto-allow: every manual approval of a `run_command` family (`git status`, `cargo build`, …) is counted; after 3 approvals the confirmation card offers to auto-allow that family from then on. The suggestion is per family — subcommand-aware for git/cargo/npm/docker-style tools so `git status` never unlocks `git push` — and shells, interpreters, and sudo-style launchers are never suggested. The hardcoded danger layer still runs first, so a learned rule can never unlock `rm -rf /`-class commands. Rules persist across restarts in `state_dir/ai_learned_allow.json`; disable with `[ai] learned_allow = false`.

### CLI (daemon control)

- New subcommands over the local daemon: `fastty spawn [--cwd DIR] [--cols N] [--rows N] [-- COMMAND...]` starts a headless session and prints its id, `fastty write <id> [--enter] <TEXT...>` types into it, `fastty resize <id> <cols> <rows>`, `fastty close <id>`, and `fastty list` (alias of `sessions`). `write`/`resize` opt into a new `done` ack so exit codes are truthful; the ack is a protocol addition that older clients simply never request. Examples in `fastty --help` and `docs/daemon-protocol.md`.

### Universal Insert

- The file path picker (`⌃⌘,` / `Ctrl+Super+,`) is now a multi-source picker: one anchored fuzzy search inserts workspace files, ssh hosts from `~/.ssh/config`, local git branches, snippet bodies, and running docker containers straight into the focused pane's prompt. Files insert as quoted paths, hosts/branches/containers pre-fill their command (`ssh … `, `git checkout … `, `docker exec -it … `), snippets insert their expanded body. Docker containers are fetched in the background so a slow or missing daemon never blocks the picker.
- Insertion now targets the focused split pane instead of the tab's main terminal, and always goes through bracketed paste when the shell supports it.
- The empty picker no longer says "no working directory" when the tab has no cwd: ssh, snippet, and docker sources are still offered.
- Fix: the placeholder and long queries no longer overflow the popup block; the input row clips them.

### Automatic Contrast Correction

- Programs bias to dark mode: explicit truecolor/256-color values designed against dark backgrounds turn invisible on light themes (and vice versa). When such a color's WCAG contrast against what it sits on drops below 3:1, fastty now binary-searches its Oklab lightness away from the background — hue and chroma preserved — until the pair reads (~4.5:1). Backgrounds are corrected against the theme foreground the same way. Theme-owned named ANSI colors are never touched, and corrections are cached per (color, background) pair so the render loop pays one hash lookup per colored cell.
- Toggle in Settings → Terminal Behavior, or `contrast_correction` in `fastty.toml` (default on). The palette/search previews are corrected too.

### Kitty Clipboard Protocol (OSC 5522)

- fastty answers the `CSI ? 5522 $ p` probe and speaks kitty's multi-format clipboard protocol: chunked multi-MIME writes, and reads that return `text/plain` and images (as PNG) in ≤4KB DATA packets — the mechanism tools like Claude Code use to paste images into the terminal. Writes land on the system clipboard through arboard (text or image).
- Reads are permission-gated by `clipboard_read` in `fastty.toml` (default on; denied reads reply `EPERM`). Writes are always allowed. The classic text-only OSC 52 write path keeps working as before.

### Migration

- Importing another terminal's config now celebrates: a theme-colored confetti burst plays over the settings window. Software should be fun.

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
