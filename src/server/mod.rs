//! Fastty Self-Hosted Server Optimizations.
//!
//! Includes:
//! - `binary_snapshot`: Microsecond terminal state binary snapshot & restore.
//! - `compression`: In-memory chunked scrollback and session compression.

pub mod binary_snapshot;
pub mod compression;

pub use binary_snapshot::{
    FasttyBinarySnapshotHeader, FasttyPackedCell, decode_snapshot, encode_snapshot,
    BINARY_SNAPSHOT_MAGIC, BINARY_SNAPSHOT_VERSION,
};
pub use compression::{CompressedScrollbackStorage, ScrollbackChunk, compress_bytes, decompress_bytes};
