//! ANSI parser — wrapper around `vte::Parser` for VT sequence parsing.

use alacritty_terminal::vte::{Params, Parser, Perform};

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Print(char),
    LineFeed,
    CarriageReturn,
    Backspace,
    Tab,
    Bell,
    CursorPos {
        row: usize,
        col: usize,
    },
    CursorUp(usize),
    CursorDown(usize),
    CursorForward(usize),
    CursorBack(usize),
    /// CNL — `CSI n E`: down *n* lines, first column (not “row n”).
    CursorDownAbs(usize),
    /// CPL — `CSI n F`: up *n* lines, first column (not “row n”).
    CursorUpAbs(usize),
    CursorColumn(usize),
    /// VPA — Vertical Line Position Absolute (`CSI n d`). Row is 1-based (ECMA-48).
    CursorLinePositionAbsolute(usize),
    CursorPosition {
        row: usize,
        col: usize,
    },
    EraseDisplay(u8),
    EraseLine(u8),
    ScrollUp(usize),
    ScrollDown(usize),
    TabSet,
    TabClear(u8),
    SaveCursor,
    RestoreCursor,
    SetSgr(Vec<u16>),
    Reset,
    SetMode(u16),
    ResetMode(u16),
    /// DECCKM — Cursor Key Mode (mode 1).
    /// When set, arrow keys send ESC OA/OB/OC/OD instead of CSI A/B/C/D.
    SetDECCKM(bool),
    /// Bracketed Paste Mode (mode 2004).
    /// When set, pasted text is wrapped with \x1b[200~ ... \x1b[201~.
    SetBracketedPaste(bool),
    /// Focus In/Out Mode (mode 1004).
    /// When set, terminal sends \x1b[I on focus in, \x1b[O on focus out.
    SetFocusMode(bool),
    /// Mouse modes (1000, 1002, 1003, etc.).
    /// See MouseMode enum in terminal module.
    SetMouseMode(u16),
    /// SGR mouse encoding (mode 1006).
    SetSGRMouse(bool),
    /// DECSET — generic DEC private mode set (csi ? h).
    DecSet(u16),
    /// DECRST — generic DEC private mode reset (csi ? l).
    DecReset(u16),
    /// DECSTBM — `CSI row;row r`: set scroll region [top;bottom].
    SetScrollRegion {
        top: usize,
        bottom: usize,
    },
    /// IL — `CSI n L`: insert n blank lines at cursor (inside scroll region only).
    InsertLines(usize),
    /// DL — `CSI n M`: delete n lines at cursor (inside scroll region only).
    DeleteLines(usize),
    /// IND — `ESC D`: index down (scroll up if at bottom margin).
    Index,
    /// NEL — `ESC E`: next line (down 1 + column 0).
    NextLine,
    /// RI — `ESC M`: reverse index (scroll down if at top margin).
    ReverseIndex,
}

pub struct VtParser {
    inner: Parser<1024>,
    performer: Performer,
}

impl VtParser {
    pub fn new() -> Self {
        Self {
            inner: Parser::new(),
            performer: Performer::new(),
        }
    }

    pub fn feed_str(&mut self, bytes: &[u8]) -> Vec<Action> {
        self.performer.actions.clear();
        self.inner.advance(&mut self.performer, bytes);
        std::mem::take(&mut self.performer.actions)
    }
}

impl Default for VtParser {
    fn default() -> Self {
        Self::new()
    }
}

struct Performer {
    actions: Vec<Action>,
}

impl Performer {
    fn new() -> Self {
        Self {
            actions: Vec::with_capacity(256),
        }
    }
}

impl Perform for Performer {
    fn print(&mut self, c: char) {
        self.actions.push(Action::Print(c));
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => self.actions.push(Action::Bell),
            0x08 => self.actions.push(Action::Backspace),
            0x09 => self.actions.push(Action::Tab),
            0x0A | 0x0B | 0x0C => self.actions.push(Action::LineFeed),
            0x0D => self.actions.push(Action::CarriageReturn),
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        // Preserve CSI parameter positions (including omitted params like `CSI ;5H`).
        // Flattening sub-params loses empty slots and corrupts cursor addressing.
        let p: Vec<u16> = params
            .iter()
            .map(|sp| sp.first().copied().unwrap_or(0))
            .collect();

        // CUP — Cursor Position: `CSI row;col H` or equivalent `CSI row;col f` (same as H in xterm).
        if (action == 'H' || action == 'f') && intermediates.is_empty() {
            let row = p.first().copied().unwrap_or(1) as usize;
            let col = p.get(1).copied().unwrap_or(1) as usize;
            self.actions.push(Action::CursorPos { row, col });
            return;
        }

        // VPA — Vertical Line Position Absolute (`CSI n d`).
        if action == 'd' && intermediates.is_empty() {
            let row = p.first().copied().unwrap_or(1) as usize;
            self.actions.push(Action::CursorLinePositionAbsolute(row));
            return;
        }

        // SCOSC / SCORC — Save/Restore Cursor (`CSI s`, `CSI u`). Same semantics as ESC 7 / ESC 8;
        // Fish and many prompts use these CSI forms instead of DECSC/DECRC.
        if action == 's' && intermediates.is_empty() {
            self.actions.push(Action::SaveCursor);
            return;
        }
        if action == 'u' && intermediates.is_empty() {
            self.actions.push(Action::RestoreCursor);
            return;
        }

        // DECSTBM — `CSI row;row r`: set scroll region [top;bottom].
        // Params are 1-based; 0 defaults to 1 for top, rows for bottom.
        if action == 'r' && intermediates.is_empty() {
            let top = p.first().copied().unwrap_or(1) as usize;
            let bottom = p.get(1).copied().unwrap_or(0) as usize;
            self.actions.push(Action::SetScrollRegion { top, bottom });
            return;
        }

        // IL — `CSI n L`: insert n blank lines at cursor (inside scroll region only).
        if action == 'L' && intermediates.is_empty() {
            let n = p.first().copied().unwrap_or(1) as usize;
            self.actions.push(Action::InsertLines(n));
            return;
        }

        // DL — `CSI n M`: delete n lines at cursor (inside scroll region only).
        if action == 'M' && intermediates.is_empty() {
            let n = p.first().copied().unwrap_or(1) as usize;
            self.actions.push(Action::DeleteLines(n));
            return;
        }

        if action == 'J' {
            let mode = p.first().copied().unwrap_or(0) as u8;
            self.actions.push(Action::EraseDisplay(mode));
            return;
        }

        if action == 'K' {
            let mode = p.first().copied().unwrap_or(0) as u8;
            self.actions.push(Action::EraseLine(mode));
            return;
        }

        if action == 'm' && intermediates.is_empty() {
            self.actions.push(Action::SetSgr(p));
            return;
        }

        // Handle SM (Set Mode) and RM (Reset Mode)
        // These use private mode numbers (prefixed with ?)
        if (action == 'h' || action == 'l') && intermediates.first() == Some(&0x3F) {
            let mode_num = p.first().copied().unwrap_or(0);

            // Decode well-known DEC modes for first-class support
            match mode_num {
                1 => self.actions.push(Action::SetDECCKM(action == 'h')),
                2004 => self.actions.push(Action::SetBracketedPaste(action == 'h')),
                1004 => self.actions.push(Action::SetFocusMode(action == 'h')),
                1000 | 1002 | 1003 => self.actions.push(Action::SetMouseMode(mode_num)),
                1006 => self.actions.push(Action::SetSGRMouse(action == 'h')),
                _ => {
                    if action == 'h' {
                        self.actions.push(Action::DecSet(mode_num));
                    } else {
                        self.actions.push(Action::DecReset(mode_num));
                    }
                }
            }
            return;
        }

        let n = p.first().copied().unwrap_or(1) as usize;
        match action {
            'A' => self.actions.push(Action::CursorUp(n)),
            'B' => self.actions.push(Action::CursorDown(n)),
            'C' => self.actions.push(Action::CursorForward(n)),
            'D' => self.actions.push(Action::CursorBack(n)),
            'E' => self.actions.push(Action::CursorDownAbs(n)),
            'F' => self.actions.push(Action::CursorUpAbs(n)),
            'G' => self.actions.push(Action::CursorColumn(n)),
            // `H` is handled above (duplicate CUP form); `f` is handled above.
            'H' => {
                let row = p.first().copied().unwrap_or(1) as usize;
                let col = p.get(1).copied().unwrap_or(1) as usize;
                self.actions.push(Action::CursorPosition { row, col });
            }
            'S' => self.actions.push(Action::ScrollUp(n)),
            'T' => self.actions.push(Action::ScrollDown(n)),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, final_byte: u8) {
        match final_byte {
            0x37 => self.actions.push(Action::SaveCursor),
            0x38 => self.actions.push(Action::RestoreCursor),
            0x48 => self.actions.push(Action::TabSet),
            // IND — `ESC D`: index down (move cursor down 1; scroll if at bottom margin).
            0x44 => self.actions.push(Action::Index),
            // NEL — `ESC E`: next line (down 1 + column 0).
            0x45 => self.actions.push(Action::NextLine),
            // RI — `ESC M`: reverse index (move cursor up 1; scroll down if at top margin).
            0x4D => self.actions.push(Action::ReverseIndex),
            _ => {}
        }
    }
}
