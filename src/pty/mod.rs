//! PTY management using `portable-pty`
//!
//! Inspired by Ghostty's Command.zig -- spawns a shell subprocess
//! connected to a pseudo-terminal.

use std::io::{Read, Write as IoWrite};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

/// Read buffer size.
const PTY_READ_BUF_SIZE: usize = 65536;

pub struct PtyWriter {
    inner: Arc<Mutex<Box<dyn IoWrite + Send>>>,
}

unsafe impl Send for PtyWriter {}
unsafe impl Sync for PtyWriter {}

impl PtyWriter {
    pub fn write(&self, bytes: &[u8]) -> anyhow::Result<()> {
        let mut w = self.inner.lock();
        w.write_all(bytes)
            .map_err(|e| anyhow::anyhow!("PTY write error: {}", e))
    }

    pub fn inner_arc(&self) -> Arc<Mutex<Box<dyn IoWrite + Send>>> {
        Arc::clone(&self.inner)
    }

    pub fn from_writer_arc(writer_arc: Arc<Mutex<Box<dyn IoWrite + Send>>>) -> Self {
        Self { inner: writer_arc }
    }
}

impl Clone for PtyWriter {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

pub struct PtyWorker {
    master: parking_lot::Mutex<Box<dyn MasterPty + Send>>,
    pub writer: PtyWriter,
    shutdown_flag: Arc<AtomicBool>,
}

unsafe impl Send for PtyWorker {}
unsafe impl Sync for PtyWorker {}

impl PtyWorker {
    pub fn spawn(
        shell: &[String],
        cols: usize,
        rows: usize,
        mut on_chunk: impl FnMut(&[u8]) + Send + 'static,
    ) -> Self {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("Failed to open PTY");

        let mut cmd = CommandBuilder::new(shell[0].as_str());
        if shell.len() > 1 {
            cmd.args(&shell[1..]);
        }

        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        if let Ok(lang) = std::env::var("LANG") {
            cmd.env("LANG", lang);
        }
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        cmd.env_remove("TMUX");
        cmd.env_remove("STY");

        let _child = pair
            .slave
            .spawn_command(cmd)
            .expect("Failed to spawn shell");

        let master = parking_lot::Mutex::new(pair.master);
        let mut reader = master
            .lock()
            .try_clone_reader()
            .expect("Failed to clone reader");
        let writer = master.lock().take_writer().expect("Failed to take writer");

        drop(pair.slave);

        let writer_boxed: Box<dyn IoWrite + Send> = Box::new(writer);
        let writer_mutex = Arc::new(Mutex::new(writer_boxed));
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        let pty_writer = PtyWriter {
            inner: Arc::clone(&writer_mutex),
        };

        let _shutdown_read = Arc::clone(&shutdown_flag);
        tracing::debug!("PTY: starting reader thread");
        thread::spawn(move || {
            let mut buf = [0u8; PTY_READ_BUF_SIZE];
            let mut batch = Vec::with_capacity(PTY_READ_BUF_SIZE * 2);
            let mut last_flush = Instant::now();

            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) => {
                        if !batch.is_empty() {
                            on_chunk(&batch);
                            batch.clear();
                        }
                        tracing::debug!("PTY: reader returned 0 (EOF)");
                        return;
                    }
                    Ok(n) => n,
                    Err(e) => {
                        tracing::error!("PTY read error: {}", e);
                        if !batch.is_empty() {
                            on_chunk(&batch);
                            batch.clear();
                        }
                        return;
                    }
                };

                batch.extend_from_slice(&buf[..n]);

                let flush_due_to_timer = last_flush.elapsed() >= Duration::from_millis(1); // 1ms - very responsive
                let flush_due_to_size = batch.len() >= 128;
                if flush_due_to_timer || flush_due_to_size {
                    if !batch.is_empty() {
                        on_chunk(&batch);
                        batch.clear();
                        last_flush = Instant::now();
                    }
                }
            }
        });

        Self {
            master,
            writer: pty_writer,
            shutdown_flag,
        }
    }

    pub fn resize(&self, cols: usize, rows: usize) -> anyhow::Result<()> {
        self.master.lock().resize(PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    pub fn shutdown(&self) {
        self.shutdown_flag.store(true, Ordering::Relaxed);
    }
}
