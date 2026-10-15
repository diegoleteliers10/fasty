# Terminal snapshots

## Native state

FST2 schema version 1 stores both terminal grids and their retained history.
It keeps the active and saved cursors, current text style, pending wrap, line-wrap
flags, named and indexed colors, explicit RGB, and combining characters.
It also stores scroll regions, tab stops, character sets, color overrides,
cursor style, and keyboard mode stacks.

The terminal engine exposes immutable capture and validated state replacement.
The pinned engine patch is in `vendor/alacritty_terminal`. `PATCHES.md` records its
source and public API. Capture does not switch screen buffers.

Native attach uses FST2 only after the host reports `terminal_state_snapshot_v1`.
FST1 remains the format for old native clients and WASM clients.
Older hosts and saved FST1 data cannot recover state that they did not store.

## Validation

The native decoder limits the decoded payload to 256 MiB and the total cell count
to 1,048,576. It checks dimensions, row lengths, cursor bounds, display offset,
scroll regions, color count, character sets, and keyboard mode values before restore.
Capture trims the oldest main history when the cell limit requires it.
Restore keeps explicit application colors even when their RGB matches a theme color.

## Saved workspaces

Workspace save captures FST2 and compresses it in the save worker.
A capture or compression failure prevents the workspace save.
The previous workspace file remains intact.

Live attach restores input state so the existing process can continue output.
Historical shell restore resets input style and modes for the new shell.
The workspace loader does not replay an ended TUI as a live process.

## Verification

The regression suite compares source and restored terminals after continued output,
TUI exit, and resize. It checks the six failures from the restore audit.
Additional checks cover compression, invalid state, saved-cursor behavior, scroll
regions, tab stops, and historical shell input reset.

New FST2 snapshots also store the VT parser and the OSC, APC, and CSI scanner state.
Tests split UTF-8 and escape sequences at each byte and compare continued output.
Old FST2 snapshots without this state start with an empty parser.
Historical restore into a new shell clears pending input.

The snapshot excludes completed multipart image and clipboard transactions,
the image store, and local command timers. GUI behavior requires separate verification.
