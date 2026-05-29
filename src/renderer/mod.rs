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
    r: 8.0 / 255.0,
    g: 8.0 / 255.0,
    b: 10.0 / 255.0,
    a: 1.0,
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

        let size = window.inner_size();
        let config = SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
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

    pub fn render(&mut self, terminal: &crate::terminal_state::TerminalState) {
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

            self.pipeline
                .render(&mut render_pass, terminal, &mut self.atlas, self.cell_width, self.cell_height, self.config.width as f32, self.config.height as f32, &self.device, &self.queue);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        self.device.poll(wgpu::Maintain::Wait);
        frame.present();
        self.dirty = false;

        return;
    }
}