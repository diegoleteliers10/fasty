# Report program status

Fastty accepts [OSC 7501](https://www.superlogical.com/rex/docs/build/program-status), revision 0.4, from programs in each terminal pane. Support is always active. Programs must send the reports.

## Send a report

Use a terminal pane in Fastty:

```sh
printf '\033]7501;state=working:app=example:progress=30\033\\'
printf '\033]7501;state=done:app=example\033\\'
```

The tab and pane show the current status. A completion indicator stays until you click or type in that pane.

For a permission request or a failure, send:

```sh
printf '\033]7501;state=blocked:kind=permission:app=example\033\\'
printf '\033]7501;state=error:app=example\033\\'
```

To include a message, encode its UTF-8 text as standard base64:

```sh
message=$(printf '%s' 'Approve deployment' | base64 | tr -d '\n')
printf '\033]7501;state=blocked:kind=permission:app=deploy:msg=%s\033\\' "$message"
```

To remove all records in the pane, send:

```sh
printf '\033]7501;state=clear\033\\'
```

## Check support

Programs can send `ESC ] 7501 ; ? ESC \` to check support. Fastty replies with the states and kinds it accepts. A program can use the reply to select a supported state. If no reply arrives, treat OSC 7501 as unsupported.

Fastty supports `idle`, `working`, `done`, `blocked`, and `error` states. It supports `permission`, `question`, and `auth` kinds.

## Set notifications

In **Settings > General > Notifications**, use **AI Status** and **Program Status**. Both settings default to `true`.

The equivalent root keys in `config.toml` are:

```toml
notify_on_ai_status = true
notify_on_program_status = true
```

Fastty sends native notifications for a completion, a request for input, or a failure when the source is in the background. Notifications require the operating system to allow Fastty notifications. Each terminal pane and the AI panel can send at most one notification every five seconds. Progress updates do not send notifications.

On macOS, install and open `Fastty.app` before you use notifications. Fastty uses its registered application identifier. A bare development binary cannot register the application. If delivery fails, Fastty writes the error to stderr. Check **System Settings > Notifications > Fastty** and allow notifications.

The AI panel uses provider events for its notifications. It supports ACP and HTTP providers without OSC reports.

## Floating cards

Fastty shows the same events as cards in the window, in the bottom right of the terminal area. A card shows the program mark, the source name, the state, the time, the message, and the actions you can take.

| Card | Actions | Time | Bar |
| --- | --- | --- | --- |
| Done | None | Countdown | Yes |
| Failed | **Open tab**, **Dismiss** | Countdown | No |
| Needs you | **Allow**, **Review** | Elapsed | No |
| Working | None | Elapsed | No |

A done card only reports a result. It has no actions, it shows the time left, and a bar counts down until it clears. A working card only reports that the source is busy, so it has no action and no bar. The other cards wait for the source or for you, so they show the elapsed time and no bar. The actions depend on the source: a permission request offers **Allow** and **Review**, and a blocked program offers **Open tab** and **Dismiss**.

**Open tab** selects the tab of the event. **Review** opens the AI panel with the request. **Dismiss** removes the card. Fastty hides a working card while its pane is on screen, because the card would cover the terminal it describes.

A dismissed result does not return for the same state. A new state for the same source shows a new card. A retried failure and a finished command are new events each time, so they always show a card.

Fastty keeps at most three cards, and a card that needs you is never removed to make room for a result. A done card clears itself three seconds after the work ends. A failed card counts down from 60 seconds, but its actions stay available until it clears. A card that reports work or a request for input stays until the source changes state or you act on it. The cards follow the **AI Status** and **Program Status** settings. A command card also needs `notify_on_command_finish = true` and a duration of two seconds or more, unless the command fails.

## OSC notifications

A program can ask for a native notification directly. Fastty reads these sequences:

| Sequence | Shape |
| --- | --- |
| OSC 9 | `ESC ] 9 ; MESSAGE ESC \` |
| OSC 9 with a title | `ESC ] 9 ; TITLE ; BODY ESC \` |
| OSC 99 | Kitty notification protocol, including `i=` fragments |
| OSC 777 | `ESC ] 777 ; notify ; TITLE ; BODY ESC \` |

Fastty sends these notifications even when **Program Status** is off. While a pane reports `working` or `blocked`, Fastty drops the OSC notifications of that pane, because the status report already covers the result.

## Background AI turns

Each conversation keeps its own active turn. Switch tabs or select another conversation to leave the agent at work in the background. Return to the conversation to see its response and tool activity.

If an agent needs permission, its conversation keeps the request pending. Open that conversation to approve or deny the request. With **AI Status** enabled, Fastty can notify you when a background conversation needs permission or finishes its turn.

Use **Stop** to cancel the visible conversation's turn. Clear or delete a conversation to cancel its turn. Close a tab to cancel its conversations. Closing Fastty cancels all active turns.

## Tab identity

Each tab shows the mark of the program that owns the pane. When the foreground process is a shell, the tab shows the green terminal mark. Otherwise Fastty reads the `app` field of the live pane records first, then the process name. A record that finished no longer owns the tab, so the next program in that pane sets the mark.

- A shell shows the green terminal mark.
- A known agent shows its brand mark: Claude, OpenCode, Codex, Gemini, GitHub Copilot, Cursor, Amp, Qwen, Cline, Windsurf, Mistral, Kimi, LM Studio, Pi, Antigravity, or Hermes.
- An agent without a bundled mark shows the generic agent mark.
- A language or runtime shows its mark: Node.js, Bun, Deno, npm, pnpm, Yarn, Python, Rust, Go, Ruby, PHP, Java, Kotlin, Swift, .NET, Lua, Elixir, Zig, Dart, Flutter, Haskell, Clojure, or R.
- A tool shows its mark: Docker, Terraform, Kubernetes, Git, Neovim, Vim, tmux, Redis, SQLite, PostgreSQL, MongoDB, CMake, Gradle, Ollama, or Hugging Face.
- Any other program shows the mark of its tool group.

When a report names a program in the family of a bundled mark, the tab uses that mark, so `claude-code` shows the Claude mark and `/usr/local/bin/opencode` shows the OpenCode mark. When the pane goes back to a shell prompt, the tab shows the terminal mark again.

While a record reports `working`, the tab shows a three-dot loader beside the title. The loader stops when the record leaves `working`. A finished or failed record shows a small status mark until you open the tab, and a blocked record shows one until the program continues.

## Record lifetime

Each report replaces one record. The optional `id` key identifies a record or a child, such as `build/tests`. A clear report removes that record and its children. Records without an `app` inherit it from the nearest parent that has one.

The next shell prompt and process exit remove `working` and `blocked` records. They preserve `done` and `error` records. A full terminal reset removes all records. A screen change or soft reset preserves them.

Fastty validates the protocol limits before it applies a report. Each pane keeps at most 256 records and removes the least recently updated record when it needs room.
