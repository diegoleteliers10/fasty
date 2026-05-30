//! Terminal state wrapper using alacritty_terminal.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::Config as AlacrittyConfig;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use parking_lot::Mutex as ParkingMutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use crate::config::FontConfig;
use crate::event_listener::EventListenerProxy;

const PTY_READ_BUF_SIZE: usize = 65536;

pub struct TerminalState {
    term: Arc<ParkingMutex<alacritty_terminal::term::Term<EventListenerProxy>>>,
    render_generation: Arc<AtomicU64>,
    writer: Arc<ParkingMutex<Box<dyn Write + Send>>>,
    master: Arc<ParkingMutex<Box<dyn MasterPty + Send>>>,
    shell_pid: Option<u32>,
}

impl TerminalState {
    pub fn new(
        shell: &str,
        scrollback: usize,
        _font_config: FontConfig,
        cell_width: f32,
        cell_height: f32,
        viewport_width: f32,
        viewport_height: f32,
        proxy: winit::event_loop::EventLoopProxy<()>,
    ) -> anyhow::Result<Self> {
        let cell_w = (cell_width as usize).max(1);
        let cell_h = (cell_height as usize).max(1);
        let cols = ((viewport_width as usize) / cell_w).max(80);
        let rows = ((viewport_height as usize) / cell_h).max(24);

        let mut config = AlacrittyConfig::default();
        config.scrolling_history = scrollback;
        let size = TermSize::new(cols, rows);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: (cols as u16) * (cell_width as u16),
                pixel_height: (rows as u16) * (cell_height as u16),
            })
            .expect("Failed to open PTY");

        let mut cmd = CommandBuilder::new(shell);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        if let Ok(lang) = std::env::var("LANG") {
            cmd.env("LANG", lang);
        }
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }

        let child = pair.slave.spawn_command(cmd).expect("Failed to spawn shell");
        let shell_pid = child.process_id();

        drop(pair.slave);

        let master: Box<dyn MasterPty + Send> = pair.master;
        let master_arc: Arc<ParkingMutex<Box<dyn MasterPty + Send>>> =
            Arc::new(ParkingMutex::new(master));
        let mut reader = master_arc
            .lock()
            .try_clone_reader()
            .expect("Failed to clone reader");
        tracing::info!("Cloned PTY reader");
        let writer = master_arc.lock().take_writer().expect("Failed to take writer");
        tracing::info!("Took PTY writer");

        let writer_boxed: Box<dyn Write + Send> = Box::new(writer);
        let writer_arc: Arc<ParkingMutex<Box<dyn Write + Send>>> =
            Arc::new(ParkingMutex::new(writer_boxed));

        let event_listener = EventListenerProxy::from_arc(writer_arc.clone());
        let term = Arc::new(ParkingMutex::new(alacritty_terminal::term::Term::new(
            config,
            &size,
            event_listener,
        )));

        // Write echo command directly in main thread (already done above)
        // Remove duplicate write in thread

        let render_generation = Arc::new(AtomicU64::new(0));
        let render_gen_clone = Arc::clone(&render_generation);
        let term_clone = Arc::clone(&term);

        thread::spawn(move || {
            use std::io::Read;
            tracing::info!("PTY reader thread started");

            let mut buf = [0u8; 8192];
            let mut parser = Processor::new();
            loop {
                tracing::debug!("Calling read()...");
                match reader.read(&mut buf) {
                    Ok(0) => {
                        tracing::info!("PTY read returned 0 (EOF)");
                        break;
                    }
                    Ok(n) => {
                        let data = &buf[..n];
                        tracing::info!("PTY read {} bytes", n);
                        Self::process_chunk(&term_clone, &mut parser, &render_gen_clone, data);
                        let _ = proxy.send_event(());
                    }
                    Err(e) => {
                        tracing::error!("PTY read error: {}", e);
                        break;
                    }
                }
            }
            tracing::info!("PTY reader thread exiting");
        });

        Ok(Self {
            term,
            render_generation,
            writer: writer_arc,
            master: master_arc,
            shell_pid,
        })
    }

    pub fn shell_pid(&self) -> Option<u32> {
        self.shell_pid
    }

    fn process_chunk(
        term: &Arc<ParkingMutex<alacritty_terminal::term::Term<EventListenerProxy>>>,
        parser: &mut Processor<StdSyncHandler>,
        render_generation: &AtomicU64,
        chunk: &[u8],
    ) {
        if chunk.is_empty() {
            return;
        }

        let mut term_locked = term.lock();
        for byte in chunk {
            parser.advance(&mut *term_locked, *byte);
        }

        drop(term_locked);
        render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn write_to_pty(&mut self, bytes: &[u8]) {
        let mut w = self.writer.lock();
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    pub fn mark_dirty(&self) {
        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn update_scrollback(&self, scrollback: usize) {
        let mut term = self.term.lock();
        term.grid_mut().update_history(scrollback);
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 {
            return;
        }
        let mut term = self.term.lock();
        let size = TermSize::new(cols, rows);
        term.resize(size);
        drop(term);

        self.master.lock().resize(PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        }).ok();

        self.render_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn scroll(&self, delta: isize) {
        let mut term = self.term.lock();
        use alacritty_terminal::grid::Scroll;
        term.scroll_display(Scroll::Delta(delta as i32));
    }

    pub fn display_offset(&self) -> usize {
        let term = self.term.lock();
        term.grid().display_offset()
    }

    pub fn history_size(&self) -> usize {
        use alacritty_terminal::grid::Dimensions;
        let term = self.term.lock();
        term.history_size()
    }

    pub fn update_render_generation(&self, rg: &Arc<AtomicU64>) {
        let current = self.render_generation.load(Ordering::Relaxed);
        rg.store(current, Ordering::Relaxed);
    }

    pub fn term(&self) -> &Arc<ParkingMutex<alacritty_terminal::term::Term<EventListenerProxy>>> {
        &self.term
    }
}