//! Binary snapshot serialization and deserialization for terminal grids.
//!
//! Provides zero-copy contiguous memory layout for sub-millisecond snapshots:
//! - Magic header `b"FST1"` (32 bytes)
//! - Packed 16-byte cell representation (`FasttyPackedCell`)
//! - Direct slice conversion and binary frame encoding.

use std::mem::size_of;

/// Magic header bytes identifying a Fastty binary terminal snapshot.
pub const BINARY_SNAPSHOT_MAGIC: [u8; 4] = *b"FST1";
pub const BINARY_SNAPSHOT_VERSION: u16 = 1;

/// Flags for terminal display attributes in FasttyPackedCell.
pub const CELL_FLAG_BOLD: u16 = 1 << 0;
pub const CELL_FLAG_DIM: u16 = 1 << 1;
pub const CELL_FLAG_ITALIC: u16 = 1 << 2;
pub const CELL_FLAG_UNDERLINE: u16 = 1 << 3;
pub const CELL_FLAG_INVERSE: u16 = 1 << 4;
pub const CELL_FLAG_HIDDEN: u16 = 1 << 5;
pub const CELL_FLAG_STRIKETHROUGH: u16 = 1 << 6;

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
    pub _reserved2: [u8; 10],
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
        if version != BINARY_SNAPSHOT_VERSION {
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
}
