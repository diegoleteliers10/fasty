//! Render pipeline for terminal cells.

use std::mem;

use alacritty_terminal::grid::Indexed;
use bytemuck::cast_slice;
use wgpu::{Buffer, Device, RenderPipeline};

use crate::renderer::{CellInstance, RenderReason, RowShapingResult};
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

fn sd_segment(p: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>) -> f32 {
    let pa = p - p1;
    let ba = p2 - p1;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn sd_connection(p: vec2<f32>, center: vec2<f32>, size: vec2<f32>, dir: u32, style: u32, light: f32, heavy: f32, space: f32) -> f32 {
    if (style == 0u) { return 1e6; }
    
    var p1 = center;
    var p2 = center;
    if (dir == 0u) { // Left
        p1 = vec2<f32>(0.0, center.y);
    } else if (dir == 1u) { // Right
        p1 = vec2<f32>(size.x, center.y);
    } else if (dir == 2u) { // Top
        p1 = vec2<f32>(center.x, 0.0);
    } else { // Bottom
        p1 = vec2<f32>(center.x, size.y);
    }
    
    if (style == 1u) {
        return sd_segment(p, p1, p2) - light / 2.0;
    } else if (style == 2u) {
        return sd_segment(p, p1, p2) - heavy / 2.0;
    } else if (style == 3u) {
        // Double lines
        var d1 = 1e6;
        var d2 = 1e6;
        if (dir == 0u || dir == 1u) { // Horizontal
            d1 = sd_segment(p, p1 - vec2<f32>(0.0, space), p2 - vec2<f32>(0.0, space));
            d2 = sd_segment(p, p1 + vec2<f32>(0.0, space), p2 + vec2<f32>(0.0, space));
        } else { // Vertical
            d1 = sd_segment(p, p1 - vec2<f32>(space, 0.0), p2 - vec2<f32>(space, 0.0));
            d2 = sd_segment(p, p1 + vec2<f32>(space, 0.0), p2 + vec2<f32>(space, 0.0));
        }
        return min(d1, d2) - light / 2.0;
    }
    return 1e6;
}

fn draw_block_or_box(code: u32, kind: u32, left_style: u32, right_style: u32, top_style: u32, bottom_style: u32, uv: vec2<f32>, size: vec2<f32>, frag_pos: vec2<f32>) -> f32 {
    let p = uv * size;
    let center = size / 2.0;
    
    if (code >= 0x2580u && code <= 0x259Fu) {
        // Block Elements
        if (code == 0x2588u) { return 1.0; } // Full block
        if (code == 0x2580u) { return f32(uv.y <= 0.5); } // Upper half
        if (code == 0x2584u) { return f32(uv.y >= 0.5); } // Lower half
        if (code == 0x258Cu) { return f32(uv.x <= 0.5); } // Left half
        if (code == 0x2590u) { return f32(uv.x >= 0.5); } // Right half
        
        // Lower levels
        if (code == 0x2581u) { return f32(uv.y >= 0.875); }
        if (code == 0x2582u) { return f32(uv.y >= 0.75); }
        if (code == 0x2583u) { return f32(uv.y >= 0.625); }
        if (code == 0x2585u) { return f32(uv.y >= 0.375); }
        if (code == 0x2586u) { return f32(uv.y >= 0.25); }
        if (code == 0x2587u) { return f32(uv.y >= 0.125); }
        
        // Left levels
        if (code == 0x2589u) { return f32(uv.x <= 0.875); }
        if (code == 0x258Au) { return f32(uv.x <= 0.75); }
        if (code == 0x258Bu) { return f32(uv.x <= 0.625); }
        if (code == 0x258Du) { return f32(uv.x <= 0.375); }
        if (code == 0x258Eu) { return f32(uv.x <= 0.25); }
        if (code == 0x258Fu) { return f32(uv.x <= 0.125); }
        
        // Right and Upper levels
        if (code == 0x2594u) { return f32(uv.y <= 0.125); }
        if (code == 0x2595u) { return f32(uv.x >= 0.875); }
        
        // Quadrants
        if (code == 0x2596u) { return f32(uv.x <= 0.5 && uv.y >= 0.5); }
        if (code == 0x2597u) { return f32(uv.x >= 0.5 && uv.y >= 0.5); }
        if (code == 0x2598u) { return f32(uv.x <= 0.5 && uv.y <= 0.5); }
        if (code == 0x259Du) { return f32(uv.x >= 0.5 && uv.y <= 0.5); }
        
        if (code == 0x2599u) { return f32(!(uv.x >= 0.5 && uv.y <= 0.5)); }
        if (code == 0x259Au) { return f32((uv.x <= 0.5 && uv.y <= 0.5) || (uv.x >= 0.5 && uv.y >= 0.5)); }
        if (code == 0x259Bu) { return f32(!(uv.x >= 0.5 && uv.y >= 0.5)); }
        if (code == 0x259Cu) { return f32(!(uv.x <= 0.5 && uv.y >= 0.5)); }
        if (code == 0x259Eu) { return f32((uv.x >= 0.5 && uv.y <= 0.5) || (uv.x <= 0.5 && uv.y >= 0.5)); }
        if (code == 0x259Fu) { return f32(!(uv.x <= 0.5 && uv.y <= 0.5)); }

        // Shades (25%, 50%, 75%) using screen coordinates for stipple pattern
        let px = u32(frag_pos.x);
        let py = u32(frag_pos.y);
        if (code == 0x2591u) { // 25% light shade
            return f32((px % 2u == 0u) && (py % 2u == 0u));
        }
        if (code == 0x2592u) { // 50% medium shade
            return f32((px + py) % 2u == 0u);
        }
        if (code == 0x2593u) { // 75% dark shade
            return f32(!((px % 2u == 0u) && (py % 2u == 0u)));
        }
    } else if (code >= 0x2500u && code <= 0x257Fu) {
        // Box Drawing
        let light = max(1.0, size.x * 0.08);
        let heavy = light * 2.2;
        let space = light * 1.5;
        
        if (kind == 1u) {
            // Round corners
            let radius = min(center.x, center.y);
            var c = center;
            var is_active_corner = false;
            if (right_style > 0u && bottom_style > 0u) { // ╭
                c = center + vec2<f32>(radius, radius);
                is_active_corner = p.x < c.x && p.y < c.y;
            } else if (left_style > 0u && bottom_style > 0u) { // ╮
                c = center + vec2<f32>(-radius, radius);
                is_active_corner = p.x > c.x && p.y < c.y;
            } else if (left_style > 0u && top_style > 0u) { // ╯
                c = center + vec2<f32>(-radius, -radius);
                is_active_corner = p.x > c.x && p.y > c.y;
            } else if (right_style > 0u && top_style > 0u) { // ╰
                c = center + vec2<f32>(radius, -radius);
                is_active_corner = p.x < c.x && p.y > c.y;
            }
            
            if (is_active_corner) {
                let dist = abs(length(p - c) - radius) - light / 2.0;
                return 1.0 - smoothstep(-0.75, 0.75, dist);
            }
            return 0.0;
        } else if (kind == 2u) {
            // Diagonals
            var dist = 1e6;
            if (code == 0x2571u) {
                dist = sd_segment(p, vec2<f32>(0.0, size.y), vec2<f32>(size.x, 0.0)) - light / 2.0;
            } else if (code == 0x2572u) {
                dist = sd_segment(p, vec2<f32>(0.0, 0.0), vec2<f32>(size.x, size.y)) - light / 2.0;
            } else if (code == 0x2573u) {
                dist = min(
                    sd_segment(p, vec2<f32>(0.0, size.y), vec2<f32>(size.x, 0.0)),
                    sd_segment(p, vec2<f32>(0.0, 0.0), vec2<f32>(size.x, size.y))
                ) - light / 2.0;
            }
            return 1.0 - smoothstep(-0.75, 0.75, dist);
        } else {
            // Normal & Dashed Lines
            var d_left = sd_connection(p, center, size, 0u, left_style, light, heavy, space);
            var d_right = sd_connection(p, center, size, 1u, right_style, light, heavy, space);
            var d_top = sd_connection(p, center, size, 2u, top_style, light, heavy, space);
            var d_bottom = sd_connection(p, center, size, 3u, bottom_style, light, heavy, space);
            
            // If dashed, mask it out
            if (kind == 3u) {
                let dash_len = light * 4.0;
                if (left_style > 0u && p.x < center.x) {
                    let dash = u32(p.x / dash_len) % 2u;
                    if (dash == 1u) { d_left = 1e6; }
                }
                if (right_style > 0u && p.x > center.x) {
                    let dash = u32((p.x - center.x) / dash_len) % 2u;
                    if (dash == 1u) { d_right = 1e6; }
                }
                if (top_style > 0u && p.y < center.y) {
                    let dash = u32(p.y / dash_len) % 2u;
                    if (dash == 1u) { d_top = 1e6; }
                }
                if (bottom_style > 0u && p.y > center.y) {
                    let dash = u32((p.y - center.y) / dash_len) % 2u;
                    if (dash == 1u) { d_bottom = 1e6; }
                }
            }
            
            let min_d = min(min(d_left, d_right), min(d_top, d_bottom));
            return 1.0 - smoothstep(-0.75, 0.75, min_d);
        }
    }
    return 0.0;
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
    if (input.is_color < -0.5) {
        let abs_val = -input.is_color;
        let code = u32(abs_val);
        let kind = u32(round((abs_val - f32(code)) * 10.0));
        
        let left_style = u32(round(input.bg_color.r));
        let right_style = u32(round(input.bg_color.g));
        let top_style = u32(round(input.bg_color.b));
        let bottom_style = u32(round(input.bg_color.a));
        
        let alpha = draw_block_or_box(
            code, kind,
            left_style, right_style, top_style, bottom_style,
            input.uv, input.size, input.position.xy
        );
        
        return vec4<f32>(input.fg_color.rgb, input.fg_color.a * alpha);
    }

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
};
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
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
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
        ligatures: bool,
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
        hovered_hyperlink: Option<&str>,
        search_matches: &[crate::renderer::SearchMatch],
        search_current_idx: usize,
        search_visible: bool,
        search_query_render: &str,
        terminal_font_size: f32,
        toast: Option<(&str, std::time::Instant, u64)>,
        active_tab_index: usize,
        tab_titles: &[String],
        _active_tab_path: &str,
        context_menu_visible: bool,
        context_menu_is_about: bool,
        context_menu_x: f32,
        context_menu_y: f32,
        context_menu_hovered_idx: Option<usize>,
        context_menu_open_time_secs: Option<f32>,
        hovered_tab_index: Option<usize>,
        hovered_close_tab_index: Option<usize>,
        hover_new_tab: bool,
        update_available: bool,
        update_in_progress: bool,
        update_completed: bool,
        hover_update: bool,
        cached_grid_instances: &mut Vec<CellInstance>,
        grid_dirty: &mut bool,
        device: &Device,
        queue: &wgpu::Queue,
        command_palette_visible: bool,
        command_palette_query: &str,
        command_palette_selected: usize,
        command_palette_filtered: &[String],
        dragging_tab: Option<usize>,
        drag_current_x: f32,
        drop_target_idx: Option<usize>,
        tab_ctx_visible: bool,
        tab_ctx_x: f32,
        tab_ctx_y: f32,
        tab_ctx_hovered: Option<usize>,
        renaming_tab: Option<usize>,
        rename_buffer: &str,
        rename_cursor: usize,
        git_status: Option<&crate::git::GitStatus>,
        ssh_picker_visible: bool,
        ssh_picker_query: &str,
        ssh_picker_selected: usize,
        ssh_filtered: &[String],
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
        let term = terminal.term();
        let term_guard = term.lock();
        let content = term_guard.renderable_content();

        let mut ui_bg_instances = Vec::new();
        let mut ui_fg_instances = Vec::new();

        // Topbar color is a darker variant of the theme bg so it visually
        // separates from the terminal area. The default theme keeps its
        // hand-tuned #0a0a0a; other themes derive their topbar from bg * 0.83.
        let (bg_r, bg_g, bg_b) = named_color_rgb(alacritty_terminal::vte::ansi::NamedColor::Background);
        let theme_bg = [bg_r as f32 / 255.0, bg_g as f32 / 255.0, bg_b as f32 / 255.0, 1.0];
        let active_theme = crate::config::ACTIVE_THEME.read().to_lowercase();
        let theme_topbar = if active_theme == "default" {
            [10.0 / 255.0, 10.0 / 255.0, 10.0 / 255.0, 1.0]
        } else {
            [
                bg_r as f32 / 255.0 * 0.83,
                bg_g as f32 / 255.0 * 0.83,
                bg_b as f32 / 255.0 * 0.83,
                1.0,
            ]
        };

        // 1. Draw unified bar background (darker variant of theme bg)
        let bar_bg = CellInstance::new(
            0.0, 0.0,
            viewport_width, 40.0,
            theme_topbar,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0, // radius of 8 at top corners
        );
        ui_bg_instances.push(bar_bg);

        // 2. Draw square block to cover bottom rounded corners of the topbar
        let bar_bottom_fill = CellInstance::new(
            0.0, 32.0,
            viewport_width, 8.0,
            theme_topbar,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 0.0, 0.0,
            0.0, // square
        );
        ui_bg_instances.push(bar_bottom_fill);

        // (Unified bar bottom border is now drawn segmented below in the tabs rendering logic to bypass the active tab)

        let scroll_fraction = scroll_current - content.display_offset as f32;

        let mut instances = if *grid_dirty {
            let mut term_bg_instances = Vec::new();
            let mut term_fg_instances = Vec::new();

            // 0. Draw window background (slate dark)
            let window_bg = CellInstance::new(
                0.0, 0.0,
                viewport_width, viewport_height,
                theme_bg, // Terminal bg color
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                8.0, // radius of 8
            );
            term_bg_instances.push(window_bg);

            // 3. Draw terminal background below the unified topbar,
            //    stopping before the bottombar area (15px).
            let terminal_bg = CellInstance::new(
                0.0, 40.0,
                viewport_width, viewport_height - 40.0 - 20.0,
                theme_bg,
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                0.0,
            );
            term_bg_instances.push(terminal_bg);

            // 4. (removed: terminal bg no longer has rounded top corners)

            let bg_instances = &mut term_bg_instances;
            let fg_instances = &mut term_fg_instances;

            let visible_rows_count = visible_rows.round() as usize;
            let mut row_cells: Vec<Vec<&alacritty_terminal::term::cell::Cell>> = vec![Vec::new(); visible_rows_count];
            let mut row_points: Vec<Vec<alacritty_terminal::index::Point>> = vec![Vec::new(); visible_rows_count];
            let mut row_hyperlinks: Vec<Vec<Option<std::sync::Arc<str>>>> = vec![Vec::new(); visible_rows_count];

            // 1. Draw solid background, selection, and underline, and collect cells for layout
            for Indexed { cell, point } in content.display_iter {
                let col = point.column.0 as usize;
                let row = (point.line.0 + content.display_offset as i32) as usize;

                if row < visible_rows_count {
                    row_cells[row].push(cell);
                    row_points[row].push(point);
                    row_hyperlinks[row].push(cell.hyperlink().map(|h| std::sync::Arc::from(h.uri())));
                }

                let is_default_bg = matches!(cell.bg,
                    alacritty_terminal::vte::ansi::Color::Named(
                        alacritty_terminal::vte::ansi::NamedColor::Background
                    )
                );

                let in_search_match = search_matches.iter().any(|m| {
                    m.line == point.line.0
                        && col >= m.col && col < m.col + m.len
                });
                let is_current_search_match = search_matches.get(search_current_idx)
                    .map(|m| m.line == point.line.0
                        && col >= m.col && col < m.col + m.len)
                    .unwrap_or(false);

                let cell_x = (col as f32 * cell_width).round() + Self::PADDING_LEFT;
                let next_cell_x = ((col + 1) as f32 * cell_width).round() + Self::PADDING_LEFT;
                let actual_cell_width = next_cell_x - cell_x;

                let cell_y = ((row as f32 + scroll_fraction) * cell_height).round() + padding_top;
                let next_cell_y = (((row + 1) as f32 + scroll_fraction) * cell_height).round() + padding_top;
                let actual_cell_height = next_cell_y - cell_y;

                // Selection check
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
                    let cell_w = if is_wide {
                        let end_x = ((col + 2) as f32 * cell_width).round() + Self::PADDING_LEFT;
                        end_x - cell_x
                    } else {
                        actual_cell_width
                    };

                    let bg = cell_bg_to_f32(cell.bg);
                    let bg_instance = CellInstance::new(
                        cell_x, cell_y,
                        cell_w, actual_cell_height,
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
                        actual_cell_width, actual_cell_height,
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
                        cell_x, cell_y + actual_cell_height - 2.0,
                        actual_cell_width, 1.0,
                        underline_color,
                        [0.0, 0.0, 0.0, 0.0],
                        0.0, 0.0, 0.0, 0.0,
                        0.0,
                    );
                    bg_instances.push(bg_instance);
                }

                if !is_hovered_url {
                    if let Some(Some(uri)) = row_hyperlinks.get(row).and_then(|r| r.get(col)) {
                        let is_this_hovered = hovered_hyperlink == Some(uri.as_ref());
                        let underline_color = if is_this_hovered {
                            [66.0 / 255.0, 135.0 / 255.0, 245.0 / 255.0, 1.0]
                        } else {
                            [66.0 / 255.0, 135.0 / 255.0, 245.0 / 255.0, 0.55]
                        };
                        let bg_instance = CellInstance::new(
                            cell_x, cell_y + actual_cell_height - 2.0,
                            actual_cell_width, 1.0,
                            underline_color,
                            [0.0, 0.0, 0.0, 0.0],
                            0.0, 0.0, 0.0, 0.0,
                            0.0,
                        );
                        bg_instances.push(bg_instance);
                    }
                }

                if in_search_match {
                    let highlight_color = if is_current_search_match {
                        [255.0 / 255.0, 140.0 / 255.0, 0.0, 0.90]
                    } else {
                        [255.0 / 255.0, 220.0 / 255.0, 0.0, 0.75]
                    };
                    let bg_instance = CellInstance::new(
                        cell_x, cell_y,
                        actual_cell_width, actual_cell_height,
                        highlight_color,
                        [0.0, 0.0, 0.0, 0.0],
                        0.0, 0.0, 0.0, 0.0,
                        0.0,
                    );
                    bg_instances.push(bg_instance);
                }
            }

            // 2. Render text / glyph foreground (either shaped or cell-by-cell)
            let use_ligatures = ligatures && atlas.hb_face().is_some();
            let mut row_shaping = vec![None; visible_rows_count];

            if use_ligatures {
                for row in 0..visible_rows_count {
                    let cells = &row_cells[row];
                    if cells.is_empty() { continue; }

                    // 1. Build row text
                    let mut row_text = String::new();
                    for cell in cells.iter() {
                        row_text.push(cell.c);
                    }

                    // 2. Check cache
                    if let Some(cached) = atlas.shaping_cache.get(&row_text) {
                        row_shaping[row] = Some(cached.clone());
                        continue;
                    }

                    // 3. Cache miss: construct col_map and shape
                    let mut col_map = Vec::new();
                    for (idx, cell) in cells.iter().enumerate() {
                        let char_len = cell.c.len_utf8();
                        for _ in 0..char_len {
                            col_map.push(idx);
                        }
                    }
                    col_map.push(cells.len());

                    if let Some(hb_face) = atlas.hb_face() {
                        let mut buffer = rustybuzz::UnicodeBuffer::new();
                        buffer.push_str(&row_text);
                        buffer.set_direction(rustybuzz::Direction::LeftToRight);
                        let shaped = rustybuzz::shape(hb_face, &[], buffer);
                        let glyph_infos = shaped.glyph_infos().to_vec();

                        if !glyph_infos.is_empty() {
                            let result = RowShapingResult {
                                glyph_infos,
                                col_map,
                            };
                            if atlas.shaping_cache.len() >= 500 {
                                atlas.shaping_cache.clear();
                            }
                            atlas.shaping_cache.insert(row_text.clone(), result.clone());
                            row_shaping[row] = Some(result);
                        }
                    }
                }
            }

            for row in 0..visible_rows_count {
                let cells = &row_cells[row];
                if cells.is_empty() { continue; }

                if let Some(shaping_result) = &row_shaping[row] {
                    let col_map = &shaping_result.col_map;
                    let glyph_infos = &shaping_result.glyph_infos;

                    for (g_idx, info) in glyph_infos.iter().enumerate() {
                        let cluster = info.cluster as usize;
                        if cluster >= col_map.len() { continue; }
                        let start_col = col_map[cluster];
                        let end_col = if g_idx + 1 < glyph_infos.len() {
                            let next_cluster = glyph_infos[g_idx + 1].cluster as usize;
                            if next_cluster < col_map.len() {
                                col_map[next_cluster]
                            } else {
                                cells.len()
                            }
                        } else {
                            cells.len()
                        };

                        if end_col <= start_col { continue; }
                        let cell = &cells[start_col];

                        if cell.c != ' ' && cell.c != '\0' {
                            let is_emoji_or_block_or_wide = crate::renderer::is_emoji(cell.c)
                                || is_custom_block_drawing(cell.c)
                                || crate::renderer::is_block_element(cell.c)
                                || cell.flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR);

                            let cell_x = (start_col as f32 * cell_width).round() + Self::PADDING_LEFT;
                            let cell_y = ((row as f32 + scroll_fraction) * cell_height).round() + padding_top;
                            let next_cell_y = (((row + 1) as f32 + scroll_fraction) * cell_height).round() + padding_top;
                            let actual_cell_height = next_cell_y - cell_y;

                            if !is_emoji_or_block_or_wide {
                                if let Some(entry) = atlas.get_or_rasterize_glyph(info.glyph_id, device, queue) {
                                    if entry.width > 0.0 && entry.height > 0.0 {
                                        let mut fg = cell_fg_to_f32(cell.fg, cell.flags);
                                        if cell.c == '❯' {
                                            fg = [0.35, 0.75, 0.35, 1.0];
                                        }
                                        let (aw, ah) = atlas.atlas_size();
                                        let raw_uv = entry.uv_coords(aw, ah);
                                        let [uv_x, uv_y, uv_end_x, uv_end_y] = raw_uv;
                                        let uv_w = uv_end_x - uv_x;
                                        let uv_h = uv_end_y - uv_y;

                                        let glyph_x = (cell_x + entry.left).round();
                                        let glyph_y = (cell_y + atlas.ascent() + entry.top).round();

                                        let text_instance = CellInstance::new(
                                            glyph_x,
                                            glyph_y,
                                            entry.width,
                                            entry.height,
                                            fg,
                                            [0.0, 0.0, 0.0, 0.0],
                                            uv_x, uv_y, uv_w, uv_h,
                                            0.0,
                                        );
                                        fg_instances.push(text_instance);
                                    }
                                }
                            } else {
                                for sub_col in start_col..end_col {
                                    let sub_cell = &cells[sub_col];
                                    if sub_cell.c != ' ' && sub_cell.c != '\0' {
                                        let sub_x = (sub_col as f32 * cell_width).round() + Self::PADDING_LEFT;
                                        let next_sub_x = ((sub_col + 1) as f32 * cell_width).round() + Self::PADDING_LEFT;
                                        let sub_w = next_sub_x - sub_x;

                                        render_single_char(
                                            sub_cell, sub_x, cell_y, sub_w, actual_cell_height,
                                            atlas, fg_instances, device, queue
                                        );
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }

                // Ligature fallback: render cell-by-cell
                for (col_idx, cell) in cells.iter().enumerate() {
                    if cell.c != ' ' && cell.c != '\0' {
                        let cell_x = (col_idx as f32 * cell_width).round() + Self::PADDING_LEFT;
                        let next_cell_x = ((col_idx + 1) as f32 * cell_width).round() + Self::PADDING_LEFT;
                        let actual_cell_width = next_cell_x - cell_x;
                        let cell_y = ((row as f32 + scroll_fraction) * cell_height).round() + padding_top;
                        let next_cell_y = (((row + 1) as f32 + scroll_fraction) * cell_height).round() + padding_top;
                        let actual_cell_height = next_cell_y - cell_y;

                        render_single_char(
                            cell, cell_x, cell_y, actual_cell_width, actual_cell_height,
                            atlas, fg_instances, device, queue
                        );
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

        // Bottombar — always rendered (15px strip at the bottom).
        // Git info is shown only when the active tab is in a git repo.
        {
            const BB_H: f32 = 20.0;
            let bb_x = 0.0f32;
            let bb_y = viewport_height - BB_H;
            let bb_w = viewport_width;

            let (bg_r, bg_g, bg_b) = named_color_rgb(alacritty_terminal::vte::ansi::NamedColor::Background);
            let theme_bg = [bg_r as f32 / 255.0, bg_g as f32 / 255.0, bg_b as f32 / 255.0, 1.0];

            // Flat bottombar background — same color as terminal bg
            bg_instances.push(CellInstance::new(
                bb_x, bb_y,
                bb_w, BB_H,
                theme_bg,
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                0.0,
            ));

            // Git info — only when in a repo
            if let Some(gs) = git_status {
                let bb_font: f32 = 12.0;
                let scale = bb_font / ui_atlas.font_size();
                let cell_w = ui_atlas.cell_size().0 * scale;
                let scaled_ascent = ui_atlas.ascent() * scale;
                let descent = scaled_ascent * 0.3;
                let text_block_h = scaled_ascent + descent;
                let baseline_y = bb_y + (BB_H - text_block_h) / 2.0 + scaled_ascent;

                macro_rules! bb_text {
                    ($text:expr, $start_x:expr, $color:expr) => {{
                        let mut x = $start_x;
                        for c in $text.chars() {
                            if c == ' ' { x += cell_w; continue; }
                            if let Some(entry) = ui_atlas.get_or_rasterize(c, device, queue) {
                                if entry.width > 0.0 {
                                    let gw = entry.width * scale;
                                    let glyph_x = (x + entry.left * scale).round();
                                    let glyph_y = (baseline_y + entry.top * scale).round();
                                    let (aw, ah) = ui_atlas.atlas_size();
                                    let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                                    fg_instances.push(CellInstance::new(
                                        glyph_x, glyph_y,
                                        gw, entry.height * scale,
                                        $color,
                                        [0.0, 0.0, 0.0, 0.0],
                                        uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                                        0.0,
                                    ));
                                    x += gw + 1.0;
                                }
                            }
                        }
                        x
                    }};
                }

                let mut x = 8.0f32;

                // Branch icon
                if let Some(entry) = &ui_atlas.icon_branch {
                    let icon_size = bb_font;
                    let icon_y = bb_y + (BB_H - icon_size) / 2.0;
                    let (aw, ah) = ui_atlas.atlas_size();
                    let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                    fg_instances.push(CellInstance::new(
                        x, icon_y,
                        icon_size, icon_size,
                        [0.75, 0.78, 0.85, 1.0],
                        [0.0, 0.0, 0.0, 0.0],
                        uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                        0.0,
                    ));
                    x += icon_size + 4.0;
                }

                // Branch name
                let branch_color = [0.85, 0.88, 0.95, 1.0];
                x = bb_text!(gs.branch.as_str(), x, branch_color);

                // Dirty indicator dot
                if !gs.is_clean() {
                    x += 6.0;
                    let dot_r = 2.5f32;
                    let dot_y = bb_y + BB_H * 0.5;
                    bg_instances.push(CellInstance::new(
                        x - dot_r, dot_y - dot_r,
                        dot_r * 2.0, dot_r * 2.0,
                        [0.95, 0.80, 0.45, 1.0],
                        [0.0, 0.0, 0.0, 0.0],
                        0.0, 0.0, 0.0, 0.0,
                        dot_r,
                    ));
                    x += dot_r * 2.0 + 6.0;
                }

                // Separator
                x += 2.0;
                let sep_color = [1.0, 1.0, 1.0, 0.08];
                bg_instances.push(CellInstance::new(
                    x, bb_y + 3.0,
                    1.0, BB_H - 6.0,
                    sep_color,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                ));
                x += 8.0;

                // Ahead
                if gs.ahead > 0 {
                    let s = format!("\u{2191}{}", gs.ahead);
                    x = bb_text!(s.as_str(), x, [0.45, 0.85, 0.55, 1.0]) + 6.0;
                }

                // Behind
                if gs.behind > 0 {
                    let s = format!("\u{2193}{}", gs.behind);
                    x = bb_text!(s.as_str(), x, [0.90, 0.55, 0.40, 1.0]) + 6.0;
                }

                // Modified
                if gs.modified > 0 {
                    let s = format!("~{}", gs.modified);
                    x = bb_text!(s.as_str(), x, [0.95, 0.80, 0.45, 1.0]) + 6.0;
                }

                // Staged
                if gs.staged > 0 {
                    let s = format!("+{}", gs.staged);
                    x = bb_text!(s.as_str(), x, [0.50, 0.80, 0.95, 1.0]) + 6.0;
                }

                // Untracked
                if gs.untracked > 0 {
                    let s = format!("?{}", gs.untracked);
                    x = bb_text!(s.as_str(), x, [0.65, 0.65, 0.75, 1.0]) + 6.0;
                }

                // Last commit summary — right-aligned, dimmed
                if !gs.last_commit_summary.is_empty() {
                    let summary_text = if gs.last_commit_summary.len() > 60 {
                        format!("{}...", &gs.last_commit_summary[..57])
                    } else {
                        gs.last_commit_summary.clone()
                    };
                    // Measure text width to right-align
                    let mut text_w = 0.0f32;
                    for c in summary_text.chars() {
                        if c == ' ' { text_w += cell_w; continue; }
                        if let Some(entry) = ui_atlas.get_or_rasterize(c, device, queue) {
                            text_w += entry.width * scale + 1.0;
                        }
                    }
                    let right_x = viewport_width - text_w - 8.0;
                    if right_x > x + 16.0 {
                        let _ = bb_text!(summary_text.as_str(), right_x, [0.50, 0.50, 0.55, 0.6]);
                    }
                }
            }
        }

        let atlas = ui_atlas;

        // In-scrollback search bar (Ctrl+Shift+F) — floating block in the
        // top-right corner, just below the topbar with an 8px top margin.
        if search_visible {
            let bar_w = (viewport_width * 0.32).clamp(280.0, 480.0);
            let bar_h = 52.0f32;
            let bar_x = viewport_width - bar_w - 16.0;
            let bar_y = 40.0f32 + 8.0f32;
            let bar_radius = 8.0f32;

            // Floating block background with rounded corners
            let bar_bg = CellInstance::new(
                bar_x, bar_y,
                bar_w, bar_h,
                [18.0 / 255.0, 18.0 / 255.0, 22.0 / 255.0, 0.98],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                bar_radius,
            );
            bg_instances.push(bar_bg);

            // Subtle 1px border to define the block against the terminal
            let border = CellInstance::new(
                bar_x + 0.5, bar_y + 0.5,
                bar_w - 1.0, bar_h - 1.0,
                [1.0, 1.0, 1.0, 0.10],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                bar_radius,
            );
            bg_instances.push(border);

            // True monospace cell: every char occupies a fixed-width slot
            // and the glyph is centered inside it. Same pattern as the
            // terminal text rendering (cell_x = col * cell_width), so the
            // visible spacing between letters is consistent and the cursor
            // can sit exactly on a cell boundary.
            //
            // The ui_atlas is rasterized at (font_size * scale_factor)
            // physical pixels, so we scale the bitmap by cell_h / physical
            // to render it inside our cell. Without that denominator, on
            // HiDPI displays the bitmap is drawn 2x too big and gets
            // clipped by the bar.
            let cell_w = 9.0f32;
            let cell_h = 14.0f32;
            let physical_font_size = atlas.font_size() * atlas.scale_factor();
            let scale = cell_h / physical_font_size;
            let ascent_phys = atlas.ascent();
            let ascent = ascent_phys * scale;
            // Descent is roughly 25% of ascent for typical monospace fonts.
            // Including it in the centered text block keeps descenders
            // ('p', 'g', 'q', 'y') inside the bar.
            let descent_approx = ascent * 0.3;
            let total_text_h = ascent + descent_approx;
            let baseline_y = bar_y + (bar_h - total_text_h) * 0.5 + ascent;

            let mut cx = bar_x + 14.0f32;
            let text_right_edge = bar_x + bar_w - 70.0f32;

            for c in search_query_render.chars() {
                if c == '\0' { continue; }
                if cx > text_right_edge { break; }
                if c == ' ' {
                    cx += cell_w;
                    continue;
                }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width <= 0.0 || entry.height <= 0.0 {
                        cx += cell_w;
                        continue;
                    }
                    let gw = entry.width * scale;
                    let gh = entry.height * scale;
                    let (aw, ah) = atlas.atlas_size();
                    let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                    let uv_w = uv_end_x - uv_x;
                    let uv_h = uv_end_y - uv_y;
                    let glyph_y = baseline_y + entry.top * scale;
                    // Center glyph horizontally in its fixed cell so the
                    // inter-letter gap is uniform regardless of glyph shape.
                    let glyph_x = cx + (cell_w - gw) * 0.5;
                    fg_instances.push(CellInstance::new(
                        glyph_x, glyph_y,
                        gw, gh,
                        [1.0, 1.0, 1.0, 1.0],
                        [0.0, 0.0, 0.0, 0.0],
                        uv_x, uv_y, uv_w, uv_h,
                        0.0,
                    ));
                    cx += cell_w;
                }
            }

            let cursor_visible = ((current_time * 2.0).sin() as f32).abs() > 0.1;
            if cursor_visible && cx <= text_right_edge {
                // Cursor spans the cap-height (from top of cap to baseline),
                // matching the actual text metrics so it lines up with the
                // rendered letters.
                let cursor_top = baseline_y - ascent;
                fg_instances.push(CellInstance::new(
                    cx, cursor_top,
                    1.5, ascent,
                    [1.0, 1.0, 1.0, 0.85],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                ));
            }

            // Count "X / Y" on the right (same monospace cells, right-aligned)
            let total = search_matches.len();
            let current = if total > 0 { search_current_idx + 1 } else { 0 };
            let count_str = format!("{} / {}", current, total);
            let count_color = if total == 0 {
                [0.6, 0.6, 0.6, 1.0]
            } else {
                [1.0, 0.85, 0.30, 1.0]
            };
            let count_len = count_str.chars().count();
            let count_total_w = count_len as f32 * cell_w;
            let mut count_x = bar_x + bar_w - count_total_w - 14.0;
            for c in count_str.chars() {
                if c == ' ' {
                    count_x += cell_w;
                    continue;
                }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width <= 0.0 || entry.height <= 0.0 {
                        count_x += cell_w;
                        continue;
                    }
                    let gw = entry.width * scale;
                    let gh = entry.height * scale;
                    let (aw, ah) = atlas.atlas_size();
                    let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                    let uv_w = uv_end_x - uv_x;
                    let uv_h = uv_end_y - uv_y;
                    let glyph_y = baseline_y + entry.top * scale;
                    let glyph_x = count_x + (cell_w - gw) * 0.5;
                    fg_instances.push(CellInstance::new(
                        glyph_x, glyph_y,
                        gw, gh,
                        count_color,
                        [0.0, 0.0, 0.0, 0.0],
                        uv_x, uv_y, uv_w, uv_h,
                        0.0,
                    ));
                    count_x += cell_w;
                }
            }
        }

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
                let glyph_x = (8.0f32 + (16.0f32 - glyph_w) / 2.0f32).round();
                let glyph_y = (12.0f32 + (16.0f32 - glyph_h) / 2.0f32).round();
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

        // Draw segmented bottom border (1px solid rgba(255,255,255,0.06)) to separate topbar from terminal
        // active tab has NO bottom border, allowing it to merge seamlessly.
        if active_tab_index < tab_titles.len() {
            let active_tab_x = tab_start_x + active_tab_index as f32 * tab_width;
            
            // Left segment
            if active_tab_x > 0.0 {
                bg_instances.push(CellInstance::new(
                    0.0, 39.0,
                    active_tab_x, 1.0,
                    [1.0, 1.0, 1.0, 0.06],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                ));
            }
            
            // Right segment
            let right_start = active_tab_x + tab_width;
            if right_start < viewport_width {
                bg_instances.push(CellInstance::new(
                    right_start, 39.0,
                    viewport_width - right_start, 1.0,
                    [1.0, 1.0, 1.0, 0.06],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                ));
            }
        } else {
            bg_instances.push(CellInstance::new(
                0.0, 39.0,
                viewport_width, 1.0,
                [1.0, 1.0, 1.0, 0.06],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 0.0, 0.0,
                0.0,
            ));
        }

        let scale = 13.0f32 / atlas.font_size();

        for (i, title) in tab_titles.iter().enumerate() {
            let tab_x = tab_start_x + i as f32 * tab_width;
            let is_active = i == active_tab_index;
            let is_hovered = hovered_tab_index == Some(i);

            // Active tab bg matches theme terminal bg so the tab merges seamlessly
            // with the terminal area below. Inactive tab bg is transparent.
            if is_active {
                bg_instances.push(CellInstance::new(
                    tab_x, 0.0,
                    tab_width, 40.0,
                    theme_bg,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                ));

                // Left border of active tab (subtle 1px vertical line)
                bg_instances.push(CellInstance::new(
                    tab_x, 0.0,
                    1.0, 40.0,
                    [1.0, 1.0, 1.0, 0.12],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                ));
                // Right border of active tab (subtle 1px vertical line)
                bg_instances.push(CellInstance::new(
                    tab_x + tab_width, 0.0,
                    1.0, 40.0,
                    [1.0, 1.0, 1.0, 0.12],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                ));
            }

            // Draw vertical separator (1px vertical line) between inactive tabs only (active tab is separated by its own borders)
            if i + 1 < tab_titles.len() && i != active_tab_index && i + 1 != active_tab_index {
                bg_instances.push(CellInstance::new(
                    tab_x + tab_width, 12.0,
                    1.0, 16.0,
                    [1.0, 1.0, 1.0, 0.05], // Very subtle separator
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
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
                [1.0, 1.0, 1.0, 1.0] // Active tab text (full opacity)
            } else if is_hovered {
                [1.0, 1.0, 1.0, 0.70] // Hover tab text (slightly increased opacity)
            } else {
                [1.0, 1.0, 1.0, 0.30] // Inactive tab text (low opacity)
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
                [1.0, 1.0, 1.0, 0.04], // Extremely subtle hover feedback
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
                [1.0, 1.0, 1.0, 0.70] // Consistent with hover tab text
            } else {
                [1.0, 1.0, 1.0, 0.35] // Consistent with inactive tab text
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

        // Tab drag reorder visual feedback
        if dragging_tab.is_some() {
            if let Some(target) = drop_target_idx {
                let tab_start_x = 36.0f32;
                let tab_w = if tab_titles.len() > 0 {
                    (tab_area_width / tab_titles.len() as f32).clamp(80.0f32, 160.0f32)
                } else {
                    160.0f32
                };
                let indicator_x = tab_start_x + target as f32 * tab_w;
                bg_instances.push(CellInstance::new(
                    indicator_x - 1.0, 4.0,
                    2.0, 32.0,
                    [0.4, 0.6, 1.0, 0.8],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 0.0, 0.0,
                    0.0,
                ));
            }
        }

        // Centered path display removed — the active tab already shows its name.

        // Draw window controls (Settings, vertical line, Minimize, Maximize, Close)
        let controls_y = 6.0f32; // centered vertically: (40 - 28)/2
        let _icon_scale = 15.0f32 / atlas.font_size();

        // 0. Update button (if update is available)
        let settings_x = viewport_width - 137.0f32;
        if update_available {
            let update_btn_w = 70.0f32;
            let update_btn_h = 20.0f32;
            let update_x = settings_x - update_btn_w - 12.0;
            let update_y = controls_y + 4.0;

            // Draw button background (rounded rect with border)
            let border_color = if hover_update {
                [255.0 / 255.0, 255.0 / 255.0, 255.0 / 255.0, 0.20] // Lighter border on hover
            } else {
                [255.0 / 255.0, 255.0 / 255.0, 255.0 / 255.0, 0.10] // Subtle border normally
            };
            let btn_bg_color = if update_in_progress {
                [40.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0, 1.0]
            } else if hover_update {
                [24.0 / 255.0, 24.0 / 255.0, 24.0 / 255.0, 1.0] // Slightly lighter grey on hover
            } else {
                [12.0 / 255.0, 12.0 / 255.0, 12.0 / 255.0, 1.0] // #0c0c0c
            };

            // Outer border
            bg_instances.push(CellInstance::new(
                update_x, update_y,
                update_btn_w, update_btn_h,
                border_color,
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                5.0, // 5px corner radius
            ));

            // Inner background
            bg_instances.push(CellInstance::new(
                update_x + 1.0, update_y + 1.0,
                update_btn_w - 2.0, update_btn_h - 2.0,
                btn_bg_color,
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                4.0, // 4px corner radius
            ));

            // Draw button text
            let text = if update_completed { "Reiniciar" } else if update_in_progress { "Updating..." } else { "Update" };
            let text_scale = 11.0f32 / atlas.font_size();
            let text_cell_w = atlas.cell_size().0 * text_scale;

            // Measure text to center it inside the button
            let mut text_w = 0.0f32;
            for c in text.chars() {
                if c == ' ' {
                    text_w += text_cell_w;
                    continue;
                }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        text_w += (entry.width + 1.0) * text_scale;
                    }
                }
            }

            let tx = (update_x + (update_btn_w - text_w) / 2.0).round();
            let ty = (update_y + (update_btn_h - 11.0) / 2.0).round();

            let mut curr_x = tx;
            let scaled_ascent = atlas.ascent() * text_scale;
            let path_baseline_y = (ty + scaled_ascent - 1.5).round();
            for c in text.chars() {
                if c == ' ' {
                    curr_x += text_cell_w;
                    continue;
                }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        let glyph_w = entry.width * text_scale;
                        let glyph_h = entry.height * text_scale;
                        let glyph_x = (curr_x + entry.left * text_scale).round();
                        let glyph_y = (path_baseline_y + entry.top * text_scale).round();
                        let (aw, ah) = atlas.atlas_size();
                        let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                        let uv_w = uv_end_x - uv_x;
                        let uv_h = uv_end_y - uv_y;

                        fg_instances.push(CellInstance::new(
                            glyph_x, glyph_y,
                            glyph_w, glyph_h,
                            [1.0, 1.0, 1.0, 1.0], // White text
                            [0.0, 0.0, 0.0, 0.0],
                            uv_x, uv_y, uv_w, uv_h,
                            0.0,
                        ));
                        curr_x += (entry.width + 1.0) * text_scale;
                    }
                }
            }
        }

        // 1. Settings button (⚙)
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
            const TOPBAR_HEIGHT: f32 = 40.0;
            let track_top = TOPBAR_HEIGHT;
            let track_width = 6.0f32;
            let track_x = viewport_width - track_width - 2.0f32; // 2px from right edge
            let track_bottom = viewport_height - 4.0f32;
            let track_height = track_bottom - track_top;

            // Draw track (rgba(255,255,255,0.08))
            let track_instance = CellInstance::new(
                track_x, track_top,
                track_width, track_height,
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
                let thumb_h = (track_height * ratio).max(20.0).min(track_height);

                let scroll_ratio = if history_size > 0.0 {
                    scroll_current / history_size
                } else {
                    0.0
                };

                let thumb_y = track_top + (1.0 - scroll_ratio) * (track_height - thumb_h);
                let thumb_y = thumb_y.clamp(track_top, track_bottom - thumb_h);

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
        if let Some((msg, start_time, duration_ms)) = toast {
            let elapsed_ms = start_time.elapsed().as_millis() as u64;
            let alpha = match elapsed_ms {
                t if t < 120 => t as f32 / 120.0,
                t if t < duration_ms - 300 => 1.0,
                t if t < duration_ms => 1.0 - (t - (duration_ms - 300)) as f32 / 300.0,
                _ => 0.0,
            };

            if alpha > 0.0 {
                let scale = 13.0 / atlas.font_size();
                let toast_cell_w = atlas.cell_size().0 * scale;
                let mut text_w = 0.0f32;
                for c in msg.chars() {
                    if c == ' ' {
                        text_w += toast_cell_w;
                        continue;
                    }
                    if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                        if entry.width > 0.0 {
                            text_w += (entry.width + 1.0) * scale;
                        }
                    }
                }

                let toast_w = text_w + 28.0;
                let (_, ui_cell_height) = atlas.cell_size();
                let text_h = ui_cell_height * scale;
                let toast_h = text_h + 16.0;

                let toast_x = viewport_width - toast_w - 24.0;
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
                    if c == ' ' {
                        tx += toast_cell_w;
                        continue;
                    }
                    if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                        if entry.width > 0.0 {
                            let glyph_w = entry.width * scale;
                            let glyph_h = entry.height * scale;
                            let glyph_x = (tx + entry.left * scale).round();
                            let glyph_y = (baseline_y + entry.top * scale).round();
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
                        }
                    }
                }
            }
        }

        // Draw Context Menu if visible
        if context_menu_visible {
            let mut menu_items = Vec::new();
            if context_menu_is_about {
                menu_items.push(crate::renderer::ContextMenuItem::About);
            } else {
                if selection.is_some() {
                    menu_items.push(crate::renderer::ContextMenuItem::Copy);
                }
                menu_items.push(crate::renderer::ContextMenuItem::Paste);
                menu_items.push(crate::renderer::ContextMenuItem::Separator);
                menu_items.push(crate::renderer::ContextMenuItem::NewTab);
                if tab_titles.len() > 1 {
                    menu_items.push(crate::renderer::ContextMenuItem::CloseTab);
                }
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

            // 2. Inner background — theme_bg for the about variant, a
            // slightly lifted theme_bg for the regular menu (matches the
            // rgba(36,38,42,0.97) lift from default #0c0c0c by +14 per channel).
            let inner_bg_color = if context_menu_is_about {
                [theme_bg[0], theme_bg[1], theme_bg[2], 1.0]
            } else {
                [
                    (theme_bg[0] + 14.0 / 255.0).min(1.0),
                    (theme_bg[1] + 14.0 / 255.0).min(1.0),
                    (theme_bg[2] + 14.0 / 255.0).min(1.0),
                    0.97,
                ]
            };
            anim_push(
                context_menu_x + 1.0,
                context_menu_y + 1.0,
                menu_w - 2.0,
                menu_h - 2.0,
                inner_bg_color,
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
                            crate::renderer::ContextMenuItem::About => ("", "About", None),
                            crate::renderer::ContextMenuItem::OpenLink => ("🔗", "Open link", Some("⌘-click")),
                            crate::renderer::ContextMenuItem::CopyWord => ("📋", "Copy word", None),
                            crate::renderer::ContextMenuItem::CopyLine => ("📋", "Copy line", None),
                            crate::renderer::ContextMenuItem::CdHere => ("📁", "cd here in new tab", None),
                            crate::renderer::ContextMenuItem::OpenInEditor => ("✏\u{FE0F}", "Open in editor", None),
                            crate::renderer::ContextMenuItem::OpenEmail => ("✉\u{FE0F}", "Compose email", None),
                            crate::renderer::ContextMenuItem::CopyHex => ("#", "Copy hex", None),
                            crate::renderer::ContextMenuItem::Separator => ("", "", None),
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
                            crate::renderer::ContextMenuItem::About => atlas.app_icon.as_ref(),
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
                        let ctx_cell_w = atlas.cell_size().0 * base_scale;
                        let mut label_x = context_menu_x + 30.0;
                        for c in label.chars() {
                            if c == ' ' {
                                label_x += ctx_cell_w;
                                continue;
                            }
                            if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                                if entry.width > 0.0 {
                                    let glyph_w = entry.width * base_scale;
                                    let glyph_h = entry.height * base_scale;
                                    let glyph_x = (label_x + entry.left * base_scale).round();
                                    let glyph_y = (text_baseline_y + entry.top * base_scale).round();

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
                                        let glyph_x = (sh_x + entry.left * base_scale).round();
                                        let glyph_y = (item_center_y - glyph_h / 2.0).round();

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

        // Tab right-click context menu
        if tab_ctx_visible {
            let menu_w = 160.0f32;
            let menu_h = 44.0f32;
            let pad = 6.0f32;

            let shadow_pad = 8.0f32;
            instances.push(CellInstance::new(
                tab_ctx_x as f32 - shadow_pad,
                tab_ctx_y as f32 - shadow_pad + 4.0,
                menu_w + 2.0 * shadow_pad,
                menu_h + 2.0 * shadow_pad,
                [0.0, 0.0, 0.0, 0.40],
                [10.0, shadow_pad, shadow_pad, 0.0],
                0.0, 0.0, 1.0, 1.0, 10.0,
            ));
            instances.push(CellInstance::new(
                tab_ctx_x as f32, tab_ctx_y as f32, menu_w, menu_h,
                [1.0, 1.0, 1.0, 0.10],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0, 10.0,
            ));
            instances.push(CellInstance::new(
                tab_ctx_x as f32 + 1.0, tab_ctx_y as f32 + 1.0, menu_w - 2.0, menu_h - 2.0,
                [
                    (theme_bg[0] + 14.0 / 255.0).min(1.0),
                    (theme_bg[1] + 14.0 / 255.0).min(1.0),
                    (theme_bg[2] + 14.0 / 255.0).min(1.0),
                    0.97,
                ],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0, 9.0,
            ));

            let is_hover = tab_ctx_hovered == Some(0);
            if is_hover {
                instances.push(CellInstance::new(
                    tab_ctx_x as f32 + 4.0, tab_ctx_y as f32 + pad,
                    menu_w - 8.0, 32.0,
                    [1.0, 1.0, 1.0, 0.08],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0, 6.0,
                ));
            }

            let base_scale = 13.0f32 / atlas.font_size();
            let text_color = if is_hover {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [220.0 / 255.0, 222.0 / 255.0, 226.0 / 255.0, 1.0]
            };

            let (_, ui_cell_height) = atlas.cell_size();
            let text_baseline_y = tab_ctx_y as f32 + pad + 16.0 - (ui_cell_height * base_scale) / 2.0 + atlas.ascent() * base_scale;
            let ctx_cell_w = atlas.cell_size().0 * base_scale;
            let mut label_x = tab_ctx_x as f32 + 12.0;
            for c in "Renombrar".chars() {
                if c == ' ' { label_x += ctx_cell_w; continue; }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        let glyph_w = entry.width * base_scale;
                        let glyph_h = entry.height * base_scale;
                        let glyph_x = (label_x + entry.left * base_scale).round();
                        let glyph_y = (text_baseline_y + entry.top * base_scale).round();
                        let (aw, ah) = atlas.atlas_size();
                        let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                        instances.push(CellInstance::new(
                            glyph_x, glyph_y, glyph_w, glyph_h,
                            text_color, [0.0, 0.0, 0.0, 0.0],
                            uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y, 0.0,
                        ));
                        label_x += (entry.width + 1.0) * base_scale;
                    }
                }
            }
        }

        // Rename input overlay on tab
        if let Some(renaming_idx) = renaming_tab {
            let tab_start_x = 36.0f32;
            let tab_w = if tab_titles.len() > 0 {
                (tab_area_width / tab_titles.len() as f32).clamp(80.0, 160.0)
            } else { 160.0 };
            let input_x = tab_start_x + renaming_idx as f32 * tab_w + 4.0;
            let input_w = tab_w - 8.0;
            let input_h = 24.0;
            let input_y = (40.0 - input_h) / 2.0;

            instances.push(CellInstance::new(
                input_x - 1.0, input_y - 1.0, input_w + 2.0, input_h + 2.0,
                [0.4, 0.6, 1.0, 0.8],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0, 6.0,
            ));
            instances.push(CellInstance::new(
                input_x, input_y, input_w, input_h,
                [theme_bg[0], theme_bg[1], theme_bg[2], 1.0],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0, 5.0,
            ));

            let base_scale = 11.0f32 / atlas.font_size();
            let (_, ui_cell_height) = atlas.cell_size();
            let text_baseline_y = input_y + input_h / 2.0 - (ui_cell_height * base_scale) / 2.0 + atlas.ascent() * base_scale;
            let ctx_cell_w = atlas.cell_size().0 * base_scale;
            let mut text_x = input_x + 6.0;

            for (i, c) in rename_buffer.chars().enumerate() {
                if i == rename_cursor {
                    instances.push(CellInstance::new(
                        text_x, input_y + 3.0, 2.0, input_h - 6.0,
                        [0.4, 0.6, 1.0, 1.0],
                        [0.0, 0.0, 0.0, 0.0],
                        0.0, 0.0, 1.0, 1.0, 0.0,
                    ));
                }
                if c == ' ' { text_x += ctx_cell_w; continue; }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        let glyph_w = entry.width * base_scale;
                        let glyph_h = entry.height * base_scale;
                        let glyph_x = (text_x + entry.left * base_scale).round();
                        let glyph_y = (text_baseline_y + entry.top * base_scale).round();
                        let (aw, ah) = atlas.atlas_size();
                        let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                        instances.push(CellInstance::new(
                            glyph_x, glyph_y, glyph_w, glyph_h,
                            [1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 0.0],
                            uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y, 0.0,
                        ));
                        text_x += (entry.width + 1.0) * base_scale;
                    }
                }
            }
            if rename_cursor >= rename_buffer.len() {
                instances.push(CellInstance::new(
                    text_x, input_y + 3.0, 2.0, input_h - 6.0,
                    [0.4, 0.6, 1.0, 1.0],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0, 0.0,
                ));
            }
        }

        if command_palette_visible {
            let mut palette_w = 500.0f32;
            let max_results = 8usize;
            let item_h = 22.0f32;
            let input_h = 28.0f32;
            let n_shown = command_palette_filtered.len().min(max_results);
            let palette_h = input_h + (n_shown as f32) * item_h + 16.0;
            let palette_x = (viewport_width - palette_w) / 2.0;
            let palette_y = 80.0;

            instances.push(CellInstance::new(
                palette_x - 1.0, palette_y - 1.0,
                palette_w + 2.0, palette_h + 2.0,
                [1.0, 1.0, 1.0, 0.18],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                8.0,
            ));
            instances.push(CellInstance::new(
                palette_x, palette_y,
                palette_w, palette_h,
                [22.0 / 255.0, 24.0 / 255.0, 28.0 / 255.0, 0.96],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                7.0,
            ));

            let input_scale = 14.0 / atlas.font_size();
            let prompt_str = "> ";
            let input_baseline_y = palette_y + 6.0 + atlas.ascent() * input_scale;
            let mut input_x = palette_x + 14.0;
            let prompt_cell_w = atlas.cell_size().0 * input_scale;
            for c in prompt_str.chars() {
                if c == ' ' {
                    input_x += prompt_cell_w;
                    continue;
                }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        let glyph_w = entry.width * input_scale;
                        let glyph_h = entry.height * input_scale;
                        let glyph_x = (input_x + entry.left * input_scale).round();
                        let glyph_y = (input_baseline_y + entry.top * input_scale).round();
                        let (aw, ah) = atlas.atlas_size();
                        let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                        let uv_w = uv_end_x - uv_x;
                        let uv_h = uv_end_y - uv_y;
                        instances.push(CellInstance::new(
                            glyph_x, glyph_y,
                            glyph_w, glyph_h,
                            [0.6, 0.85, 1.0, 0.95],
                            [0.0, 0.0, 0.0, 0.0],
                            uv_x, uv_y, uv_w, uv_h,
                            0.0,
                        ));
                        input_x += (entry.width + 1.0) * input_scale;
                    }
                }
            }
                for c in command_palette_query.chars() {
                    if c == ' ' {
                        input_x += prompt_cell_w;
                        continue;
                    }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        let glyph_w = entry.width * input_scale;
                        let glyph_h = entry.height * input_scale;
                        let glyph_x = (input_x + entry.left * input_scale).round();
                        let glyph_y = (input_baseline_y + entry.top * input_scale).round();
                        let (aw, ah) = atlas.atlas_size();
                        let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                        let uv_w = uv_end_x - uv_x;
                        let uv_h = uv_end_y - uv_y;
                        instances.push(CellInstance::new(
                            glyph_x, glyph_y,
                            glyph_w, glyph_h,
                            [0.92, 0.94, 0.98, 1.0],
                            [0.0, 0.0, 0.0, 0.0],
                            uv_x, uv_y, uv_w, uv_h,
                            0.0,
                        ));
                        input_x += (entry.width + 1.0) * input_scale;
                    }
                }
            }
            let cursor_x = input_x + 1.0;
            let cursor_y = palette_y + 6.0;
            instances.push(CellInstance::new(
                cursor_x, cursor_y,
                1.5, input_h - 12.0,
                [0.6, 0.85, 1.0, 0.85],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                0.0,
            ));

            let result_scale = 12.0 / atlas.font_size();
            let result_cell_w = atlas.cell_size().0 * result_scale;
            let result_baseline_offset = atlas.ascent() * result_scale;
            for (i, label) in command_palette_filtered.iter().take(max_results).enumerate() {
                let item_y = palette_y + input_h + 8.0 + (i as f32) * item_h;
                if i == command_palette_selected {
                    instances.push(CellInstance::new(
                        palette_x + 4.0, item_y,
                        palette_w - 8.0, item_h - 2.0,
                        [0.20, 0.45, 0.85, 0.35],
                        [0.0, 0.0, 0.0, 0.0],
                        0.0, 0.0, 1.0, 1.0,
                        4.0,
                    ));
                }
                let baseline_y = item_y + (item_h - atlas.cell_size().1 * result_scale) / 2.0 + result_baseline_offset;
                let mut lx = palette_x + 14.0;
                for c in label.chars() {
                    if c == ' ' {
                        lx += result_cell_w * 1.3;
                        continue;
                    }
                    if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                        if entry.width > 0.0 {
                            let glyph_w = entry.width * result_scale;
                            let glyph_h = entry.height * result_scale;
                            let glyph_x = (lx + entry.left * result_scale).round();
                            let glyph_y = (baseline_y + entry.top * result_scale).round();
                            let (aw, ah) = atlas.atlas_size();
                            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                            let uv_w = uv_end_x - uv_x;
                            let uv_h = uv_end_y - uv_y;
                            instances.push(CellInstance::new(
                                glyph_x, glyph_y,
                                glyph_w, glyph_h,
                                [0.92, 0.94, 0.98, 1.0],
                                [0.0, 0.0, 0.0, 0.0],
                                uv_x, uv_y, uv_w, uv_h,
                                0.0,
                            ));
                            lx += (entry.width + 1.0) * result_scale;
                        }
                    }
                }
            }
        }

        if ssh_picker_visible {
            let picker_w = 500.0f32;
            let max_results = 8usize;
            let item_h = 22.0f32;
            let input_h = 28.0f32;
            let n_shown = ssh_filtered.len().min(max_results);
            let picker_h = input_h + (n_shown as f32) * item_h + 16.0;
            let picker_x = (viewport_width - picker_w) / 2.0;
            let picker_y = 80.0;

            instances.push(CellInstance::new(
                picker_x - 1.0, picker_y - 1.0,
                picker_w + 2.0, picker_h + 2.0,
                [1.0, 1.0, 1.0, 0.18],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                8.0,
            ));
            instances.push(CellInstance::new(
                picker_x, picker_y,
                picker_w, picker_h,
                [22.0 / 255.0, 24.0 / 255.0, 28.0 / 255.0, 0.96],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                7.0,
            ));

            let input_scale = 14.0 / atlas.font_size();
            let prompt_str = "> ";
            let input_baseline_y = picker_y + 6.0 + atlas.ascent() * input_scale;
            let mut input_x = picker_x + 14.0;
            let prompt_cell_w = atlas.cell_size().0 * input_scale;
            for c in prompt_str.chars() {
                if c == ' ' {
                    input_x += prompt_cell_w;
                    continue;
                }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        let gw = entry.width * input_scale;
                        let gh = entry.height * input_scale;
                        let gx = (input_x + entry.left * input_scale).round();
                        let gy = (input_baseline_y + entry.top * input_scale).round();
                        let (aw, ah) = atlas.atlas_size();
                        let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                        instances.push(CellInstance::new(
                            gx, gy, gw, gh,
                            [0.6, 0.85, 1.0, 0.95],
                            [0.0, 0.0, 0.0, 0.0],
                            uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                            0.0,
                        ));
                        input_x += (entry.width + 1.0) * input_scale;
                    }
                }
            }
            for c in ssh_picker_query.chars() {
                if c == ' ' {
                    input_x += prompt_cell_w;
                    continue;
                }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        let gw = entry.width * input_scale;
                        let gh = entry.height * input_scale;
                        let gx = (input_x + entry.left * input_scale).round();
                        let gy = (input_baseline_y + entry.top * input_scale).round();
                        let (aw, ah) = atlas.atlas_size();
                        let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                        instances.push(CellInstance::new(
                            gx, gy, gw, gh,
                            [0.92, 0.94, 0.98, 1.0],
                            [0.0, 0.0, 0.0, 0.0],
                            uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                            0.0,
                        ));
                        input_x += (entry.width + 1.0) * input_scale;
                    }
                }
            }
            let cursor_x = input_x + 1.0;
            let cursor_y = picker_y + 6.0;
            instances.push(CellInstance::new(
                cursor_x, cursor_y,
                1.5, input_h - 12.0,
                [0.6, 0.85, 1.0, 0.85],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                0.0,
            ));

            let result_scale = 12.0 / atlas.font_size();
            let result_cell_w = atlas.cell_size().0 * result_scale;
            let result_baseline_offset = atlas.ascent() * result_scale;
            for (i, label) in ssh_filtered.iter().take(max_results).enumerate() {
                let item_y = picker_y + input_h + 8.0 + (i as f32) * item_h;
                if i == ssh_picker_selected {
                    instances.push(CellInstance::new(
                        picker_x + 4.0, item_y,
                        picker_w - 8.0, item_h - 2.0,
                        [0.20, 0.45, 0.85, 0.35],
                        [0.0, 0.0, 0.0, 0.0],
                        0.0, 0.0, 1.0, 1.0,
                        4.0,
                    ));
                }
                let baseline_y = item_y + (item_h - atlas.cell_size().1 * result_scale) / 2.0 + result_baseline_offset;
                let mut lx = picker_x + 14.0;
                for c in label.chars() {
                    if c == ' ' || c == '@' {
                        lx += result_cell_w * 1.3;
                        continue;
                    }
                    if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                        if entry.width > 0.0 {
                            let gw = entry.width * result_scale;
                            let gh = entry.height * result_scale;
                            let gx = (lx + entry.left * result_scale).round();
                            let gy = (baseline_y + entry.top * result_scale).round();
                            let (aw, ah) = atlas.atlas_size();
                            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
                            instances.push(CellInstance::new(
                                gx, gy, gw, gh,
                                [0.92, 0.94, 0.98, 1.0],
                                [0.0, 0.0, 0.0, 0.0],
                                uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                                0.0,
                            ));
                            lx += (entry.width + 1.0) * result_scale;
                        }
                    }
                }
            }
        }

        let ui_extra_instances_final = instances;

        let term_count = term_instances_final.len();
        let ui_count = ui_bg_instances.len() + ui_fg_instances.len() + ui_extra_instances_final.len();

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
        hover_theme: bool,
        hover_open_config: bool,
        system_fonts: &[String],
        font_scroll_y: f32,
        hovered_font_idx: Option<usize>,
        theme: &str,
        themes: &[String],
        hovered_theme_idx: Option<usize>,
        theme_scroll_y: f32,
        visual_picker_active: bool,
        hover_visual_toggle: bool,
        hovered_card_idx: Option<usize>,
        card_scroll_y: f32,
        device: &Device,
        queue: &wgpu::Queue,
    ) {
        let scale = if cfg!(target_os = "windows") {
            viewport_width / 400.0
        } else {
            1.0
        };

        let mut bg_instances = Vec::new();
        let mut fg_instances = Vec::new();

        // Same theme_bg / theme_topbar as the main window so the dialogs
        // match the active terminal palette. Default keeps #0a0a0a; others
        // derive from bg * 0.83. Also pull the theme's bright blue for
        // selected-item highlights so the active dropdown row reads as
        // "selected" across every theme.
        let (bg_r, bg_g, bg_b) = named_color_rgb(alacritty_terminal::vte::ansi::NamedColor::Background);
        let theme_bg = [bg_r as f32 / 255.0, bg_g as f32 / 255.0, bg_b as f32 / 255.0, 1.0];
        let (bb_r, bb_g, bb_b) = named_color_rgb(alacritty_terminal::vte::ansi::NamedColor::BrightBlue);
        let theme_accent = [bb_r as f32 / 255.0, bb_g as f32 / 255.0, bb_b as f32 / 255.0, 1.0];
        let active_theme = crate::config::ACTIVE_THEME.read().to_lowercase();
        let theme_topbar = if active_theme == "default" {
            [10.0 / 255.0, 10.0 / 255.0, 10.0 / 255.0, 1.0]
        } else {
            [
                bg_r as f32 / 255.0 * 0.83,
                bg_g as f32 / 255.0 * 0.83,
                bg_b as f32 / 255.0 * 0.83,
                1.0,
            ]
        };
        // Lifted theme_bg variants for the closed-box rest/hover/active
        // states. Default ratios: rest = #101014, hover = #16161C, active = #191920.
        let lift = |offset: f32| -> [f32; 4] {
            [
                (bg_r as f32 / 255.0 + offset / 255.0).min(1.0),
                (bg_g as f32 / 255.0 + offset / 255.0).min(1.0),
                (bg_b as f32 / 255.0 + offset / 255.0).min(1.0),
                1.0,
            ]
        };
        let theme_lift_rest = lift(4.0);
        let theme_lift_box_hover = lift(6.0);
        let theme_lift_box_active = lift(9.0);
        let theme_lift_hover = lift(13.0);
        // Item hover uses a stronger theme-bg lift so the row stands out
        // against the dropdown bg without resorting to a hardcoded white
        // overlay that fights light themes.
        let theme_item_hover = lift(22.0);

        // 0. Draw window background (theme bg)
        bg_instances.push(CellInstance::new(
            0.0, 0.0,
            viewport_width, viewport_height,
            theme_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0 * scale,
        ));

        // 1. Draw topbar background (darker variant of theme bg)
        bg_instances.push(CellInstance::new(
            0.0, 0.0,
            viewport_width, 36.0 * scale,
            theme_topbar,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0 * scale,
        ));
        bg_instances.push(CellInstance::new(
            0.0, 28.0 * scale,
            viewport_width, 8.0 * scale,
            theme_topbar,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 0.0, 0.0,
            0.0,
        ));
        bg_instances.push(CellInstance::new(
            0.0, 36.0 * scale,
            viewport_width, 1.0 * scale,
            [1.0, 1.0, 1.0, 0.06], // Consistent with main topbar border
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 0.0, 0.0,
            0.0,
        ));

        // Helper to draw text. Spaces must be handled BEFORE the get_or_rasterize
        // call (a space can return None or a 0-width entry) and must advance by
        // the actual cell width — a hardcoded 8px is too narrow compared to
        // letter advances (~entry.width + 2), which is why "Zed Mono Extended"
        // was rendering as "ZedMonoExtended".
        let cell_w = atlas.cell_size().0 * scale;
        let draw_text = |atlas: &mut crate::renderer::Atlas, text: &str, start_x: f32, start_y: f32, color: [f32; 4], fg_list: &mut Vec<CellInstance>| {
            let mut x = start_x * scale;
            for c in text.chars() {
                if c == ' ' {
                    x += cell_w;
                    continue;
                }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        let glyph_x = (x + entry.left).round();
                        let glyph_y = (start_y * scale + atlas.ascent() + entry.top).round();
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
                        x += entry.width + 2.0 * scale;
                    }
                }
            }
        };

        // Draw title
        draw_text(atlas, "Settings", 12.0, 6.0, [0.85, 0.85, 0.85, 1.0], &mut fg_instances);

        // Draw "Visual" toggle button in the topbar (between title and close)
        let visual_btn_x = 80.0f32 * scale;
        let visual_btn_w = 56.0f32 * scale;
        let visual_btn_h = 24.0f32 * scale;
        let visual_btn_y = 6.0f32 * scale;
        let visual_btn_bg = if visual_picker_active {
            theme_accent
        } else if hover_visual_toggle {
            theme_lift_hover
        } else {
            theme_lift_rest
        };
        bg_instances.push(CellInstance::new(
            visual_btn_x, visual_btn_y,
            visual_btn_w, visual_btn_h,
            visual_btn_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            5.0 * scale,
        ));
        draw_text(
            atlas,
            "Visual",
            visual_btn_x / scale + 8.0,
            visual_btn_y / scale + 6.0,
            if visual_picker_active { [0.05, 0.05, 0.05, 1.0] } else { [0.85, 0.85, 0.90, 1.0] },
            &mut fg_instances,
        );

        // Draw topbar close button
        let close_x = viewport_width - 32.0 * scale;
        let close_y = 4.0 * scale;
        if hover_close {
            bg_instances.push(CellInstance::new(
                close_x, close_y,
                28.0 * scale, 28.0 * scale,
                [0.85, 0.25, 0.25, 0.9],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0 * scale, // Rounded rectangle
            ));
        }
        if let Some(entry) = &atlas.icon_close {
            let entry_w = 14.0f32 * scale;
            let entry_h = 14.0f32 * scale;
            let glyph_x = close_x + (28.0f32 * scale - entry_w) / 2.0;
            let glyph_y = close_y + (28.0f32 * scale - entry_h) / 2.0;
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
        draw_text(atlas, "Theme:", 20.0, 176.0, [0.75, 0.75, 0.80, 1.0], &mut fg_instances);

        // Draw inputs
        // 1. Font Family select box
        let family_bg = if active_field == 1 {
            theme_lift_box_active
        } else if hover_font_family {
            theme_lift_box_hover
        } else {
            theme_lift_rest
        };
        bg_instances.push(CellInstance::new(
            140.0 * scale, 52.0 * scale,
            240.0 * scale, 26.0 * scale,
            family_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0 * scale,
        ));

        // Draw text-font SVG icon
        if let Some(entry) = &atlas.icon_text_font {
            let entry_w = 14.0f32 * scale;
            let entry_h = 14.0f32 * scale;
            let glyph_x = 148.0f32 * scale;
            let glyph_y = 52.0f32 * scale + (26.0f32 * scale - entry_h) / 2.0f32;
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
            140.0 * scale, 92.0 * scale,
            28.0 * scale, 26.0 * scale,
            size_minus_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0 * scale,
        ));
        if let Some(entry) = &atlas.icon_less {
            let entry_w = 14.0f32 * scale;
            let entry_h = 14.0f32 * scale;
            let glyph_x = 140.0f32 * scale + (28.0f32 * scale - entry_w) / 2.0f32;
            let glyph_y = 92.0f32 * scale + (26.0f32 * scale - entry_h) / 2.0f32;
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
            220.0 * scale, 92.0 * scale,
            28.0 * scale, 26.0 * scale,
            size_plus_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0 * scale,
        ));
        if let Some(entry) = &atlas.icon_add {
            let entry_w = 14.0f32 * scale;
            let entry_h = 14.0f32 * scale;
            let glyph_x = 220.0f32 * scale + (28.0f32 * scale - entry_w) / 2.0f32;
            let glyph_y = 92.0f32 * scale + (26.0f32 * scale - entry_h) / 2.0f32;
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
            140.0 * scale, 132.0 * scale,
            28.0 * scale, 26.0 * scale,
            scroll_minus_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0 * scale,
        ));
        if let Some(entry) = &atlas.icon_less {
            let entry_w = 14.0f32 * scale;
            let entry_h = 14.0f32 * scale;
            let glyph_x = 140.0f32 * scale + (28.0f32 * scale - entry_w) / 2.0f32;
            let glyph_y = 132.0f32 * scale + (26.0f32 * scale - entry_h) / 2.0f32;
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
            240.0 * scale, 132.0 * scale,
            28.0 * scale, 26.0 * scale,
            scroll_plus_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0 * scale,
        ));
        if let Some(entry) = &atlas.icon_add {
            let entry_w = 14.0f32 * scale;
            let entry_h = 14.0f32 * scale;
            let glyph_x = 240.0f32 * scale + (28.0f32 * scale - entry_w) / 2.0f32;
            let glyph_y = 132.0f32 * scale + (26.0f32 * scale - entry_h) / 2.0f32;
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

        // 4. Theme picker
        let theme_bg = if active_field == 2 {
            theme_lift_box_active
        } else if hover_theme {
            theme_lift_box_hover
        } else {
            theme_lift_rest
        };
        bg_instances.push(CellInstance::new(
            140.0 * scale, 172.0 * scale,
            240.0 * scale, 26.0 * scale,
            theme_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0 * scale,
        ));

        // Color swatch for the active theme (left side, 4 small color dots)
        let swatch_x = 148.0f32 * scale;
        let swatch_y = 172.0f32 * scale + (26.0f32 * scale - 12.0f32 * scale) / 2.0f32;
        let theme_colors = named_color_rgb_for_theme(theme);
        let swatch_colors: [(u8, u8, u8); 4] = [
            theme_colors.0, // foreground
            theme_colors.1, // red
            theme_colors.2, // green
            theme_colors.3, // blue
        ];
        for (i, (r, g, b)) in swatch_colors.iter().enumerate() {
            let cx = swatch_x + i as f32 * 4.0f32 * scale;
            fg_instances.push(CellInstance::new(
                cx, swatch_y,
                3.0f32 * scale, 12.0f32 * scale,
                [*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 0.0, 0.0,
                1.0f32 * scale,
            ));
        }

        // Theme name text
        draw_text(atlas, theme, 172.0, 176.0, [0.9, 0.9, 0.95, 1.0], &mut fg_instances);
        // Dropdown arrow
        draw_text(atlas, "▾", 362.0, 176.0, [0.7, 0.7, 0.75, 1.0], &mut fg_instances);

        // 5. Config File option
        draw_text(atlas, "Config File:", 20.0, 216.0, [0.75, 0.75, 0.80, 1.0], &mut fg_instances);

        let config_bg = if hover_open_config {
            theme_lift_hover
        } else {
            theme_lift_rest
        };
        bg_instances.push(CellInstance::new(
            140.0 * scale, 212.0 * scale,
            240.0 * scale, 26.0 * scale,
            config_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            6.0 * scale,
        ));

        // Draw settings SVG icon for Config
        if let Some(entry) = &atlas.icon_settings {
            let entry_w = 14.0f32 * scale;
            let entry_h = 14.0f32 * scale;
            let glyph_x = 148.0f32 * scale;
            let glyph_y = 212.0f32 * scale + (26.0f32 * scale - entry_h) / 2.0f32;
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

        draw_text(atlas, "Open fasty.toml", 168.0, 216.0, [0.9, 0.9, 0.95, 1.0], &mut fg_instances);

        // Save & Cancel buttons removed — settings apply live.

        // Draw scrollable dropdown list if active_field == 1
        if active_field == 1 {
            let drop_x = 140.0f32 * scale;
            let drop_y = 78.0f32 * scale; // 52.0 + 26.0
            let drop_w = 240.0f32 * scale;
            let drop_h = 180.0f32 * scale;

            // Draw dropdown background shadow
            fg_instances.push(CellInstance::new(
                drop_x - 4.0 * scale, drop_y - 2.0 * scale,
                drop_w + 8.0 * scale, drop_h + 8.0 * scale,
                [0.0, 0.0, 0.0, 0.35],
                [4.0 * scale, 4.0 * scale, 4.0 * scale, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0 * scale,
            ));

            // Draw dropdown border
            fg_instances.push(CellInstance::new(
                drop_x, drop_y,
                drop_w, drop_h,
                [1.0, 1.0, 1.0, 0.15],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0 * scale,
            ));

            // Draw dropdown background (theme bg) - 100% opaque alpha!
            fg_instances.push(CellInstance::new(
                drop_x + 1.0 * scale, drop_y + 1.0 * scale,
                drop_w - 2.0 * scale, drop_h - 2.0 * scale,
                theme_bg,
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                5.0 * scale,
            ));

            // Draw scrollable items
            let item_h = 22.0f32 * scale;
            let padding_x = 8.0f32 * scale;

            for (i, font) in system_fonts.iter().enumerate() {
                let item_top_y = drop_y + i as f32 * item_h - font_scroll_y * scale;
                let item_bottom_y = item_top_y + item_h;

                // Simple clipping check: only render if inside the dropdown height
                if item_bottom_y > drop_y && item_top_y < drop_y + drop_h {
                    let is_selected = font == font_family;
                    let is_hovered = hovered_font_idx == Some(i);

                    // Draw item background on hover/selection (themed)
                    if is_hovered {
                        fg_instances.push(CellInstance::new(
                            drop_x + 4.0 * scale, item_top_y + 1.0 * scale,
                            drop_w - 8.0 * scale, item_h - 2.0 * scale,
                            theme_item_hover,
                            [0.0, 0.0, 0.0, 0.0],
                            0.0, 0.0, 1.0, 1.0,
                            4.0 * scale,
                        ));
                    } else if is_selected {
                        fg_instances.push(CellInstance::new(
                            drop_x + 4.0 * scale, item_top_y + 1.0 * scale,
                            drop_w - 8.0 * scale, item_h - 2.0 * scale,
                            [theme_accent[0], theme_accent[1], theme_accent[2], 0.20],
                            [0.0, 0.0, 0.0, 0.0],
                            0.0, 0.0, 1.0, 1.0,
                            4.0 * scale,
                        ));
                    }

                    // Render item text
                    let text_color = if is_selected {
                        theme_accent
                    } else if is_hovered {
                        [1.0, 1.0, 1.0, 1.0]
                    } else {
                        [0.85, 0.85, 0.90, 1.0]
                    };

                    // Draw text inside item (vertically centered)
                    let text_y = item_top_y + (item_h - atlas.cell_size().1) / 2.0;
                    let mut tx = drop_x + padding_x;
                    
                    // Simple text clipping: truncate if too long
                    let max_text_w = drop_w - padding_x * 2.0 - 10.0 * scale; 
                    let mut current_w = 0.0f32;

                    // Space must be handled BEFORE the rasterize call so a
                    // cached-missing space still advances. Advance by the
                    // cell width so spaces look like spaces, not squashed.
                    let item_cell_w = atlas.cell_size().0 * scale;
                    for c in font.chars() {
                        if c == ' ' {
                            tx += item_cell_w;
                            current_w += item_cell_w;
                            continue;
                        }
                        if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                            if entry.width > 0.0 {
                                let glyph_w = entry.width;
                                let glyph_h = entry.height;
                                if current_w + glyph_w > max_text_w {
                                    break;
                                }

                                let glyph_x = (tx + entry.left).round();
                                let glyph_y = (text_y + atlas.ascent() + entry.top).round();

                                // Clip glyph vertically to dropdown client area
                                if glyph_y + glyph_h <= drop_y + drop_h - 2.0 * scale && glyph_y >= drop_y + 2.0 * scale {
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
                                tx += entry.width + 1.0 * scale;
                                current_w += entry.width + 1.0 * scale;
                            }
                        }
                    }
                }
            }

            // Draw scrollbar if necessary
            let total_h = system_fonts.iter().enumerate().count() as f32 * item_h;
            if total_h > drop_h {
                let sbar_w = 4.0f32 * scale;
                let sbar_x = drop_x + drop_w - sbar_w - 2.0 * scale;
                let sbar_h = (drop_h / total_h) * drop_h;
                let sbar_y = drop_y + ((font_scroll_y * scale) / total_h) * drop_h;

                fg_instances.push(CellInstance::new(
                    sbar_x, sbar_y,
                    sbar_w, sbar_h,
                    [1.0, 1.0, 1.0, 0.25],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    2.0 * scale,
                ));
            }
        }

        // Draw theme dropdown list if active_field == 2
        if active_field == 2 {
            let drop_x = 140.0f32 * scale;
            let drop_y = 198.0f32 * scale; // 172.0 + 26.0
            let drop_w = 240.0f32 * scale;
            let item_h = 22.0f32 * scale;
            let max_items = (viewport_height - drop_y - 8.0 * scale) / item_h;
            let visible_h = max_items.max(1.0) * item_h;
            let total_h = themes.len() as f32 * item_h;
            let drop_h = total_h.min(visible_h).max(item_h);

            // Draw dropdown background shadow
            fg_instances.push(CellInstance::new(
                drop_x - 4.0 * scale, drop_y - 2.0 * scale,
                drop_w + 8.0 * scale, drop_h + 8.0 * scale,
                [0.0, 0.0, 0.0, 0.35],
                [4.0 * scale, 4.0 * scale, 4.0 * scale, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0 * scale,
            ));

            // Draw dropdown border
            fg_instances.push(CellInstance::new(
                drop_x, drop_y,
                drop_w, drop_h,
                [1.0, 1.0, 1.0, 0.15],
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                6.0 * scale,
            ));

            // Draw dropdown background (theme bg)
            fg_instances.push(CellInstance::new(
                drop_x + 1.0 * scale, drop_y + 1.0 * scale,
                drop_w - 2.0 * scale, drop_h - 2.0 * scale,
                theme_bg,
                [0.0, 0.0, 0.0, 0.0],
                0.0, 0.0, 1.0, 1.0,
                5.0 * scale,
            ));

            let padding_x = 8.0f32 * scale;
            for (i, theme_name) in themes.iter().enumerate() {
                let item_top_y = drop_y + i as f32 * item_h - theme_scroll_y * scale;
                let item_bottom_y = item_top_y + item_h;

                if item_bottom_y > drop_y + drop_h || item_top_y < drop_y {
                    continue;
                }

                let is_selected = theme_name == theme;
                let is_hovered = hovered_theme_idx == Some(i);

                if is_hovered {
                    fg_instances.push(CellInstance::new(
                        drop_x + 4.0 * scale, item_top_y + 1.0 * scale,
                        drop_w - 8.0 * scale, item_h - 2.0 * scale,
                        theme_item_hover,
                        [0.0, 0.0, 0.0, 0.0],
                        0.0, 0.0, 1.0, 1.0,
                        4.0 * scale,
                    ));
                } else if is_selected {
                    fg_instances.push(CellInstance::new(
                        drop_x + 4.0 * scale, item_top_y + 1.0 * scale,
                        drop_w - 8.0 * scale, item_h - 2.0 * scale,
                        [theme_accent[0], theme_accent[1], theme_accent[2], 0.20],
                        [0.0, 0.0, 0.0, 0.0],
                        0.0, 0.0, 1.0, 1.0,
                        4.0 * scale,
                    ));
                }

                // Color swatch dots for the theme
                let swatch_colors = named_color_rgb_for_theme(theme_name);
                let swatch_colors_arr: [(u8, u8, u8); 4] = [
                    swatch_colors.0,
                    swatch_colors.1,
                    swatch_colors.2,
                    swatch_colors.3,
                ];
                let swatch_x = drop_x + padding_x;
                let swatch_y = item_top_y + (item_h - 10.0f32 * scale) / 2.0;
                for (j, (r, g, b)) in swatch_colors_arr.iter().enumerate() {
                    let cx = swatch_x + j as f32 * 4.0f32 * scale;
                    fg_instances.push(CellInstance::new(
                        cx, swatch_y,
                        3.0f32 * scale, 10.0f32 * scale,
                        [*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0, 1.0],
                        [0.0, 0.0, 0.0, 0.0],
                        0.0, 0.0, 0.0, 0.0,
                        1.0f32 * scale,
                    ));
                }

                let text_color = if is_selected {
                    theme_accent
                } else if is_hovered {
                    [1.0, 1.0, 1.0, 1.0]
                } else {
                    [0.85, 0.85, 0.90, 1.0]
                };

                let text_y = item_top_y + (item_h - atlas.cell_size().1) / 2.0;
                let mut tx = drop_x + padding_x + 4.0 * 4.0 * scale + 4.0 * scale;

                let max_text_w = drop_w - (tx - drop_x) - padding_x;
                let mut current_w = 0.0f32;

                // Space handled first; advance by cell width so spaces look right.
                let item_cell_w = atlas.cell_size().0 * scale;
                for c in theme_name.chars() {
                    if c == ' ' {
                        tx += item_cell_w;
                        current_w += item_cell_w;
                        continue;
                    }
                    if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                        if entry.width > 0.0 {
                            let glyph_w = entry.width;
                            let glyph_h = entry.height;
                            if current_w + glyph_w > max_text_w {
                                break;
                            }
                            let glyph_x = (tx + entry.left).round();
                            let glyph_y = (text_y + atlas.ascent() + entry.top).round();

                            if glyph_y + glyph_h <= drop_y + drop_h - 2.0 * scale && glyph_y >= drop_y + 2.0 * scale {
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
                            tx += entry.width + 1.0 * scale;
                            current_w += entry.width + 1.0 * scale;
                        }
                    }
                }
            }

            // Draw scrollbar if necessary
            if total_h > drop_h {
                let sbar_w = 4.0f32 * scale;
                let sbar_x = drop_x + drop_w - sbar_w - 2.0 * scale;
                let sbar_h = (drop_h / total_h) * drop_h;
                let sbar_y = drop_y + ((theme_scroll_y * scale) / total_h) * drop_h;

                fg_instances.push(CellInstance::new(
                    sbar_x, sbar_y,
                    sbar_w, sbar_h,
                    [1.0, 1.0, 1.0, 0.25],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    2.0 * scale,
                ));
            }
        }

        // Visual Picker — theme preview cards
        if visual_picker_active {
            let grid_x = 20.0f32 * scale;
            let grid_y = 256.0f32 * scale;
            let card_w = 170.0f32 * scale;
            let card_h = 58.0f32 * scale;
            let gap = 8.0f32 * scale;
            let cols = 2;
            let rows_visible = ((viewport_height - grid_y - 12.0 * scale) / (card_h + gap)).floor() as i32;
            let rows_visible = rows_visible.max(1);
            let total_rows = ((themes.len() as i32 + cols - 1) / cols).max(1);

            draw_text(atlas, "Theme previews", grid_x / scale, grid_y / scale - 16.0, [0.75, 0.75, 0.80, 1.0], &mut fg_instances);

            for (i, theme_name) in themes.iter().enumerate() {
                let col = (i as i32) % cols;
                let row = (i as i32) / cols;
                let card_x = grid_x + col as f32 * (card_w + gap);
                let card_y = grid_y + row as f32 * (card_h + gap) - card_scroll_y * scale;
                let card_bottom = card_y + card_h;
                if card_bottom < grid_y || card_y > viewport_height - 8.0 * scale {
                    continue;
                }

                let swatch = named_color_rgb_for_theme(theme_name);
                let (fg, accent_r, accent_g, _accent_b) = swatch;
                let prev = crate::config::ACTIVE_THEME.read().clone();
                crate::config::set_active_theme(theme_name);
                let t = get_active_theme();
                let (br, bgc, bb) = t.background;
                crate::config::set_active_theme(&prev);

                let card_bg = [br as f32 / 255.0, bgc as f32 / 255.0, bb as f32 / 255.0, 1.0];
                let card_fg = [fg.0 as f32 / 255.0, fg.1 as f32 / 255.0, fg.2 as f32 / 255.0, 1.0];
                let card_accent = [accent_r.0 as f32 / 255.0, accent_r.1 as f32 / 255.0, accent_r.2 as f32 / 255.0, 1.0];

                let is_active = theme_name == theme;
                let is_hovered = hovered_card_idx == Some(i);

                let border_color = if is_active {
                    card_accent
                } else if is_hovered {
                    [1.0, 1.0, 1.0, 0.30]
                } else {
                    [1.0, 1.0, 1.0, 0.12]
                };

                // Shadow
                bg_instances.push(CellInstance::new(
                    card_x + 2.0 * scale, card_y + 2.0 * scale,
                    card_w, card_h,
                    [0.0, 0.0, 0.0, 0.25],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    8.0 * scale,
                ));
                // Card background — uses the theme's actual bg color
                bg_instances.push(CellInstance::new(
                    card_x, card_y,
                    card_w, card_h,
                    card_bg,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    8.0 * scale,
                ));
                // Border ring
                fg_instances.push(CellInstance::new(
                    card_x, card_y,
                    card_w, card_h,
                    border_color,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    8.0 * scale,
                ));

                let sample_x = card_x + 8.0 * scale;
                let sample_y = card_y + 6.0 * scale;
                draw_text(atlas, "Aa Bb", sample_x / scale, sample_y / scale, card_fg, &mut fg_instances);
                // Accent strip on the right (small bar using the theme's red/green accent)
                fg_instances.push(CellInstance::new(
                    card_x + card_w - 18.0 * scale, card_y + 8.0 * scale,
                    10.0 * scale, 4.0 * scale,
                    card_accent,
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    2.0 * scale,
                ));
                fg_instances.push(CellInstance::new(
                    card_x + card_w - 32.0 * scale, card_y + 8.0 * scale,
                    10.0 * scale, 4.0 * scale,
                    [accent_g.0 as f32 / 255.0, accent_g.1 as f32 / 255.0, accent_g.2 as f32 / 255.0, 1.0],
                    [0.0, 0.0, 0.0, 0.0],
                    0.0, 0.0, 1.0, 1.0,
                    2.0 * scale,
                ));

                draw_text(atlas, theme_name, (card_x + 8.0 * scale) / scale, (card_y + 30.0 * scale) / scale, card_fg, &mut fg_instances);
                if is_active {
                    draw_text(atlas, "v", (card_x + card_w - 18.0 * scale) / scale, (card_y + 30.0 * scale) / scale, card_accent, &mut fg_instances);
                }
            }
            let _ = (total_rows, rows_visible);
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

    pub fn render_about(
        &self,
        render_pass: &mut wgpu::RenderPass,
        atlas: &mut crate::renderer::Atlas,
        viewport_width: f32,
        viewport_height: f32,
        version: &str,
        hover_close: bool,
        device: &Device,
        queue: &wgpu::Queue,
    ) {
        let mut bg_instances = Vec::new();
        let mut fg_instances = Vec::new();

        // Theme-aware bg / topbar, same convention as the main window.
        let (bg_r, bg_g, bg_b) = named_color_rgb(alacritty_terminal::vte::ansi::NamedColor::Background);
        let theme_bg = [bg_r as f32 / 255.0, bg_g as f32 / 255.0, bg_b as f32 / 255.0, 1.0];
        let active_theme = crate::config::ACTIVE_THEME.read().to_lowercase();
        let theme_topbar = if active_theme == "default" {
            [10.0 / 255.0, 10.0 / 255.0, 10.0 / 255.0, 1.0]
        } else {
            [
                bg_r as f32 / 255.0 * 0.83,
                bg_g as f32 / 255.0 * 0.83,
                bg_b as f32 / 255.0 * 0.83,
                1.0,
            ]
        };

        // 0. Draw window background (theme bg)
        bg_instances.push(CellInstance::new(
            0.0, 0.0,
            viewport_width, viewport_height,
            theme_bg,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0,
        ));

        // 1. Draw topbar background (darker variant of theme bg)
        bg_instances.push(CellInstance::new(
            0.0, 0.0,
            viewport_width, 36.0,
            theme_topbar,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 1.0, 1.0,
            8.0,
        ));
        bg_instances.push(CellInstance::new(
            0.0, 28.0,
            viewport_width, 8.0,
            theme_topbar,
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 0.0, 0.0,
            0.0,
        ));
        bg_instances.push(CellInstance::new(
            0.0, 36.0,
            viewport_width, 1.0,
            [1.0, 1.0, 1.0, 0.06], // Divider line
            [0.0, 0.0, 0.0, 0.0],
            0.0, 0.0, 0.0, 0.0,
            0.0,
        ));

        // Helper to draw text. Spaces must be handled BEFORE the get_or_rasterize
        // call (a space can return None or a 0-width entry) and must advance by
        // the actual cell width so spaces look like spaces.
        let about_cell_w = atlas.cell_size().0;
        let draw_text = |atlas: &mut crate::renderer::Atlas, text: &str, start_x: f32, start_y: f32, color: [f32; 4], fg_list: &mut Vec<CellInstance>| {
            let mut x = start_x;
            for c in text.chars() {
                if c == ' ' {
                    x += about_cell_w;
                    continue;
                }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        let glyph_x = (x + entry.left).round();
                        let glyph_y = (start_y + atlas.ascent() + entry.top).round();
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
                    }
                }
            }
        };

        let get_text_width = |atlas: &mut crate::renderer::Atlas, text: &str| -> f32 {
            let mut w = 0.0f32;
            for c in text.chars() {
                if c == ' ' {
                    w += about_cell_w;
                    continue;
                }
                if let Some(entry) = atlas.get_or_rasterize(c, device, queue) {
                    if entry.width > 0.0 {
                        w += entry.width + 2.0;
                    }
                }
            }
            w
        };

        // Draw title
        draw_text(atlas, "About Fasty", 12.0, 6.0, [0.85, 0.85, 0.90, 1.0], &mut fg_instances);

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

        // Draw Fasty App Icon in the center
        if let Some(entry) = &atlas.app_icon {
            let logo_w = 48.0f32;
            let logo_h = 48.0f32;
            let logo_x = (viewport_width - logo_w) / 2.0;
            let logo_y = 52.0f32;
            let (aw, ah) = atlas.atlas_size();
            let [uv_x, uv_y, uv_end_x, uv_end_y] = entry.uv_coords(aw, ah);
            fg_instances.push(CellInstance::new(
                logo_x, logo_y,
                logo_w, logo_h,
                [1.0, 1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0, 0.0],
                uv_x, uv_y, uv_end_x - uv_x, uv_end_y - uv_y,
                1.0,
            ));
        }

        // Draw App Name: Fasty
        let name_str = "Fasty";
        let name_w = get_text_width(atlas, name_str);
        let name_x = (viewport_width - name_w) / 2.0;
        draw_text(atlas, name_str, name_x, 110.0, [1.0, 1.0, 1.0, 1.0], &mut fg_instances);

        // Draw Version: e.g. Version 0.1.3
        let ver_clean = version.trim_start_matches('v');
        let ver_str = format!("Version {}", ver_clean);
        let ver_w = get_text_width(atlas, &ver_str);
        let ver_x = (viewport_width - ver_w) / 2.0;
        draw_text(atlas, &ver_str, ver_x, 134.0, [0.6, 0.6, 0.65, 1.0], &mut fg_instances);

        // Draw description
        let desc_str = "GPU-accelerated Terminal Emulator";
        let desc_w = get_text_width(atlas, desc_str);
        let desc_x = (viewport_width - desc_w) / 2.0;
        draw_text(atlas, desc_str, desc_x, 158.0, [0.45, 0.45, 0.5, 1.0], &mut fg_instances);

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

struct Theme {
    foreground: (u8, u8, u8),
    background: (u8, u8, u8),
    black: (u8, u8, u8),
    red: (u8, u8, u8),
    green: (u8, u8, u8),
    yellow: (u8, u8, u8),
    blue: (u8, u8, u8),
    magenta: (u8, u8, u8),
    cyan: (u8, u8, u8),
    white: (u8, u8, u8),
    bright_black: (u8, u8, u8),
    bright_red: (u8, u8, u8),
    bright_green: (u8, u8, u8),
    bright_yellow: (u8, u8, u8),
    bright_blue: (u8, u8, u8),
    bright_magenta: (u8, u8, u8),
    bright_cyan: (u8, u8, u8),
    bright_white: (u8, u8, u8),
}

fn get_active_theme() -> Theme {
    let theme_name = crate::config::ACTIVE_THEME.read().clone();
    let name_lower = theme_name.to_lowercase();
    if let Some(slots) = crate::config::try_get_custom_theme_full(&theme_name) {
        // slots: 0=bg, 1=fg, 2..17=ANSI[0..15]. Fall back to fg for any missing slot.
        let fallback = slots[1].unwrap_or((0, 0, 0));
        let get = |i: usize| slots.get(i).copied().flatten().unwrap_or(fallback);
        return Theme {
            foreground: slots[1].unwrap_or((0, 0, 0)),
            background: slots[0].unwrap_or((0, 0, 0)),
            black: get(2),
            red: get(3),
            green: get(4),
            yellow: get(5),
            blue: get(6),
            magenta: get(7),
            cyan: get(8),
            white: get(9),
            bright_black: get(10),
            bright_red: get(11),
            bright_green: get(12),
            bright_yellow: get(13),
            bright_blue: get(14),
            bright_magenta: get(15),
            bright_cyan: get(16),
            bright_white: get(17),
        };
    }
    match name_lower.as_str() {
        "catppuccin" => Theme {
            foreground: (0xCA, 0xD3, 0xF5),
            background: (0x24, 0x27, 0x3A),
            black: (0x49, 0x4D, 0x64),
            red: (0xED, 0x87, 0x96),
            green: (0xA6, 0xDA, 0x95),
            yellow: (0xEE, 0xDD, 0xB2),
            blue: (0x8A, 0xAD, 0xF4),
            magenta: (0xF5, 0xB8, 0x95),
            cyan: (0x91, 0xD7, 0xE3),
            white: (0xB8, 0xC0, 0xE0),
            bright_black: (0x5B, 0x60, 0x78),
            bright_red: (0xED, 0x87, 0x96),
            bright_green: (0xA6, 0xDA, 0x95),
            bright_yellow: (0xEE, 0xDD, 0xB2),
            bright_blue: (0x8A, 0xAD, 0xF4),
            bright_magenta: (0xF4, 0xBD, 0xD8),
            bright_cyan: (0x91, 0xD7, 0xE3),
            bright_white: (0xA5, 0xAD, 0xCB),
        },
        "one-dark" | "onedark" => Theme {
            foreground: (0xAB, 0xB2, 0xBF),
            background: (0x28, 0x2C, 0x34),
            black: (0x28, 0x2C, 0x34),
            red: (0xE0, 0x6C, 0x75),
            green: (0x98, 0xC3, 0x79),
            yellow: (0xD1, 0x9A, 0x66),
            blue: (0x61, 0xAF, 0xEF),
            magenta: (0xC6, 0x78, 0xDD),
            cyan: (0x56, 0xB6, 0xC2),
            white: (0xAB, 0xB2, 0xBF),
            bright_black: (0x5C, 0x63, 0x70),
            bright_red: (0xE0, 0x6C, 0x75),
            bright_green: (0x98, 0xC3, 0x79),
            bright_yellow: (0xD1, 0x9A, 0x66),
            bright_blue: (0x61, 0xAF, 0xEF),
            bright_magenta: (0xC6, 0x78, 0xDD),
            bright_cyan: (0x56, 0xB6, 0xC2),
            bright_white: (0xFF, 0xFF, 0xFF),
        },
        "high-contrast" => Theme {
            foreground: (0xFF, 0xFF, 0xFF),
            background: (0x00, 0x00, 0x00),
            black: (0x00, 0x00, 0x00),
            red: (0xFF, 0x55, 0x55),
            green: (0x50, 0xFA, 0x7B),
            yellow: (0xFF, 0xF0, 0x5A),
            blue: (0x62, 0xD6, 0xFF),
            magenta: (0xFF, 0x79, 0xC6),
            cyan: (0x8B, 0xEC, 0xFF),
            white: (0xFF, 0xFF, 0xFF),
            bright_black: (0x7F, 0x7F, 0x7F),
            bright_red: (0xFF, 0x55, 0x55),
            bright_green: (0x50, 0xFA, 0x7B),
            bright_yellow: (0xFF, 0xF0, 0x5A),
            bright_blue: (0x62, 0xD6, 0xFF),
            bright_magenta: (0xFF, 0x79, 0xC6),
            bright_cyan: (0x8B, 0xEC, 0xFF),
            bright_white: (0xFF, 0xFF, 0xFF),
        },
        "solarized-dark" | "solarized" => Theme {
            foreground: (0x83, 0x94, 0x96),
            background: (0x00, 0x2B, 0x36),
            black: (0x07, 0x36, 0x42),
            red: (0xDC, 0x32, 0x2F),
            green: (0x85, 0x99, 0x00),
            yellow: (0xB5, 0x89, 0x00),
            blue: (0x26, 0x8B, 0xD2),
            magenta: (0xD3, 0x36, 0x82),
            cyan: (0x2A, 0xA1, 0x98),
            white: (0xEE, 0xE8, 0xD5),
            bright_black: (0x58, 0x6E, 0x75),
            bright_red: (0xCB, 0x4B, 0x16),
            bright_green: (0x58, 0x6E, 0x75),
            bright_yellow: (0x65, 0x7B, 0x83),
            bright_blue: (0x83, 0x94, 0x96),
            bright_magenta: (0x6C, 0x71, 0xC4),
            bright_cyan: (0x93, 0xA1, 0xA1),
            bright_white: (0xFD, 0xF6, 0xE3),
        },
        _ => Theme { // Fasty (Default)
            foreground: (0xC5, 0xC8, 0xC6),
            background: (0x0C, 0x0C, 0x0C),
            black: (0x1D, 0x1F, 0x21),
            red: (0xCC, 0x66, 0x66),
            green: (0xB5, 0xBD, 0x68),
            yellow: (0xF0, 0xC6, 0x74),
            blue: (0x81, 0xA2, 0xBE),
            magenta: (0xB2, 0x94, 0xBB),
            cyan: (0x8A, 0xBE, 0xB7),
            white: (0xC5, 0xC8, 0xC6),
            bright_black: (0x66, 0x66, 0x66),
            bright_red: (0xCC, 0x66, 0x66),
            bright_green: (0xB5, 0xBD, 0x68),
            bright_yellow: (0xF0, 0xC6, 0x74),
            bright_blue: (0x81, 0xA2, 0xBE),
            bright_magenta: (0xB2, 0x94, 0xBB),
            bright_cyan: (0x8A, 0xBE, 0xB7),
            bright_white: (0xFF, 0xFF, 0xFF),
        }
    }
}

fn named_color_rgb_for_theme(theme_name: &str) -> ((u8, u8, u8), (u8, u8, u8), (u8, u8, u8), (u8, u8, u8)) {
    let previous = crate::config::ACTIVE_THEME.read().clone();
    crate::config::set_active_theme(theme_name);
    let t = get_active_theme();
    let result = (t.foreground, t.red, t.green, t.blue);
    crate::config::set_active_theme(&previous);
    result
}

fn named_color_rgb(named: alacritty_terminal::vte::ansi::NamedColor) -> (u8, u8, u8) {
    let theme = get_active_theme();
    match named {
        alacritty_terminal::vte::ansi::NamedColor::Foreground => theme.foreground,
        alacritty_terminal::vte::ansi::NamedColor::Background => theme.background,
        alacritty_terminal::vte::ansi::NamedColor::Black => theme.black,
        alacritty_terminal::vte::ansi::NamedColor::Red => theme.red,
        alacritty_terminal::vte::ansi::NamedColor::Green => theme.green,
        alacritty_terminal::vte::ansi::NamedColor::Yellow => theme.yellow,
        alacritty_terminal::vte::ansi::NamedColor::Blue => theme.blue,
        alacritty_terminal::vte::ansi::NamedColor::Magenta => theme.magenta,
        alacritty_terminal::vte::ansi::NamedColor::Cyan => theme.cyan,
        alacritty_terminal::vte::ansi::NamedColor::White => theme.white,
        alacritty_terminal::vte::ansi::NamedColor::BrightBlack => theme.bright_black,
        alacritty_terminal::vte::ansi::NamedColor::BrightRed => theme.bright_red,
        alacritty_terminal::vte::ansi::NamedColor::BrightGreen => theme.bright_green,
        alacritty_terminal::vte::ansi::NamedColor::BrightYellow => theme.bright_yellow,
        alacritty_terminal::vte::ansi::NamedColor::BrightBlue => theme.bright_blue,
        alacritty_terminal::vte::ansi::NamedColor::BrightMagenta => theme.bright_magenta,
        alacritty_terminal::vte::ansi::NamedColor::BrightCyan => theme.bright_cyan,
        alacritty_terminal::vte::ansi::NamedColor::BrightWhite => theme.bright_white,
        _ => theme.foreground,
    }
}

fn index_to_ansi_color(idx: usize) -> (u8, u8, u8) {
    if idx < 16 {
        let theme = get_active_theme();
        match idx {
            0 => theme.black,
            1 => theme.red,
            2 => theme.green,
            3 => theme.yellow,
            4 => theme.blue,
            5 => theme.magenta,
            6 => theme.cyan,
            7 => theme.white,
            8 => theme.bright_black,
            9 => theme.bright_red,
            10 => theme.bright_green,
            11 => theme.bright_yellow,
            12 => theme.bright_blue,
            13 => theme.bright_magenta,
            14 => theme.bright_cyan,
            15 => theme.bright_white,
            _ => theme.white,
        }
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

pub fn is_custom_block_drawing(ch: char) -> bool {
    matches!(ch as u32, 0x2500..=0x259F)
}

pub fn decode_box_drawing(ch: char) -> Option<(u8, u8, u8, u8, u8)> {
    let code = ch as u32;
    if !(0x2500..=0x257F).contains(&code) {
        return None;
    }
    
    // Returns (left, right, top, bottom, kind)
    // Styles: 0: none, 1: light, 2: heavy, 3: double
    // Kinds: 0: normal, 1: round corner, 2: diagonal, 3: dashed
    Some(match code {
        // Horizontal / Vertical lines
        0x2500 => (1, 1, 0, 0, 0), // ─
        0x2501 => (2, 2, 0, 0, 0), // ━
        0x2502 => (0, 0, 1, 1, 0), // │
        0x2503 => (0, 0, 2, 2, 0), // ┃
        
        // Dashed lines
        0x2504 => (1, 1, 0, 0, 3), // ┄
        0x2505 => (2, 2, 0, 0, 3), // ┅
        0x2506 => (0, 0, 1, 1, 3), // ┆
        0x2507 => (0, 0, 2, 2, 3), // ┇
        0x2508 => (1, 1, 0, 0, 3), // ┈
        0x2509 => (2, 2, 0, 0, 3), // ┉
        0x250A => (0, 0, 1, 1, 3), // ┊
        0x250B => (0, 0, 2, 2, 3), // ┋
        
        // Corners: Down and Right
        0x250C => (0, 1, 0, 1, 0), // ┌
        0x250D => (0, 2, 0, 1, 0), // ┍
        0x250E => (0, 1, 0, 2, 0), // ┎
        0x250F => (0, 2, 0, 2, 0), // ┏
        
        // Corners: Down and Left
        0x2510 => (1, 0, 0, 1, 0), // ┐
        0x2511 => (2, 0, 0, 1, 0), // ┑
        0x2512 => (1, 0, 0, 2, 0), // ┒
        0x2513 => (2, 0, 0, 2, 0), // ┓
        
        // Corners: Up and Right
        0x2514 => (0, 1, 1, 0, 0), // └
        0x2515 => (0, 2, 1, 0, 0), // ┕
        0x2516 => (0, 1, 2, 0, 0), // ┚ (Actually ┖)
        0x2517 => (0, 2, 2, 0, 0), // ┗
        
        // Corners: Up and Left
        0x2518 => (1, 0, 1, 0, 0), // ┘
        0x2519 => (2, 0, 1, 0, 0), // ┙
        0x251A => (1, 0, 2, 0, 0), // ┚
        0x251B => (2, 0, 2, 0, 0), // ┛
        
        // Tees: Vertical and Right
        0x251C => (0, 1, 1, 1, 0), // ├
        0x251D => (0, 2, 1, 1, 0), // ┝
        0x251E => (0, 1, 2, 1, 0), // ┞
        0x251F => (0, 1, 1, 2, 0), // ┟
        0x2520 => (0, 1, 2, 2, 0), // ┠
        0x2521 => (0, 2, 2, 1, 0), // ┡
        0x2522 => (0, 2, 1, 2, 0), // ┢
        0x2523 => (0, 2, 2, 2, 0), // ┣
        
        // Tees: Vertical and Left
        0x2524 => (1, 0, 1, 1, 0), // ┤
        0x2525 => (2, 0, 1, 1, 0), // ┥
        0x2526 => (1, 0, 2, 1, 0), // ┦
        0x2527 => (1, 0, 1, 2, 0), // ┧
        0x2528 => (1, 0, 2, 2, 0), // ┨
        0x2529 => (2, 0, 2, 1, 0), // ┩
        0x252A => (2, 0, 1, 2, 0), // ┪
        0x252B => (2, 0, 2, 2, 0), // ┫
        
        // Tees: Down and Horizontal
        0x252C => (1, 1, 0, 1, 0), // ┬
        0x252D => (2, 1, 0, 1, 0), // ┭
        0x252E => (1, 2, 0, 1, 0), // ┮
        0x252F => (2, 2, 0, 1, 0), // ┯
        0x2530 => (1, 1, 0, 2, 0), // ┰
        0x2531 => (2, 1, 0, 2, 0), // ┱
        0x2532 => (1, 2, 0, 2, 0), // ┲
        0x2533 => (2, 2, 0, 2, 0), // ┳
        
        // Tees: Up and Horizontal
        0x2534 => (1, 1, 1, 0, 0), // ┴
        0x2535 => (2, 1, 1, 0, 0), // ┵
        0x2536 => (1, 2, 1, 0, 0), // ┶
        0x2537 => (2, 2, 1, 0, 0), // ┷
        0x2538 => (1, 1, 2, 0, 0), // ┸
        0x2539 => (2, 1, 2, 0, 0), // ┹
        0x253A => (1, 2, 2, 0, 0), // ┺
        0x253B => (2, 2, 2, 0, 0), // ┻
        
        // Crossings
        0x253C => (1, 1, 1, 1, 0), // ┼
        0x253D => (2, 1, 1, 1, 0), // ┽
        0x253E => (1, 2, 1, 1, 0), // ┾
        0x253F => (2, 2, 1, 1, 0), // ┿
        0x2540 => (1, 1, 2, 1, 0), // ╀
        0x2541 => (1, 1, 1, 2, 0), // ╁
        0x2542 => (1, 1, 2, 2, 0), // ╂
        0x2543 => (2, 1, 2, 1, 0), // ╃
        0x2544 => (1, 2, 2, 1, 0), // ╄
        0x2545 => (2, 2, 2, 1, 0), // ╅
        0x2546 => (2, 1, 1, 2, 0), // ╆
        0x2547 => (1, 2, 1, 2, 0), // ╇
        0x2548 => (2, 2, 1, 2, 0), // ╈
        0x2549 => (2, 1, 2, 2, 0), // ╉
        0x254A => (1, 2, 2, 2, 0), // ╊
        0x254B => (2, 2, 2, 2, 0), // ╋
        
        // Double dashed
        0x254C => (1, 1, 0, 0, 3), // ╌
        0x254D => (2, 2, 0, 0, 3), // ╍
        0x254E => (0, 0, 1, 1, 3), // ╎
        0x254F => (0, 0, 2, 2, 3), // ╏
        
        // Double Lines
        0x2550 => (3, 3, 0, 0, 0), // ═
        0x2551 => (0, 0, 3, 3, 0), // ║
        0x2552 => (0, 3, 0, 1, 0), // ╒
        0x2553 => (0, 1, 0, 3, 0), // ╓
        0x2554 => (0, 3, 0, 3, 0), // ╔
        0x2555 => (3, 0, 0, 1, 0), // ╕
        0x2556 => (1, 0, 0, 3, 0), // ╖
        0x2557 => (3, 0, 0, 3, 0), // ╗
        0x2558 => (0, 3, 1, 0, 0), // ╘
        0x2559 => (0, 1, 3, 0, 0), // ╙
        0x255A => (0, 3, 3, 0, 0), // ╚
        0x255B => (3, 0, 1, 0, 0), // ╛
        0x255C => (1, 0, 3, 0, 0), // ╜
        0x255D => (3, 0, 3, 0, 0), // ╝
        0x255E => (0, 3, 1, 1, 0), // ╞
        0x255F => (0, 1, 3, 3, 0), // ╟
        0x2560 => (0, 3, 3, 3, 0), // ╠
        0x2561 => (3, 0, 1, 1, 0), // ╡
        0x2562 => (1, 0, 3, 3, 0), // ╢
        0x2563 => (3, 0, 3, 3, 0), // ╣
        0x2564 => (3, 3, 0, 1, 0), // ╤
        0x2565 => (1, 1, 0, 3, 0), // ╥
        0x2566 => (3, 3, 0, 3, 0), // ╦
        0x2567 => (3, 3, 1, 0, 0), // ╧
        0x2568 => (1, 1, 3, 0, 0), // ╨
        0x2569 => (3, 3, 3, 0, 0), // ╩
        0x256A => (3, 3, 1, 1, 0), // ╪
        0x256B => (1, 1, 3, 3, 0), // ╫
        0x256C => (3, 3, 3, 3, 0), // ╬
        
        // Round corners
        0x256D => (0, 1, 0, 1, 1), // ╭
        0x256E => (1, 0, 0, 1, 1), // ╮
        0x256F => (1, 0, 1, 0, 1), // ╯
        0x2570 => (0, 1, 1, 0, 1), // ╰
        
        // Diagonals
        0x2571 => (0, 0, 0, 0, 2), // ╱
        0x2572 => (0, 0, 0, 0, 2), // ╲
        0x2573 => (0, 0, 0, 0, 2), // ╳
        
        // Light / Heavy half lines
        0x2574 => (1, 0, 0, 0, 0), // ╴
        0x2575 => (0, 0, 1, 0, 0), // ╵
        0x2576 => (0, 1, 0, 0, 0), // ╶
        0x2577 => (0, 0, 0, 1, 0), // ╷
        0x2578 => (2, 0, 0, 0, 0), // ╸
        0x2579 => (0, 0, 2, 0, 0), // ╹
        0x257A => (0, 2, 0, 0, 0), // ╺
        0x257B => (0, 0, 0, 2, 0), // ╻
        0x257C => (1, 2, 0, 0, 0), // ╼
        0x257D => (0, 0, 1, 2, 0), // ╽
        0x257E => (2, 1, 0, 0, 0), // ╾
        0x257F => (0, 0, 2, 1, 0), // ╿
        
        _ => (0, 0, 0, 0, 0),
    })
}

fn is_arrow_symbol(ch: char) -> bool {
    matches!(ch as u32, 0x2190..=0x21FF | 0x2B00..=0x2BFF)
}

fn render_single_char(
    cell: &alacritty_terminal::term::cell::Cell,
    cell_x: f32,
    cell_y: f32,
    actual_cell_width: f32,
    actual_cell_height: f32,
    atlas: &mut crate::renderer::Atlas,
    fg_instances: &mut Vec<CellInstance>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) {
    if cell.c != ' ' && cell.c != '\0' {
        if is_custom_block_drawing(cell.c) {
            let fg = cell_fg_to_f32(cell.fg, cell.flags);
            let mut left = 0.0;
            let mut right = 0.0;
            let mut top = 0.0;
            let mut bottom = 0.0;
            let mut kind = 0.0;

            if let Some((l, r, t, b, k)) = decode_box_drawing(cell.c) {
                left = l as f32;
                right = r as f32;
                top = t as f32;
                bottom = b as f32;
                kind = k as f32;
            }

            let code = cell.c as u32 as f32;
            let is_color_val = -(code + kind / 10.0);

            let block_instance = CellInstance::new(
                cell_x,
                cell_y,
                actual_cell_width,
                actual_cell_height,
                fg,
                [left, right, top, bottom],
                0.0, 0.0, 1.0, 1.0,
                is_color_val,
            );
            fg_instances.push(block_instance);
        } else if let Some(entry) = atlas.get_or_rasterize(cell.c, device, queue) {
            if entry.width > 0.0 && entry.height > 0.0 {
                let mut fg = cell_fg_to_f32(cell.fg, cell.flags);
                if cell.c == '❯' {
                    fg = [0.35, 0.75, 0.35, 1.0];
                }
                let (aw, ah) = atlas.atlas_size();
                let raw_uv = entry.uv_coords(aw, ah);
                let [uv_x, uv_y, uv_end_x, uv_end_y] = raw_uv;
                let uv_w = uv_end_x - uv_x;
                let uv_h = uv_end_y - uv_y;

                let text_instance = if crate::renderer::is_block_element(cell.c) {
                    CellInstance::new(
                        cell_x,
                        cell_y,
                        actual_cell_width,
                        actual_cell_height,
                        fg,
                        [0.0, 0.0, 0.0, 0.0],
                        uv_x, uv_y, uv_w, uv_h,
                        if entry.is_color { 1.0 } else { 0.0 },
                    )
                } else if entry.is_color {
                    let char_width = if cell.flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR) { 2.0 } else { 1.0 };
                    let scale = actual_cell_height / entry.height;
                    let emoji_render_width = entry.width * scale;
                    let emoji_render_height = actual_cell_height;
                    let x_offset = ((actual_cell_width * char_width) - emoji_render_width) / 2.0;
                    let glyph_x = (cell_x + x_offset.max(0.0)).round();
                    let glyph_y = cell_y.round();
                    CellInstance::new(
                        glyph_x,
                        glyph_y,
                        emoji_render_width,
                        emoji_render_height,
                        fg,
                        [0.0, 0.0, 0.0, 0.0],
                        uv_x, uv_y, uv_w, uv_h,
                        1.0,
                    )
                } else {
                    let glyph_x = if is_arrow_symbol(cell.c) {
                        let x_offset = (actual_cell_width - entry.width) / 2.0;
                        (cell_x + x_offset).round()
                    } else {
                        (cell_x + entry.left).round()
                    };
                    let glyph_y = (cell_y + atlas.ascent() + entry.top).round();
                    CellInstance::new(
                        glyph_x,
                        glyph_y,
                        entry.width,
                        entry.height,
                        fg,
                        [0.0, 0.0, 0.0, 0.0],
                        uv_x, uv_y, uv_w, uv_h,
                        0.0,
                    )
                };
                fg_instances.push(text_instance);
            }
        }
    }
}