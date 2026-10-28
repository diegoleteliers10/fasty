//! Custom EventListener for alacritty_terminal that forwards PtyWrite events
//! and emits OSC-derived events (clipboard, cwd) via the event sender.

use alacritty_terminal::event::{Event, EventListener};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use parking_lot::Mutex;
use crate::terminal_state::AppEvent;

/// Base64 encoder for OSC 52 clipboard responses.
pub fn base64_encode(input: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}

/// Lazy clipboard helper. Returns None if clipboard init fails.
pub fn clipboard_helper() -> Option<arboard::Clipboard> {
    arboard::Clipboard::new().ok()
}

pub type EventCallback = Arc<dyn Fn(AppEvent) + Send + Sync>;

#[derive(Clone)]
#[derive(Default)]
pub enum EventSender {
    Callback(EventCallback),
    #[default]
    None,
}

impl EventSender {
    pub fn send(&self, event: AppEvent) {
        match self {
            Self::Callback(cb) => cb(event),
            Self::None => {}
        }
    }
}


#[derive(Clone)]
pub struct EventListenerProxy {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    app_sender: Arc<Mutex<EventSender>>,
    replies_enabled: Arc<AtomicBool>,
}

unsafe impl Send for EventListenerProxy {}
unsafe impl Sync for EventListenerProxy {}

impl EventListenerProxy {
    pub fn from_arc(writer: Arc<Mutex<Box<dyn Write + Send>>>) -> Self {
        Self {
            writer,
            app_sender: Arc::new(Mutex::new(EventSender::None)),
            replies_enabled: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Disable `PtyWrite` and `ColorRequest` replies for daemon-attached
    /// terminals. The session host parses the same output stream and answers
    /// every terminal query once; a second reply from the attached client
    /// arrives while no reader consumes it and aborts prompt libraries
    /// (`gh`, `survey`) with "unexpected escape sequence".
    pub fn set_replies_enabled(&self, enabled: bool) {
        self.replies_enabled.store(enabled, Ordering::Release);
    }

    fn replies_enabled(&self) -> bool {
        self.replies_enabled.load(Ordering::Acquire)
    }

    pub fn set_event_sender(&self, sender: EventSender) {
        *self.app_sender.lock() = sender;
    }

    pub fn send_app_event(&self, event: AppEvent) {
        self.app_sender.lock().send(event);
    }
}

impl EventListener for EventListenerProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(response) => {
                if !self.replies_enabled() {
                    return;
                }
                let mut w = self.writer.lock();
                if w.write_all(response.as_bytes()).is_ok() {
                    let _ = w.flush();
                }
            }
            Event::Bell => {
                #[cfg(target_os = "linux")]
                {
                    use std::io::Write;
                    if let Ok(mut console) = std::fs::OpenOptions::new()
                        .write(true)
                        .open("/dev/console")
                    {
                        let _ = console.write_all(&[0x07]);
                    }
                }
                self.app_sender.lock().send(AppEvent::Bell);
            }
            Event::ClipboardStore(_ty, text) => {
                if let Some(mut ctx) = clipboard_helper() {
                    let _ = ctx.set_text(text);
                }
            }
            Event::ClipboardLoad(_ty, cb) => {
                if let Some(mut ctx) = clipboard_helper() {
                    if let Ok(text) = ctx.get_text() {
                        cb(&base64_encode(&text));
                    } else {
                        cb("");
                    }
                } else {
                    cb("");
                }
            }
            Event::Title(title) => {
                self.app_sender.lock().send(AppEvent::TitleChanged(title));
            }
            Event::ColorRequest(index, format) => {
                if !self.replies_enabled() {
                    return;
                }
                let theme_name = crate::config::ACTIVE_THEME.read().clone();
                let theme = crate::ui::theme::Theme::from_name(&theme_name);
                let hsla = match index {
                    0 => theme.black,
                    1 => theme.red,
                    2 => theme.green,
                    3 => theme.yellow,
                    4 => theme.blue,
                    5 => theme.magenta,
                    6 => theme.cyan,
                    7 => theme.white,
                    8 => theme.bright_black,
                    9 => theme.bright_red,
                    10 => theme.bright_green,
                    11 => theme.bright_yellow,
                    12 => theme.bright_blue,
                    13 => theme.bright_magenta,
                    14 => theme.bright_cyan,
                    15 => theme.bright_white,
                    256 => theme.foreground,
                    257 => theme.background,
                    258 => theme.cursor,
                    _ => theme.foreground,
                };
                let rgba: gpui::Rgba = hsla.into();
                let r = (rgba.r * 255.0).clamp(0.0, 255.0) as u8;
                let g = (rgba.g * 255.0).clamp(0.0, 255.0) as u8;
                let b = (rgba.b * 255.0).clamp(0.0, 255.0) as u8;
                let rgb = alacritty_terminal::vte::ansi::Rgb { r, g, b };
                let response = format(rgb);
                let mut w = self.writer.lock();
                if w.write_all(response.as_bytes()).is_ok() {
                    let _ = w.flush();
                }
            }
            _ => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::Rgb;

    #[derive(Default)]
    struct Sink(parking_lot::Mutex<Vec<u8>>);

    struct SinkHandle(Arc<Sink>);

    impl Write for SinkHandle {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 .0.lock().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn reply_suppression_drops_query_replies_for_attached_terminals() {
        let sink = Arc::new(Sink::default());
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(Box::new(SinkHandle(Arc::clone(&sink)))));
        let proxy = EventListenerProxy::from_arc(writer);
        proxy.set_replies_enabled(false);

        proxy.send_event(Event::PtyWrite("\x1b[31;56R".to_string()));
        let format_color = Arc::new(|rgb: Rgb| {
            format!("\x1b]11;rgb:{:02x}{:02x}{:02x}\x1b\\", rgb.r, rgb.g, rgb.b)
        });
        proxy.send_event(Event::ColorRequest(257, format_color));

        assert!(
            sink.0.lock().is_empty(),
            "attached terminals must not answer terminal queries"
        );

        proxy.set_replies_enabled(true);
        proxy.send_event(Event::PtyWrite("\x1b[31;56R".to_string()));
        assert_eq!(sink.0.lock().as_slice(), b"\x1b[31;56R");
    }
}
