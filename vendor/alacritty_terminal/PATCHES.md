# Terminal snapshot patch

Source: `alacritty_terminal` 0.26.0 from crates.io. License: Apache-2.0.

The `snapshot` module adds public logical state types. `Term::snapshot` reads both grids without changing either grid. It keeps the newest history rows up to the requested limit.

`Term::restore_snapshot` validates all grid bounds before it changes the terminal. It makes new grids from logical rows and allocates the exact history row count. It restores the cursor template, pending wrap, saved cursor, character sets, cell flags, combining characters, color identities, color overrides, tabs, scroll region, cursor style, and keyboard mode stacks.

Validation limits the combined cell count to 1,048,576. The wire decoder must also limit bytes before it deserializes the snapshot.

The patch keeps the event proxy and local configuration. The history limit follows the snapshot. It clears the selection and resets the vi cursor. It marks all screen cells as damaged.

Upstream reference test fixtures and the Cargo reference test target are omitted. Upstream source unit tests remain.
