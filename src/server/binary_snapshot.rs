//! Binary snapshot serialization and deserialization for terminal grids.
//!
//! Provides zero-copy contiguous memory layout for sub-millisecond snapshots:
//! - Magic header `b"FST1"` (32 bytes)
//! - Packed 16-byte cell representation (`FasttyPackedCell`)
//! - Direct slice conversion and binary frame encoding.

use std::mem::size_of;

/// Magic header bytes identifying a Fastty binary terminal snapshot.
pub const BINARY_SNAPSHOT_MAGIC: [u8; 4] = *b"FST1";
/// v2 adds scrollback history: `history_rows` rides in the header (bytes
/// 22..26, previously reserved-zero in v1, so v1 payloads decode as
/// screen-only with no branching) and `cell_count` covers history + screen.
pub const BINARY_SNAPSHOT_VERSION: u16 = 2;
/// Versions this decoder accepts. v1 snapshots (screen only) decode with
/// zero history rows because v1 left the history field zeroed.
pub const SUPPORTED_SNAPSHOT_VERSIONS: [u16; 2] = [1, 2];

/// Flags for terminal display attributes in FasttyPackedCell.
pub const CELL_FLAG_BOLD: u16 = 1 << 0;
pub const CELL_FLAG_DIM: u16 = 1 << 1;
pub const CELL_FLAG_ITALIC: u16 = 1 << 2;
pub const CELL_FLAG_UNDERLINE: u16 = 1 << 3;
pub const CELL_FLAG_INVERSE: u16 = 1 << 4;
pub const CELL_FLAG_HIDDEN: u16 = 1 << 5;
pub const CELL_FLAG_STRIKETHROUGH: u16 = 1 << 6;
/// First half of a double-width glyph (the glyph cell itself).
pub const CELL_FLAG_WIDE_CHAR: u16 = 1 << 7;
/// The blank spacer cell that follows a double-width glyph.
pub const CELL_FLAG_WIDE_CHAR_SPACER: u16 = 1 << 8;

/// Flat 16-byte cell representation with explicit memory alignment.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FasttyPackedCell {
    /// Unicode codepoint (UTF-32). 0 means blank space.
    pub c: u32,
    /// Foreground 24-bit RGB packed as 0x00RRGGBB.
    pub fg: u32,
    /// Background 24-bit RGB packed as 0x00RRGGBB.
    pub bg: u32,
    /// Cell style flags (bold, dim, underline, etc.).
    pub flags: u16,
    /// Reserved padding to ensure strict 16-byte alignment.
    pub _reserved: u16,
}

const _: () = assert!(size_of::<FasttyPackedCell>() == 16);

/// Flags for FasttyBinarySnapshotHeader.
pub const SNAPSHOT_FLAG_ALT_SCREEN: u16 = 1 << 0;
pub const SNAPSHOT_FLAG_CURSOR_VISIBLE: u16 = 1 << 1;
pub const SNAPSHOT_FLAG_DEFLATE: u16 = 1 << 2;

/// Binary header prepended to every binary snapshot (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FasttyBinarySnapshotHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub flags: u16,
    pub cols: u16,
    pub rows: u16,
    pub cursor_col: u16,
    pub cursor_row: u16,
    pub cell_count: u32,
    pub cursor_style: u8,
    pub _reserved1: u8,
    /// Bytes 22..32 of the wire header. Bytes 0..4 of this array carry the
    /// scrollback (history) row count as little-endian u32 — v1 snapshots
    /// left them zeroed, so v1 decodes as screen-only with no branching.
    /// Kept as a raw array (not a `u32` field) to preserve the exact
    /// 32-byte `repr(C)` layout without interior padding.
    pub _reserved2: [u8; 10],
}

impl FasttyBinarySnapshotHeader {
    /// Scrollback (history) rows preceding the screen rows in the cell
    /// payload, ordered oldest row first.
    pub fn history_rows(&self) -> u32 {
        u32::from_le_bytes(self._reserved2[0..4].try_into().unwrap_or([0; 4]))
    }

    pub fn set_history_rows(&mut self, rows: u32) {
        self._reserved2[0..4].copy_from_slice(&rows.to_le_bytes());
    }
}

const _: () = assert!(size_of::<FasttyBinarySnapshotHeader>() == 32);

impl Default for FasttyBinarySnapshotHeader {
    fn default() -> Self {
        Self {
            magic: BINARY_SNAPSHOT_MAGIC,
            version: BINARY_SNAPSHOT_VERSION,
            flags: 0,
            cols: 0,
            rows: 0,
            cursor_col: 0,
            cursor_row: 0,
            cell_count: 0,
            cursor_style: 0,
            _reserved1: 0,
            _reserved2: [0; 10],
        }
    }
}

impl FasttyBinarySnapshotHeader {
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        bytes[0..4].copy_from_slice(&self.magic);
        bytes[4..6].copy_from_slice(&self.version.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.flags.to_le_bytes());
        bytes[8..10].copy_from_slice(&self.cols.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.rows.to_le_bytes());
        bytes[12..14].copy_from_slice(&self.cursor_col.to_le_bytes());
        bytes[14..16].copy_from_slice(&self.cursor_row.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.cell_count.to_le_bytes());
        bytes[20] = self.cursor_style;
        bytes[21] = self._reserved1;
        bytes[22..32].copy_from_slice(&self._reserved2);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 32 {
            return None;
        }
        let magic = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if magic != BINARY_SNAPSHOT_MAGIC {
            return None;
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if !SUPPORTED_SNAPSHOT_VERSIONS.contains(&version) {
            return None;
        }
        let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
        let cols = u16::from_le_bytes([bytes[8], bytes[9]]);
        let rows = u16::from_le_bytes([bytes[10], bytes[11]]);
        let cursor_col = u16::from_le_bytes([bytes[12], bytes[13]]);
        let cursor_row = u16::from_le_bytes([bytes[14], bytes[15]]);
        let cell_count = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let cursor_style = bytes[20];
        let reserved1 = bytes[21];
        let mut reserved2 = [0u8; 10];
        reserved2.copy_from_slice(&bytes[22..32]);

        Some(Self {
            magic,
            version,
            flags,
            cols,
            rows,
            cursor_col,
            cursor_row,
            cell_count,
            cursor_style,
            _reserved1: reserved1,
            _reserved2: reserved2,
        })
    }
}

/// Convert slice of FasttyPackedCell into raw bytes with zero copy.
pub fn cells_to_bytes(cells: &[FasttyPackedCell]) -> &[u8] {
    let byte_len = cells.len() * size_of::<FasttyPackedCell>();
    unsafe { std::slice::from_raw_parts(cells.as_ptr() as *const u8, byte_len) }
}

/// Parse slice of bytes into FasttyPackedCell vector.
pub fn bytes_to_cells(bytes: &[u8]) -> Option<Vec<FasttyPackedCell>> {
    if bytes.len() % size_of::<FasttyPackedCell>() != 0 {
        return None;
    }
    let count = bytes.len() / size_of::<FasttyPackedCell>();
    let mut cells = Vec::with_capacity(count);
    for chunk in bytes.chunks_exact(16) {
        let c = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let fg = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        let bg = u32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
        let flags = u16::from_le_bytes([chunk[12], chunk[13]]);
        let reserved = u16::from_le_bytes([chunk[14], chunk[15]]);
        cells.push(FasttyPackedCell {
            c,
            fg,
            bg,
            flags,
            _reserved: reserved,
        });
    }
    Some(cells)
}

/// Encode full binary snapshot into a byte vector with header and cells.
pub fn encode_snapshot(
    header: &FasttyBinarySnapshotHeader,
    cells: &[FasttyPackedCell],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + cells.len() * 16);
    out.extend_from_slice(&header.to_bytes());
    out.extend_from_slice(cells_to_bytes(cells));
    out
}

/// Encode full binary snapshot into a byte vector with header and Deflate-compressed cells.
pub fn encode_snapshot_compressed(
    header: &FasttyBinarySnapshotHeader,
    cells: &[FasttyPackedCell],
) -> Vec<u8> {
    let mut header_copy = *header;
    header_copy.flags |= SNAPSHOT_FLAG_DEFLATE;
    let raw_bytes = cells_to_bytes(cells);
    let compressed = miniz_oxide::deflate::compress_to_vec(raw_bytes, 6);
    let mut out = Vec::with_capacity(32 + compressed.len());
    out.extend_from_slice(&header_copy.to_bytes());
    out.extend_from_slice(&compressed);
    out
}

/// Decode full binary snapshot into header and cells.
pub fn decode_snapshot(bytes: &[u8]) -> Option<(FasttyBinarySnapshotHeader, Vec<FasttyPackedCell>)> {
    if bytes.len() < 32 {
        return None;
    }
    let header = FasttyBinarySnapshotHeader::from_bytes(&bytes[0..32])?;
    if (header.flags & SNAPSHOT_FLAG_DEFLATE) != 0 {
        let decompressed = miniz_oxide::inflate::decompress_to_vec(&bytes[32..]).ok()?;
        let expected_len = (header.cell_count as usize).checked_mul(16)?;
        if decompressed.len() < expected_len {
            return None;
        }
        let cells = bytes_to_cells(&decompressed[0..expected_len])?;
        Some((header, cells))
    } else {
        let required_len = (header.cell_count as usize)
            .checked_mul(16)
            .and_then(|n| n.checked_add(32));
        let expected_len = match required_len {
            Some(len) if bytes.len() >= len => len,
            _ => return None,
        };
        let cells = bytes_to_cells(&bytes[32..expected_len])?;
        Some((header, cells))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_header_roundtrip() {
        let header = FasttyBinarySnapshotHeader {
            magic: BINARY_SNAPSHOT_MAGIC,
            version: BINARY_SNAPSHOT_VERSION,
            flags: SNAPSHOT_FLAG_ALT_SCREEN | SNAPSHOT_FLAG_CURSOR_VISIBLE,
            cols: 120,
            rows: 40,
            cursor_col: 10,
            cursor_row: 5,
            cursor_style: 2,
            _reserved1: 0,
            cell_count: 4800,
            _reserved2: [0; 10],
        };

        let bytes = header.to_bytes();
        let decoded = FasttyBinarySnapshotHeader::from_bytes(&bytes).expect("Failed to decode header");
        assert_eq!(header, decoded);
    }

    #[test]
    fn test_snapshot_encode_decode() {
        let header = FasttyBinarySnapshotHeader {
            magic: BINARY_SNAPSHOT_MAGIC,
            version: BINARY_SNAPSHOT_VERSION,
            flags: 0,
            cols: 2,
            rows: 1,
            cursor_col: 1,
            cursor_row: 0,
            cursor_style: 0,
            _reserved1: 0,
            cell_count: 2,
            _reserved2: [0; 10],
        };

        let cells = vec![
            FasttyPackedCell {
                c: 'H' as u32,
                fg: 0x00FF0000,
                bg: 0x00000000,
                flags: CELL_FLAG_BOLD,
                _reserved: 0,
            },
            FasttyPackedCell {
                c: 'i' as u32,
                fg: 0x0000FF00,
                bg: 0x00111111,
                flags: CELL_FLAG_UNDERLINE,
                _reserved: 0,
            },
        ];

        let encoded = encode_snapshot(&header, &cells);
        assert_eq!(encoded.len(), 32 + 2 * 16);

        let (dec_header, dec_cells) = decode_snapshot(&encoded).expect("Decode failed");
        assert_eq!(dec_header, header);
        assert_eq!(dec_cells, cells);
    }

    #[test]
    fn test_snapshot_compressed_roundtrip() {
        let header = FasttyBinarySnapshotHeader {
            magic: BINARY_SNAPSHOT_MAGIC,
            version: BINARY_SNAPSHOT_VERSION,
            flags: 0,
            cols: 2,
            rows: 1,
            cursor_col: 1,
            cursor_row: 0,
            cursor_style: 0,
            _reserved1: 0,
            cell_count: 2,
            _reserved2: [0; 10],
        };

        let cells = vec![
            FasttyPackedCell {
                c: 'H' as u32,
                fg: 0x00FF0000,
                bg: 0x00000000,
                flags: CELL_FLAG_BOLD,
                _reserved: 0,
            },
            FasttyPackedCell {
                c: 'i' as u32,
                fg: 0x0000FF00,
                bg: 0x00111111,
                flags: CELL_FLAG_UNDERLINE,
                _reserved: 0,
            },
        ];

        let compressed = encode_snapshot_compressed(&header, &cells);
        assert_ne!(compressed.len(), 32 + 2 * 16);

        let (dec_header, dec_cells) = decode_snapshot(&compressed).expect("Decode compressed failed");
        assert_eq!(dec_header.flags & SNAPSHOT_FLAG_DEFLATE, SNAPSHOT_FLAG_DEFLATE);
        assert_eq!(dec_header.cols, header.cols);
        assert_eq!(dec_header.rows, header.rows);
        assert_eq!(dec_cells, cells);
    }

    #[test]
    fn test_v2_header_carries_history_rows() {
        let mut header = FasttyBinarySnapshotHeader::default();
        header.cols = 80;
        header.rows = 24;
        header.cell_count = (80 * (24 + 10)) as u32;
        header.set_history_rows(10);

        let bytes = header.to_bytes();
        // History rides in bytes 22..26, previously reserved.
        let raw_history = u32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(raw_history, 10);
        assert_eq!(bytes.len(), 32);

        let decoded = FasttyBinarySnapshotHeader::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.history_rows(), 10);
        assert_eq!(decoded, header);
    }

    #[test]
    fn test_v1_payload_decodes_with_zero_history() {
        // A v1 payload: version 1, history bytes zeroed.
        let mut header = FasttyBinarySnapshotHeader::default();
        header.version = 1;
        header.cols = 2;
        header.rows = 1;
        header.cell_count = 2;
        let cells = vec![FasttyPackedCell::default(); 2];
        let encoded = encode_snapshot(&header, &cells);

        let (dec_header, dec_cells) = decode_snapshot(&encoded).unwrap();
        assert_eq!(dec_header.version, 1);
        assert_eq!(dec_header.history_rows(), 0);
        assert_eq!(dec_cells, cells);
    }

    #[test]
    fn test_v2_roundtrip_with_history() {
        const COLS: usize = 4;
        const HISTORY: usize = 5;
        const ROWS: usize = 2;
        let cell = |ch: char| FasttyPackedCell {
            c: ch as u32,
            fg: 0x00ABCDEF,
            bg: 0x00123456,
            flags: 0,
            _reserved: 0,
        };
        // History rows 'a'..'e', screen rows 'x'/'y'.
        let mut cells = Vec::new();
        for ch in ['a', 'b', 'c', 'd', 'e'] {
            for c in 0..COLS {
                cells.push(cell(ch));
            }
        }
        for ch in ['x', 'y'] {
            for c in 0..COLS {
                cells.push(cell(ch));
            }
        }
        let mut header = FasttyBinarySnapshotHeader {
            magic: BINARY_SNAPSHOT_MAGIC,
            version: BINARY_SNAPSHOT_VERSION,
            flags: SNAPSHOT_FLAG_CURSOR_VISIBLE | SNAPSHOT_FLAG_DEFLATE,
            cols: COLS as u16,
            rows: ROWS as u16,
            cursor_col: 1,
            cursor_row: 1,
            cursor_style: 0,
            _reserved1: 0,
            cell_count: cells.len() as u32,
            _reserved2: [0; 10],
        };
        header.set_history_rows(HISTORY as u32);

        let encoded = encode_snapshot_compressed(&header, &cells);
        let (dec_header, dec_cells) = decode_snapshot(&encoded).unwrap();
        assert_eq!(dec_header.history_rows(), HISTORY as u32);
        assert_eq!(dec_header.cell_count, ((HISTORY + ROWS) * COLS) as u32);
        assert_eq!(dec_cells, cells);
        // Newest history row is 'e'.
        assert_eq!(dec_cells[(HISTORY - 1) * COLS].c, 'e' as u32);
        // First screen row follows the history.
        assert_eq!(dec_cells[HISTORY * COLS].c, 'x' as u32);
    }

    /// Benchmark: snapshot of a 2000x200 grid (~3.2MB of cells) — realistic
    /// scrollback content compresses well; run with --release for real
    /// numbers. Asserts correctness at scale and prints timings.
    #[test]
    fn benchmark_snapshot_2000x200_compressed() {
        const COLS: usize = 200;
        const HISTORY: usize = 2000;
        const ROWS: usize = 40;
        let mut cells = Vec::with_capacity((HISTORY + ROWS) * COLS);
        for r in 0..HISTORY + ROWS {
            let text = format!("$ cargo build --release # row {r} [finished in 12.34s]");
            for c in 0..COLS {
                cells.push(FasttyPackedCell {
                    c: if c < text.len() { text.as_bytes()[c] as u32 } else { ' ' as u32 },
                    fg: 0x00CDD6F4,
                    bg: 0x001E1E2E,
                    flags: if r % 7 == 0 { CELL_FLAG_BOLD } else { 0 },
                    _reserved: 0,
                });
            }
        }
        let mut header = FasttyBinarySnapshotHeader::default();
        header.cols = COLS as u16;
        header.rows = ROWS as u16;
        header.cell_count = cells.len() as u32;
        header.set_history_rows(HISTORY as u32);

        let start = std::time::Instant::now();
        let encoded = encode_snapshot_compressed(&header, &cells);
        let encode_elapsed = start.elapsed();
        let start = std::time::Instant::now();
        let (dec_header, dec_cells) = decode_snapshot(&encoded).unwrap();
        let decode_elapsed = start.elapsed();

        assert_eq!(dec_header.history_rows(), HISTORY as u32);
        assert_eq!(dec_cells.len(), cells.len());
        assert_eq!(dec_cells[12345], cells[12345]);

        println!(
            "benchmark 2000x200: raw {:.1}MB -> compressed {:.1}KB ({:.0}x), encode {:?}, decode {:?}",
            (cells.len() * 16) as f64 / 1_048_576.0,
            encoded.len() as f64 / 1024.0,
            (cells.len() * 16) as f64 / encoded.len() as f64,
            encode_elapsed,
            decode_elapsed,
        );
        // Generous debug-build bound; release runs are orders faster.
        assert!(encode_elapsed < std::time::Duration::from_secs(5));
        assert!(decode_elapsed < std::time::Duration::from_secs(5));
    }
}
