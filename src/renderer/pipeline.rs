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
        // Corner radius of 4.0 pixels (since track width is 8.0, 4.0 radius forms a perfect pill shape)
        let r = 4.0;
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
        return vec4<f32>(input.fg_color.rgb, tex_color.r);
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
    const PADDING_TOP: f32 = 10.0;

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
        device: &Device,
        queue: &wgpu::Queue,
    ) {
        let term = terminal.term();
        let term_guard = term.lock();
        let content = term_guard.renderable_content();

        let mut bg_instances = Vec::new();
        let mut fg_instances = Vec::new();

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

            // 1. Draw solid background if not default
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

        let bg_count = bg_instances.len();
        let fg_count = fg_instances.len();

        let mut instances = Vec::with_capacity(bg_count + fg_count + 2);
        instances.extend(bg_instances);
        instances.extend(fg_instances);

        if scrollbar_alpha > 0.001 {
            let track_width = 8.0f32;
            let track_margin = 2.0f32;
            let track_x = viewport_width - track_width - track_margin;
            let track_y = 0.0f32;
            let track_h = viewport_height;

            // Draw track (semi-transparent gray/white, noticeable but subtle)
            let track_instance = CellInstance::new(
                track_x, track_y,
                track_width, track_h,
                [1.0, 1.0, 1.0, 0.15 * scrollbar_alpha],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                2.0,
            );
            instances.push(track_instance);

            // Draw thumb
            let total_lines = visible_rows + history_size;
            if total_lines > 0.0 {
                let ratio = visible_rows / total_lines;
                let thumb_h = (track_h * ratio).max(30.0).min(track_h);

                let scroll_ratio = if history_size > 0.0 {
                    scroll_current / history_size
                } else {
                    0.0
                };

                let thumb_y = (1.0 - scroll_ratio) * (track_h - thumb_h);

                // Draw thumb (noticeable gray but not too bright)
                let thumb_instance = CellInstance::new(
                    track_x, thumb_y,
                    track_width, thumb_h,
                    [0.6, 0.6, 0.6, 0.60 * scrollbar_alpha],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    2.0,
                );
                instances.push(thumb_instance);
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