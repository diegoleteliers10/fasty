//! Glyph atlas for terminal rendering.
//!
//! Uses a shelf-packing strategy to store rasterized glyphs in a single GPU texture.
//! Uses FreeType for high-quality antialiased/hinted text rendering and color emoji support.

use std::collections::HashMap;
use anyhow::Context;
use wgpu::{Device, Queue, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages};
use freetype::Library;
use freetype::face::LoadFlag;

thread_local! {
    static FT_LIB: Library = Library::init().expect("Failed to initialize FreeType");
}

#[derive(Copy, Clone, Debug)]
pub struct AtlasEntry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub left: f32,
    pub top: f32,
    pub is_color: bool,
}

impl AtlasEntry {
    pub fn uv_coords(&self, atlas_width: u32, atlas_height: u32) -> [f32; 4] {
        [
            self.x / atlas_width as f32,
            self.y / atlas_height as f32,
            (self.x + self.width) / atlas_width as f32,
            (self.y + self.height) / atlas_height as f32,
        ]
    }
}

struct ShelfPacker {
    width: u32,
    height: u32,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

impl ShelfPacker {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
        }
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if self.cursor_x + w > self.width {
            self.cursor_x = 0;
            self.cursor_y += self.row_height;
            self.row_height = 0;
        }

        if self.cursor_y + h > self.height {
            return None;
        }

        let x = self.cursor_x;
        let y = self.cursor_y;

        self.cursor_x += w;
        self.row_height = self.row_height.max(h);

        Some((x, y))
    }
}

pub struct Atlas {
    texture: wgpu::Texture,
    entries: HashMap<char, AtlasEntry>,
    packer: ShelfPacker,
    primary_path: String,
    fallback_paths: Vec<String>,
    fallback_glyph: Option<AtlasEntry>,
    cell_width: f32,
    cell_height: f32,
    ascent: f32,
    atlas_width: u32,
    atlas_height: u32,
    font_size: f32,
    scale_factor: f32,
    pub app_icon: Option<AtlasEntry>,
}

impl Atlas {
    pub fn new(
        device: &Device,
        queue: &Queue,
        width: u32,
        height: u32,
        font_size: f32,
        scale_factor: f32,
    ) -> anyhow::Result<Self> {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("glyph-atlas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            mip_level_count: 1,
            sample_count: 1,
            view_formats: &[],
        });

        let primary_path = Self::load_font_path()?;
        let fallback_paths = Self::load_fallback_paths(&primary_path);
        let physical_size = font_size * scale_factor;

        let (cell_width, cell_height, ascent) = FT_LIB.with(|lib| -> anyhow::Result<(f32, f32, f32)> {
            let face = lib.new_face(&primary_path, 0).context("Failed to load primary face")?;
            let _ = face.set_pixel_sizes(0, physical_size as u32);
            
            let metrics = face.size_metrics().context("Failed to get size metrics")?;
            let cell_height = (metrics.height as f32 / 64.0).ceil();
            
            // Get advance of '0' for cell width
            let zero_idx = face.get_char_index('0' as usize).unwrap_or(0);
            let cell_width = if zero_idx != 0 {
                let _ = face.load_glyph(zero_idx, LoadFlag::RENDER);
                (face.glyph().advance().x as f32 / 64.0).round()
            } else {
                (physical_size * 0.6).round()
            };
            
            let ascent = (metrics.ascender as f32 / 64.0).round();
            
            Ok((cell_width, cell_height, ascent))
        })?;

        let mut packer = ShelfPacker::new(width, height);
        packer.alloc(10, 10);

        let white_pixel = [255u8; 4];
        queue.write_texture(
            wgpu::ImageCopyTextureBase {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &white_pixel,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );

        let mut atlas = Self {
            texture,
            entries: HashMap::new(),
            packer,
            primary_path,
            fallback_paths,
            fallback_glyph: None,
            cell_width,
            cell_height,
            ascent,
            atlas_width: width,
            atlas_height: height,
            font_size,
            scale_factor,
            app_icon: None,
        };

        atlas.rasterize_basic_glyphs(device, queue)?;

        if let Some(space) = atlas.entries.get(&' ') {
            atlas.fallback_glyph = Some(*space);
        }

        // Try to load the app icon
        match atlas.load_custom_image("assets/fastySmallIcon.png", 24, queue) {
            Ok(entry) => {
                atlas.app_icon = Some(entry);
                tracing::info!("Successfully loaded app icon into atlas");
            }
            Err(e) => {
                tracing::warn!("Failed to load app icon: {:?}", e);
            }
        }

        tracing::info!(
            "ATLAS: cell={:.1}x{:.1}, entries={}, atlas={}x{}",
            cell_width,
            cell_height,
            atlas.entries.len(),
            width,
            height
        );

        Ok(atlas)
    }

    fn load_font_path() -> anyhow::Result<String> {
        let output = std::process::Command::new("fc-match")
            .arg("-f")
            .arg("%{file}")
            .arg("monospace")
            .output()
            .context("Failed to run fc-match")?;

        if !output.status.success() {
            anyhow::bail!("fc-match returned error");
        }

        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if path.is_empty() {
            anyhow::bail!("fc-match returned empty path");
        }

        Ok(path)
    }

    fn load_fallback_paths(primary_path: &str) -> Vec<String> {
        let mut names: Vec<String> = vec![
            "Symbols Nerd Font".to_string(),
            "DejaVu Sans".to_string(),
            "Noto Sans Symbols".to_string(),
            "Noto Color Emoji".to_string(),
            "FreeMono".to_string(),
        ];

        // Try to find installed Nerd Fonts dynamically from fontconfig
        if let Ok(output) = std::process::Command::new("fc-list")
            .arg(":")
            .arg("family")
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut nerd_families = std::collections::HashSet::new();
                for line in stdout.lines() {
                    for family in line.split(',') {
                        let family = family.trim();
                        if family.to_lowercase().contains("nerd font") {
                            if !family.contains("Mono") && !family.contains("Propo") {
                                nerd_families.insert(family.to_string());
                            } else if nerd_families.is_empty() {
                                nerd_families.insert(family.to_string());
                            }
                        }
                    }
                    if nerd_families.len() >= 5 {
                        break;
                    }
                }
                for fam in nerd_families {
                    names.insert(0, fam);
                }
            }
        }

        let mut fallback_paths = Vec::new();
        use std::collections::HashSet;
        let mut loaded_paths = HashSet::new();
        loaded_paths.insert(primary_path.to_string());

        for name in names {
            if let Ok(output) = std::process::Command::new("fc-match")
                .arg("-f")
                .arg("%{file}")
                .arg(&name)
                .output()
            {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() && !loaded_paths.contains(&path) {
                        loaded_paths.insert(path.clone());
                        fallback_paths.push(path);
                    }
                }
            }
        }
        fallback_paths
    }

    fn rasterize_basic_glyphs(&mut self, device: &Device, queue: &Queue) -> anyhow::Result<()> {
        let chars: Vec<char> = (0x20u32..=0x7Fu32)
            .chain(0x2500u32..=0x257Fu32)
            .filter_map(std::char::from_u32)
            .collect();

        FT_LIB.with(|lib| {
            if let Ok(face) = lib.new_face(&self.primary_path, 0) {
                let physical_size = self.font_size * self.scale_factor;
                let _ = face.set_pixel_sizes(0, physical_size as u32);
                let is_color = face.has_fixed_sizes();

                for c in chars {
                    if let Some(idx) = face.get_char_index(c as usize) {
                        if idx != 0 {
                            let load_flags = if is_color {
                                LoadFlag::RENDER | LoadFlag::COLOR
                            } else {
                                LoadFlag::RENDER
                            };
                            if face.load_glyph(idx, load_flags).is_ok() {
                                let _ = self.rasterize_freetype_glyph(device, queue, c, &face.glyph(), is_color);
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }

    fn rasterize_freetype_glyph(
        &mut self,
        _device: &Device,
        queue: &Queue,
        c: char,
        glyph: &freetype::GlyphSlot,
        is_color: bool,
    ) -> anyhow::Result<()> {
        let bitmap = glyph.bitmap();
        let w = bitmap.width() as u32;
        let h = bitmap.rows() as u32;

        if w == 0 || h == 0 {
            let entry = AtlasEntry {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
                left: 0.0,
                top: 0.0,
                is_color,
            };
            self.entries.insert(c, entry);
            return Ok(());
        }

        // 1. Get raw RGBA data from FreeType bitmap
        let mut rgba_data = vec![0u8; (w * h * 4) as usize];
        let buffer = bitmap.buffer();
        let pitch = bitmap.pitch() as usize;

        let pixel_mode = bitmap.pixel_mode().unwrap_or(freetype::bitmap::PixelMode::Gray);
        match pixel_mode {
            freetype::bitmap::PixelMode::Gray => {
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let src_idx = y * pitch + x;
                        if src_idx < buffer.len() {
                            let val = buffer[src_idx];
                            let dst_idx = (y * w as usize + x) * 4;
                            rgba_data[dst_idx] = val;
                            rgba_data[dst_idx + 1] = val;
                            rgba_data[dst_idx + 2] = val;
                            rgba_data[dst_idx + 3] = val;
                        }
                    }
                }
            }
            freetype::bitmap::PixelMode::Bgra => {
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let src_idx = y * pitch + x * 4;
                        if src_idx + 3 < buffer.len() {
                            let b = buffer[src_idx];
                            let g = buffer[src_idx + 1];
                            let r = buffer[src_idx + 2];
                            let a = buffer[src_idx + 3];
                            let dst_idx = (y * w as usize + x) * 4;
                            rgba_data[dst_idx] = r;
                            rgba_data[dst_idx + 1] = g;
                            rgba_data[dst_idx + 2] = b;
                            rgba_data[dst_idx + 3] = a;
                        }
                    }
                }
            }
            _ => {
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let dst_idx = (y * w as usize + x) * 4;
                        rgba_data[dst_idx] = 255;
                        rgba_data[dst_idx + 1] = 255;
                        rgba_data[dst_idx + 2] = 255;
                        rgba_data[dst_idx + 3] = 255;
                    }
                }
            }
        }

        // 2. Scale down if taller than cell_height
        let (final_rgba, final_w, final_h, scale) = if h > self.cell_height.round() as u32 {
            let scale = self.cell_height / h as f32;
            let new_w = (w as f32 * scale).round() as u32;
            let new_h = self.cell_height.round() as u32;
            let scaled = scale_rgba_bitmap(&rgba_data, w as usize, h as usize, new_w as usize, new_h as usize);
            (scaled, new_w, new_h, scale)
        } else {
            (rgba_data, w, h, 1.0)
        };

        // 3. Allocate and upload
        let padding = 1u32;
        let alloc_w = final_w + padding * 2;
        let alloc_h = final_h + padding * 2;

        if let Some(pos) = self.packer.alloc(alloc_w, alloc_h) {
            let entry = AtlasEntry {
                x: (pos.0 + padding) as f32,
                y: (pos.1 + padding) as f32,
                width: final_w as f32,
                height: final_h as f32,
                left: (glyph.bitmap_left() as f32 * scale).round(),
                top: (-glyph.bitmap_top() as f32 * scale).round(),
                is_color,
            };

            self.entries.insert(c, entry);

            queue.write_texture(
                wgpu::ImageCopyTextureBase {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: pos.0 + padding,
                        y: pos.1 + padding,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &final_rgba,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(final_w * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: final_w,
                    height: final_h,
                    depth_or_array_layers: 1,
                },
            );
        }

        Ok(())
    }

    pub fn get_or_rasterize(&mut self, c: char, device: &Device, queue: &Queue) -> Option<AtlasEntry> {
        if let Some(entry) = self.entries.get(&c) {
            return Some(*entry);
        }

        let entry = FT_LIB.with(|lib| {
            // 1. Try primary font
            if let Ok(face) = lib.new_face(&self.primary_path, 0) {
                let physical_size = self.font_size * self.scale_factor;
                let _ = face.set_pixel_sizes(0, physical_size as u32);
                if let Some(idx) = face.get_char_index(c as usize) {
                    if idx != 0 {
                        let is_color = face.has_fixed_sizes();
                        let load_flags = if is_color {
                            LoadFlag::RENDER | LoadFlag::COLOR
                        } else {
                            LoadFlag::RENDER
                        };

                        if face.load_glyph(idx, load_flags).is_ok() {
                            let _ = self.rasterize_freetype_glyph(device, queue, c, &face.glyph(), is_color);
                            return self.entries.get(&c).copied();
                        }
                    }
                }
            }

            // 2. Try fallbacks
            for path in &self.fallback_paths {
                if let Ok(face) = lib.new_face(path, 0) {
                    let physical_size = self.font_size * self.scale_factor;
                    let is_color = face.has_fixed_sizes();

                    if is_color {
                        let num_fixed_sizes = face.raw().num_fixed_sizes;
                        if num_fixed_sizes > 0 {
                            let mut best_index = 0;
                            let mut best_diff = i32::MAX;
                            let target_size = physical_size as i32;
                            let sizes = unsafe {
                                std::slice::from_raw_parts(face.raw().available_sizes, num_fixed_sizes as usize)
                            };
                            for (i, sz) in sizes.iter().enumerate() {
                                let diff = (sz.height as i32 - target_size).abs();
                                if diff < best_diff {
                                    best_diff = diff;
                                    best_index = i;
                                }
                            }
                            unsafe {
                                freetype::ffi::FT_Select_Size(face.raw() as *const _ as *mut _, best_index as i32);
                            }
                        }
                    } else {
                        let _ = face.set_pixel_sizes(0, physical_size as u32);
                    }

                    if let Some(idx) = face.get_char_index(c as usize) {
                        if idx != 0 {
                            let load_flags = if is_color {
                                LoadFlag::RENDER | LoadFlag::COLOR
                            } else {
                                LoadFlag::RENDER
                            };
                            if face.load_glyph(idx, load_flags).is_ok() {
                                let _ = self.rasterize_freetype_glyph(device, queue, c, &face.glyph(), is_color);
                                return self.entries.get(&c).copied();
                            }
                        }
                    }
                }
            }

            None
        });

        if let Some(entry) = entry {
            Some(entry)
        } else {
            let dummy = AtlasEntry {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
                left: 0.0,
                top: 0.0,
                is_color: false,
            };
            self.entries.insert(c, dummy);
            Some(dummy)
        }
    }

    pub fn entries_len(&self) -> usize {
        self.entries.len()
    }

    pub fn cell_size(&self) -> (f32, f32) {
        (self.cell_width, self.cell_height)
    }

    pub fn ascent(&self) -> f32 {
        self.ascent
    }

    pub fn atlas_size(&self) -> (u32, u32) {
        (self.atlas_width, self.atlas_height)
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn load_custom_image(
        &mut self,
        path: &str,
        target_size: u32,
        queue: &Queue,
    ) -> anyhow::Result<AtlasEntry> {
        let img = image::open(path).context("Failed to open image file")?;
        let scaled = img.resize(target_size, target_size, image::imageops::FilterType::Lanczos3);
        let rgba = scaled.to_rgba8();
        let (w, h) = rgba.dimensions();
        
        let padding = 1;
        let alloc_w = w + padding * 2;
        let alloc_h = h + padding * 2;
        
        if let Some(pos) = self.packer.alloc(alloc_w, alloc_h) {
            let entry = AtlasEntry {
                x: (pos.0 + padding) as f32,
                y: (pos.1 + padding) as f32,
                width: w as f32,
                height: h as f32,
                left: 0.0,
                top: 0.0,
                is_color: true,
            };
            
            queue.write_texture(
                wgpu::ImageCopyTextureBase {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: pos.0 + padding,
                        y: pos.1 + padding,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &rgba,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            Ok(entry)
        } else {
            anyhow::bail!("Failed to allocate space in atlas for custom image")
        }
    }
}

fn scale_rgba_bitmap(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut dst = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        for x in 0..dw {
            let sx = (x * sw / dw).min(sw - 1);
            let sy = (y * sh / dh).min(sh - 1);
            let src_idx = (sy * sw + sx) * 4;
            let dst_idx = (y * dw + x) * 4;
            dst[dst_idx] = src[src_idx];
            dst[dst_idx + 1] = src[src_idx + 1];
            dst[dst_idx + 2] = src[src_idx + 2];
            dst[dst_idx + 3] = src[src_idx + 3];
        }
    }
    dst
}