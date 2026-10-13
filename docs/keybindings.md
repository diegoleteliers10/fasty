# Key bindings

Fastty reads single-key actions and multi-key sequences from `[keybindings]` in `fastty.toml`.

```toml
[keybindings]
"ctrl+b>n" = "new_tab"
"ctrl+b>c" = "close_tab"
```

Press each key in order within 900 ms. Press `Esc` to cancel. A key that starts a sequence waits for the next key. Fastty reports conflicts in its log. A sequence takes priority over a single-key action with the same prefix. If one sequence is a prefix of another, the shorter sequence takes priority.

## Key modes

Define a named mode under `[key_modes.<name>]`. The `entry` key starts the mode. An exclusive mode consumes keys that have no binding. `Esc` and `Enter` always exit the mode.

```toml
[key_modes.resize]
entry = "super+alt+r"
exclusive = true

[key_modes.resize.bindings]
left = "resize_left"
right = "resize_right"
up = "resize_up"
down = "resize_down"
```

The built-in `resize` mode uses arrow keys to resize the active pane. `Shift` makes each resize step larger. A user definition can change its entry or bindings. Other mode bindings can use any action id shown in the Settings key binding list, plus `exit`.

Fastty clears a mode or pending sequence when the active tab changes or the config reloads.

`fastty keybindings --json` lists active single-key bindings, sequences, and modes. The built-in resize mode uses `super+alt+r` on macOS and `alt+shift+r` on other systems.
