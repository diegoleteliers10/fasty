//! Custom EventListener for alacritty_terminal that forwards PtyWrite events.

use alacritty_terminal::event::{Event, EventListener};
use std::io::Write;
use std::sync::Arc;
use parking_lot::Mutex;

#[derive(Clone)]
pub struct EventListenerProxy {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

unsafe impl Send for EventListenerProxy {}
unsafe impl Sync for EventListenerProxy {}

impl EventListenerProxy {
    pub fn new(writer: Box<dyn Write + Send>) -> Self {
        Self {
            writer: Arc::new(Mutex::new(writer)),
        }
    }

    pub fn from_arc(writer: Arc<Mutex<Box<dyn Write + Send>>>) -> Self {
        Self { writer }
    }
}

impl EventListener for EventListenerProxy {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(response) = event {
            let mut w = self.writer.lock();
            if w.write_all(response.as_bytes()).is_ok() {
                let _ = w.flush();
            }
        }
    }
}