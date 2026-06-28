use std::sync::Arc;

use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::{self, WindowAttributes};

use crate::renderer::Renderer;

pub struct SecondaryWindow {
    pub window: Arc<window::Window>,
    pub renderer: Renderer<'static>,
    pub hover_close: bool,
    pub mouse_y: f64,
}

pub enum EventResult {
    Consumed,
    Closed,
    Ignored,
}

impl SecondaryWindow {
    pub fn create(
        target: &ActiveEventLoop,
        title: &str,
        width: f32,
        height: f32,
        main_renderer: &parking_lot::Mutex<Renderer<'_>>,
        font_family: &str,
    ) -> Result<Self, anyhow::Error> {
        let visible = !cfg!(target_os = "windows");
        let window = target.create_window(
            WindowAttributes::default()
                .with_title(title)
                .with_decorations(false)
                .with_transparent(true)
                .with_visible(visible)
                .with_inner_size(LogicalSize::new(width, height)),
        )?;
        let arc = Arc::new(window);
        let w_ref: &window::Window = &*arc;
        let w_static: &'static window::Window = unsafe { std::mem::transmute(w_ref) };

        let (shared_instance, shared_device, shared_queue, format, alpha_mode) = {
            let r = main_renderer.lock();
            (
                r.instance.clone(),
                r.device.clone(),
                r.queue.clone(),
                r.config.format,
                r.config.alpha_mode,
            )
        };

        let renderer = Renderer::new_shared(
            w_static,
            font_family,
            13.0,
            shared_instance,
            shared_device,
            shared_queue,
            format,
            alpha_mode,
        )?;

        Ok(Self {
            window: arc,
            renderer,
            hover_close: false,
            mouse_y: 0.0,
        })
    }

    #[cfg(target_os = "windows")]
    pub fn show_and_focus(&mut self) {
        self.renderer.set_dirty(true);
        self.window.set_visible(true);
        self.window.focus_window();
    }

    pub fn handle_event(
        &mut self,
        event: &WindowEvent,
        app_dirty: &mut bool,
    ) -> EventResult {
        match event {
            WindowEvent::CloseRequested => {
                *app_dirty = true;
                return EventResult::Closed;
            }
            WindowEvent::Resized(size) => {
                self.renderer.resize(size.width, size.height);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale_factor = self.window.scale_factor();
                let m_x = position.x / scale_factor;
                let m_y = position.y / scale_factor;
                self.mouse_y = m_y;

                let old_hover = self.hover_close;
                let w_width = self.window.inner_size().width as f64 / scale_factor;
                self.hover_close = if cfg!(target_os = "macos") {
                    m_y >= 4.0 && m_y <= 32.0 && m_x >= 4.0 && m_x < 32.0
                } else {
                    m_y >= 4.0 && m_y <= 32.0 && m_x >= (w_width - 32.0) && m_x < (w_width - 4.0)
                };

                if self.hover_close != old_hover {
                    self.renderer.set_dirty(true);
                    self.window.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if *button == MouseButton::Left && *state == ElementState::Pressed {
                    if self.hover_close {
                        *app_dirty = true;
                        return EventResult::Closed;
                    } else if self.mouse_y <= 36.0 {
                        let _ = self.window.drag_window();
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.hover_close = false;
                self.renderer.set_dirty(true);
                self.window.request_redraw();
            }
            WindowEvent::Focused(focused) => {
                if !focused {
                    self.hover_close = false;
                    self.renderer.set_dirty(true);
                    self.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {}
            _ => return EventResult::Ignored,
        }
        EventResult::Consumed
    }
}
