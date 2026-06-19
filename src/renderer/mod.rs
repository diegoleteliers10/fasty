//! Renderer module using wgpu + cosmic-text.

mod atlas;
mod cell;
mod pipeline;

use wgpu::{Device, DeviceDescriptor, Features, Instance, Queue, Surface, SurfaceConfiguration};
use winit::window::Window;

pub use atlas::Atlas;
pub use atlas::RowShapingResult;
pub use atlas::is_block_element;
pub use atlas::is_emoji;
 pub use cell::CellInstance;
 pub use pipeline::Pipeline;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderReason {
    CursorBlink,
    GridChanged,
}

/// All non-terminal inputs to [`Renderer::render`]. Bundled so callers
/// with minimal UI (popped-out windows, future detached panes) can build
/// a default config without restating every field.
#[allow(clippy::too_many_fields)]
pub struct RenderInputs<'a> {
    pub ligatures: bool,
    pub scrollbar_alpha: f32,
    pub scroll_current: f32,
    pub history_size: f32,
    pub visible_rows: f32,
    pub hover_close: bool,
    pub hover_max: bool,
    pub hover_min: bool,
    pub hover_settings: bool,
    pub last_activity_time_secs: f32,
    pub current_time: f32,
    pub selection: Option<Selection>,
    pub hovered_url: Option<HoveredUrl>,
    pub hovered_hyperlink: Option<&'a str>,
    pub search_matches: &'a [crate::renderer::SearchMatch],
    pub search_current_idx: usize,
    pub search_visible: bool,
    pub search_query_render: &'a str,
    pub terminal_font_size: f32,
    pub toast: Option<(&'a str, std::time::Instant, u64)>,
    pub active_tab_index: usize,
    pub tab_titles: &'a [String],
    pub tab_running_states: &'a [bool],
    pub tab_exit_codes: &'a [Option<i32>],
    pub active_tab_path: &'a str,
    pub context_menu_visible: bool,
    pub context_menu_is_about: bool,
    pub context_menu_x: f32,
    pub context_menu_y: f32,
    pub context_menu_hovered_idx: Option<usize>,
    pub context_menu_open_time_secs: Option<f32>,
    pub context_menu_items: &'a [crate::renderer::ContextMenuItem],
    pub hovered_tab_index: Option<usize>,
    pub hovered_close_tab_index: Option<usize>,
    pub hover_new_tab: bool,
    pub command_palette_visible: bool,
    pub command_palette_query: &'a str,
    pub command_palette_selected: usize,
    pub command_palette_filtered: &'a [String],
    pub command_palette_scroll: usize,
    pub dragging_tab: Option<usize>,
    pub drag_current_x: f32,
    pub drag_tab_offset: f32,
    pub drop_target_idx: Option<usize>,
    pub tab_ctx_visible: bool,
    pub tab_ctx_x: f32,
    pub tab_ctx_y: f32,
    pub tab_ctx_hovered: Option<usize>,
    pub renaming_tab: Option<usize>,
    pub rename_buffer: &'a str,
    pub rename_cursor: usize,
    pub git_status: Option<&'a crate::git::GitStatus>,
    pub bar_segments: &'a [crate::widgets::LaidOutWidget],
    pub bar_y: f32,
    pub bar_h: f32,
    pub ssh_picker_visible: bool,
    pub ssh_picker_query: &'a str,
    pub ssh_picker_selected: usize,
    pub ssh_filtered: &'a [String],
    pub project_jumper_visible: bool,
    pub project_jumper_query: &'a str,
    pub project_jumper_selected: usize,
    pub project_filtered: &'a [String],
    pub worktree_picker_visible: bool,
    pub worktree_picker_query: &'a str,
    pub worktree_picker_selected: usize,
    pub worktree_filtered: &'a [String],
    pub bell_flash_elapsed_ms: Option<f32>,
    pub last_command_duration_ms: Option<u128>,
    pub command_duration_display_secs: Option<f32>,
    pub exit_code: Option<i32>,
    pub current_mouse_x: f32,
    pub current_mouse_y: f32,
    pub hovered_url_text: Option<&'a str>,
    pub opacity: f32,
}

impl<'a> Default for RenderInputs<'a> {
    fn default() -> Self {
        Self {
            ligatures: false,
            scrollbar_alpha: 0.0,
            scroll_current: 0.0,
            history_size: 0.0,
            visible_rows: 0.0,
            hover_close: false,
            hover_max: false,
            hover_min: false,
            hover_settings: false,
            last_activity_time_secs: 0.0,
            current_time: 0.0,
            selection: None,
            hovered_url: None,
            hovered_hyperlink: None,
            search_matches: &[],
            search_current_idx: 0,
            search_visible: false,
            search_query_render: "",
            terminal_font_size: 14.0,
            toast: None,
            active_tab_index: 0,
            tab_titles: &[],
            tab_running_states: &[],
            tab_exit_codes: &[],
            active_tab_path: "",
            context_menu_visible: false,
            context_menu_is_about: false,
            context_menu_x: 0.0,
            context_menu_y: 0.0,
            context_menu_hovered_idx: None,
            context_menu_open_time_secs: None,
            context_menu_items: &[],
            hovered_tab_index: None,
            hovered_close_tab_index: None,
            hover_new_tab: false,
            command_palette_visible: false,
            command_palette_query: "",
            command_palette_selected: 0,
            command_palette_filtered: &[],
            command_palette_scroll: 0,
            dragging_tab: None,
            drag_current_x: 0.0,
            drag_tab_offset: 0.0,
            drop_target_idx: None,
            tab_ctx_visible: false,
            tab_ctx_x: 0.0,
            tab_ctx_y: 0.0,
            tab_ctx_hovered: None,
            renaming_tab: None,
            rename_buffer: "",
            rename_cursor: 0,
            git_status: None,
            bar_segments: &[],
            bar_y: 0.0,
            bar_h: 0.0,
            ssh_picker_visible: false,
            ssh_picker_query: "",
            ssh_picker_selected: 0,
            ssh_filtered: &[],
            project_jumper_visible: false,
            project_jumper_query: "",
            project_jumper_selected: 0,
            project_filtered: &[],
            worktree_picker_visible: false,
            worktree_picker_query: "",
            worktree_picker_selected: 0,
            worktree_filtered: &[],
            bell_flash_elapsed_ms: None,
            last_command_duration_ms: None,
            command_duration_display_secs: None,
            exit_code: None,
            current_mouse_x: 0.0,
            current_mouse_y: 0.0,
            hovered_url_text: None,
            opacity: 1.0,
        }
    }
}

const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

const ATLAS_SIZE: u32 = 1024;

pub struct Renderer<'a> {
    surface: Surface<'a>,
    pub instance: std::sync::Arc<Instance>,
    pub device: std::sync::Arc<Device>,
    pub queue: std::sync::Arc<Queue>,
    pub config: SurfaceConfiguration,
    pipeline: Pipeline,
    pub atlas: Atlas,
    pub ui_atlas: Atlas,
    cell_width: f32,
    cell_height: f32,
    pub dirty: bool,
    pub grid_dirty: bool,
    pub cached_grid_instances: Vec<CellInstance>,
    pub update_available: bool,
    pub update_in_progress: bool,
    pub update_completed: bool,
    pub hover_update: bool,
}

impl<'a> Renderer<'a> {
    pub async fn new(
        window: &'a Window,
        font_family: &str,
        font_size: f32,
    ) -> anyhow::Result<Self> {
        let instance = Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::from_build_config(),
            ..Default::default()
        });
        let instance = std::sync::Arc::new(instance);

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
                    memory_hints: if cfg!(target_os = "windows") {
                        wgpu::MemoryHints::MemoryUsage
                    } else {
                        wgpu::MemoryHints::Performance
                    },
                },
                None,
            )
            .await?;
        let device = std::sync::Arc::new(device);
        let queue = std::sync::Arc::new(queue);

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

        let scale_factor = window.scale_factor() as f32;
        let atlas = Atlas::new(&device, &queue, ATLAS_SIZE, ATLAS_SIZE, font_family, font_size, scale_factor)?;
        let ui_atlas = Atlas::new(&device, &queue, ATLAS_SIZE, ATLAS_SIZE, font_family, 13.0, scale_factor)?;
        let (cell_width, cell_height) = atlas.cell_size();
        tracing::info!("Atlas created with {} entries, cell_size: {}x{}", atlas.entries_len(), cell_width, cell_height);
        let pipeline = Pipeline::new(&device, &atlas, &ui_atlas, format);

        Ok(Self {
            surface,
            instance,
            device,
            queue,
            config,
            pipeline,
            atlas,
            ui_atlas,
            cell_width,
            cell_height,
            dirty: true,
            grid_dirty: true,
            cached_grid_instances: Vec::new(),
            update_available: false,
            update_in_progress: false,
            update_completed: false,
            hover_update: false,
        })
    }

    pub fn new_shared(
        window: &'a Window,
        font_family: &str,
        font_size: f32,
        instance: std::sync::Arc<Instance>,
        device: std::sync::Arc<Device>,
        queue: std::sync::Arc<Queue>,
        format: wgpu::TextureFormat,
        alpha_mode: wgpu::CompositeAlphaMode,
    ) -> anyhow::Result<Self> {
        let surface = instance.create_surface(window)?;

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

        let scale_factor = window.scale_factor() as f32;
        let atlas = Atlas::new(&device, &queue, ATLAS_SIZE, ATLAS_SIZE, font_family, font_size, scale_factor)?;
        let ui_atlas = Atlas::new(&device, &queue, ATLAS_SIZE, ATLAS_SIZE, font_family, 13.0, scale_factor)?;
        let (cell_width, cell_height) = atlas.cell_size();
        tracing::info!("Atlas created with {} entries, cell_size: {}x{}", atlas.entries_len(), cell_width, cell_height);
        let pipeline = Pipeline::new(&device, &atlas, &ui_atlas, format);

        Ok(Self {
            surface,
            instance,
            device,
            queue,
            config,
            pipeline,
            atlas,
            ui_atlas,
            cell_width,
            cell_height,
            dirty: true,
            grid_dirty: true,
            cached_grid_instances: Vec::new(),
            update_available: false,
            update_in_progress: false,
            update_completed: false,
            hover_update: false,
        })
    }

    pub fn new_shared_fast(
        window: &'a Window,
        instance: std::sync::Arc<Instance>,
        device: std::sync::Arc<Device>,
        queue: std::sync::Arc<Queue>,
        format: wgpu::TextureFormat,
        alpha_mode: wgpu::CompositeAlphaMode,
        atlas: Atlas,
        ui_atlas: Atlas,
    ) -> anyhow::Result<Self> {
        let surface = instance.create_surface(window)?;

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

        let (cell_width, cell_height) = atlas.cell_size();
        let pipeline = Pipeline::new(&device, &atlas, &ui_atlas, format);

        Ok(Self {
            surface,
            instance,
            device,
            queue,
            config,
            pipeline,
            atlas,
            ui_atlas,
            cell_width,
            cell_height,
            dirty: true,
            grid_dirty: true,
            cached_grid_instances: Vec::new(),
            update_available: false,
            update_in_progress: false,
            update_completed: false,
            hover_update: false,
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
        self.grid_dirty = true;
    }

    /// Present a single transparent-cleared frame. Used by windows that have
    /// no full render path yet (e.g. popped-out windows whose renderer is
    /// stubbed). On Wayland the surface must be presented at least once to
    /// be mapped by the compositor.
    pub fn present_blank(&mut self) {
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(_) => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fasty-blank-present"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fasty-blank-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
    }

    pub fn update_font(&mut self, font_family: &str, font_size: f32) -> anyhow::Result<()> {
        let scale_factor = self.atlas.scale_factor();
        let new_primary_path = Atlas::load_font_path(font_family).unwrap_or_else(|_| "monospace".to_string());
        
        let font_family_changed = new_primary_path != self.atlas.primary_path();
        let font_size_changed = font_size != self.atlas.font_size();
        
        if !font_family_changed && !font_size_changed {
            return Ok(());
        }

        let new_atlas = Atlas::new(
            &self.device,
            &self.queue,
            ATLAS_SIZE,
            ATLAS_SIZE,
            font_family,
            font_size,
            scale_factor,
        )?;

        if font_family_changed {
            let new_ui_atlas = Atlas::new(
                &self.device,
                &self.queue,
                ATLAS_SIZE,
                ATLAS_SIZE,
                font_family,
                13.0,
                scale_factor,
            )?;
            let pipeline = Pipeline::new(&self.device, &new_atlas, &new_ui_atlas, self.config.format);
            self.atlas = new_atlas;
            self.ui_atlas = new_ui_atlas;
            self.pipeline = pipeline;
        } else {
            // Re-use the existing ui_atlas
            let pipeline = Pipeline::new(&self.device, &new_atlas, &self.ui_atlas, self.config.format);
            self.atlas = new_atlas;
            self.pipeline = pipeline;
        }

        let (cell_width, cell_height) = self.atlas.cell_size();
        self.cell_width = cell_width;
        self.cell_height = cell_height;
        self.dirty = true;
        self.grid_dirty = true;
        Ok(())
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
        reason: RenderReason,
        terminal: &crate::terminal_state::TerminalState,
        cursor_visible: bool,
        inputs: RenderInputs<'_>,
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
                reason,
                &mut render_pass,
                terminal,
                cursor_visible,
                inputs.ligatures,
                &mut self.atlas,
                &mut self.ui_atlas,
                self.cell_width,
                self.cell_height,
                self.config.width as f32,
                self.config.height as f32,
                inputs.scrollbar_alpha,
                inputs.scroll_current,
                inputs.history_size,
                inputs.visible_rows,
                inputs.hover_close,
                inputs.hover_max,
                inputs.hover_min,
                inputs.hover_settings,
                inputs.last_activity_time_secs,
                inputs.current_time,
                inputs.selection,
                inputs.hovered_url,
                inputs.hovered_hyperlink,
                inputs.search_matches,
                inputs.search_current_idx,
                inputs.search_visible,
                inputs.search_query_render,
                inputs.terminal_font_size,
                inputs.toast,
                inputs.active_tab_index,
                inputs.tab_titles,
                inputs.tab_running_states,
                inputs.tab_exit_codes,
                inputs.active_tab_path,
                inputs.context_menu_visible,
                inputs.context_menu_is_about,
                inputs.context_menu_x,
                inputs.context_menu_y,
                inputs.context_menu_hovered_idx,
                inputs.context_menu_items,
                inputs.context_menu_open_time_secs,
                inputs.hovered_tab_index,
                inputs.hovered_close_tab_index,
                inputs.hover_new_tab,
                self.update_available,
                self.update_in_progress,
                self.update_completed,
                self.hover_update,
                &mut self.cached_grid_instances,
                &mut self.grid_dirty,
                &self.device,
                &self.queue,
                inputs.command_palette_visible,
                inputs.command_palette_query,
                inputs.command_palette_selected,
                inputs.command_palette_filtered,
                inputs.command_palette_scroll,
                inputs.dragging_tab,
                inputs.drag_current_x,
                inputs.drag_tab_offset,
                inputs.drop_target_idx,
                inputs.tab_ctx_visible,
                inputs.tab_ctx_x,
                inputs.tab_ctx_y,
                inputs.tab_ctx_hovered,
                inputs.renaming_tab,
                inputs.rename_buffer,
                inputs.rename_cursor,
                inputs.git_status,
                inputs.bar_segments,
                inputs.bar_y,
                inputs.bar_h,
                inputs.ssh_picker_visible,
                inputs.ssh_picker_query,
                inputs.ssh_picker_selected,
                inputs.ssh_filtered,
                inputs.project_jumper_visible,
                inputs.project_jumper_query,
                inputs.project_jumper_selected,
                inputs.project_filtered,
                inputs.worktree_picker_visible,
                inputs.worktree_picker_query,
                inputs.worktree_picker_selected,
                inputs.worktree_filtered,
                inputs.bell_flash_elapsed_ms,
                inputs.last_command_duration_ms,
                inputs.command_duration_display_secs,
                inputs.exit_code,
                inputs.current_mouse_x,
                inputs.current_mouse_y,
                inputs.hovered_url_text,
                inputs.opacity,
            );
        }

        self.queue.submit(std::iter::once(encoder.finish()));
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
        hover_theme: bool,
        hover_open_config: bool,
        system_fonts: &[String],
        font_scroll_y: f32,
        hovered_font_idx: Option<usize>,
        theme: &str,
        themes: &[String],
        hovered_theme_idx: Option<usize>,
        theme_scroll_y: f32,
        opacity: f32,
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
                hover_theme,
                hover_open_config,
                system_fonts,
                font_scroll_y,
                hovered_font_idx,
                theme,
                themes,
                hovered_theme_idx,
                theme_scroll_y,
                &self.device,
                &self.queue,
                opacity,
            );
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        self.dirty = false;
    }

    pub fn render_about(
        &mut self,
        version: &str,
        hover_close: bool,
        opacity: f32,
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
            label: Some("about-render-encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("about-render-pass"),
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

            self.pipeline.render_about(
                &mut render_pass,
                &mut self.ui_atlas,
                self.config.width as f32,
                self.config.height as f32,
                version,
                hover_close,
                &self.device,
                &self.queue,
                opacity,
            );
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        self.dirty = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub start: alacritty_terminal::index::Point,
    pub end: alacritty_terminal::index::Point,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoveredUrl {
    pub line: i32,
    pub start_col: usize,
    pub end_col: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub line: i32,
    pub col: usize,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ContextMenuItem {
    Copy,
    Paste,
    Separator,
    NewTab,
    CloseTab,
    About,
    OpenLink,
    CopyWord,
    CopyLine,
    CdHere,
    OpenInEditor,
    OpenEmail,
    MoveToNewWindow,
    CopyHex,
}