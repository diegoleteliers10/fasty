//! Renderer module using wgpu + cosmic-text.

mod atlas;
mod cell;
mod pipeline;

use wgpu::{Device, DeviceDescriptor, Features, Instance, Queue, Surface, SurfaceConfiguration};
use winit::window::Window;

pub use atlas::Atlas;
pub use cell::CellInstance;
pub use pipeline::Pipeline;

const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

pub struct Renderer<'a> {
    surface: Surface<'a>,
    device: Device,
    queue: Queue,
    pub config: SurfaceConfiguration,
    pipeline: Pipeline,
    atlas: Atlas,
    cell_width: f32,
    cell_height: f32,
    pub dirty: bool,
}

impl<'a> Renderer<'a> {
    pub async fn new(window: &'a Window) -> anyhow::Result<Self> {
        let instance = Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::GL,
            flags: wgpu::InstanceFlags::VALIDATION,
            ..Default::default()
        });

        let surface = instance.create_surface(window)?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .expect("Failed to find compatible graphics adapter");

        let (device, queue) = adapter
            .request_device(
                &DeviceDescriptor {
                    label: Some("fasty-renderer"),
                    required_features: Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await?;

        let format = surface
            .get_capabilities(&adapter)
            .formats
            .first()
            .copied()
            .unwrap_or(wgpu::TextureFormat::Bgra8Unorm);

        let caps = surface.get_capabilities(&adapter);
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
            wgpu::CompositeAlphaMode::PostMultiplied
        } else if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Inherit) {
            wgpu::CompositeAlphaMode::Inherit
        } else {
            wgpu::CompositeAlphaMode::Opaque
        };

        let size = window.inner_size();
        let config = SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let font_size = 16.0;
        let scale_factor = window.scale_factor() as f32;
        let atlas = Atlas::new(&device, &queue, 2048, 2048, font_size, scale_factor)?;
        let (cell_width, cell_height) = atlas.cell_size();
        tracing::info!("Atlas created with {} entries, cell_size: {}x{}", atlas.entries_len(), cell_width, cell_height);
        let pipeline = Pipeline::new(&device, &atlas, format);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            atlas,
            cell_width,
            cell_height,
            dirty: true,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.dirty = true;
    }

    pub fn cell_width(&self) -> f32 {
        self.cell_width
    }

    pub fn cell_height(&self) -> f32 {
        self.cell_height
    }

    pub fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    pub fn render(
        &mut self,
        terminal: &crate::terminal_state::TerminalState,
        scrollbar_alpha: f32,
        scroll_current: f32,
        history_size: f32,
        visible_rows: f32,
        hover_close: bool,
        hover_max: bool,
        hover_min: bool,
        hover_settings: bool,
    ) {
        if !self.dirty {
            tracing::debug!("Renderer::render early exit - dirty=false");
            return;
        }

        tracing::info!("Renderer::render executing");
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(e) => {
                tracing::error!("Failed to get current texture: {}", e);
                return;
            }
        };

        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("fasty-render-encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fasty-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            self.pipeline.render(
                &mut render_pass,
                terminal,
                &mut self.atlas,
                self.cell_width,
                self.cell_height,
                self.config.width as f32,
                self.config.height as f32,
                scrollbar_alpha,
                scroll_current,
                history_size,
                visible_rows,
                hover_close,
                hover_max,
                hover_min,
                hover_settings,
                &self.device,
                &self.queue,
            );
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        self.device.poll(wgpu::Maintain::Wait);
        frame.present();
        self.dirty = false;
    }

    pub fn render_settings(
        &mut self,
        font_family: &str,
        font_size: f32,
        scrollback: usize,
        active_field: usize,
        hover_close: bool,
        hover_font_family: bool,
        hover_size_minus: bool,
        hover_size_plus: bool,
        hover_scroll_minus: bool,
        hover_scroll_plus: bool,
        hover_save: bool,
        hover_cancel: bool,
    ) {
        if !self.dirty {
            return;
        }

        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(e) => {
                tracing::error!("Failed to get current texture: {}", e);
                return;
            }
        };

        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("settings-render-encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("settings-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            self.pipeline.render_settings(
                &mut render_pass,
                &mut self.atlas,
                self.config.width as f32,
                self.config.height as f32,
                font_family,
                font_size,
                scrollback,
                active_field,
                hover_close,
                hover_font_family,
                hover_size_minus,
                hover_size_plus,
                hover_scroll_minus,
                hover_scroll_plus,
                hover_save,
                hover_cancel,
                &self.device,
                &self.queue,
            );
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        self.device.poll(wgpu::Maintain::Wait);
        frame.present();
        self.dirty = false;
    }
}