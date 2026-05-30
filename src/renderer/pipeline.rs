//! Render pipeline for terminal cells.

use std::mem;

use alacritty_terminal::grid::Indexed;
use bytemuck::cast_slice;
use wgpu::{Buffer, Device, RenderPipeline};

use crate::renderer::{CellInstance, RenderReason};
use crate::terminal_state::TerminalState;

pub struct Pipeline {
    pipeline: RenderPipeline,
    instance_buffer: Buffer,
    vertex_buffer: Buffer,
    uniform_buffer: Buffer,
    bind_group: wgpu::BindGroup,
    pub ui_bind_group: wgpu::BindGroup,
    max_instances: usize,
    pub last_cursor_index: Option<usize>,
    pub last_term_draw_count: usize,
    pub last_ui_draw_count: usize,
    pub cached_final_instances: Vec<CellInstance>,
}

const SHADER_SOURCE: &str = r#"
struct CellInstance {
    position: vec2<f32>,
    size: vec2<f32>,
    fg_color: vec4<f32>,
    bg_color: vec4<f32>,
    uv_offset: vec2<f32>,
    uv_size: vec2<f32>,
    is_color: f32,
    padding: vec3<f32>,
};

struct Uniforms {
    viewport: vec2<f32>,
};

@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) bg_color: vec4<f32>,
    @location(2) fg_color: vec4<f32>,
    @location(3) is_color: f32,
    @location(4) size: vec2<f32>,
}

@vertex
fn vertex_main(
    @location(0) vertex_pos: vec2<f32>,
    @location(1) vertex_uv: vec2<f32>,
    @location(3) cell_pos: vec2<f32>,
    @location(4) cell_size: vec2<f32>,
    @location(5) cell_fg: vec4<f32>,
    @location(6) cell_bg: vec4<f32>,
    @location(7) cell_uv_offset: vec2<f32>,
    @location(8) cell_uv_size: vec2<f32>,
    @location(9) cell_is_color: f32,
) -> VertexOutput {
    var output: VertexOutput;

    let pixel_x = cell_pos.x + vertex_pos.x * cell_size.x;
    let pixel_y = cell_pos.y + vertex_pos.y * cell_size.y;

    let ndc_x = pixel_x / uniforms.viewport.x * 2.0 - 1.0;
    let ndc_y = 1.0 - pixel_y / uniforms.viewport.y * 2.0;

    output.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    output.uv = cell_uv_offset + vertex_uv * cell_uv_size;
    output.bg_color = cell_bg;
    output.fg_color = cell_fg;
    output.is_color = cell_is_color;
    output.size = cell_size;

    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if (input.is_color > 1.5) {
        let r = input.is_color;
        let p = (input.uv - vec2<f32>(0.5)) * input.size;
        
        var b = input.size / 2.0;
        if (input.bg_color.g > 0.0 || input.bg_color.b > 0.0) {
            b = (input.size - 2.0 * vec2<f32>(input.bg_color.g, input.bg_color.b)) / 2.0;
        }
        
        let q = abs(p) - b + vec2<f32>(r);
        let dist = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
        
        var blur = 0.5;
        if (input.bg_color.r > 0.0) {
            blur = input.bg_color.r;
        }
        
        let alpha = 1.0 - smoothstep(-blur, blur, dist);
        return vec4<f32>(input.fg_color.rgb, input.fg_color.a * alpha);
    }

    let tex_color = textureSample(atlas, atlas_sampler, input.uv);
    if (input.is_color > 0.5) {
        return tex_color;
    } else {
        return vec4<f32>(input.fg_color.rgb, tex_color.r * input.fg_color.a);
    }
}
"#;

const VERTEX_BUFFER_DATA: &[f32] = &[
    0.0, 0.0,  0.0, 0.0,
    1.0, 0.0,  1.0, 0.0,
    0.0, 1.0,  0.0, 1.0,
    0.0, 1.0,  0.0, 1.0,
    1.0, 0.0,  1.0, 0.0,
    1.0, 1.0,  1.0, 1.0,
];

impl Pipeline {
    pub fn new(
        device: &Device,
        atlas: &crate::renderer::Atlas,
        ui_atlas: &crate::renderer::Atlas,
        format: wgpu::TextureFormat,
    ) -> Self {
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cell-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let atlas_view = atlas.texture().create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8Unorm),
            ..Default::default()
        });
        let ui_atlas_view = ui_atlas.texture().create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8Unorm),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let max_instances = 32768;
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance-buffer"),
            size: mem::size_of::<CellInstance>() as u64 * max_instances as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vertex-buffer"),
            size: (VERTEX_BUFFER_DATA.len() * 4) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        vertex_buffer.slice(..).get_mapped_range_mut().copy_from_slice(
            cast_slice(VERTEX_BUFFER_DATA),
        );
        vertex_buffer.unmap();

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniform-buffer"),
            size: 8,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas-bind-group"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        has_dynamic_offset: false,
                        min_binding_size: None,
                        ty: wgpu::BufferBindingType::Uniform,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cell-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cell-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: "vertex_main",
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: 4 * 4,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 0,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 8,
                                shader_location: 1,
                            },
                        ],
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: mem::size_of::<CellInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 3,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 8,
                                shader_location: 4,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: 16,
                                shader_location: 5,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: 32,
                                shader_location: 6,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 48,
                                shader_location: 7,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 56,
                                shader_location: 8,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32,
                                offset: 64,
                                shader_location: 9,
                            },
                        ],
                    },
                ],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: "fragment_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview: None,
            cache: None,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas-bind-group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });

        let ui_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui-atlas-bind-group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&ui_atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });

        Self {
            pipeline,
            instance_buffer,
            vertex_buffer,
            uniform_buffer,
            bind_group,
            ui_bind_group,
            max_instances,
            last_cursor_index: None,
            last_term_draw_count: 0,
            last_ui_draw_count: 0,
            cached_final_instances: Vec::new(),
        }
    }

    const PADDING_LEFT: f32 = 10.0;

    pub fn render(
        &mut self,
        reason: RenderReason,
        render_pass: &mut wgpu::RenderPass,
        terminal: &TerminalState,
        cursor_visible: bool,
        atlas: &mut crate::renderer::Atlas,
        ui_atlas: &mut crate::renderer::Atlas,
        cell_width: f32,
        cell_height: f32,
        viewport_width: f32,
        viewport_height: f32,
        scrollbar_alpha: f32,
        scroll_current: f32,
        history_size: f32,
        visible_rows: f32,
        hover_close: bool,
        hover_max: bool,
        hover_min: bool,
        hover_settings: bool,
        _last_activity_time_secs: f32,
        current_time: f32,
        selection: Option<crate::renderer::Selection>,
        hovered_url: Option<crate::renderer::HoveredUrl>,
        toast: Option<(&str, std::time::Instant)>,
        active_tab_index: usize,
        tab_titles: &[String],
        active_tab_path: &str,
        context_menu_visible: bool,
        context_menu_x: f32,
        context_menu_y: f32,
        context_menu_hovered_idx: Option<usize>,
        context_menu_open_time_secs: Option<f32>,
        hovered_tab_index: Option<usize>,
        hovered_close_tab_index: Option<usize>,
        hover_new_tab: bool,
        cached_grid_instances: &mut Vec<CellInstance>,
        grid_dirty: &mut bool,
        device: &Device,
        queue: &wgpu::Queue,
    ) {
        if reason == RenderReason::CursorBlink {
            let term = terminal.term();
            let term_guard = term.lock();
            let content = term_guard.renderable_content();
            
            let (_c_w, _c_h, _c_ox, _c_oy, c_alpha) = match content.cursor.shape {
                alacritty_terminal::vte::ansi::CursorShape::Block => (1.0f32, cell_height, 0.0f32, 0.0f32, 0.9f32),
                alacritty_terminal::vte::ansi::CursorShape::Underline => (cell_width, 1.0f32, 0.0f32, cell_height - 1.0f32, 0.9f32),
                alacritty_terminal::vte::ansi::CursorShape::Beam => (1.0f32, cell_height, 0.0f32, 0.0f32, 0.9f32),
                alacritty_terminal::vte::ansi::CursorShape::Hidden => (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32),
                _ => (1.0f32, cell_height, 0.0f32, 0.0f32, 0.9f32),
            };

            let final_alpha = if cursor_visible {
                c_alpha
            } else {
                0.0
            };

            if let Some(idx) = self.last_cursor_index {
                if idx < self.cached_final_instances.len() {
                    self.cached_final_instances[idx].fg_color[3] = final_alpha;
                    let offset = (idx * mem::size_of::<CellInstance>()) as u64;
                    queue.write_buffer(
                        &self.instance_buffer,
                        offset,
                        cast_slice(&[self.cached_final_instances[idx]]),
                    );
                }
            }

            let term_draw_count = self.last_term_draw_count;
            if term_draw_count > 0 {
                render_pass.set_pipeline(&self.pipeline);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.set_bind_group(0, &self.bind_group, &[]);
                let end_offset = (term_draw_count * mem::size_of::<CellInstance>()) as u64;
                render_pass.set_vertex_buffer(1, self.instance_buffer.slice(0..end_offset));
                render_pass.draw(0..6, 0..term_draw_count as u32);
            }
            
            let ui_draw_count = self.last_ui_draw_count;
            if ui_draw_count > 0 {
                render_pass.set_pipeline(&self.pipeline);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.set_bind_group(0, &self.ui_bind_group, &[]);
                let start_offset = (term_draw_count * mem::size_of::<CellInstance>()) as u64;
                let end_offset = ((term_draw_count + ui_draw_count) * mem::size_of::<CellInstance>()) as u64;
                render_pass.set_vertex_buffer(1, self.instance_buffer.slice(start_offset..end_offset));
                render_pass.draw(0..6, 0..ui_draw_count as u32);
            }
            return;
        }
        let padding_top = 48.0f32;
        let orig_grid_dirty = *grid_dirty;
        let term = terminal.term();
        let term_guard = term.lock();
        let content = term_guard.renderable_content();

        let mut ui_bg_instances = Vec::new();
        let mut ui_fg_instances = Vec::new();

        // 1. Draw unified bar background (#0a0a0a)
        let bar_bg = CellInstance::new(
            0.0, 0.0,
            viewport_width, 40.0,
            [10.0 / 255.0, 10.0 / 255.0, 10.0 / 255.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0, // radius of 8 at top corners
        );
        ui_bg_instances.push(bar_bg);

        // 2. Draw square block to cover bottom rounded corners of the topbar
        let bar_bottom_fill = CellInstance::new(
            0.0, 32.0,
            viewport_width, 8.0,
            [10.0 / 255.0, 10.0 / 255.0, 10.0 / 255.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 0.0, 0.0,
            0.0, // square
        );
        ui_bg_instances.push(bar_bottom_fill);

        // 5. Draw unified bar bottom border (1px solid rgba(255,255,255,0.06))
        let bar_border = CellInstance::new(
            0.0, 39.0,
            viewport_width, 1.0,
            [1.0, 1.0, 1.0, 0.06],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            0.0,
        );
        ui_bg_instances.push(bar_border);

        let scroll_fraction = scroll_current - content.display_offset as f32;

        let mut instances = if *grid_dirty {
            let mut term_bg_instances = Vec::new();
            let mut term_fg_instances = Vec::new();

            // 0. Draw window background (slate dark)
            let window_bg = CellInstance::new(
                0.0, 0.0,
                viewport_width, viewport_height,
                [12.0 / 255.0, 12.0 / 255.0, 12.0 / 255.0, 1.0], // Terminal bg color (#0c0c0c)
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                8.0, // radius of 8
            );
            term_bg_instances.push(window_bg);

            // 3. Draw terminal background (#0c0c0c) below the unified topbar
            let terminal_bg = CellInstance::new(
                0.0, 40.0,
                viewport_width, viewport_height - 40.0,
                [12.0 / 255.0, 12.0 / 255.0, 12.0 / 255.0, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                8.0, // bottom corners radius of 8
            );
            term_bg_instances.push(terminal_bg);

            // 4. Draw square block to cover top rounded corners of the terminal bg
            let terminal_top_fill = CellInstance::new(
                0.0, 40.0,
                viewport_width, 8.0,
                [12.0 / 255.0, 12.0 / 255.0, 12.0 / 255.0, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 0.0, 0.0,
                0.0, // square
            );
            term_bg_instances.push(terminal_top_fill);

            let bg_instances = &mut term_bg_instances;
            let fg_instances = &mut term_fg_instances;

        for Indexed { cell, point } in content.display_iter {
            let col = point.column.0 as usize;
            let row = (point.line.0 + content.display_offset as i32) as usize;

            let is_default_bg = matches!(cell.bg,
                alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Background
                )
            );

            let cell_x = (col as f32 * cell_width).round() + Self::PADDING_LEFT;
            let cell_y = ((row as f32 + scroll_fraction) * cell_height).round() + padding_top;

            // 1. Draw solid background if not default or if selected
            let is_selected = if let Some(sel) = selection {
                let (min_p, max_p) = if sel.start <= sel.end { (sel.start, sel.end) } else { (sel.end, sel.start) };
                point >= min_p && point <= max_p
            } else {
                false
            };

            let is_hovered_url = if let Some(url) = hovered_url {
                point.line.0 == url.line && col >= url.start_col && col <= url.end_col
            } else {
                false
            };

            if !is_default_bg {
                let is_wide = cell.flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR);
                let char_width = if is_wide { 2 } else { 1 };
                let cell_w = cell_width * char_width as f32;

                let bg = cell_bg_to_f32(cell.bg);

                // Push solid background instance sampling white pixel at (0, 0)
                let bg_instance = CellInstance::new(
                    cell_x, cell_y,
                    cell_w, cell_height,
                    bg,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                );
                bg_instances.push(bg_instance);
            }

            if is_selected {
                let sel_color = [66.0 / 255.0, 135.0 / 255.0, 245.0 / 255.0, 0.3];
                let bg_instance = CellInstance::new(
                    cell_x, cell_y,
                    cell_width, cell_height,
                    sel_color,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                );
                bg_instances.push(bg_instance);
            }

            if is_hovered_url {
                let underline_color = [66.0 / 255.0, 135.0 / 255.0, 245.0 / 255.0, 0.8];
                let bg_instance = CellInstance::new(
                    cell_x, cell_y + cell_height - 2.0,
                    cell_width, 1.0,
                    underline_color,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                );
                bg_instances.push(bg_instance);
            }

            // 2. Draw glyph if non-space/non-null
            if cell.c != ' ' && cell.c != '\0' {
                if let Some(entry) = atlas.get_or_rasterize(cell.c, device, queue) {
                    if entry.width > 0.0 && entry.height > 0.0 {
                        let fg = cell_fg_to_f32(cell.fg, cell.flags);
                        let glyph_x = cell_x + entry.left;
                        let glyph_y = cell_y + atlas.ascent() + entry.top;

                        let (aw, ah) = atlas.atlas_size();
                        let raw_uv = entry.uv_coords(aw, ah);
                        let [uv_x, uv_y, uv_end_x, uv_end_y] = raw_uv;
                        let uv_w = uv_end_x - uv_x;
                        let uv_h = uv_end_y - uv_y;

                        let text_instance = CellInstance::new(
                            glyph_x, glyph_y,
                            entry.width, entry.height,
                            fg,
                            [0.0, 0.0, 0.0, 0.0],
                            uv_x, uv_y, uv_w, uv_h,
                            if entry.is_color { 1.0 } else { 0.0 },
                        );
                        fg_instances.push(text_instance);
                    }
                }
            }
        }

            let mut term_instances_flat = Vec::with_capacity(term_bg_instances.len() + term_fg_instances.len() + 10);
            term_instances_flat.extend(term_bg_instances);
            term_instances_flat.extend(term_fg_instances);
            *cached_grid_instances = term_instances_flat.clone();
            *grid_dirty = false;
            term_instances_flat
        } else {
            cached_grid_instances.clone()
        };


        let bg_instances = &mut ui_bg_instances;
        let fg_instances = &mut ui_fg_instances;
        let atlas = ui_atlas;

        // Draw unified topbar app icon or fallback lightning icon (⚡) U+26A1
        if let Some(entry) = &atlas.app_icon {
            let icon_scale = 16.0f32 / entry.height;
            let glyph_w = entry.width * icon_scale;
            let glyph_h = entry.height * icon_scale;
            let glyph_x = 8.0f32 + (16.0f32 - glyph_w) / 2.0f32;
            let glyph_y = 12.0f32 + (16.0f32 - glyph_h) / 2.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            let uv_w = uv_end_x - uv_x;
            let uv_h = uv_end_y - uv_y;

            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                glyph_w, glyph_h,
                [1.0, 1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_w, uv_h,
                1.0, // is_color = 1.0
            ));
        } else {
            let icon_char = '⚡';
            if let Some(entry) = atlas.get_or_rasterize(icon_char, device, queue) {
                let icon_scale = 16.0f32 / atlas.font_size();
                let glyph_w = entry.width * icon_scale;
                let glyph_h = entry.height * icon_scale;
                let glyph_x = 8.0f32 + (16.0f32 - glyph_w) / 2.0f32;
                let glyph_y = 12.0f32 + (16.0f32 - glyph_h) / 2.0f32;
                let (aw, ah) = atlas.atlas_size();
                let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                let uv_w = uv_end_x - uv_x;
                let uv_h = uv_end_y - uv_y;

                fg_instances.push(CellInstance::new(
                    glyph_x, glyph_y,
                    glyph_w, glyph_h,
                    [1.0, 0.85, 0.20, 1.0], // Vibrant yellow
                    [0.0, 0.0, 0.0, 0.0],
                    uv_x, uv_y, uv_w, uv_h,
                    0.0,
                ));
            }
        }

        // Draw tabs
        let tab_start_x = 36.0f32;
        let path_center_x = viewport_width / 2.0f32;
        let tab_area_max_x = path_center_x - 40.0f32;
        let tab_area_width = tab_area_max_x - tab_start_x - 32.0f32; // 32px for new tab button
        let tab_width = if tab_titles.len() > 0 {
            (tab_area_width / tab_titles.len() as f32).clamp(80.0f32, 160.0f32)
        } else {
            160.0f32
        };

        let scale = 13.0f32 / atlas.font_size();

        for (i, title) in tab_titles.iter().enumerate() {
            let tab_x = tab_start_x + i as f32 * tab_width;
            let is_active = i == active_tab_index;
            let is_hovered = hovered_tab_index == Some(i);

            // Active tab bg (#1e2024), Inactive tab bg (transparent or hover rgba(255,255,255,0.05))
            if is_active {
                // Background fills up to 40.0 to merge visually with terminal
                bg_instances.push(CellInstance::new(
                    tab_x, 0.0,
                    tab_width, 40.0,
                    [12.0 / 255.0, 12.0 / 255.0, 12.0 / 255.0, 1.0], // Terminal bg color (#0c0c0c)
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    0.0,
                ));
                // Top accent edge (2px solid #5b8af0)
                bg_instances.push(CellInstance::new(
                    tab_x, 0.0,
                    tab_width, 2.0,
                    [91.0 / 255.0, 138.0 / 255.0, 240.0 / 255.0, 1.0], // Blue accent
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    0.0,
                ));
            } else if is_hovered {
                bg_instances.push(CellInstance::new(
                    tab_x, 0.0,
                    tab_width, 40.0,
                    [1.0, 1.0, 1.0, 0.05], // Hover bg
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    0.0,
                ));
            }

            // Draw vertical separator (1px vertical line) between tabs
            if i + 1 < tab_titles.len() {
                bg_instances.push(CellInstance::new(
                    tab_x + tab_width, 12.0,
                    1.0, 16.0,
                    [1.0, 1.0, 1.0, 0.07], // Separator
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    0.0,
                ));
            }

            let is_close_visible = is_active || is_hovered;
            let max_w = tab_width - 28.0f32 - (if is_close_visible { 20.0f32 } else { 0.0f32 });
            let display_title = title;

            let mut truncated_title = String::new();
            let mut current_w = 0.0f32;
            for c in display_title.chars() {
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    let w = entry.width * scale + 1.0;
                    if current_w + w > max_w {
                        break;
                    }
                    truncated_title.push(c);
                    current_w += w;
                }
            }
            if truncated_title.len() < display_title.len() {
                while !truncated_title.is_empty() && current_w > max_w - 18.0 {
                    if let Some(c) = truncated_title.pop() {
                        if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                           current_w -= entry.width * scale + 1.0;
                        }
                    }
                }
                truncated_title.push_str("...");
            }

            let mut char_x = tab_x + 14.0f32;
            let fg_color = if is_active {
                [1.0, 1.0, 1.0, 0.90] // Active tab text
            } else if is_hovered {
                [1.0, 1.0, 1.0, 0.70] // Hover tab text
            } else {
                [1.0, 1.0, 1.0, 0.40] // Inactive tab text
            };

            let scaled_ascent = atlas.ascent() * scale;
            let baseline_y = (40.0f32 - 13.0f32) / 2.0f32 + scaled_ascent;

            for c in truncated_title.chars() {
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    let entry_w = entry.width * scale;
                    let entry_h = entry.height * scale;
                    let glyph_x = (char_x + entry.left * scale).round();
                    let glyph_y = (baseline_y + entry.top * scale).round();

                    let (aw, ah) = atlas.atlas_size();
                    let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                    let uv_w = uv_end_x - uv_x;
                    let uv_h = uv_end_y - uv_y;

                    fg_instances.push(CellInstance::new(
                        glyph_x, glyph_y,
                        entry_w, entry_h,
                        fg_color,
                        [0.0, 0.0, 0.0, 0.0],
                        uv_x, uv_y, uv_w, uv_h,
                        0.0,
                    ));

                    if is_active {
                        // Bold simulation
                        fg_instances.push(CellInstance::new(
                            glyph_x + 0.35, glyph_y,
                            entry_w, entry_h,
                            fg_color,
                            [0.0, 0.0, 0.0, 0.0],
                            uv_x, uv_y, uv_w, uv_h,
                            0.0,
                        ));
                    }

                    char_x += entry.width * scale + 1.0;
                }
            }

            // Tab Close Button U+00D7 (×) -> \u{2715} (✕)
            if is_close_visible {
                if let Some(entry) = &atlas.icon_close {
                    let close_x = tab_x + tab_width - 30.0f32;
                    let is_close_hovered = hovered_close_tab_index == Some(i);

                    if is_close_hovered {
                        // Subtle circle background rgba(255,255,255,0.10)
                        bg_instances.push(CellInstance::new(
                            close_x - 1.0, 11.0,
                            18.0, 18.0,
                            [1.0, 1.0, 1.0, 0.10],
                            [0.0, 0.0, 0.0, 0.0],
                            0.0, 0.0, 1.0, 1.0,
                            9.0, // perfect circle
                        ));
                    }

                    let entry_w = 12.0f32;
                    let entry_h = 12.0f32;
                    let cx = close_x + (16.0f32 - entry_w) / 2.0f32;
                    let cy = (40.0f32 - entry_h) / 2.0f32;
                    let (aw, ah) = atlas.atlas_size();
                    let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                    let uv_w = uv_end_x - uv_x;
                    let uv_h = uv_end_y - uv_y;

                    let close_color = if is_close_hovered {
                        [1.0, 1.0, 1.0, 0.90]
                    } else {
                        [1.0, 1.0, 1.0, 0.40]
                    };

                    fg_instances.push(CellInstance::new(
                        cx, cy,
                        entry_w, entry_h,
                        close_color,
                        [0.0, 0.0, 0.0, 0.0],
                        uv_x, uv_y, uv_w, uv_h,
                        0.0,
                    ));
                }
            }
        }

        // Draw New Tab Button (+)
        let new_tab_x = tab_start_x + tab_titles.len() as f32 * tab_width;
        if hover_new_tab {
            bg_instances.push(CellInstance::new(
                new_tab_x, 0.0,
                32.0, 40.0,
                [1.0, 1.0, 1.0, 0.08], // Hover background
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                0.0,
            ));
        }
        if let Some(entry) = &atlas.icon_add {
            let entry_w = 16.0f32;
            let entry_h = 16.0f32;
            let cx = new_tab_x + (32.0f32 - entry_w) / 2.0f32;
            let cy = (40.0f32 - entry_h) / 2.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            let uv_w = uv_end_x - uv_x;
            let uv_h = uv_end_y - uv_y;

            let icon_color = if hover_new_tab {
                [1.0, 1.0, 1.0, 0.80]
            } else {
                [1.0, 1.0, 1.0, 0.35]
            };

            fg_instances.push(CellInstance::new(
                cx, cy,
                entry_w, entry_h,
                icon_color,
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_w, uv_h,
                0.0,
            ));
        }

        // Draw centered path display
        let basename = active_tab_path.split(|c| c == '/' || c == '\\').last().unwrap_or("fasty");
        let path_scale = 14.0f32 / atlas.font_size();
        let mut text_w = 0.0f32;
        for c in basename.chars() {
            if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                text_w += (entry.width + 1.0) * path_scale;
            }
        }

        let mut tx = (viewport_width - text_w) / 2.0f32;
        let scaled_ascent = atlas.ascent() * path_scale;
        let path_baseline_y = (40.0f32 - 14.0f32) / 2.0f32 + scaled_ascent;
        
        for c in basename.chars() {
            if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                if entry.width > 0.0 {
                    let glyph_w = entry.width * path_scale;
                    let glyph_h = entry.height * path_scale;
                    let glyph_x = tx + entry.left * path_scale;
                    let glyph_y = path_baseline_y + entry.top * path_scale;
                    let (aw, ah) = atlas.atlas_size();
                    let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                    let uv_w = uv_end_x - uv_x;
                    let uv_h = uv_end_y - uv_y;

                    fg_instances.push(CellInstance::new(
                        glyph_x, glyph_y,
                        glyph_w, glyph_h,
                        [1.0, 1.0, 1.0, 0.30], // rgba(255,255,255,0.30)
                        [0.0, 0.0, 0.0, 0.0],
                        uv_x, uv_y, uv_w, uv_h,
                        0.0,
                    ));
                    tx += (entry.width + 1.0) * path_scale;
                } else if c == ' ' {
                    tx += 8.0 * path_scale;
                }
            }
        }

        // Draw window controls (Settings, vertical line, Minimize, Maximize, Close)
        let controls_y = 6.0f32; // centered vertically: (40 - 28)/2
        let _icon_scale = 15.0f32 / atlas.font_size();

        // 1. Settings button (⚙)
        let settings_x = viewport_width - 137.0f32;
        if hover_settings {
            bg_instances.push(CellInstance::new(
                settings_x, controls_y,
                28.0, 28.0,
                [1.0, 1.0, 1.0, 0.12], // rgba(255,255,255,0.12)
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0,
            ));
        }
        if let Some(entry) = &atlas.icon_settings {
            let entry_w = 16.0f32;
            let entry_h = 16.0f32;
            let glyph_x = settings_x + (28.0f32 - entry_w) / 2.0f32;
            let glyph_y = controls_y + (28.0f32 - entry_h) / 2.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            let uv_w = uv_end_x - uv_x;
            let uv_h = uv_end_y - uv_y;

            let fg_color = if hover_settings { [1.0, 1.0, 1.0, 1.0] } else { [0.7, 0.7, 0.75, 1.0] };
            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                entry_w, entry_h,
                fg_color,
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_w, uv_h,
                0.0,
            ));
        }

        // 2. Vertical separator line (rgba(255,255,255,0.08))
        bg_instances.push(CellInstance::new(
            viewport_width - 105.0f32, 12.0f32,
            1.0, 16.0,
            [1.0, 1.0, 1.0, 0.08],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            0.0,
        ));

        // 3. Minimize button (─)
        let min_x = viewport_width - 100.0f32;
        if hover_min {
            bg_instances.push(CellInstance::new(
                min_x, controls_y,
                28.0, 28.0,
                [1.0, 1.0, 1.0, 0.12], // rgba(255,255,255,0.12)
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0,
            ));
        }
        if let Some(entry) = &atlas.icon_less {
            let entry_w = 14.0f32;
            let entry_h = 14.0f32;
            let glyph_x = min_x + (28.0f32 - entry_w) / 2.0f32;
            let glyph_y = controls_y + (28.0f32 - entry_h) / 2.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            let uv_w = uv_end_x - uv_x;
            let uv_h = uv_end_y - uv_y;

            let fg_color = if hover_min { [1.0, 1.0, 1.0, 1.0] } else { [0.8, 0.8, 0.85, 1.0] };
            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                entry_w, entry_h,
                fg_color,
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_w, uv_h,
                0.0,
            ));
        }

        // 4. Maximize button (▢)
        let max_x = viewport_width - 68.0f32;
        if hover_max {
            bg_instances.push(CellInstance::new(
                max_x, controls_y,
                28.0, 28.0,
                [1.0, 1.0, 1.0, 0.12], // rgba(255,255,255,0.12)
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0,
            ));
        }
        if let Some(entry) = &atlas.icon_maximize {
            let entry_w = 14.0f32;
            let entry_h = 14.0f32;
            let glyph_x = max_x + (28.0f32 - entry_w) / 2.0f32;
            let glyph_y = controls_y + (28.0f32 - entry_h) / 2.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            let uv_w = uv_end_x - uv_x;
            let uv_h = uv_end_y - uv_y;

            let fg_color = if hover_max { [1.0, 1.0, 1.0, 1.0] } else { [0.8, 0.8, 0.85, 1.0] };
            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                entry_w, entry_h,
                fg_color,
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_w, uv_h,
                0.0,
            ));
        }

        // 5. Close button (✕)
        let close_x = viewport_width - 36.0f32;
        if hover_close {
            bg_instances.push(CellInstance::new(
                close_x, controls_y,
                28.0, 28.0,
                [255.0 / 255.0, 96.0 / 255.0, 96.0 / 255.0, 0.80], // rgba(255,96,96,0.80)
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0,
            ));
        }
        if let Some(entry) = &atlas.icon_close {
            let entry_w = 14.0f32;
            let entry_h = 14.0f32;
            let glyph_x = close_x + (28.0f32 - entry_w) / 2.0f32;
            let glyph_y = controls_y + (28.0f32 - entry_h) / 2.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            let uv_w = uv_end_x - uv_x;
            let uv_h = uv_end_y - uv_y;

            let fg_color = if hover_close { [1.0, 1.0, 1.0, 1.0] } else { [0.8, 0.8, 0.85, 1.0] };
            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                entry_w, entry_h,
                fg_color,
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_w, uv_h,
                0.0,
            ));
        }



        // Draw cursor
        let mut cursor_index = None;
        let cursor_row = content.cursor.point.line.0 + content.display_offset as i32;
        if cursor_row >= 0 && cursor_row < visible_rows as i32 {
            let cursor_x = (content.cursor.point.column.0 as f32 * cell_width).round() + Self::PADDING_LEFT;
            let cursor_y = ((cursor_row as f32 + scroll_fraction) * cell_height).round() + padding_top;

            // Match shape to determine cursor size, offsets, and base opacity
            let (c_w, c_h, c_ox, c_oy, c_alpha) = match content.cursor.shape {
                alacritty_terminal::vte::ansi::CursorShape::Block => (1.0f32, cell_height, 0.0f32, 0.0f32, 0.9f32),
                alacritty_terminal::vte::ansi::CursorShape::Underline => (cell_width, 1.0f32, 0.0f32, cell_height - 1.0f32, 0.9f32),
                alacritty_terminal::vte::ansi::CursorShape::Beam => (1.0f32, cell_height, 0.0f32, 0.0f32, 0.9f32),
                alacritty_terminal::vte::ansi::CursorShape::Hidden => (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32),
                _ => (1.0f32, cell_height, 0.0f32, 0.0f32, 0.9f32),
            };

            if c_w > 0.0 && c_h > 0.0 {
                let final_alpha = if cursor_visible {
                    c_alpha
                } else {
                    0.0
                };

                let cursor_color = [1.0, 1.0, 1.0, final_alpha];
                let cursor_instance = CellInstance::new(
                    cursor_x + c_ox,
                    cursor_y + c_oy,
                    c_w,
                    c_h,
                    cursor_color,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                );
                cursor_index = Some(instances.len());
                instances.push(cursor_instance);
            }
        }

        if scrollbar_alpha > 0.001 {
            let scrollbar_top_margin = padding_top - 10.0f32; // clears the custom title bar and tab bar height
            let track_width = 6.0f32;
            let track_x = viewport_width - track_width - 2.0f32; // 2px from right edge
            let track_y = scrollbar_top_margin;
            let track_h = viewport_height - scrollbar_top_margin - 4.0f32; // 4px bottom margin

            // Draw track (rgba(255,255,255,0.08))
            let track_instance = CellInstance::new(
                track_x, track_y,
                track_width, track_h,
                [1.0, 1.0, 1.0, 0.08 * scrollbar_alpha],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                3.0, // corner radius of 3px (half of 6px width for perfect pill shape)
            );
            instances.push(track_instance);

            // Draw thumb
            let total_lines = visible_rows + history_size;
            if total_lines > 0.0 {
                let ratio = visible_rows / total_lines;
                let thumb_h = (track_h * ratio).max(20.0).min(track_h);

                let scroll_ratio = if history_size > 0.0 {
                    scroll_current / history_size
                } else {
                    0.0
                };

                let thumb_y = track_y + (1.0 - scroll_ratio) * (track_h - thumb_h);

                // Draw thumb (rgba(255,255,255,0.3))
                let thumb_instance = CellInstance::new(
                    track_x, thumb_y,
                    track_width, thumb_h,
                    [1.0, 1.0, 1.0, 0.3 * scrollbar_alpha],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    3.0, // corner radius of 3px
                );
                instances.push(thumb_instance);
            }
        }

        let term_instances_final = instances;
        let mut instances = Vec::new();

        // Draw Toast Popup if any
        if let Some((msg, start_time)) = toast {
            let elapsed_ms = start_time.elapsed().as_millis();
            let alpha = match elapsed_ms {
                t if t < 120 => t as f32 / 120.0,
                t if t < 1620 => 1.0,
                t if t < 1920 => 1.0 - (t - 1620) as f32 / 300.0,
                _ => 0.0,
            };

            if alpha > 0.0 {
                let scale = 13.0 / atlas.font_size();
                let mut text_w = 0.0f32;
                for c in msg.chars() {
                    if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                        if entry.width > 0.0 {
                            text_w += (entry.width + 1.0) * scale;
                        } else if c == ' ' {
                            text_w += 8.0 * scale;
                        }
                    }
                }

                let toast_w = text_w + 28.0;
                let (_, ui_cell_height) = atlas.cell_size();
                let text_h = ui_cell_height * scale;
                let toast_h = text_h + 16.0;

                let toast_x = (viewport_width - toast_w) / 2.0;
                let toast_y = viewport_height - toast_h - 24.0;

                // Outer border
                let border_color = [1.0, 1.0, 1.0, 0.12 * alpha];
                instances.push(CellInstance::new(
                    toast_x, toast_y,
                    toast_w, toast_h,
                    border_color,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    8.0,
                ));

                // Inner background
                let bg_color = [40.0 / 255.0, 42.0 / 255.0, 46.0 / 255.0, 0.95 * alpha];
                instances.push(CellInstance::new(
                    toast_x + 1.0, toast_y + 1.0,
                    toast_w - 2.0, toast_h - 2.0,
                    bg_color,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    7.0,
                ));

                // Text characters
                let mut tx = toast_x + 14.0;
                let baseline_y = toast_y + 8.0 + atlas.ascent() * scale;
                for c in msg.chars() {
                    if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                        if entry.width > 0.0 {
                            let glyph_w = entry.width * scale;
                            let glyph_h = entry.height * scale;
                            let glyph_x = tx + entry.left * scale;
                            let glyph_y = baseline_y + entry.top * scale;
                            let (aw, ah) = atlas.atlas_size();
                            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                            let uv_w = uv_end_x - uv_x;
                            let uv_h = uv_end_y - uv_y;

                            instances.push(CellInstance::new(
                                glyph_x, glyph_y,
                                glyph_w, glyph_h,
                                [1.0, 1.0, 1.0, 0.9 * alpha],
                                [0.0, 0.0, 0.0, 0.0],
                                uv_x, uv_y, uv_w, uv_h,
                                0.0,
                            ));
                            tx += (entry.width + 1.0) * scale;
                        } else if c == ' ' {
                            tx += 8.0 * scale;
                        }
                    }
                }
            }
        }

        // Draw Context Menu if visible
        if context_menu_visible {
            let mut menu_items = Vec::new();
            if selection.is_some() {
                menu_items.push(crate::renderer::ContextMenuItem::Copy);
            }
            menu_items.push(crate::renderer::ContextMenuItem::Paste);
            menu_items.push(crate::renderer::ContextMenuItem::Separator);
            menu_items.push(crate::renderer::ContextMenuItem::NewTab);
            if tab_titles.len() > 1 {
                menu_items.push(crate::renderer::ContextMenuItem::CloseTab);
            }

            let menu_w = 180.0f32; // Target design: Min width 180px
            let padding_y = 6.0f32; // Target design: Padding 6px top and bottom

            // Compute menu_h based on target item sizes
            let mut menu_h = padding_y * 2.0;
            for item in &menu_items {
                menu_h += match item {
                    crate::renderer::ContextMenuItem::Separator => 9.0, // 1px line + 4px top margin + 4px bottom margin
                    _ => 32.0, // Regular items height 32px
                };
            }

            // Animation factors: 80ms fade-in
            let mut opacity_factor = 1.0f32;
            let mut scale_factor = 1.0f32;
            if let Some(open_time) = context_menu_open_time_secs {
                let elapsed = current_time - open_time;
                let progress = (elapsed / 0.080).min(1.0).max(0.0);
                opacity_factor = progress;
                scale_factor = 0.96 + 0.04 * progress;
            }

            // Animation helper closure to translate, scale, and fade every menu element
            let mut anim_push = |x: f32, y: f32, w: f32, h: f32, mut fg: [f32; 4], bg: [f32; 4], uv_x: f32, uv_y: f32, uv_w: f32, uv_h: f32, is_color: f32| {
                let ax = context_menu_x + (x - context_menu_x) * scale_factor;
                let ay = context_menu_y + (y - context_menu_y) * scale_factor;
                let aw = w * scale_factor;
                let ah = h * scale_factor;
                fg[3] *= opacity_factor;
                let mut bg_mod = bg;
                if is_color > 1.5 && (bg[1] > 0.0 || bg[2] > 0.0) {
                    bg_mod[0] = bg[0] * scale_factor;
                    bg_mod[1] = bg[1] * scale_factor;
                    bg_mod[2] = bg[2] * scale_factor;
                }
                instances.push(CellInstance::new(
                    ax, ay, aw, ah,
                    fg, bg_mod,
                    uv_x, uv_y, uv_w, uv_h,
                    is_color
                ));
            };

            // 0. Soft drop shadow (rgba(0, 0, 0, 0.4), offset 0px 4px, blur 8px)
            let shadow_blur = 8.0f32;
            let shadow_pad = 16.0f32;
            let shadow_offset_y = 4.0f32;
            anim_push(
                context_menu_x - shadow_pad,
                context_menu_y - shadow_pad + shadow_offset_y,
                menu_w + 2.0 * shadow_pad,
                menu_h + 2.0 * shadow_pad,
                [0.0, 0.0, 0.0, 0.40],
                [shadow_blur, shadow_pad, shadow_pad, 0.0],
                0.0, 0.0, 1.0, 1.0,
                10.0, // Match menu radius
            );

            // 1. Outer border (rgba(255, 255, 255, 0.10))
            anim_push(
                context_menu_x,
                context_menu_y,
                menu_w,
                menu_h,
                [1.0, 1.0, 1.0, 0.10],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                10.0, // 10px corner radius on all corners
            );

            // 2. Inner background (rgba(36, 38, 42, 0.97))
            anim_push(
                context_menu_x + 1.0,
                context_menu_y + 1.0,
                menu_w - 2.0,
                menu_h - 2.0,
                [36.0 / 255.0, 38.0 / 255.0, 42.0 / 255.0, 0.97],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                9.0, // Alignment inner radius
            );

            // 3. Draw items
            let base_scale = 13.0f32 / atlas.font_size();
            let mut current_y = context_menu_y + padding_y;

            for (idx, item) in menu_items.iter().enumerate() {
                let is_hovered = context_menu_hovered_idx == Some(idx);

                match item {
                    crate::renderer::ContextMenuItem::Separator => {
                        // Horizontal separator line (color: rgba(255, 255, 255, 0.08), height 1px)
                        // width: menu_width - 16px (8px margin on each side)
                        // vertical margin: 4px above and below
                        let sep_x = context_menu_x + 8.0;
                        let sep_w = menu_w - 16.0;
                        anim_push(
                            sep_x,
                            current_y + 4.0,
                            sep_w,
                            1.0,
                            [1.0, 1.0, 1.0, 0.08],
                            [0.0, 0.0, 0.0, 0.0],
                            0.0, 0.0, 1.0, 1.0,
                            0.0,
                        );
                        current_y += 9.0;
                    }
                    _ => {
                        let item_h = 32.0f32; // Height: 32px

                        // Draw hover background: rgba(255, 255, 255, 0.08), border radius 6px
                        // inset 4px from menu edges (x starts at 4, width is menu_w - 8)
                        if is_hovered {
                            anim_push(
                                context_menu_x + 4.0,
                                current_y,
                                menu_w - 8.0,
                                item_h,
                                [1.0, 1.0, 1.0, 0.08],
                                [0.0, 0.0, 0.0, 0.0],
                                0.0, 0.0, 1.0, 1.0,
                                6.0, // border radius of 6px
                            );
                        }

                        // Determine text, icon, and optional shortcut
                        let (_, label, shortcut) = match item {
                            crate::renderer::ContextMenuItem::Copy => ("📋", "Copiar", Some("⌘C")),
                            crate::renderer::ContextMenuItem::Paste => ("📋", "Pegar", Some("⌘V")),
                            crate::renderer::ContextMenuItem::NewTab => ("+", "Nueva pestaña", None),
                            crate::renderer::ContextMenuItem::CloseTab => ("\u{2715}", "Cerrar pestaña", None),
                            _ => ("", "", None),
                        };

                        // Text color: hover = white, normal = rgba(220, 222, 226, 1.0)
                        let text_color = if is_hovered {
                            [1.0, 1.0, 1.0, 1.0]
                        } else {
                            [220.0 / 255.0, 222.0 / 255.0, 226.0 / 255.0, 1.0]
                        };

                        let item_center_y = current_y + item_h / 2.0;
                        let icon_x = context_menu_x + 12.0;
                        let icon_entry = match item {
                            crate::renderer::ContextMenuItem::Copy => atlas.icon_copy.as_ref(),
                            crate::renderer::ContextMenuItem::Paste => atlas.icon_paste.as_ref(),
                            crate::renderer::ContextMenuItem::NewTab => atlas.icon_add.as_ref(),
                            crate::renderer::ContextMenuItem::CloseTab => atlas.icon_close.as_ref(),
                            _ => None,
                        };

                        if let Some(entry) = icon_entry {
                            let entry_w = 14.0f32;
                            let entry_h = 14.0f32;
                            let glyph_x = icon_x;
                            let glyph_y = item_center_y - entry_h / 2.0;

                            let (aw, ah) = atlas.atlas_size();
                            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                            let uv_w = uv_end_x - uv_x;
                            let uv_h = uv_end_y - uv_y;

                            anim_push(
                                glyph_x,
                                glyph_y,
                                entry_w,
                                entry_h,
                                text_color,
                                [0.0, 0.0, 0.0, 0.0],
                                uv_x,
                                uv_y,
                                uv_w,
                                uv_h,
                                0.0,
                            );
                        }

                        // Render text label left-aligned at relative x=30px (context_menu_x + 30px)
                        let (_, ui_cell_height) = atlas.cell_size();
                        let text_baseline_y = item_center_y - (ui_cell_height * base_scale) / 2.0 + atlas.ascent() * base_scale;
                        let mut label_x = context_menu_x + 30.0;
                        for c in label.chars() {
                            if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                                if entry.width > 0.0 {
                                    let glyph_w = entry.width * base_scale;
                                    let glyph_h = entry.height * base_scale;
                                    let glyph_x = label_x + entry.left * base_scale;
                                    let glyph_y = text_baseline_y + entry.top * base_scale;

                                    let (aw, ah) = atlas.atlas_size();
                                    let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                                    let uv_w = uv_end_x - uv_x;
                                    let uv_h = uv_end_y - uv_y;

                                    anim_push(
                                        glyph_x,
                                        glyph_y,
                                        glyph_w,
                                        glyph_h,
                                        text_color,
                                        [0.0, 0.0, 0.0, 0.0],
                                        uv_x,
                                        uv_y,
                                        uv_w,
                                        uv_h,
                                        0.0,
                                    );
                                    label_x += (entry.width + 1.0) * base_scale;
                                } else if c == ' ' {
                                    label_x += 8.0 * base_scale;
                                }
                            }
                        }

                        // Render shortcut text (e.g. ⌘V) aligned right, ending at menu_w - 12.0
                        if let Some(sh) = shortcut {
                            let mut shortcut_w = 0.0f32;
                            for c in sh.chars() {
                                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                                    shortcut_w += (entry.width + 1.0) * base_scale;
                                }
                            }

                            let mut sh_x = context_menu_x + menu_w - 12.0 - shortcut_w;
                            for c in sh.chars() {
                                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                                    if entry.width > 0.0 {
                                        let glyph_w = entry.width * base_scale;
                                        let glyph_h = entry.height * base_scale;
                                        let glyph_x = sh_x + entry.left * base_scale;
                                        let glyph_y = item_center_y - glyph_h / 2.0;

                                        let (aw, ah) = atlas.atlas_size();
                                        let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                                        let uv_w = uv_end_x - uv_x;
                                        let uv_h = uv_end_y - uv_y;

                                        anim_push(
                                            glyph_x,
                                            glyph_y,
                                            glyph_w,
                                            glyph_h,
                                            text_color,
                                            [0.0, 0.0, 0.0, 0.0],
                                            uv_x,
                                            uv_y,
                                            uv_w,
                                            uv_h,
                                            0.0,
                                        );
                                        sh_x += (entry.width + 1.0) * base_scale;
                                    }
                                }
                            }
                        }

                        current_y += item_h;
                    }
                }
            }
        }
        
        let ui_extra_instances_final = instances;

        let term_count = term_instances_final.len();
        let ui_count = ui_bg_instances.len() + ui_fg_instances.len() + ui_extra_instances_final.len();
        tracing::debug!("Render stats: orig_grid_dirty={}, term_count={}, ui_bg={}, ui_fg={}, ui_extra={}", orig_grid_dirty, term_count, ui_bg_instances.len(), ui_fg_instances.len(), ui_extra_instances_final.len());

        let mut final_instances = Vec::with_capacity(term_count + ui_count);
        final_instances.extend(term_instances_final);
        final_instances.extend(ui_bg_instances);
        final_instances.extend(ui_fg_instances);
        final_instances.extend(ui_extra_instances_final);

        let instance_count = final_instances.len().min(self.max_instances);
        
        if instance_count > 0 {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                cast_slice(&final_instances[..instance_count]),
            );
        }
        
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            cast_slice(&[viewport_width, viewport_height]),
        );
        
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        
        let term_draw_count = term_count.min(instance_count);
        if term_draw_count > 0 {
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            let end_offset = (term_draw_count * mem::size_of::<CellInstance>()) as u64;
            render_pass.set_vertex_buffer(1, self.instance_buffer.slice(0..end_offset));
            render_pass.draw(0..6, 0..term_draw_count as u32);
        }

        let ui_draw_count = ui_count.min(instance_count - term_draw_count);
        if ui_draw_count > 0 {
            render_pass.set_bind_group(0, &self.ui_bind_group, &[]);
            let start_offset = (term_draw_count * mem::size_of::<CellInstance>()) as u64;
            let end_offset = ((term_draw_count + ui_draw_count) * mem::size_of::<CellInstance>()) as u64;
            render_pass.set_vertex_buffer(1, self.instance_buffer.slice(start_offset..end_offset));
            render_pass.draw(0..6, 0..ui_draw_count as u32);
        }

        self.last_cursor_index = cursor_index;
        self.last_term_draw_count = term_draw_count;
        self.last_ui_draw_count = ui_draw_count;
        self.cached_final_instances = final_instances;
    }

    pub fn render_settings(
        &self,
        render_pass: &mut wgpu::RenderPass,
        atlas: &mut crate::renderer::Atlas,
        viewport_width: f32,
        viewport_height: f32,
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
        system_fonts: &[String],
        font_scroll_y: f32,
        hovered_font_idx: Option<usize>,
        device: &Device,
        queue: &wgpu::Queue,
    ) {
        let mut bg_instances = Vec::new();
        let mut fg_instances = Vec::new();

        // 0. Draw window background (slate dark)
        bg_instances.push(CellInstance::new(
            0.0, 0.0,
            viewport_width, viewport_height,
            [12.0 / 255.0, 12.0 / 255.0, 12.0 / 255.0, 1.0], // Settings bg (#0c0c0c)
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0,
        ));

        // 1. Draw topbar background (#0a0a0a)
        bg_instances.push(CellInstance::new(
            0.0, 0.0,
            viewport_width, 36.0,
            [10.0 / 255.0, 10.0 / 255.0, 10.0 / 255.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0,
        ));
        bg_instances.push(CellInstance::new(
            0.0, 28.0,
            viewport_width, 8.0,
            [10.0 / 255.0, 10.0 / 255.0, 10.0 / 255.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 0.0, 0.0,
            0.0,
        ));
        bg_instances.push(CellInstance::new(
            0.0, 36.0,
            viewport_width, 1.0,
            [1.0, 1.0, 1.0, 0.06], // Consistent with main topbar border
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 0.0, 0.0,
            0.0,
        ));

        // Helper to draw text helper
        let draw_text = |atlas: &mut crate::renderer::Atlas, text: &str, start_x: f32, start_y: f32, color: [f32; 4], fg_list: &mut Vec<CellInstance>| {
            let mut x = start_x;
            for c in text.chars() {
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        let glyph_x = x + entry.left;
                        let glyph_y = start_y + atlas.ascent() + entry.top;
                        let (aw, ah) = atlas.atlas_size();
                        let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                        fg_list.push(CellInstance::new(
                            glyph_x, glyph_y,
                            entry.width, entry.height,
                            color,
                            [0.0, 0.0, 0.0, 0.0],
                            uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                            0.0,
                        ));
                        x += entry.width + 2.0;
                    } else if c == ' ' {
                        x += 8.0;
                    }
                }
            }
        };

        // Draw title
        draw_text(atlas, "Settings", 12.0, 6.0, [0.85, 0.85, 0.90, 1.0], &mut fg_instances);

        // Draw topbar close button
        if hover_close {
            bg_instances.push(CellInstance::new(
                viewport_width - 32.0, 4.0,
                28.0, 28.0,
                [0.85, 0.25, 0.25, 0.9],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0, // Rounded rectangle
            ));
        }
        if let Some(entry) = &atlas.icon_close {
            let entry_w = 14.0f32;
            let entry_h = 14.0f32;
            let glyph_x = (viewport_width - 32.0) + (28.0 - entry_w) / 2.0;
            let glyph_y = 4.0 + (28.0 - entry_h) / 2.0;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                entry_w, entry_h,
                [0.8, 0.8, 0.85, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                0.0,
            ));
        }

        // Draw labels
        draw_text(atlas, "Font Family:", 20.0, 56.0, [0.75, 0.75, 0.80, 1.0], &mut fg_instances);
        draw_text(atlas, "Font Size:", 20.0, 96.0, [0.75, 0.75, 0.80, 1.0], &mut fg_instances);
        draw_text(atlas, "Scrollback:", 20.0, 136.0, [0.75, 0.75, 0.80, 1.0], &mut fg_instances);

        // Draw inputs
        // 1. Font Family select box
        let family_bg = if active_field == 1 {
            [25.0 / 255.0, 25.0 / 255.0, 32.0 / 255.0, 1.0]
        } else if hover_font_family {
            [22.0 / 255.0, 22.0 / 255.0, 28.0 / 255.0, 1.0]
        } else {
            [16.0 / 255.0, 16.0 / 255.0, 20.0 / 255.0, 1.0]
        };
        bg_instances.push(CellInstance::new(
            140.0, 52.0,
            240.0, 26.0,
            family_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0,
        ));

        // Draw text-font SVG icon
        if let Some(entry) = &atlas.icon_text_font {
            let entry_w = 14.0f32;
            let entry_h = 14.0f32;
            let glyph_x = 148.0f32;
            let glyph_y = 52.0f32 + (26.0f32 - entry_h) / 2.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                entry_w, entry_h,
                [0.7, 0.7, 0.75, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                0.0,
            ));
        }

        // Draw font family text name
        draw_text(atlas, font_family, 168.0, 56.0, [0.9, 0.9, 0.95, 1.0], &mut fg_instances);
        // Draw dropdown arrow icon (▾)
        draw_text(atlas, "▾", 362.0, 56.0, [0.7, 0.7, 0.75, 1.0], &mut fg_instances);

        // 2. Font Size controls
        let size_minus_bg = if hover_size_minus { [1.0, 1.0, 1.0, 0.15] } else { [1.0, 1.0, 1.0, 0.05] };
        bg_instances.push(CellInstance::new(
            140.0, 92.0,
            28.0, 26.0,
            size_minus_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0,
        ));
        if let Some(entry) = &atlas.icon_less {
            let entry_w = 14.0f32;
            let entry_h = 14.0f32;
            let glyph_x = 140.0f32 + (28.0f32 - entry_w) / 2.0f32;
            let glyph_y = 92.0f32 + (26.0f32 - entry_h) / 2.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                entry_w, entry_h,
                [0.9, 0.9, 0.95, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                0.0,
            ));
        }

        draw_text(atlas, &format!("{:.1}", font_size), 180.0, 96.0, [0.9, 0.9, 0.95, 1.0], &mut fg_instances);

        let size_plus_bg = if hover_size_plus { [1.0, 1.0, 1.0, 0.15] } else { [1.0, 1.0, 1.0, 0.05] };
        bg_instances.push(CellInstance::new(
            220.0, 92.0,
            28.0, 26.0,
            size_plus_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0,
        ));
        if let Some(entry) = &atlas.icon_add {
            let entry_w = 14.0f32;
            let entry_h = 14.0f32;
            let glyph_x = 220.0f32 + (28.0f32 - entry_w) / 2.0f32;
            let glyph_y = 92.0f32 + (26.0f32 - entry_h) / 2.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                entry_w, entry_h,
                [0.9, 0.9, 0.95, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                0.0,
            ));
        }

        // 3. Scrollback controls
        let scroll_minus_bg = if hover_scroll_minus { [1.0, 1.0, 1.0, 0.15] } else { [1.0, 1.0, 1.0, 0.05] };
        bg_instances.push(CellInstance::new(
            140.0, 132.0,
            28.0, 26.0,
            scroll_minus_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0,
        ));
        if let Some(entry) = &atlas.icon_less {
            let entry_w = 14.0f32;
            let entry_h = 14.0f32;
            let glyph_x = 140.0f32 + (28.0f32 - entry_w) / 2.0f32;
            let glyph_y = 132.0f32 + (26.0f32 - entry_h) / 2.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                entry_w, entry_h,
                [0.9, 0.9, 0.95, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                0.0,
            ));
        }

        draw_text(atlas, &format!("{}", scrollback), 180.0, 136.0, [0.9, 0.9, 0.95, 1.0], &mut fg_instances);

        let scroll_plus_bg = if hover_scroll_plus { [1.0, 1.0, 1.0, 0.15] } else { [1.0, 1.0, 1.0, 0.05] };
        bg_instances.push(CellInstance::new(
            240.0, 132.0,
            28.0, 26.0,
            scroll_plus_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0,
        ));
        if let Some(entry) = &atlas.icon_add {
            let entry_w = 14.0f32;
            let entry_h = 14.0f32;
            let glyph_x = 240.0f32 + (28.0f32 - entry_w) / 2.0f32;
            let glyph_y = 132.0f32 + (26.0f32 - entry_h) / 2.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                entry_w, entry_h,
                [0.9, 0.9, 0.95, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                0.0,
            ));
        }

        // Save & Cancel buttons
        let save_bg = if hover_save {
            [40.0 / 255.0, 120.0 / 255.0, 60.0 / 255.0, 1.0]
        } else {
            [30.0 / 255.0, 90.0 / 255.0, 45.0 / 255.0, 1.0]
        };
        bg_instances.push(CellInstance::new(
            90.0, 220.0,
            100.0, 32.0,
            save_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0,
        ));
        draw_text(atlas, "Save", 125.0, 226.0, [1.0, 1.0, 1.0, 1.0], &mut fg_instances);

        let cancel_bg = if hover_cancel {
            [80.0 / 255.0, 80.0 / 255.0, 90.0 / 255.0, 1.0]
        } else {
            [60.0 / 255.0, 60.0 / 255.0, 70.0 / 255.0, 1.0]
        };
        bg_instances.push(CellInstance::new(
            210.0, 220.0,
            100.0, 32.0,
            cancel_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0,
        ));
        draw_text(atlas, "Cancel", 235.0, 226.0, [1.0, 1.0, 1.0, 1.0], &mut fg_instances);

        // Draw scrollable dropdown list if active_field == 1
        if active_field == 1 {
            let drop_x = 140.0f32;
            let drop_y = 78.0f32; // 52.0 + 26.0
            let drop_w = 240.0f32;
            let drop_h = 180.0f32;

            // Draw dropdown background shadow
            bg_instances.push(CellInstance::new(
                drop_x - 4.0, drop_y - 2.0,
                drop_w + 8.0, drop_h + 8.0,
                [0.0, 0.0, 0.0, 0.35],
                [4.0, 4.0, 4.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0,
            ));

            // Draw dropdown border
            bg_instances.push(CellInstance::new(
                drop_x, drop_y,
                drop_w, drop_h,
                [1.0, 1.0, 1.0, 0.08],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0,
            ));

            // Draw dropdown background (#08080a) - 100% opaque alpha!
            bg_instances.push(CellInstance::new(
                drop_x + 1.0, drop_y + 1.0,
                drop_w - 2.0, drop_h - 2.0,
                [8.0 / 255.0, 8.0 / 255.0, 10.0 / 255.0, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                5.0,
            ));

            // Draw scrollable items
            let item_h = 22.0f32;
            let padding_x = 8.0f32;

            for (i, font) in system_fonts.iter().enumerate() {
                let item_top_y = drop_y + i as f32 * item_h - font_scroll_y;
                let item_bottom_y = item_top_y + item_h;

                // Simple clipping check: only render if inside the dropdown height
                if item_bottom_y > drop_y && item_top_y < drop_y + drop_h {
                    let is_selected = font == font_family;
                    let is_hovered = hovered_font_idx == Some(i);

                    // Draw item background on hover/selection
                    if is_hovered {
                        bg_instances.push(CellInstance::new(
                            drop_x + 4.0, item_top_y + 1.0,
                            drop_w - 8.0, item_h - 2.0,
                            [1.0, 1.0, 1.0, 0.08],
                            [0.0, 0.0, 0.0, 0.0],
                            0.0, 0.0, 1.0, 1.0,
                            4.0,
                        ));
                    } else if is_selected {
                        bg_instances.push(CellInstance::new(
                            drop_x + 4.0, item_top_y + 1.0,
                            drop_w - 8.0, item_h - 2.0,
                            [91.0 / 255.0, 138.0 / 255.0, 240.0 / 255.0, 0.15],
                            [0.0, 0.0, 0.0, 0.0],
                            0.0, 0.0, 1.0, 1.0,
                            4.0,
                        ));
                    }

                    // Render item text
                    let text_color = if is_selected {
                        [91.0 / 255.0, 138.0 / 255.0, 240.0 / 255.0, 1.0] // Blue text for active selection
                    } else if is_hovered {
                        [1.0, 1.0, 1.0, 1.0]
                    } else {
                        [0.85, 0.85, 0.90, 1.0]
                    };

                    // Draw text inside item (vertically centered)
                    let text_y = item_top_y + (item_h - atlas.cell_size().1) / 2.0;
                    let mut tx = drop_x + padding_x;
                    
                    // Simple text clipping: truncate if too long
                    let max_text_w = drop_w - padding_x * 2.0 - 10.0; 
                    let mut current_w = 0.0f32;

                    for c in font.chars() {
                        if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                            if entry.width > 0.0 {
                                let glyph_w = entry.width;
                                let glyph_h = entry.height;
                                if current_w + glyph_w > max_text_w {
                                    break;
                                }
                                
                                let glyph_x = tx + entry.left;
                                let glyph_y = text_y + atlas.ascent() + entry.top;

                                // Clip glyph vertically to dropdown client area
                                if glyph_y + glyph_h <= drop_y + drop_h - 2.0 && glyph_y >= drop_y + 2.0 {
                                    let (aw, ah) = atlas.atlas_size();
                                    let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                                    fg_instances.push(CellInstance::new(
                                        glyph_x, glyph_y,
                                        glyph_w, glyph_h,
                                        text_color,
                                        [0.0, 0.0, 0.0, 0.0],
                                        uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                                        0.0,
                                    ));
                                }
                                tx += entry.width + 1.0;
                                current_w += entry.width + 1.0;
                            } else if c == ' ' {
                                tx += 8.0;
                                current_w += 8.0;
                            }
                        }
                    }
                }
            }

            // Draw scrollbar if necessary
            let total_h = system_fonts.len() as f32 * item_h;
            if total_h > drop_h {
                let sbar_w = 4.0f32;
                let sbar_x = drop_x + drop_w - sbar_w - 2.0;
                let sbar_h = (drop_h / total_h) * drop_h;
                let sbar_y = drop_y + (font_scroll_y / total_h) * drop_h;

                bg_instances.push(CellInstance::new(
                    sbar_x, sbar_y,
                    sbar_w, sbar_h,
                    [1.0, 1.0, 1.0, 0.25],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    2.0,
                ));
            }
        }

        // Write buffer and draw
        let bg_count = bg_instances.len();
        let fg_count = fg_instances.len();
        let mut instances = Vec::with_capacity(bg_count + fg_count);
        instances.extend(bg_instances);
        instances.extend(fg_instances);

        let instance_count = instances.len().min(self.max_instances);
        if instance_count > 0 {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                cast_slice(&instances[..instance_count]),
            );
        }

        queue.write_buffer(
            &self.uniform_buffer,
            0,
            cast_slice(&[viewport_width, viewport_height]),
        );

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));

        if instance_count > 0 {
            render_pass.draw(0..6, 0..instance_count as u32);
        }
    }
}

fn cell_fg_to_f32(
    color: alacritty_terminal::vte::ansi::Color,
    _flags: alacritty_terminal::term::cell::Flags,
) -> [f32; 4] {
    match color {
        alacritty_terminal::vte::ansi::Color::Spec(rgb) => {
            [rgb.r as f32 / 255.0, rgb.g as f32 / 255.0, rgb.b as f32 / 255.0, 1.0]
        }
        alacritty_terminal::vte::ansi::Color::Named(named) => {
            let (r, g, b) = named_color_rgb(named);
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
        }
        alacritty_terminal::vte::ansi::Color::Indexed(idx) => {
            let rgb = index_to_ansi_color(idx as usize);
            [rgb.0 as f32 / 255.0, rgb.1 as f32 / 255.0, rgb.2 as f32 / 255.0, 1.0]
        }
    }
}

fn cell_bg_to_f32(color: alacritty_terminal::vte::ansi::Color) -> [f32; 4] {
    match color {
        alacritty_terminal::vte::ansi::Color::Spec(rgb) => {
            [rgb.r as f32 / 255.0, rgb.g as f32 / 255.0, rgb.b as f32 / 255.0, 1.0]
        }
        alacritty_terminal::vte::ansi::Color::Named(named) => {
            let (r, g, b) = named_color_rgb(named);
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
        }
        alacritty_terminal::vte::ansi::Color::Indexed(idx) => {
            let rgb = index_to_ansi_color(idx as usize);
            [rgb.0 as f32 / 255.0, rgb.1 as f32 / 255.0, rgb.2 as f32 / 255.0, 1.0]
        }
    }
}

fn named_color_rgb(named: alacritty_terminal::vte::ansi::NamedColor) -> (u8, u8, u8) {
    match named {
        alacritty_terminal::vte::ansi::NamedColor::Foreground => (0xC5, 0xC8, 0xC6),
        alacritty_terminal::vte::ansi::NamedColor::Background => (0x1D, 0x1F, 0x21),
        alacritty_terminal::vte::ansi::NamedColor::Black => (0x1D, 0x1F, 0x21),
        alacritty_terminal::vte::ansi::NamedColor::Red => (0xCC, 0x66, 0x66),
        alacritty_terminal::vte::ansi::NamedColor::Green => (0xB5, 0xBD, 0x68),
        alacritty_terminal::vte::ansi::NamedColor::Yellow => (0xF0, 0xC6, 0x74),
        alacritty_terminal::vte::ansi::NamedColor::Blue => (0x81, 0xA2, 0xBE),
        alacritty_terminal::vte::ansi::NamedColor::Magenta => (0xB2, 0x94, 0xBB),
        alacritty_terminal::vte::ansi::NamedColor::Cyan => (0x8A, 0xBE, 0xB7),
        alacritty_terminal::vte::ansi::NamedColor::White => (0xC5, 0xC8, 0xC6),
        alacritty_terminal::vte::ansi::NamedColor::BrightBlack => (0x66, 0x66, 0x66),
        alacritty_terminal::vte::ansi::NamedColor::BrightRed => (0xFF, 0x33, 0x34),
        alacritty_terminal::vte::ansi::NamedColor::BrightGreen => (0x9E, 0xC4, 0x00),
        alacritty_terminal::vte::ansi::NamedColor::BrightYellow => (0xF0, 0xC6, 0x74),
        alacritty_terminal::vte::ansi::NamedColor::BrightBlue => (0x81, 0xA2, 0xBE),
        alacritty_terminal::vte::ansi::NamedColor::BrightMagenta => (0xB7, 0x7E, 0xE0),
        alacritty_terminal::vte::ansi::NamedColor::BrightCyan => (0x54, 0xCE, 0xD6),
        alacritty_terminal::vte::ansi::NamedColor::BrightWhite => (0xFF, 0xFF, 0xFF),
        _ => (0xC5, 0xC8, 0xC6),
    }
}

fn index_to_ansi_color(idx: usize) -> (u8, u8, u8) {
    if idx < 16 {
        const ANSI_COLORS: [(u8, u8, u8); 16] = [
            (0x1D, 0x1F, 0x21),
            (0xCC, 0x66, 0x66),
            (0xB5, 0xBD, 0x68),
            (0xF0, 0xC6, 0x74),
            (0x81, 0xA2, 0xBE),
            (0xB2, 0x94, 0xBB),
            (0x8A, 0xBE, 0xB7),
            (0xC5, 0xC8, 0xC6),
            (0x66, 0x66, 0x66),
            (0xFF, 0x33, 0x34),
            (0x9E, 0xC4, 0x00),
            (0xF0, 0xC6, 0x74),
            (0x81, 0xA2, 0xBE),
            (0xB7, 0x7E, 0xE0),
            (0x54, 0xCE, 0xD6),
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