# Session scrollback snapshots

A pane normally lives in the session host. When the host restarts or dies, the
session id stored in the window session no longer resolves, so the pane cannot
attach.

`src/session_snapshots.rs` keeps the last terminal snapshot per host session.
The app records one snapshot every 20 seconds for each pane that has a live
terminal. On restore, a pane whose host session is gone is rebuilt in place and
the snapshot is replayed into it.

## Record shape

| Field | Meaning |
|---|---|
| `session_id` | Session id issued by the host |
| `host_instance` | Host that issued the id. An id from another instance never matches |
| `foreground_running` | A foreground program owned the pane, so the screen is a dead TUI |
| `updated_at` | Seconds since the epoch, used to expire records |
| `encoded` | Base64 of the snapshot bytes (`snapshot_state_for_save`) |

A record above 1 MiB is rejected and the previous one is kept. The cap is
enforced on write because the snapshot generator accepts a much larger payload.

## Rules

1. A record is matched by `(host_instance, session_id)`. A bare session id only
   matches when exactly one instance recorded it, which covers legacy session
   files without an identity.
2. A snapshot taken while a foreground program was running is not replayed. The
   pane still comes back, as a shell in its own directory, because a dead TUI
   screen answers no keys and losing the pane loses more than the screen.
3. A successful attach deletes the record, including a retry that succeeds after
   an earlier failure; the live host is the source of truth.
4. Records expire after seven days, and the store keeps at most 48 entries, so
   the file stays at or below about 48 MiB.
5. The flush loop covers every live pane in one pass and one file write, and only
   one flush runs at a time.
6. The store lives in the state directory as `session-snapshots.json`. A write is
   atomic, and a missing or corrupt file is treated as empty.
