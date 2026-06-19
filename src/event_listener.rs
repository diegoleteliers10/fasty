//! Custom EventListener for alacritty_terminal that forwards PtyWrite events
//! and emits OSC-derived events (clipboard, cwd) via the proxy channel.

use alacritty_terminal::event::{Event, EventListener};
use std::io::Write;
use std::sync::Arc;
use parking_lot::Mutex;
use crate::terminal_state::AppEvent;
use winit::event_loop::EventLoopProxy;

/// Base64 encoder for OSC 52 clipboard responses.
pub fn base64_encode(input: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
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

#[derive(Clone)]
pub struct EventListenerProxy {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    app_proxy: Option<EventLoopProxy<AppEvent>>,
}

unsafe impl Send for EventListenerProxy {}
unsafe impl Sync for EventListenerProxy {}

impl EventListenerProxy {
    pub fn from_arc(writer: Arc<Mutex<Box<dyn Write + Send>>>) -> Self {
        Self { writer, app_proxy: None }
    }

    pub fn set_app_proxy(&mut self, proxy: EventLoopProxy<AppEvent>) {
        self.app_proxy = Some(proxy);
    }
}

impl EventListener for EventListenerProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(response) => {
                let mut w = self.writer.lock();
                if w.write_all(response.as_bytes()).is_ok() {
                    let _ = w.flush();
                }
            }
            Event::Bell => {
                // Audible bell: write BEL to /dev/console for system beep
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
                if let Some(p) = &self.app_proxy {
                    let _ = p.send_event(AppEvent::Bell);
                }
            }
            Event::ClipboardStore(_ty, text) => {
                // Write directly to system clipboard
                if let Some(mut ctx) = clipboard_helper() {
                    let _ = ctx.set_text(text);
                }
            }
            Event::ClipboardLoad(_ty, cb) => {
                // Respond with system clipboard contents
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
                if let Some(p) = &self.app_proxy {
                    let _ = p.send_event(AppEvent::TitleChanged(title));
                }
            }
            _ => {}
        }
    }
}