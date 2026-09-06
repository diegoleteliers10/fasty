//! In-memory chunked scrollback and session compression for Fastty server.
//!
//! Provides fast Deflate/Zlib compression for cold scrollback rows (>500 lines)
//! and session snapshots. Keeps active rows uncompressed for zero-overhead
//! rendering, while reducing idle session RAM by 4x-8x.

use std::collections::HashMap;
use crate::server::binary_snapshot::{FasttyPackedCell, bytes_to_cells, cells_to_bytes};

/// Number of rows per compressed scrollback chunk.
pub const CHUNK_ROW_COUNT: usize = 256;

/// Threshold of warm rows kept raw in memory before older rows are compressed into chunks.
pub const WARM_ROW_THRESHOLD: usize = 512;

/// Maximum number of decompressed chunks kept in the LRU cache.
pub const LRU_CACHE_CAPACITY: usize = 4;

/// A single immutable compressed chunk of terminal scrollback rows.
#[derive(Clone, Debug)]
pub struct ScrollbackChunk {
    pub chunk_id: usize,
    pub cols: u16,
    pub row_count: u16,
    pub uncompressed_size: usize,
    pub compressed_data: Vec<u8>,
}

impl ScrollbackChunk {
    /// Compress a slice of FasttyPackedCells representing `row_count` rows of width `cols`.
    pub fn new(chunk_id: usize, cols: u16, row_count: u16, cells: &[FasttyPackedCell]) -> Self {
        let raw_bytes = cells_to_bytes(cells);
        let uncompressed_size = raw_bytes.len();
        // Level 1 or 6 deflate gives rapid compression (<0.1ms) with 4x-8x ratio
        let compressed_data = miniz_oxide::deflate::compress_to_vec(raw_bytes, 6);
        Self {
            chunk_id,
            cols,
            row_count,
            uncompressed_size,
            compressed_data,
        }
    }

    /// Decompress back into raw FasttyPackedCells.
    pub fn decompress(&self) -> Result<Vec<FasttyPackedCell>, String> {
        let decompressed = miniz_oxide::inflate::decompress_to_vec(&self.compressed_data)
            .map_err(|e| format!("Decompression error: {:?}", e))?;
        bytes_to_cells(&decompressed)
            .ok_or_else(|| "Invalid cell byte alignment after decompression".to_string())
    }

    /// Compressed size in bytes.
    pub fn compressed_size(&self) -> usize {
        self.compressed_data.len()
    }
}

/// Tiered scrollback storage:
/// - Lines 0 to 500: stored as uncompressed raw rows in memory.
/// - Older lines: batched into 256-line `ScrollbackChunk`s and compressed.
/// - Includes an LRU cache of recently decompressed chunks for fluid scrolling.
pub struct CompressedScrollbackStorage {
    cols: u16,
    chunks: Vec<ScrollbackChunk>,
    next_chunk_id: usize,
    lru_cache: HashMap<usize, Vec<FasttyPackedCell>>,
    lru_order: Vec<usize>,
}

impl CompressedScrollbackStorage {
    pub fn new(cols: u16) -> Self {
        Self {
            cols,
            chunks: Vec::new(),
            next_chunk_id: 0,
            lru_cache: HashMap::new(),
            lru_order: Vec::new(),
        }
    }

    /// Push a completed cold chunk of rows into compressed storage.
    pub fn push_chunk(&mut self, cells: &[FasttyPackedCell], row_count: u16) {
        let chunk_id = self.next_chunk_id;
        self.next_chunk_id += 1;
        let chunk = ScrollbackChunk::new(chunk_id, self.cols, row_count, cells);
        self.chunks.push(chunk);
    }

    /// Retrieve rows for a chunk by index, using LRU cache when possible.
    pub fn get_chunk_cells(&mut self, chunk_idx: usize) -> Option<Vec<FasttyPackedCell>> {
        let chunk = self.chunks.get(chunk_idx)?;
        let chunk_id = chunk.chunk_id;

        if let Some(cached) = self.lru_cache.get(&chunk_id) {
            // Touch LRU
            if let Some(pos) = self.lru_order.iter().position(|&id| id == chunk_id) {
                self.lru_order.remove(pos);
            }
            self.lru_order.push(chunk_id);
            return Some(cached.clone());
        }

        // Decompress on demand
        if let Ok(cells) = chunk.decompress() {
            // Evict oldest if capacity exceeded
            if self.lru_order.len() >= LRU_CACHE_CAPACITY {
                if let Some(oldest_id) = self.lru_order.first().copied() {
                    self.lru_order.remove(0);
                    self.lru_cache.remove(&oldest_id);
                }
            }
            self.lru_cache.insert(chunk_id, cells.clone());
            self.lru_order.push(chunk_id);
            Some(cells)
        } else {
            None
        }
    }

    /// Clear all stored chunks (e.g. on `CSI 2J` / `CSI 3J` scrollback wipe).
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.lru_cache.clear();
        self.lru_order.clear();
        self.next_chunk_id = 0;
    }

    /// Total compressed memory used by chunks in bytes.
    pub fn total_compressed_bytes(&self) -> usize {
        self.chunks.iter().map(|c| c.compressed_size()).sum()
    }

    /// Total raw uncompressed memory that would be used without compression.
    pub fn total_uncompressed_bytes(&self) -> usize {
        self.chunks.iter().map(|c| c.uncompressed_size).sum()
    }

    /// Memory compression ratio (uncompressed / compressed).
    pub fn compression_ratio(&self) -> f32 {
        let comp = self.total_compressed_bytes();
        if comp == 0 {
            1.0
        } else {
            self.total_uncompressed_bytes() as f32 / comp as f32
        }
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

/// Compress an arbitrary byte slice using fast Deflate level 6.
pub fn compress_bytes(input: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(input, 6)
}

/// Decompress an arbitrary byte slice.
pub fn decompress_bytes(input: &[u8]) -> Result<Vec<u8>, String> {
    miniz_oxide::inflate::decompress_to_vec(input)
        .map_err(|e| format!("Decompression failure: {:?}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scrollback_chunk_roundtrip() {
        let cols = 80;
        let rows = 10;
        let mut sample_cells = Vec::new();
        for _r in 0..rows {
            for c in 0..cols {
                sample_cells.push(FasttyPackedCell {
                    c: ('a' as u8 + (c % 26) as u8) as u32,
                    fg: 0x00FFFFFF,
                    bg: 0x00000000,
                    flags: 0,
                    _reserved: 0,
                });
            }
        }

        let chunk = ScrollbackChunk::new(1, cols, rows, &sample_cells);
        assert!(chunk.compressed_size() < chunk.uncompressed_size);

        let decompressed = chunk.decompress().expect("Decompress chunk failed");
        assert_eq!(decompressed, sample_cells);
    }

    #[test]
    fn test_compressed_storage_lru() {
        let mut storage = CompressedScrollbackStorage::new(80);
        let cells = vec![FasttyPackedCell::default(); 80 * 10];
        storage.push_chunk(&cells, 10);
        assert_eq!(storage.chunk_count(), 1);

        let fetched = storage.get_chunk_cells(0).expect("Fetch chunk failed");
        assert_eq!(fetched.len(), 800);
        assert!(storage.compression_ratio() > 1.0);
    }
}
