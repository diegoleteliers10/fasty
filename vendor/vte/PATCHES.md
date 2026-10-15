# Fastty changes

Base: vte 0.15.0. Keep the upstream Apache-2.0 and MIT licenses.

The `std` and `serde` features enable a typed `ProcessorSnapshot`.
The snapshot stores the parser, preceding character, synchronized-update
buffer, and pending update state. Restore assigns this state without
replay of completed terminal actions. A pending update gets a new timeout.

Validation checks parser indexes, parameter group boundaries, partial
UTF-8, OSC bounds, and synchronized-update buffers before restore.
Tests cover continuation without repeated actions and oversized state.

The native Fastty snapshot also stores its separate OSC/APC/CSI scanner.
This crate does not own that scanner.
