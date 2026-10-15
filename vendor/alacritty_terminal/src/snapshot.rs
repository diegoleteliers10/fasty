//! Logical terminal state with validation before restore.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::term::TermMode;
use crate::term::cell::Cell;
use crate::term::color;
use crate::vte::ansi::{KeyboardModes, Rgb};

pub const MAX_SNAPSHOT_CELLS: usize = 1_048_576;

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CursorSnapshot {
    pub line: i32,
    pub column: usize,
    pub template: Cell,
    pub charsets: [u8; 4],
    pub input_needs_wrap: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct GridSnapshot {
    pub columns: usize,
    pub screen_lines: usize,
    pub history_size: usize,
    pub history_limit: usize,
    pub display_offset: usize,
    /// Rows from the oldest retained history line to the last screen line.
    pub rows: Vec<Vec<Cell>>,
    pub cursor: CursorSnapshot,
    pub saved_cursor: CursorSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CursorStyleSnapshot {
    /// Block, underline, beam, hollow block, or hidden, in that order.
    pub shape: u8,
    pub blinking: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TerminalSnapshot {
    pub main: GridSnapshot,
    pub alternate: GridSnapshot,
    pub modes: u32,
    pub active_charset: u8,
    pub scroll_region_start: i32,
    pub scroll_region_end: i32,
    pub tabs: Vec<bool>,
    pub colors: Vec<Option<Rgb>>,
    pub cursor_style: Option<CursorStyleSnapshot>,
    pub keyboard_mode_stack: Vec<u8>,
    pub inactive_keyboard_mode_stack: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotError(pub &'static str);

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for SnapshotError {}

impl TerminalSnapshot {
    pub fn validate(&self) -> Result<(), SnapshotError> {
        let main_cells = self.main.validate()?;
        let alternate_cells = self.alternate.validate()?;
        if main_cells.saturating_add(alternate_cells) > MAX_SNAPSHOT_CELLS {
            return Err(SnapshotError("Snapshot cell count exceeds the limit"));
        }
        if self.main.columns != self.alternate.columns
            || self.main.screen_lines != self.alternate.screen_lines
            || self.alternate.history_size != 0
            || self.alternate.history_limit != 0
        {
            return Err(SnapshotError("Snapshot grids have invalid dimensions"));
        }
        if TermMode::from_bits(self.modes).is_none() || self.active_charset > 3 {
            return Err(SnapshotError("Snapshot mode or character set is invalid"));
        }
        if self.scroll_region_start < 0
            || self.scroll_region_end <= self.scroll_region_start
            || self.scroll_region_end as usize > self.main.screen_lines
        {
            return Err(SnapshotError("Snapshot scroll region is invalid"));
        }
        if self.tabs.len() != self.main.columns || self.colors.len() != color::COUNT {
            return Err(SnapshotError("Snapshot tab or color count is invalid"));
        }
        if self.cursor_style.is_some_and(|style| style.shape > 4) {
            return Err(SnapshotError("Snapshot cursor style is invalid"));
        }
        for stack in [
            &self.keyboard_mode_stack,
            &self.inactive_keyboard_mode_stack,
        ] {
            if stack.len() > 4096
                || stack
                    .iter()
                    .any(|bits| KeyboardModes::from_bits(*bits).is_none())
            {
                return Err(SnapshotError("Snapshot keyboard stack is invalid"));
            }
        }
        Ok(())
    }
}

impl GridSnapshot {
    fn validate(&self) -> Result<usize, SnapshotError> {
        if self.columns < 2
            || self.columns > u16::MAX as usize
            || self.screen_lines == 0
            || self.screen_lines > u16::MAX as usize
            || self.history_limit > 100_000
            || self.history_size > self.history_limit
            || self.display_offset > self.history_size
        {
            return Err(SnapshotError("Snapshot grid bounds are invalid"));
        }
        let rows = self
            .screen_lines
            .checked_add(self.history_size)
            .ok_or(SnapshotError("Snapshot row count overflows"))?;
        let cells = rows
            .checked_mul(self.columns)
            .ok_or(SnapshotError("Snapshot cell count overflows"))?;
        if cells > MAX_SNAPSHOT_CELLS
            || self.rows.len() != rows
            || self.rows.iter().any(|row| row.len() != self.columns)
        {
            return Err(SnapshotError("Snapshot row width or count is invalid"));
        }
        for cursor in [&self.cursor, &self.saved_cursor] {
            if cursor.line < 0
                || cursor.line as usize >= self.screen_lines
                || cursor.column >= self.columns
                || cursor.charsets.iter().any(|charset| *charset > 1)
            {
                return Err(SnapshotError("Snapshot cursor is invalid"));
            }
            validate_cell(&cursor.template)?;
        }
        for cell in self.rows.iter().flatten() {
            validate_cell(cell)?;
        }
        Ok(cells)
    }
}

fn validate_cell(cell: &Cell) -> Result<(), SnapshotError> {
    if cell
        .zerowidth()
        .is_some_and(|characters| characters.len() > 1024)
        || cell
            .hyperlink()
            .is_some_and(|link| link.uri().len() > 65536 || link.id().len() > 65536)
    {
        return Err(SnapshotError("Snapshot cell extra data exceeds the limit"));
    }
    Ok(())
}
