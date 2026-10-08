# Report program status

Fastty accepts [OSC 7501](https://www.superlogical.com/rex/docs/build/program-status), revision 0.2, from programs in each terminal pane. Support is always active. Programs must send the reports.

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

Send `ESC ] 7501 ; ? ESC \`. Fastty writes the same sequence to the program's input. Programs must read and consume this reply. Do not send a query from an interactive shell without a reader for the reply.

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

## Background AI turns

Each conversation keeps its own active turn. Switch tabs or select another conversation to leave the agent at work in the background. Return to the conversation to see its response and tool activity.

If an agent needs permission, its conversation keeps the request pending. Open that conversation to approve or deny the request. With **AI Status** enabled, Fastty can notify you when a background conversation needs permission or finishes its turn.

Use **Stop** to cancel the visible conversation's turn. Clear or delete a conversation to cancel its turn. Close a tab to cancel its conversations. Closing Fastty cancels all active turns.

## Record lifetime

Each report replaces one record. The optional `id` key identifies a record or a child, such as `build/tests`. A clear report removes that record and its children. Records without an `app` inherit it from the nearest parent that has one.

The next shell prompt and process exit remove `working` and `blocked` records. They preserve `done` and `error` records. A full terminal reset removes all records. A screen change or soft reset preserves them.

Fastty validates the protocol limits before it applies a report. Each pane keeps at most 256 records and removes the least recently updated record when it needs room.
