//! Render pipeline for terminal cells.

use std::mem;

use alacritty_terminal::grid::Indexed;
use bytemuck::cast_slice;
use wgpu::{Buffer, Device, RenderPipeline};

use crate::renderer::CellInstance;
use crate::terminal_state::TerminalState;

pub struct Pipeline {
    pipeline: RenderPipeline,
    instance_buffer: Buffer,
    vertex_buffer: Buffer,
    uniform_buffer: Buffer,
    bind_group: wgpu::BindGroup,
    max_instances: usize,
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
        let b = input.size / 2.0;
        let q = abs(p) - b + vec2<f32>(r);
        let dist = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
        let alpha = 1.0 - smoothstep(-0.5, 0.5, dist);
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
    pub fn new(device: &Device, atlas: &crate::renderer::Atlas, format: wgpu::TextureFormat) -> Self {
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cell-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let atlas_view = atlas.texture().create_view(&wgpu::TextureViewDescriptor {
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

        Self {
            pipeline,
            instance_buffer,
            vertex_buffer,
            uniform_buffer,
            bind_group,
            max_instances,
        }
    }

    const PADDING_LEFT: f32 = 10.0;
    const PADDING_TOP: f32 = 46.0;

    pub fn render(
        &self,
        render_pass: &mut wgpu::RenderPass,
        terminal: &TerminalState,
        atlas: &mut crate::renderer::Atlas,
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
        last_activity_time_secs: f32,
        current_time: f32,
        selection: Option<crate::renderer::Selection>,
        hovered_url: Option<crate::renderer::HoveredUrl>,
        toast: Option<(&str, std::time::Instant)>,
        device: &Device,
        queue: &wgpu::Queue,
    ) {
        let term = terminal.term();
        let term_guard = term.lock();
        let content = term_guard.renderable_content();

        let mut bg_instances = Vec::new();
        let mut fg_instances = Vec::new();

        // 0. Draw window background (slate dark)
        let window_bg = CellInstance::new(
            0.0, 0.0,
            viewport_width, viewport_height,
            [8.0 / 255.0, 8.0 / 255.0, 10.0 / 255.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0, // radius of 8
        );
        bg_instances.push(window_bg);

        // 1. Draw topbar background (slightly lighter dark)
        let topbar_bg = CellInstance::new(
            0.0, 0.0,
            viewport_width, 36.0,
            [18.0 / 255.0, 18.0 / 255.0, 22.0 / 255.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0, // radius of 8 at top
        );
        bg_instances.push(topbar_bg);

        // 2. Draw square block to cover bottom rounded corners of the topbar
        let topbar_bottom_fill = CellInstance::new(
            0.0, 28.0,
            viewport_width, 8.0,
            [18.0 / 255.0, 18.0 / 255.0, 22.0 / 255.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 0.0, 0.0,
            0.0, // radius 0 (square)
        );
        bg_instances.push(topbar_bottom_fill);

        // 3. Draw a subtle bottom border/divider line for the topbar
        let topbar_border = CellInstance::new(
            0.0, 36.0,
            viewport_width, 1.0,
            [35.0 / 255.0, 35.0 / 255.0, 45.0 / 255.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 0.0, 0.0,
            0.0,
        );
        bg_instances.push(topbar_border);

        let scroll_fraction = scroll_current - content.display_offset as f32;

        for Indexed { cell, point } in content.display_iter {
            let col = point.column.0 as usize;
            let row = (point.line.0 + content.display_offset as i32) as usize;

            let is_default_bg = matches!(cell.bg,
                alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Background
                )
            );

            let cell_x = (col as f32 * cell_width).round() + Self::PADDING_LEFT;
            let cell_y = ((row as f32 + scroll_fraction) * cell_height).round() + Self::PADDING_TOP;

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

        // Draw app icon in the topbar
        if let Some(entry) = atlas.app_icon {
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            let uv_w = uv_end_x - uv_x;
            let uv_h = uv_end_y - uv_y;
            
            fg_instances.push(CellInstance::new(
                10.0, 6.0,
                24.0, 24.0,
                [1.0, 1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_w, uv_h,
                1.0, // is_color = 1.0 (rgba from texture)
            ));
        }

        // Draw title text "fasty"
        let title = "fasty";
        let mut title_x = 42.0;
        for c in title.chars() {
            if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                let glyph_x = title_x + entry.left;
                let glyph_y = 6.0 + atlas.ascent() + entry.top;
                let (aw, ah) = atlas.atlas_size();
                let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                let uv_w = uv_end_x - uv_x;
                let uv_h = uv_end_y - uv_y;

                fg_instances.push(CellInstance::new(
                    glyph_x, glyph_y,
                    entry.width, entry.height,
                    [0.85, 0.85, 0.90, 1.0],
                    [0.0, 0.0, 0.0, 0.0],
                    uv_x, uv_y, uv_w, uv_h,
                    0.0,
                ));
                title_x += entry.width + 2.0;
            }
        }

        // Draw control buttons (Settings, Minimize, Maximize, Close)
        let buttons = [
            ('\u{2699}', viewport_width - 128.0, hover_settings, [0.7, 0.7, 0.75, 1.0], [1.0, 1.0, 1.0, 0.15]),
            ('\u{2500}', viewport_width - 96.0, hover_min, [0.8, 0.8, 0.85, 1.0], [1.0, 1.0, 1.0, 0.15]),
            ('\u{25A2}', viewport_width - 64.0, hover_max, [0.8, 0.8, 0.85, 1.0], [1.0, 1.0, 1.0, 0.15]),
            ('\u{2715}', viewport_width - 32.0, hover_close, [0.8, 0.8, 0.85, 1.0], [0.85, 0.25, 0.25, 0.9]),
        ];
        for (c, bx, is_hovered, fg_color, bg_color) in buttons {
            if is_hovered {
                bg_instances.push(CellInstance::new(
                    bx, 4.0,
                    28.0, 28.0,
                    bg_color,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    6.0, // corner radius of 6.0
                ));
            }
            if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                let glyph_x = bx + (28.0 - entry.width) / 2.0;
                let glyph_y = 4.0 + (28.0 - entry.height) / 2.0;
                let (aw, ah) = atlas.atlas_size();
                let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                let uv_w = uv_end_x - uv_x;
                let uv_h = uv_end_y - uv_y;

                fg_instances.push(CellInstance::new(
                    glyph_x, glyph_y,
                    entry.width, entry.height,
                    fg_color,
                    [0.0, 0.0, 0.0, 0.0],
                    uv_x, uv_y, uv_w, uv_h,
                    0.0,
                ));
            }
        }

        let bg_count = bg_instances.len();
        let fg_count = fg_instances.len();

        let mut instances = Vec::with_capacity(bg_count + fg_count + 10);
        instances.extend(bg_instances);
        instances.extend(fg_instances);

        // Draw cursor
        let cursor_row = content.cursor.point.line.0 + content.display_offset as i32;
        if cursor_row >= 0 && cursor_row < visible_rows as i32 {
            let cursor_x = (content.cursor.point.column.0 as f32 * cell_width).round() + Self::PADDING_LEFT;
            let cursor_y = ((cursor_row as f32 + scroll_fraction) * cell_height).round() + Self::PADDING_TOP;

            // Match shape to determine cursor size, offsets, and base opacity
            let (c_w, c_h, c_ox, c_oy, c_alpha) = match content.cursor.shape {
                alacritty_terminal::vte::ansi::CursorShape::Block => (1.0f32, cell_height, 0.0f32, 0.0f32, 0.9f32),
                alacritty_terminal::vte::ansi::CursorShape::Underline => (cell_width, 1.0f32, 0.0f32, cell_height - 1.0f32, 0.9f32),
                alacritty_terminal::vte::ansi::CursorShape::Beam => (1.0f32, cell_height, 0.0f32, 0.0f32, 0.9f32),
                alacritty_terminal::vte::ansi::CursorShape::Hidden => (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32),
                _ => (1.0f32, cell_height, 0.0f32, 0.0f32, 0.9f32),
            };

            if c_w > 0.0 && c_h > 0.0 {
                let cursor_is_active = (current_time - last_activity_time_secs) < 0.5f32;
                // Apply pulsing animation if cursor is idle (quieto)
                let final_alpha = if cursor_is_active {
                    c_alpha
                } else {
                    // Start pulsing animation smoothly from c_alpha by basing phase on the time since the activity period ended (500ms after last activity)
                    let activity_end_time = last_activity_time_secs + 0.5f32;
                    let idle_time = (current_time - activity_end_time).max(0.0f32);
                    // Use a cosine wave so it starts at its maximum value (cos(0) = 1.0) and starts fading down smoothly.
                    // Frequency of 0.8 Hz means 0.8 cycles per second (1.25s per full cycle).
                    let pulse = 0.5f32 + 0.5f32 * (idle_time * std::f32::consts::PI * 2.0f32 * 0.8f32).cos();
                    c_alpha * pulse
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
                instances.push(cursor_instance);
            }
        }

        if scrollbar_alpha > 0.001 {
            const SCROLLBAR_TOP_MARGIN: f32 = 36.0; // px — clears the custom title bar height
            let track_width = 6.0f32;
            let track_x = viewport_width - track_width - 2.0f32; // 2px from right edge
            let track_y = SCROLLBAR_TOP_MARGIN;
            let track_h = viewport_height - SCROLLBAR_TOP_MARGIN - 4.0f32; // 4px bottom margin

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
                let scale = 13.0 / 16.0;
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
                let text_h = cell_height * scale;
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
        device: &Device,
        queue: &wgpu::Queue,
    ) {
        let mut bg_instances = Vec::new();
        let mut fg_instances = Vec::new();

        // 0. Draw window background (slate dark)
        bg_instances.push(CellInstance::new(
            0.0, 0.0,
            viewport_width, viewport_height,
            [8.0 / 255.0, 8.0 / 255.0, 10.0 / 255.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0,
        ));

        // 1. Draw topbar background
        bg_instances.push(CellInstance::new(
            0.0, 0.0,
            viewport_width, 36.0,
            [18.0 / 255.0, 18.0 / 255.0, 22.0 / 255.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0,
        ));
        bg_instances.push(CellInstance::new(
            0.0, 28.0,
            viewport_width, 8.0,
            [18.0 / 255.0, 18.0 / 255.0, 22.0 / 255.0, 1.0],
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 0.0, 0.0,
            0.0,
        ));
        bg_instances.push(CellInstance::new(
            0.0, 36.0,
            viewport_width, 1.0,
            [35.0 / 255.0, 35.0 / 255.0, 45.0 / 255.0, 1.0],
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
                6.0,
            ));
        }
        if let Some(entry) = atlas.get_or_rasterize('\u{2715}', device, queue) {
            let glyph_x = (viewport_width - 32.0) + (28.0 - entry.width) / 2.0;
            let glyph_y = 4.0 + (28.0 - entry.height) / 2.0;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            fg_instances.push(CellInstance::new(
                glyph_x, glyph_y,
                entry.width, entry.height,
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
        // 1. Font Family input box
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
        
        let display_family = if active_field == 1 {
            format!("{}|", font_family)
        } else {
            font_family.to_string()
        };
        draw_text(atlas, &display_family, 146.0, 56.0, [0.9, 0.9, 0.95, 1.0], &mut fg_instances);

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
        draw_text(atlas, "-", 150.0, 96.0, [0.9, 0.9, 0.95, 1.0], &mut fg_instances);

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
        draw_text(atlas, "+", 230.0, 96.0, [0.9, 0.9, 0.95, 1.0], &mut fg_instances);

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
        draw_text(atlas, "-", 150.0, 136.0, [0.9, 0.9, 0.95, 1.0], &mut fg_instances);

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
        draw_text(atlas, "+", 250.0, 136.0, [0.9, 0.9, 0.95, 1.0], &mut fg_instances);

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