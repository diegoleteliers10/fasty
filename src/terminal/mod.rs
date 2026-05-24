//! Terminal emulation using alacritty_terminal as the backend.
//!
//! Key types:
//!   - `Term<VoidListener>`: full terminal state from alacritty_terminal
//!   - `ansi::Processor`: feeds bytes to Term (implements vte::ansi::Handler)
//!   - `Grid::display_iter()`: iterates visible cells for rendering
//!   - `TermMode`: bitflags for ALT_SCREEN, APP_CURSOR, BRACKETED_PASTE, etc.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::FontConfig;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

use gpui::{
    canvas, div, fill, px, App, AppContext, Context, Entity, FocusHandle, Focusable, Font,
    FontFeatures, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Styled,
    Window,
};

use crate::input::key_to_bytes;
use crate::pty::PtyWriter;

pub use alacritty_terminal::grid::Scroll;
pub use alacritty_terminal::term::TermMode;

pub struct Terminal {
    pub term: Arc<Mutex<alacritty_terminal::term::Term<VoidListener>>>,
    pub render_generation: Arc<AtomicU64>,
    pub pty_writer: PtyWriter,
    pub font_config: FontConfig,
    grid_wake_rx: Option<flume::Receiver<()>>,
    last_cols: Cell<usize>,
    last_rows: Cell<usize>,
    pending_font_update: bool,
}

impl Terminal {
    pub fn from_pre_spawned(
        _cx: &mut Context<Self>,
        pty_writer: PtyWriter,
        term: Arc<Mutex<alacritty_terminal::term::Term<VoidListener>>>,
        render_generation: Arc<AtomicU64>,
        grid_wake_rx: flume::Receiver<()>,
        font_config: FontConfig,
    ) -> Self {
        let last_cols = Cell::new(80);
        let last_rows = Cell::new(24);

        Self {
            term,
            render_generation,
            pty_writer,
            font_config,
            grid_wake_rx: Some(grid_wake_rx),
            last_cols,
            last_rows,
            pending_font_update: false,
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 {
            return;
        }
        {
            let mut term = self.term.lock().unwrap();
            let size = TermSize::new(cols, rows);
            term.resize(size);
        }
        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn take_grid_wake_receiver(&mut self) -> Option<flume::Receiver<()>> {
        self.grid_wake_rx.take()
    }

    pub fn update_font_config(&mut self, font_config: FontConfig) {
        self.font_config = font_config;
        self.pending_font_update = true;
        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn scroll(&self, scroll: Scroll) {
        let mut term = self.term.lock().unwrap();
        term.scroll_display(scroll);
    }
}

// ─── TerminalView ─────────────────────────────────────────────────────────────

const TERMINAL_FONT_SIZE_PX: f32 = 14.0;
const TERMINAL_LINE_HEIGHT_FACTOR: f32 = 1.30;
const TERMINAL_ROW_EXTRA_PADDING_PX: f32 = 2.0;

pub struct TerminalView {
    terminal: Entity<Terminal>,
    focus_handle: FocusHandle,
    cursor_blink_visible: bool,
    blink_enabled: bool,
    blink_resume_token: u64,
    last_render_generation: u64,
    last_blink_instant: std::time::Instant,
    cached_pack: Option<TerminalPaintPack>,
    pending_notify: bool,
    pending_font_update: bool,
    cached_cell_w_px: f32,
    cached_line_h_px: f32,
    cached_writer: PtyWriter,
    cached_display_offset: usize,
    app_cursor_keys: bool,
    alt_screen: bool,
    scrollbar_drag_state: Option<ScrollbarDragState>,
}

struct ScrollbarDragState {
    start_y: gpui::Pixels,
    start_offset: usize,
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TerminalView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, terminal: Entity<Terminal>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        let render_generation = terminal.read(cx).render_generation.clone();

        let cached_writer = terminal.read(cx).pty_writer.clone();
        let (app_cursor_keys, alt_screen) = {
            let term = terminal.read(cx);
            let guard = term.term.lock().unwrap();
            let mode = guard.mode();
            (
                mode.contains(TermMode::APP_CURSOR),
                mode.contains(TermMode::ALT_SCREEN),
            )
        };

        let mut view = Self {
            terminal,
            focus_handle,
            cursor_blink_visible: true,
            blink_enabled: true,
            blink_resume_token: 0,
            last_render_generation: render_generation.load(Ordering::Relaxed),
            last_blink_instant: std::time::Instant::now(),
            cached_pack: None,
            pending_notify: false,
            pending_font_update: false,
            cached_cell_w_px: 0.0,
            cached_line_h_px: 0.0,
            cached_writer,
            cached_display_offset: 0,
            app_cursor_keys,
            alt_screen,
            scrollbar_drag_state: None,
        };

        if let Some(rx) = view
            .terminal
            .update(cx, |terminal, _| terminal.take_grid_wake_receiver())
        {
            view.start_grid_wake_listener(cx, rx);
        }

        view.start_blink_timer(cx);

        view
    }

    pub fn update_font_config(&mut self, font_config: FontConfig, cx: &mut Context<Self>) {
        self.terminal.update(cx, |terminal, _| {
            terminal.update_font_config(font_config);
        });
        self.pending_font_update = true;
        self.cached_cell_w_px = 0.0;
        self.cached_line_h_px = 0.0;
        self.cached_pack = None;
        cx.notify();
    }

    fn start_blink_timer(&mut self, cx: &mut Context<Self>) {
        self.blink_enabled = true;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(530))
                .await;

            let updated = this
                .update(cx, |this, cx| {
                    this.toggle_blink();
                    cx.notify();
                })
                .is_ok();

            if !updated {
                break;
            }
        })
        .detach();
    }

    fn toggle_blink(&mut self) {
        if self.blink_enabled {
            self.cursor_blink_visible = !self.cursor_blink_visible;
        }
    }

    fn start_grid_wake_listener(&mut self, cx: &mut Context<Self>, rx: flume::Receiver<()>) {
        cx.spawn(async move |this, cx| {
            while rx.recv_async().await.is_ok() {
                let updated = this
                    .update(cx, |this, _| {
                        this.cursor_blink_visible = true;
                        this.blink_enabled = true;
                        this.pending_notify = true;
                    })
                    .is_ok();

                if !updated {
                    break;
                }
            }
        })
        .detach();
    }

    fn pause_blinking(&mut self, cx: &mut Context<Self>) {
        self.cursor_blink_visible = true;
        self.blink_enabled = false;
        self.blink_resume_token = self.blink_resume_token.wrapping_add(1);
        self.last_blink_instant = std::time::Instant::now();
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.cursor_blink_visible = true;
        self.blink_enabled = false;
        cx.notify();

        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();

        // Handle Ctrl+C - must check modifier
        if keystroke.modifiers.control && key.eq_ignore_ascii_case("c") {
            if let Err(e) = self.cached_writer.write(&[0x03]) {
                tracing::error!("PTY write error: {}", e);
            }
            return;
        }

        // Handle Ctrl+D - similar
        if keystroke.modifiers.control && key.eq_ignore_ascii_case("d") {
            if let Err(e) = self.cached_writer.write(&[0x04]) {
                tracing::error!("PTY write error: {}", e);
            }
            return;
        }

        // Handle Ctrl+Z - suspend
        if keystroke.modifiers.control && key.eq_ignore_ascii_case("z") {
            if let Err(e) = self.cached_writer.write(&[0x1A]) {
                tracing::error!("PTY write error: {}", e);
            }
            return;
        }

        // Handle Ctrl+L - clear screen
        if keystroke.modifiers.control && key.eq_ignore_ascii_case("l") {
            if let Err(e) = self.cached_writer.write(&[0x0C]) {
                tracing::error!("PTY write error: {}", e);
            }
            return;
        }

        let bytes = if key.eq_ignore_ascii_case("enter")
            || key.eq_ignore_ascii_case("return")
            || key.eq_ignore_ascii_case("numpadenter")
        {
            vec![b'\r']
        } else if keystroke
            .key_char
            .as_ref()
            .is_some_and(|s| s == "\r" || s == "\n")
        {
            vec![b'\r']
        } else if key.eq_ignore_ascii_case("space") {
            vec![b' ']
        } else if key.eq_ignore_ascii_case("backspace") {
            vec![0x7F]
        } else if let Some(ref c) = keystroke.key_char {
            c.as_bytes().to_vec()
        } else {
            let bytes = key_to_bytes(key, &keystroke.modifiers, self.app_cursor_keys);
            if let Err(e) = self.cached_writer.write(&bytes) {
                tracing::error!("PTY write error: {}", e);
            }
            return;
        };

        if !bytes.is_empty() {
            if let Err(e) = self.cached_writer.write(&bytes) {
                tracing::error!("PTY write error: {}", e);
            }
        }
    }

    fn on_scroll_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta_rows = match event.delta {
            gpui::ScrollDelta::Pixels(_) => return,
            gpui::ScrollDelta::Lines(l) => l.y as isize,
        };

        if delta_rows == 0 {
            return;
        }

        if self.alt_screen {
            let bytes = if delta_rows > 0 {
                vec![0x1B, 0x5B, 0x35, 0x7E]
            } else {
                vec![0x1B, 0x5B, 0x36, 0x7E]
            };
            self.cached_writer.write(&bytes).ok();
            return;
        }

        self.terminal.update(cx, |term, _| {
            term.scroll(Scroll::Delta(delta_rows as i32));
        });
    }

    fn on_mouse_move(
        &mut self,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(drag) = &self.scrollbar_drag_state {
            let total_lines = self
                .cached_pack
                .as_ref()
                .map(|p| p.total_lines)
                .unwrap_or(0);
            let screen_lines = self
                .cached_pack
                .as_ref()
                .map(|p| p.screen_lines)
                .unwrap_or(0);
            if total_lines > screen_lines && drag.start_offset != 0 {
                let bounds = window.bounds();
                let scrollable_height: f32 = bounds.size.height.into();
                let thumb_height: f32 =
                    ((screen_lines as f32 / total_lines as f32) * scrollable_height).max(30.0);
                let thumb_y_offset: f32 = (drag.start_offset as f32
                    / (total_lines - screen_lines).max(1) as f32)
                    * (scrollable_height - thumb_height);
                let max_thumb_y = scrollable_height - thumb_height;
                let relative_y: f32 =
                    (event.position.y - bounds.origin.y - gpui::px(thumb_y_offset)).into();
                let relative_y = relative_y.max(0.0).min(max_thumb_y);
                let new_offset = ((relative_y / (scrollable_height - thumb_height))
                    * (total_lines - screen_lines) as f32)
                    .round() as usize;
                let delta = new_offset as isize - drag.start_offset as isize;
                if delta != 0 {
                    self.terminal.update(cx, |term, _| {
                        term.scroll(Scroll::Delta(delta as i32));
                    });
                }
            }
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scrollbar_width = gpui::px(6.0);
        let scrollbar_x = window.bounds().size.width - scrollbar_width;
        if event.position.x >= scrollbar_x {
            if let Some(pack) = &self.cached_pack {
                let total_lines = pack.total_lines;
                let screen_lines = pack.screen_lines;
                if total_lines > screen_lines {
                    let bounds = window.bounds();
                    let scrollable_height: f32 = bounds.size.height.into();
                    let thumb_height: f32 =
                        ((screen_lines as f32 / total_lines as f32) * scrollable_height).max(30.0);
                    let thumb_y_offset: f32 = (pack.display_offset as f32
                        / (total_lines - screen_lines).max(1) as f32)
                        * (scrollable_height - thumb_height);
                    let click_relative_y: f32 = (event.position.y - bounds.origin.y).into();
                    if click_relative_y >= thumb_y_offset
                        && click_relative_y <= thumb_y_offset + thumb_height
                    {
                        self.scrollbar_drag_state = Some(ScrollbarDragState {
                            start_y: event.position.y,
                            start_offset: pack.display_offset,
                        });
                    } else {
                        let click_in_upper_half =
                            click_relative_y < thumb_y_offset + thumb_height / 2.0;
                        let jump_lines = if click_in_upper_half {
                            -(screen_lines as i32 / 2)
                        } else {
                            screen_lines as i32 / 2
                        };
                        self.terminal.update(cx, |term, _| {
                            term.scroll(Scroll::Delta(jump_lines));
                        });
                    }
                }
            }
        }
    }

    fn on_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.scrollbar_drag_state = None;
    }
}

// ─── Rendering helpers ────────────────────────────────────────────────────────

fn rgb_to_hsla(r: u8, g: u8, b: u8) -> gpui::Hsla {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    if (max - min).abs() < 0.001 {
        gpui::Hsla {
            h: 0.0,
            s: 0.0,
            l: lightness,
            a: 1.0,
        }
    } else {
        let d = max - min;
        let saturation = if lightness > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let hue = if (max - r).abs() < 0.001 {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if (max - g).abs() < 0.001 {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        gpui::Hsla {
            h: hue / 6.0,
            s: saturation,
            l: lightness,
            a: 1.0,
        }
    }
}

fn cell_fg_to_hsla(fg: Color, flags: Flags, _default: gpui::Hsla) -> gpui::Hsla {
    match fg {
        Color::Spec(rgb) => rgb_to_hsla(rgb.r, rgb.g, rgb.b),
        Color::Named(named) => {
            let (r, g, b) = named_color_rgb(named);
            let mut hsla = rgb_to_hsla(r, g, b);
            if flags.contains(Flags::DIM) {
                hsla.l = (hsla.l * 0.55).max(0.20);
            }
            hsla
        }
        Color::Indexed(idx) => {
            let rgb = index_to_ansi_color(idx as usize);
            rgb_to_hsla(rgb.0, rgb.1, rgb.2)
        }
    }
}

fn cell_bg_to_hsla(bg: Color, is_inverse: bool, default_fg: gpui::Hsla) -> gpui::Hsla {
    if is_inverse {
        // When INVERSE flag is set, background becomes the foreground color
        // Foreground is default light gray in fasty
        default_fg
    } else {
        match bg {
            Color::Spec(rgb) => rgb_to_hsla(rgb.r, rgb.g, rgb.b),
            Color::Named(named) => {
                let (r, g, b) = named_color_rgb(named);
                rgb_to_hsla(r, g, b)
            }
            Color::Indexed(idx) => {
                let rgb = index_to_ansi_color(idx as usize);
                rgb_to_hsla(rgb.0, rgb.1, rgb.2)
            }
        }
    }
}

fn named_color_rgb(named: NamedColor) -> (u8, u8, u8) {
    match named {
        NamedColor::Foreground => (0xD8, 0xD8, 0xD8),
        NamedColor::Background => (0x18, 0x18, 0x18),
        NamedColor::Black => (0x18, 0x18, 0x18),
        NamedColor::Red => (0xAC, 0x42, 0x42),
        NamedColor::Green => (0x90, 0xA9, 0x59),
        NamedColor::Yellow => (0xF4, 0xBF, 0x75),
        NamedColor::Blue => (0x6A, 0x9F, 0xB5),
        NamedColor::Magenta => (0xAA, 0x75, 0x9F),
        NamedColor::Cyan => (0x75, 0xB5, 0xAA),
        NamedColor::White => (0xD8, 0xD8, 0xD8),
        NamedColor::BrightBlack => (0x6B, 0x6B, 0x6B),
        NamedColor::BrightRed => (0xC5, 0x55, 0x55),
        NamedColor::BrightGreen => (0xAA, 0xC4, 0x74),
        NamedColor::BrightYellow => (0xFE, 0xCA, 0x88),
        NamedColor::BrightBlue => (0x82, 0xB8, 0xC8),
        NamedColor::BrightMagenta => (0xC2, 0x8C, 0xB8),
        NamedColor::BrightCyan => (0x93, 0xD3, 0xC3),
        NamedColor::BrightWhite => (0xF8, 0xF8, 0xF8),
        _ => (0xD8, 0xD8, 0xD8),
    }
}

fn index_to_ansi_color(idx: usize) -> (u8, u8, u8) {
    if idx < 16 {
        const ANSI_COLORS: [(u8, u8, u8); 16] = [
            (0x00, 0x00, 0x00),
            (0xCD, 0x00, 0x00),
            (0x00, 0xCD, 0x00),
            (0xCD, 0xCD, 0x00),
            (0x00, 0x00, 0xEE),
            (0xCD, 0x00, 0xCD),
            (0x00, 0xCD, 0xCD),
            (0xE5, 0xE5, 0xE5),
            (0x7F, 0x7F, 0x7F),
            (0xFF, 0x00, 0x00),
            (0x00, 0xFF, 0x00),
            (0xFF, 0xFF, 0x00),
            (0x00, 0x00, 0xFF),
            (0xFF, 0x00, 0xFF),
            (0x00, 0xFF, 0xFF),
            (0xFF, 0xFF, 0xFF),
        ];
        ANSI_COLORS[idx]
    } else if idx < 232 {
        let idx = idx - 16;
        (
            ((idx / 36) * 51) as u8,
            (((idx / 6) % 6) * 51) as u8,
            ((idx % 6) * 51) as u8,
        )
    } else {
        let v = (((idx - 232) * 10 + 8) as u8).min(255);
        (v, v, v)
    }
}

struct TerminalPaintPack {
    render_generation: u64,
    shaped_rows: Vec<gpui::ShapedLine>,
    cell_backgrounds: Vec<Vec<(usize, usize, gpui::Hsla)>>,
    rows_to_use: usize,
    cursor_utf8_byte: Option<usize>,
    text_metrics_height_px: f32,
    line_height_px: f32,
    line_height: gpui::Pixels,
    cell_width: gpui::Pixels,
    bg_color: gpui::Hsla,
    cursor_visible: bool,
    blink_visible: bool,
    cursor_screen_row: Option<usize>,
    cursor_col: usize,
    cursor_color: gpui::Hsla,
    display_offset: usize,
    total_lines: usize,
    screen_lines: usize,
}

// ─── Render ───────────────────────────────────────────────────────────────────

impl Render for TerminalView {
    fn render(
        &mut self,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let term_entity = self.terminal.clone();
        let blink_visible = self.cursor_blink_visible;
        let current_render_gen = self
            .terminal
            .read(cx)
            .render_generation
            .load(Ordering::Relaxed);
        let should_render = self.pending_notify
            || self.pending_font_update
            || current_render_gen != self.last_render_generation
            || self.cached_pack.is_none();

        let display_offset = self.terminal.read(cx).term.lock().unwrap().grid().display_offset();
        let display_offset_changed = display_offset != self.cached_display_offset;

        if should_render {
            self.pending_notify = false;
            self.pending_font_update = false;
            self.last_render_generation = current_render_gen;
            self.cached_display_offset = display_offset;
        }

        if !should_render && !display_offset_changed && self.cached_pack.is_some() {
            return div().size_full().into_element();
        }

        let last_render_gen = self.last_render_generation;
        let pending_notify_flag = self.pending_notify;
        let cached_pack_exists = self.cached_pack.is_some();
        let cached_cell_w_px = self.cached_cell_w_px;
        let cached_line_h_px = self.cached_line_h_px;

        div()
            .key_context("terminal")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(
                canvas(
                    move |bounds, window, cx_app: &mut App| {
                        let current_render_gen = term_entity
                            .read(cx_app)
                            .render_generation
                            .load(Ordering::Relaxed);
                        let should_skip = !pending_notify_flag
                            && current_render_gen == last_render_gen
                            && cached_pack_exists;

                        if should_skip {
                            return None;
                        }

                        let font_config = term_entity.read(cx_app).font_config.clone();
                        let font_size = px(font_config.size);
                        // Configure font with ligatures based on config
                        let ligature_features = if font_config.ligatures {
                            vec![("calt".into(), 1), ("liga".into(), 1)]
                        } else {
                            vec![]
                        };
                        let mono = Font {
                            family: font_config.family.into(),
                            features: FontFeatures(Arc::new(ligature_features)),
                            weight: gpui::FontWeight::default(),
                            style: gpui::FontStyle::default(),
                            fallbacks: None,
                        };

                        let cell_w_px;
                        let probe_line_height_px;

                        // Font metrics are constant - only compute once
                        if cached_cell_w_px == 0.0 {
                            let probe_run = gpui::TextRun {
                                len: 1,
                                font: mono.clone(),
                                color: rgb_to_hsla(0xEE, 0xEE, 0xEE),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let cell_probe = window.text_system().shape_line(
                                "M".into(),
                                font_size,
                                &[probe_run],
                                None,
                            );
                            cell_w_px = f32::from(cell_probe.width).max(1.0);
                            probe_line_height_px =
                                (f32::from(cell_probe.ascent + cell_probe.descent)
                                    * TERMINAL_LINE_HEIGHT_FACTOR)
                                    + TERMINAL_ROW_EXTRA_PADDING_PX;
                        } else {
                            cell_w_px = cached_cell_w_px;
                            probe_line_height_px = cached_line_h_px;
                        }

                        let w = f32::from(bounds.size.width);
                        let h = f32::from(bounds.size.height);

                        let cols = ((w / cell_w_px).floor() as usize).max(80);
                        let rows = ((h / probe_line_height_px).floor() as usize).max(24);

                        cx_app.update_entity(&term_entity, move |term, _| {
                            if cols != term.last_cols.get() || rows != term.last_rows.get() {
                                term.last_cols.set(cols);
                                term.last_rows.set(rows);
                                term.resize(cols, rows);
                            }
                        });

                        let (visible, cursor_info, _term_mode) = {
                            let term_read = term_entity.read(cx_app);
                            let term = term_read.term.lock().unwrap();
                            let display_offset = term.grid().display_offset();
                            let screen_lines = term.grid().screen_lines();
                            let cols = term.grid().columns();
                            let total = term.grid().total_lines();

                            let visible: Vec<Vec<(char, Color, Color, Flags)>> = (0..screen_lines)
                                .filter_map(|row| {
                                    let line_idx = -(display_offset as i32) + row as i32;
                                    let screen = screen_lines as i32;

                                    let in_scrollback = display_offset > 0
                                        && line_idx >= -(display_offset as i32)
                                        && line_idx < 0;
                                    let in_screen = line_idx >= 0 && line_idx < screen;

                                    if !in_scrollback && !in_screen {
                                        return None;
                                    }

                                    let line = Line(line_idx);
                                    let mut cells: Vec<(char, Color, Color, Flags)> =
                                        Vec::with_capacity(cols);
                                    for col in 0..cols {
                                        let point = Point::new(line, Column(col));
                                        let cell = &term.grid()[point];
                                        // Skip WIDE_CHAR_SPACER cells - these are trailing cells of double-width chars
                                        if cell.flags.intersects(
                                            Flags::WIDE_CHAR_SPACER
                                                | Flags::LEADING_WIDE_CHAR_SPACER,
                                        ) {
                                            continue;
                                        }
                                        cells.push((cell.c, cell.fg, cell.bg, cell.flags));
                                    }
                                    Some(cells)
                                })
                                .collect();

                            let cursor = &term.grid().cursor;
                            let cursor_visible = term.mode().contains(TermMode::SHOW_CURSOR);
                            let mut cursor_point = cursor.point;
                            let cursor_screen_row = if cursor_point.line
                                >= Line(-(display_offset as i32))
                                && cursor_point.line
                                    < Line(-(display_offset as i32) + screen_lines as i32)
                            {
                                let line_idx: i32 = (*cursor_point.line).into();
                                Some((line_idx + display_offset as i32) as usize)
                            } else {
                                None
                            };

                            // If cursor is on a WIDE_CHAR_SPACER cell, adjust it back to the leading cell
                            if cursor_point.column.0 > 0 {
                                let prev_col = Column(cursor_point.column.0 - 1);
                                let prev_cell =
                                    &term.grid()[Point::new(cursor_point.line, prev_col)];
                                if prev_cell.flags.contains(Flags::WIDE_CHAR) {
                                    cursor_point.column = prev_col;
                                }
                            }

                            (
                                visible,
                                (
                                    cursor_visible,
                                    cursor_point.column.0,
                                    cursor_screen_row,
                                    display_offset,
                                    total,
                                    screen_lines,
                                ),
                                term.mode().bits(),
                            )
                        };

                        let (
                            cursor_visible,
                            cursor_col_physical,
                            cursor_screen_row,
                            display_offset,
                            total_lines,
                            screen_lines,
                        ) = cursor_info;

                        let row_data: Vec<(
                            gpui::ShapedLine,
                            Vec<(usize, usize, gpui::Hsla)>,
                            Option<usize>,
                        )> = visible
                            .iter()
                            .map(|row| {
                                let mut row_text = String::new();
                                let mut runs: Vec<gpui::TextRun> = Vec::new();
                                let mut cell_backgrounds: Vec<(usize, usize, gpui::Hsla)> =
                                    Vec::new();
                                let mut current_run_start_byte = 0;
                                let mut current_fg = rgb_to_hsla(0xD8, 0xD8, 0xD8);
                                let mut current_bold = false;
                                let mut current_underline = false;
                                let mut current_strikethrough = false;
                                let default_bg = rgb_to_hsla(0x18, 0x18, 0x18);

                                let mut row_ci = 0;
                                for &(c, fg, cell_bg_color, flags) in row.iter() {
                                    // ci is the filtered cell index for text runs and background positions
                                    let ci = row_ci;
                                    // Determine if this cell has inverse applied
                                    let is_inverse = flags.contains(Flags::INVERSE);

                                    // When inverse, swap fg and bg colors for proper rendering
                                    let (visual_fg, visual_bg) = if is_inverse {
                                        (cell_bg_color, fg)
                                    } else {
                                        (fg, cell_bg_color)
                                    };

                                    // Background color for this cell
                                    let cell_bg = cell_bg_to_hsla(visual_bg, false, default_bg);

                                    // Calculate foreground color
                                    let fg_hsla = if flags.contains(Flags::HIDDEN) {
                                        // Hidden: show as background (concealed)
                                        default_bg
                                    } else {
                                        cell_fg_to_hsla(visual_fg, flags, current_fg)
                                    };

                                    let run_changed = fg_hsla != current_fg
                                        || flags.contains(Flags::BOLD) != current_bold
                                        || flags.contains(Flags::UNDERLINE) != current_underline
                                        || flags.contains(Flags::STRIKEOUT)
                                            != current_strikethrough;
                                    if run_changed && !row_text.is_empty() {
                                        let run_len = row_text.len() - current_run_start_byte;
                                        if run_len > 0 {
                                            runs.push(gpui::TextRun {
                                                len: run_len,
                                                font: mono.clone(),
                                                color: current_fg,
                                                background_color: None,
                                                underline: None,
                                                strikethrough: None,
                                            });
                                        }
                                        current_run_start_byte = row_text.len();
                                        current_fg = fg_hsla;
                                        current_bold = flags.contains(Flags::BOLD);
                                        current_underline = flags.contains(Flags::UNDERLINE);
                                        current_strikethrough = flags.contains(Flags::STRIKEOUT);
                                    }

                                    if cell_bg != default_bg {
                                        if let Some(last) = cell_backgrounds.last_mut() {
                                            if last.2 == cell_bg && last.1 == ci {
                                                last.1 = ci + 1;
                                            } else {
                                                cell_backgrounds.push((ci, ci + 1, cell_bg));
                                            }
                                        } else {
                                            cell_backgrounds.push((ci, ci + 1, cell_bg));
                                        }
                                    }

                                    row_text.push(c);
                                    row_ci += 1;
                                }

                                if !row_text.is_empty() {
                                    let run_len = row_text.len() - current_run_start_byte;
                                    if run_len > 0 {
                                        runs.push(gpui::TextRun {
                                            len: run_len,
                                            font: mono.clone(),
                                            color: current_fg,
                                            background_color: None,
                                            underline: None,
                                            strikethrough: None,
                                        });
                                    }
                                }

                                let cc = cursor_col_physical.min(cols);
                                let cursor_utf8_byte = match cursor_screen_row {
                                    Some(cr) if row == &visible[cr] => {
                                        row_text.char_indices().nth(cc).map(|(byte, _)| byte)
                                    }
                                    _ => None,
                                };

                                let shaped_line = window.text_system().shape_line(
                                    if row_text.is_empty() {
                                        gpui::SharedString::from("")
                                    } else {
                                        gpui::SharedString::from(row_text)
                                    },
                                    font_size,
                                    &runs,
                                    None,
                                );

                                (shaped_line, cell_backgrounds, cursor_utf8_byte)
                            })
                            .collect();

                        let shaped_rows: Vec<_> =
                            row_data.iter().map(|(sl, _, _)| sl.clone()).collect();
                        let cell_backgrounds: Vec<_> =
                            row_data.iter().map(|(_, bgs, _)| bgs.clone()).collect();
                        let rows_to_use = visible.len().min(shaped_rows.len());
                        let cursor_utf8_byte = cursor_screen_row
                            .and_then(|cr| row_data.get(cr))
                            .and_then(|(_, _, cursor_byte)| *cursor_byte);
                        let text_metrics_height_px = if !shaped_rows.is_empty() {
                            let first = &shaped_rows[0];
                            f32::from(first.ascent + first.descent)
                        } else {
                            14.0
                        };
                        let line_height_px = (text_metrics_height_px * TERMINAL_LINE_HEIGHT_FACTOR)
                            + TERMINAL_ROW_EXTRA_PADDING_PX;
                        let line_height = px(line_height_px);
                        let cell_width = px(cell_w_px);

                        if visible.is_empty() || shaped_rows.is_empty() {
                            return Some(TerminalPaintPack {
                                render_generation: 0,
                                shaped_rows: vec![],
                                cell_backgrounds: vec![],
                                rows_to_use: 0,
                                cursor_utf8_byte: None,
                                text_metrics_height_px,
                                line_height_px,
                                line_height,
                                cell_width,
                                bg_color: rgb_to_hsla(0x1E, 0x1E, 0x1E),
                                cursor_visible: false,
                                blink_visible: false,
                                cursor_screen_row: None,
                                cursor_col: 0,
                                cursor_color: rgb_to_hsla(0xFF, 0xFF, 0xFF),
                                display_offset: 0,
                                total_lines: 0,
                                screen_lines: 0,
                            });
                        }

                        let pack = Some(TerminalPaintPack {
                            render_generation: current_render_gen,
                            shaped_rows,
                            cell_backgrounds,
                            rows_to_use,
                            cursor_utf8_byte,
                            text_metrics_height_px,
                            line_height_px,
                            line_height,
                            cell_width,
                            bg_color: rgb_to_hsla(0x1E, 0x1E, 0x1E),
                            cursor_visible,
                            blink_visible,
                            cursor_screen_row,
                            cursor_col: cursor_col_physical,
                            cursor_color: rgb_to_hsla(0xFF, 0xFF, 0xFF),
                            display_offset,
                            total_lines,
                            screen_lines,
                        });

                        pack
                    },
                    move |bounds, pack: Option<TerminalPaintPack>, window, canvas_cx| {
                        let Some(pack) = pack else {
                            return;
                        };
                        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                            let padding_x = px(1.0);
                            let bg_fill = fill(bounds, pack.bg_color);
                            window.paint_quad(bg_fill);

                            let content_x = bounds.origin.x + padding_x;
                            let mut y = bounds.origin.y;
                            for ri in 0..pack.rows_to_use {
                                if ri < pack.shaped_rows.len() {
                                    let row = &pack.shaped_rows[ri];
                                    if row.len() > 0 {
                                        if ri < pack.cell_backgrounds.len() {
                                            for &(start_cell, end_cell, bg_color) in
                                                &pack.cell_backgrounds[ri]
                                            {
                                                let bg_x =
                                                    content_x + pack.cell_width * start_cell as f32;
                                                let bg_w = pack.cell_width
                                                    * (end_cell - start_cell) as f32;
                                                let bg_bounds = gpui::Bounds::new(
                                                    gpui::point(bg_x, y),
                                                    gpui::size(bg_w, pack.line_height),
                                                );
                                                let bg_fill = fill(bg_bounds, bg_color);
                                                window.paint_quad(bg_fill);
                                            }
                                        }
                                        let _ = row.paint(
                                            gpui::point(content_x, y),
                                            pack.line_height,
                                            window,
                                            canvas_cx,
                                        );
                                    }
                                }
                                y = y + pack.line_height;
                            }

                            if pack.cursor_visible && pack.blink_visible {
                                if let Some(cs_row) = pack.cursor_screen_row {
                                    let cursor_x_offset = if cs_row < pack.shaped_rows.len() {
                                        let row = &pack.shaped_rows[cs_row];
                                        if let Some(cursor_byte) = pack.cursor_utf8_byte {
                                            row.x_for_index(cursor_byte)
                                        } else {
                                            pack.cell_width * pack.cursor_col
                                        }
                                    } else {
                                        pack.cell_width * pack.cursor_col
                                    };
                                    let cursor_x = content_x + cursor_x_offset;
                                    let cursor_y = bounds.origin.y
                                        + gpui::px(cs_row as f32 * pack.line_height_px);
                                    let cursor_h = gpui::px(
                                        pack.text_metrics_height_px.max(TERMINAL_FONT_SIZE_PX),
                                    );
                                    let cursor_w = gpui::px(1.5);
                                    let cursor_y = cursor_y
                                        + gpui::px(
                                            (pack.line_height_px - f32::from(cursor_h)).max(0.0)
                                                / 2.0,
                                        );

                                    let cursor_bounds = gpui::Bounds::new(
                                        gpui::point(cursor_x, cursor_y),
                                        gpui::size(cursor_w, cursor_h),
                                    );
                                    let cursor_fill = fill(cursor_bounds, pack.cursor_color);
                                    window.paint_quad(cursor_fill);
                                }
                            }

                            if pack.total_lines > pack.screen_lines {
                                let scrollbar_width = gpui::px(6.0);
                                let scrollbar_x =
                                    bounds.origin.x + bounds.size.width - scrollbar_width;
                                let scrollbar_bounds = gpui::Bounds::new(
                                    gpui::point(scrollbar_x, bounds.origin.y),
                                    gpui::size(scrollbar_width, bounds.size.height),
                                );
                                let scrollbar_bg_fill = fill(
                                    scrollbar_bounds,
                                    gpui::Hsla {
                                        h: 0.0,
                                        s: 0.0,
                                        l: 0.0,
                                        a: 0.15,
                                    },
                                );
                                window.paint_quad(scrollbar_bg_fill);

                                let scrollable_height: f32 = bounds.size.height.into();
                                let thumb_height: f32 = ((pack.screen_lines as f32
                                    / pack.total_lines as f32)
                                    * scrollable_height)
                                    .max(30.0);
                                let thumb_y_offset: gpui::Pixels = gpui::px(
                                    (pack.display_offset as f32
                                        / (pack.total_lines.saturating_sub(pack.screen_lines))
                                            .max(1)
                                            as f32)
                                        * (scrollable_height - thumb_height),
                                );

                                let thumb_bounds = gpui::Bounds::new(
                                    gpui::point(scrollbar_x, bounds.origin.y + thumb_y_offset),
                                    gpui::size(scrollbar_width, gpui::px(thumb_height)),
                                );
                                let thumb_fill = fill(
                                    thumb_bounds,
                                    gpui::Hsla {
                                        h: 0.0,
                                        s: 0.0,
                                        l: 0.4,
                                        a: 0.6,
                                    },
                                );
                                window.paint_quad(thumb_fill);
                            }
                        });
                    },
                )
                .size_full(),
            )
            .on_key_down(cx.listener(
                |this: &mut Self,
                 event: &gpui::KeyDownEvent,
                 window: &mut Window,
                 cx: &mut gpui::Context<Self>| {
                    this.on_key_down(event, window, cx);
                },
            ))
            .on_scroll_wheel(cx.listener(
                |this: &mut Self,
                 event: &gpui::ScrollWheelEvent,
                 window: &mut Window,
                 cx: &mut gpui::Context<Self>| {
                    this.on_scroll_wheel(event, window, cx);
                },
            ))
            .on_mouse_move(cx.listener(
                |this: &mut Self,
                 event: &gpui::MouseMoveEvent,
                 window: &mut Window,
                 cx: &mut gpui::Context<Self>| {
                    this.on_mouse_move(event, window, cx);
                },
            ))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(
                    |this: &mut Self,
                     event: &gpui::MouseDownEvent,
                     window: &mut Window,
                     cx: &mut gpui::Context<Self>| {
                        this.on_mouse_down(event, window, cx);
                    },
                ),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(
                    |this: &mut Self,
                     event: &gpui::MouseUpEvent,
                     window: &mut Window,
                     cx: &mut gpui::Context<Self>| {
                        this.on_mouse_up(event, window, cx);
                    },
                ),
            )
    }
}
