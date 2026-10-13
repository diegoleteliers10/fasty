# Actions and workflows

List the built-in action catalog as JSON. The output includes `schema_version`.

```sh
fastty actions --json
```

List the active key bindings, sequences, and modes as JSON. The output includes `schema_version`.

```sh
fastty keybindings --json
```

## Custom workflows

Add a workflow under `[workflows.<id>]`. Fastty shows it in the command palette.

```toml
[workflows.review_layout]
label = "Prepare review layout"
actions = ["new_tab", "split_right", "focus_left", "zoom_pane"]
```

Workflow actions come from a fixed list of Fastty UI actions. They cannot run shell commands or scripts. Supported actions are `new_tab`, `new_window`, `split_right`, `split_down`, `split_left`, `split_top`, `focus_right`, `focus_down`, `focus_left`, `focus_top`, `zoom_pane`, `toggle_tab_sidebar`, `toggle_project_sidebar`, `toggle_ai_sidebar`, `search`, `global_search`, `tab_overview`, `clear`, `fullscreen`, `zoom_in`, `zoom_out`, and `zoom_reset`.
