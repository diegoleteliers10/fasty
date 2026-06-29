//! Glyph atlas for terminal rendering.
//!
//! Uses a shelf-packing strategy to store rasterized glyphs in a single GPU texture.
//! Uses FreeType for high-quality antialiased/hinted text rendering and color emoji support.

use std::collections::HashMap;
use anyhow::Context;
use wgpu::{Device, Queue, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages};
use freetype::Library;
use freetype::face::LoadFlag;

use usvg::Tree;

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
    pub is_block: bool,
}

impl AtlasEntry {
    pub fn uv_coords(&self, atlas_width: u32, atlas_height: u32) -> [f32; 4] {
        if self.is_block {
            // Inset by 0.5 pixels to sample exactly from the center of the edge texels.
            // Under Linear filtering, this prevents color bleeding/gaps at the cell boundaries.
            let inset_x = 0.5;
            let inset_y = 0.5;
            [
                (self.x + inset_x) / atlas_width as f32,
                (self.y + inset_y) / atlas_height as f32,
                (self.x + self.width - inset_x) / atlas_width as f32,
                (self.y + self.height - inset_y) / atlas_height as f32,
            ]
        } else {
            [
                self.x / atlas_width as f32,
                self.y / atlas_height as f32,
                (self.x + self.width) / atlas_width as f32,
                (self.y + self.height) / atlas_height as f32,
            ]
        }
    }
}

#[derive(Clone)]
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

#[derive(Clone)]
pub struct RowShapingResult {
    pub glyph_infos: Vec<rustybuzz::GlyphInfo>,
    pub col_map: Vec<usize>,
}

#[derive(Hash, Eq, PartialEq, Debug, Clone, Copy)]
pub enum GlyphKey {
    Char(char),
    GlyphId(u32),
}

pub struct Atlas {
    texture: std::sync::Arc<wgpu::Texture>,
    entries: HashMap<GlyphKey, AtlasEntry>,
    packer: ShelfPacker,
    primary_path: String,
    primary_font_bytes: Option<Vec<u8>>,
    hb_face: Option<rustybuzz::Face<'static>>,
    pub shaping_cache: HashMap<String, RowShapingResult>,
    pub hyperlink_cache: HashMap<String, std::sync::Arc<str>>,
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
    pub icon_add: Option<AtlasEntry>,
    pub icon_close: Option<AtlasEntry>,
    pub icon_copy: Option<AtlasEntry>,
    pub icon_paste: Option<AtlasEntry>,
    pub icon_settings: Option<AtlasEntry>,
    pub icon_text_font: Option<AtlasEntry>,
    pub icon_less: Option<AtlasEntry>,
    pub icon_maximize: Option<AtlasEntry>,
    pub icon_branch: Option<AtlasEntry>,
    #[cfg(target_os = "macos")]
    pub mac_close: Option<AtlasEntry>,
    #[cfg(target_os = "macos")]
    pub mac_min: Option<AtlasEntry>,
    #[cfg(target_os = "macos")]
    pub mac_max: Option<AtlasEntry>,
}

struct CachedIcon {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

struct CachedIcons {
    app_icon: CachedIcon,
    icon_add: CachedIcon,
    icon_close: CachedIcon,
    icon_copy: CachedIcon,
    icon_paste: CachedIcon,
    icon_settings: CachedIcon,
    icon_text_font: CachedIcon,
    icon_less: CachedIcon,
    icon_maximize: CachedIcon,
    icon_branch: CachedIcon,
    #[cfg(target_os = "macos")]
    mac_close: CachedIcon,
    #[cfg(target_os = "macos")]
    mac_min: CachedIcon,
    #[cfg(target_os = "macos")]
    mac_max: CachedIcon,
}

fn get_cached_icons() -> &'static CachedIcons {
    static CACHE: std::sync::OnceLock<CachedIcons> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let app_icon = {
            let img = image::load_from_memory(include_bytes!("../../assets/fasttySmallIcon.png")).unwrap();
            let scaled = img.resize(24, 24, image::imageops::FilterType::Lanczos3);
            let rgba = scaled.to_rgba8();
            let (w, h) = rgba.dimensions();
            CachedIcon {
                rgba: rgba.into_raw(),
                width: w,
                height: h,
            }
        };
        
        let render_svg = |svg_data: &str, target_size: u32| -> CachedIcon {
            let svg_data = svg_data.replace("currentColor", "white");
            let opt = usvg::Options::default();
            let tree = Tree::from_str(&svg_data, &opt).unwrap();
            let mut pixmap = tiny_skia::Pixmap::new(target_size, target_size).unwrap();
            pixmap.fill(tiny_skia::Color::TRANSPARENT);
            let size = tree.size();
            let scale_x = target_size as f32 / size.width();
            let scale_y = target_size as f32 / size.height();
            let scale = scale_x.min(scale_y);
            let transform = tiny_skia::Transform::from_scale(scale, scale);
            resvg::render(&tree, transform, &mut pixmap.as_mut());
            CachedIcon {
                rgba: pixmap.data().to_vec(),
                width: target_size,
                height: target_size,
            }
        };

        CachedIcons {
            app_icon,
            icon_add: render_svg(include_str!("../../assets/icons/add.svg"), 64),
            icon_close: render_svg(include_str!("../../assets/icons/close.svg"), 64),
            icon_copy: render_svg(include_str!("../../assets/icons/copy.svg"), 64),
            icon_paste: render_svg(include_str!("../../assets/icons/paste.svg"), 64),
            icon_settings: render_svg(include_str!("../../assets/icons/settings.svg"), 64),
            icon_text_font: render_svg(include_str!("../../assets/icons/text-font.svg"), 64),
            icon_less: render_svg(include_str!("../../assets/icons/less.svg"), 64),
            icon_maximize: render_svg(include_str!("../../assets/icons/maximize.svg"), 64),
            icon_branch: render_svg(include_str!("../../assets/icons/branch.svg"), 64),
            #[cfg(target_os = "macos")]
            mac_close: render_svg(include_str!("../../assets/icons/mac_close.svg"), 64),
            #[cfg(target_os = "macos")]
            mac_min: render_svg(include_str!("../../assets/icons/mac_min.svg"), 64),
            #[cfg(target_os = "macos")]
            mac_max: render_svg(include_str!("../../assets/icons/mac_max.svg"), 64),
        }
    })
}

impl Atlas {
    pub fn try_clone(&self) -> anyhow::Result<Self> {
        let hb_face = if let Some(bytes) = &self.primary_font_bytes {
            rustybuzz::Face::from_slice(bytes, 0).map(|face| unsafe {
                std::mem::transmute::<rustybuzz::Face<'_>, rustybuzz::Face<'static>>(face)
            })
        } else {
            None
        };
        Ok(Self {
            texture: self.texture.clone(),
            entries: self.entries.clone(),
            packer: self.packer.clone(),
            primary_path: self.primary_path.clone(),
            primary_font_bytes: self.primary_font_bytes.clone(),
            hb_face,
            shaping_cache: self.shaping_cache.clone(),
            hyperlink_cache: self.hyperlink_cache.clone(),
            fallback_paths: self.fallback_paths.clone(),
            fallback_glyph: self.fallback_glyph.clone(),
            cell_width: self.cell_width,
            cell_height: self.cell_height,
            ascent: self.ascent,
            atlas_width: self.atlas_width,
            atlas_height: self.atlas_height,
            font_size: self.font_size,
            scale_factor: self.scale_factor,
            app_icon: self.app_icon.clone(),
            icon_add: self.icon_add.clone(),
            icon_close: self.icon_close.clone(),
            icon_copy: self.icon_copy.clone(),
            icon_paste: self.icon_paste.clone(),
            icon_settings: self.icon_settings.clone(),
            icon_text_font: self.icon_text_font.clone(),
            icon_less: self.icon_less.clone(),
            icon_maximize: self.icon_maximize.clone(),
            icon_branch: self.icon_branch.clone(),
            #[cfg(target_os = "macos")]
            mac_close: self.mac_close.clone(),
            #[cfg(target_os = "macos")]
            mac_min: self.mac_min.clone(),
            #[cfg(target_os = "macos")]
            mac_max: self.mac_max.clone(),
        })
    }

    pub fn new(
        device: &Device,
        queue: &Queue,
        width: u32,
        height: u32,
        font_family: &str,
        font_size: f32,
        scale_factor: f32,
    ) -> anyhow::Result<Self> {
        let texture = std::sync::Arc::new(device.create_texture(&TextureDescriptor {
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
        }));

        let primary_path_initial = Self::load_font_path(font_family).unwrap_or_else(|_| "monospace".to_string());
        let physical_size = font_size * scale_factor;

        let (cell_width, cell_height, ascent, primary_path_resolved) = FT_LIB.with(|lib| -> anyhow::Result<(f32, f32, f32, String)> {
            // Helper to get metrics from a face if they are valid for monospace rendering
            let get_metrics = |face: &freetype::Face| -> Option<(f32, f32, f32)> {
                let _ = face.set_pixel_sizes(0, physical_size as u32);
                let metrics = face.size_metrics()?;
                let cell_height = (metrics.height as f32 / 64.0).ceil();
                
                // Get advance of '0' for cell width
                let zero_idx = face.get_char_index('0' as usize).unwrap_or(0);
                let cell_width = if zero_idx != 0 {
                    let _ = face.load_glyph(zero_idx, LoadFlag::RENDER).ok()?;
                    (face.glyph().advance().x as f32 / 64.0).round()
                } else {
                    (physical_size * 0.6).round()
                };
                
                let ascent = (metrics.ascender as f32 / 64.0).round();
                
                if cell_width >= 3.0 && cell_height >= 5.0 {
                    Some((cell_width, cell_height, ascent))
                } else {
                    None
                }
            };

            // 1. Try primary font path
            if let Ok(face) = lib.new_face(&primary_path_initial, 0) {
                if let Some((w, h, a)) = get_metrics(&face) {
                    return Ok((w, h, a, primary_path_initial));
                }
            }

            // 2. Try default fallback system monospace path
            tracing::warn!("Font '{}' lacks valid monospace metrics. Falling back to default system monospace.", font_family);
            let fallback_path = Self::load_font_path("monospace").unwrap_or_else(|_| "monospace".to_string());
            if let Ok(face) = lib.new_face(&fallback_path, 0) {
                if let Some((w, h, a)) = get_metrics(&face) {
                    return Ok((w, h, a, fallback_path));
                }
            }

            // 3. Absolute last resort hardcoded defaults
            let cell_h = physical_size.ceil().max(12.0);
            let cell_w = (physical_size * 0.6).round().max(7.0);
            let ascent = (physical_size * 0.8).round().max(9.0);
            Ok((cell_w, cell_h, ascent, fallback_path))
        })?;

        let primary_path = primary_path_resolved;
        let fallback_paths = Self::load_fallback_paths(&primary_path);

        let mut packer = ShelfPacker::new(width, height);
        packer.alloc(10, 10);

        let white_pixel = [255u8; 4];
        queue.write_texture(
            wgpu::ImageCopyTextureBase {
                texture: texture.as_ref(),
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

        let primary_font_bytes = std::fs::read(&primary_path).ok();

        let mut hb_face = None;
        if let Some(bytes) = &primary_font_bytes {
            if let Some(face) = rustybuzz::Face::from_slice(bytes, 0) {
                unsafe {
                    hb_face = Some(std::mem::transmute::<rustybuzz::Face<'_>, rustybuzz::Face<'static>>(face));
                }
            }
        }

        let mut atlas = Self {
            texture,
            entries: HashMap::new(),
            packer,
            primary_path,
            primary_font_bytes,
            hb_face,
            shaping_cache: HashMap::new(),
            hyperlink_cache: HashMap::new(),
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
            icon_add: None,
            icon_close: None,
            icon_copy: None,
            icon_paste: None,
            icon_settings: None,
            icon_text_font: None,
            icon_less: None,
            icon_maximize: None,
            icon_branch: None,
            #[cfg(target_os = "macos")]
            mac_close: None,
            #[cfg(target_os = "macos")]
            mac_min: None,
            #[cfg(target_os = "macos")]
            mac_max: None,
        };

        // Pre-rasterize basic alphanumeric and punctuation characters at startup
        // to avoid calling queue.write_texture inside active RenderPasses for UI windows.
        let startup_chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.- :";
        atlas.rasterize_batch(startup_chars, device, queue);
        if let Some(space) = atlas.entries.get(&GlyphKey::Char(' ')) {
            atlas.fallback_glyph = Some(*space);
        }

        // Try to load the app icon and custom SVG icons using the pre-rasterized cache
        let cached = get_cached_icons();
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.app_icon.rgba, cached.app_icon.width, cached.app_icon.height, true, queue) {
            atlas.app_icon = Some(entry);
        }
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.icon_add.rgba, cached.icon_add.width, cached.icon_add.height, false, queue) {
            atlas.icon_add = Some(entry);
        }
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.icon_close.rgba, cached.icon_close.width, cached.icon_close.height, false, queue) {
            atlas.icon_close = Some(entry);
        }
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.icon_copy.rgba, cached.icon_copy.width, cached.icon_copy.height, false, queue) {
            atlas.icon_copy = Some(entry);
        }
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.icon_paste.rgba, cached.icon_paste.width, cached.icon_paste.height, false, queue) {
            atlas.icon_paste = Some(entry);
        }
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.icon_settings.rgba, cached.icon_settings.width, cached.icon_settings.height, false, queue) {
            atlas.icon_settings = Some(entry);
        }
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.icon_text_font.rgba, cached.icon_text_font.width, cached.icon_text_font.height, false, queue) {
            atlas.icon_text_font = Some(entry);
        }
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.icon_less.rgba, cached.icon_less.width, cached.icon_less.height, false, queue) {
            atlas.icon_less = Some(entry);
        }
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.icon_maximize.rgba, cached.icon_maximize.width, cached.icon_maximize.height, false, queue) {
            atlas.icon_maximize = Some(entry);
        }
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.icon_branch.rgba, cached.icon_branch.width, cached.icon_branch.height, false, queue) {
            atlas.icon_branch = Some(entry);
        }
        #[cfg(target_os = "macos")]
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.mac_close.rgba, cached.mac_close.width, cached.mac_close.height, false, queue) {
            atlas.mac_close = Some(entry);
        }
        #[cfg(target_os = "macos")]
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.mac_min.rgba, cached.mac_min.width, cached.mac_min.height, false, queue) {
            atlas.mac_min = Some(entry);
        }
        #[cfg(target_os = "macos")]
        if let Ok(entry) = atlas.load_raw_rgba_image(&cached.mac_max.rgba, cached.mac_max.width, cached.mac_max.height, false, queue) {
            atlas.mac_max = Some(entry);
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

    #[cfg(target_os = "windows")]
    pub fn load_font_path(family: &str) -> anyhow::Result<String> {
        let family_lower = family.to_lowercase();
        let target_family = if family_lower == "monospace" || family_lower == "sans-serif" || family_lower == "serif" {
            "consolas"
        } else {
            &family_lower
        };

        let query_reg = |key: &str| -> Option<String> {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new("reg")
                .creation_flags(0x08000000)
                .args(&["query", key])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let line_lower = line.to_lowercase();
                if line_lower.contains(target_family) {
                    if let Some(pos) = line.find("REG_SZ") {
                        let val = line[pos + 6..].trim();
                        if !val.is_empty() {
                            return Some(val.to_string());
                        }
                    }
                }
            }
            None
        };

        let font_file = query_reg("HKCU\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts")
            .or_else(|| query_reg("HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts"));

        if let Some(file) = font_file {
            let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
            let localappdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
            
            let sys_path = std::path::Path::new(&windir).join("Fonts").join(&file);
            if sys_path.exists() {
                return Ok(sys_path.to_string_lossy().to_string());
            }

            if !localappdata.is_empty() {
                let user_path = std::path::Path::new(&localappdata)
                    .join("Microsoft\\Windows\\Fonts")
                    .join(&file);
                if user_path.exists() {
                    return Ok(user_path.to_string_lossy().to_string());
                }
            }

            let path = std::path::Path::new(&file);
            if path.exists() {
                return Ok(file);
            }
        }

        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        let fonts_dir = std::path::Path::new(&windir).join("Fonts");
        
        let fallbacks = [
            "consola.ttf",
            "consolab.ttf",
            "cour.ttf",
            "lucon.ttf",
        ];

        for fallback in &fallbacks {
            let path = fonts_dir.join(fallback);
            if path.exists() {
                return Ok(path.to_string_lossy().to_string());
            }
        }

        anyhow::bail!("Could not find font on Windows")
    }

    #[cfg(target_os = "macos")]
    pub fn load_font_path(family: &str) -> anyhow::Result<String> {
        let family_lower = family.to_lowercase();
        let target_family = if family_lower == "monospace" || family_lower == "sans-serif" || family_lower == "serif" {
            "menlo"
        } else {
            &family_lower
        };

        let paths = [
            "/System/Library/Fonts/Constants/Menlo.ttc",
            "/System/Library/Fonts/Menlo.ttc",
            "/System/Library/Fonts/Courier.dfont",
            "/System/Library/Fonts/Supplemental/Courier New.ttf",
            "/System/Library/Fonts/Supplemental/Courier New Bold.ttf",
            "/System/Library/Fonts/Monaco.ttf",
            "/Library/Fonts/Arial.ttf",
        ];

        for path in &paths {
            let p = std::path::Path::new(path);
            if p.exists() {
                let name = p.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
                if name.contains(target_family) {
                    return Ok(path.to_string());
                }
            }
        }

        for path in &paths {
            if std::path::Path::new(path).exists() {
                return Ok(path.to_string());
            }
        }

        anyhow::bail!("Could not find font on macOS")
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    pub fn load_font_path(family: &str) -> anyhow::Result<String> {
        let output = std::process::Command::new("fc-match")
            .arg("-f")
            .arg("%{file}")
            .arg(family)
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

    fn is_color_font_path(path: &str) -> bool {
        let lower = path.to_lowercase();
        lower.contains("coloremoji")
            || lower.contains("color-emoji")
            || lower.contains("notoemoji")
            || lower.contains("applecoloremoji")
            || lower.contains("twemoji")
    }

    fn load_fallback_paths(primary_path: &str) -> Vec<String> {
        static FALLBACK_PATHS_CACHE: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        let paths = FALLBACK_PATHS_CACHE.get_or_init(|| {
            use std::path::Path;

            let mut names: Vec<String> = vec![
                "Noto Sans Symbols 2".to_string(),
                "Noto Sans Symbols".to_string(),
                "Noto Sans".to_string(),
                "DejaVu Sans".to_string(),
                "Noto Sans CJK SC".to_string(),
                "Noto Sans CJK".to_string(),
                "Symbols Nerd Font".to_string(),
                "JetBrainsMono Nerd Font".to_string(),
                "Noto Color Emoji".to_string(),
                "FreeMono".to_string(),
            ];

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
                                nerd_families.insert(family.to_string());
                            }
                        }
                        if nerd_families.len() >= 8 { break; }
                    }
                    for fam in nerd_families {
                        names.insert(0, fam);
                    }
                }
            }

            let mut fallback_paths = Vec::new();
            use std::collections::HashSet;
            let mut loaded_paths = HashSet::new();

            for name in &names {
                if let Ok(output) = std::process::Command::new("fc-match")
                    .arg("-f")
                    .arg("%{file}")
                    .arg(name)
                    .output()
                {
                    if output.status.success() {
                        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        if !path.is_empty()
                            && !loaded_paths.contains(&path)
                            && Path::new(&path).exists()
                        {
                            loaded_paths.insert(path.clone());
                            fallback_paths.push(path);
                        }
                    }
                }
            }

            // Direct filesystem probes (used when fontconfig is unavailable,
            // or as a hard guarantee for glyph coverage on each platform).
            #[cfg(target_os = "linux")]
            let extra_paths: Vec<String> = vec![
                "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf".to_string(),
                "/usr/share/fonts/noto/NotoSansSymbols-Regular.ttf".to_string(),
                "/usr/share/fonts/noto/NotoSans-Regular.ttf".to_string(),
                "/usr/share/fonts/TTF/DejaVuSans.ttf".to_string(),
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".to_string(),
                "/usr/share/fonts/noto/NotoSansCJK-Regular.ttc".to_string(),
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc".to_string(),
                "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf".to_string(),
                "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf".to_string(),
                "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf".to_string(),
                "/usr/share/fonts/gnu-free/FreeMono.ttf".to_string(),
                "/usr/share/fonts/100dpi/charM08.ttf".to_string(),
            ];

            #[cfg(target_os = "macos")]
            let extra_paths: Vec<String> = {
                let mut v = vec![
                    // System-bundled fonts that cover Dingbats + Symbols
                    "/System/Library/Fonts/AppleSymbols.ttf".to_string(),
                    "/System/Library/Fonts/SFNS.ttf".to_string(),
                    "/System/Library/Fonts/Helvetica.ttc".to_string(),
                    "/System/Library/Fonts/HelveticaNeue.ttc".to_string(),
                    "/System/Library/Fonts/ArialHB.ttc".to_string(),
                    "/System/Library/Fonts/Arial Unicode.ttf".to_string(),
                    "/Library/Fonts/Arial.ttf".to_string(),
                    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf".to_string(),
                    "/System/Library/Fonts/Supplemental/AppleGothic.ttf".to_string(),
                    "/System/Library/Fonts/Supplemental/DejaVuSans.ttf".to_string(),
                    "/System/Library/Fonts/Supplemental/NotoSansSymbols-Regular.ttf".to_string(),
                    "/System/Library/Fonts/Supplemental/NotoSansSymbols2-Regular.ttf".to_string(),
                    "/System/Library/Fonts/Supplemental/NotoColorEmoji.ttf".to_string(),
                ];
                // User-installed fonts
                if let Ok(home) = std::env::var("HOME") {
                    v.push(format!("{}/Library/Fonts/NotoSansSymbols2-Regular.ttf", home));
                    v.push(format!("{}/Library/Fonts/NotoSans-Regular.ttf", home));
                    v.push(format!("{}/Library/Fonts/JetBrainsMonoNerdFont-Regular.ttf", home));
                }
                v
            };

            #[cfg(target_os = "windows")]
            let extra_paths: Vec<String> = {
                let mut v = Vec::new();
                if let Ok(windir) = std::env::var("WINDIR") {
                    // Dingbats (U+2700-U+27BF), Symbols, Arrows
                    v.push(format!("{}\\Fonts\\seguisym.ttf", windir));
                    // Emoji + Symbol coverage
                    v.push(format!("{}\\Fonts\\seguiemj.ttf", windir));
                    // Standard fallbacks
                    v.push(format!("{}\\Fonts\\seguibl.ttf", windir));
                    v.push(format!("{}\\Fonts\\segoeui.ttf", windir));
                    v.push(format!("{}\\Fonts\\seguisb.ttf", windir));
                    v.push(format!("{}\\Fonts\\arial.ttf", windir));
                    v.push(format!("{}\\Fonts\\consola.ttf", windir));
                    v.push(format!("{}\\Fonts\\segmdl2.ttf", windir)); // Segoe MDL2 Assets (icons)
                }
                if let Ok(local) = std::env::var("LOCALAPPDATA") {
                    v.push(format!("{}\\Microsoft\\Windows\\Fonts\\seguiemj.ttf", local));
                }
                v
            };

            for path in extra_paths {
                if !loaded_paths.contains(&path) && Path::new(&path).exists() {
                    loaded_paths.insert(path.clone());
                    fallback_paths.push(path);
                }
            }

            fallback_paths
        });

        paths.iter()
            .filter(|p| *p != primary_path)
            .cloned()
            .collect()
    }

    fn rasterize_freetype_glyph_key(
        &mut self,
        _device: &Device,
        queue: &Queue,
        key: GlyphKey,
        glyph: &freetype::GlyphSlot,
        is_color: bool,
    ) -> anyhow::Result<()> {
        let bitmap = glyph.bitmap();
        let w = bitmap.width() as u32;
        let h = bitmap.rows() as u32;

        let pixel_mode = bitmap.pixel_mode().unwrap_or(freetype::bitmap::PixelMode::Gray);
        let is_emoji_char = match key {
            GlyphKey::Char(ch) => is_emoji(ch),
            GlyphKey::GlyphId(_) => false,
        };
        let is_block_char = match key {
            GlyphKey::Char(ch) => is_block_element(ch),
            GlyphKey::GlyphId(_) => false,
        };
        let actual_is_color = is_color || pixel_mode == freetype::bitmap::PixelMode::Bgra || is_emoji_char;

        if w == 0 || h == 0 {
            let entry = AtlasEntry {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
                left: 0.0,
                top: 0.0,
                is_color: actual_is_color,
                is_block: is_block_char,
            };
            self.entries.insert(key, entry);
            return Ok(());
        }

        // 1. Get raw RGBA data from FreeType bitmap
        let mut rgba_data = vec![0u8; (w * h * 4) as usize];
        let buffer = bitmap.buffer();
        let pitch = bitmap.pitch() as usize;

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

        // 2. Scale block elements to cell dimensions; keep other glyphs at their native FreeType size
        let (mut final_rgba, final_w, final_h, scale) = if is_block_char {
            let target_w = self.cell_width.round() as u32;
            let target_h = self.cell_height.round() as u32;
            let scaled = scale_rgba_bitmap(&rgba_data, w as usize, h as usize, target_w as usize, target_h as usize);
            (scaled, target_w, target_h, 1.0)
        } else {
            (rgba_data, w, h, 1.0)
        };

        if is_block_char {
            for pixel_chunk in final_rgba.chunks_mut(4) {
                if pixel_chunk[3] > 0 || pixel_chunk[0] > 0 {
                    pixel_chunk[0] = 255;
                    pixel_chunk[1] = 255;
                    pixel_chunk[2] = 255;
                    pixel_chunk[3] = 255;
                } else {
                    pixel_chunk[0] = 0;
                    pixel_chunk[1] = 0;
                    pixel_chunk[2] = 0;
                    pixel_chunk[3] = 0;
                }
            }
        }

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
                left: if is_block_char {
                    0.0
                } else {
                    (glyph.bitmap_left() as f32 * scale).round()
                },
                top: if is_block_char {
                    -self.ascent
                } else {
                    (-glyph.bitmap_top() as f32 * scale).round()
                },
                is_color: actual_is_color,
                is_block: is_block_char,
            };

            self.entries.insert(key, entry);

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

    pub fn rasterize_batch(&mut self, chars: &str, device: &Device, queue: &Queue) {
        let mut missing_chars = Vec::new();
        for c in chars.chars() {
            if let Some(entry) = self.entries.get(&GlyphKey::Char(c)) {
                if entry.width > 0.0 || entry.height > 0.0 {
                    continue;
                }
            }
            missing_chars.push(c);
        }

        if missing_chars.is_empty() {
            return;
        }

        let physical_size = self.font_size * self.scale_factor;

        FT_LIB.with(|lib| {
            // 1. Try primary font face
            if let Ok(face) = lib.new_face(&self.primary_path, 0) {
                let _ = face.set_pixel_sizes(0, physical_size as u32);
                let is_color_font = face.has_fixed_sizes();

                let mut still_missing = Vec::new();
                for c in missing_chars {
                    let is_color = is_color_font || is_emoji(c);
                    let load_flags = if is_color {
                        LoadFlag::RENDER | LoadFlag::COLOR
                    } else {
                        LoadFlag::RENDER
                    };

                    let mut found = false;
                    if let Some(idx) = face.get_char_index(c as usize) {
                        if idx != 0 {
                            if face.load_glyph(idx, load_flags).is_ok() {
                                let _ = self.rasterize_freetype_glyph_key(device, queue, GlyphKey::Char(c), &face.glyph(), is_color);
                                found = true;
                            }
                        }
                    }

                    if !found {
                        still_missing.push(c);
                    }
                }
                missing_chars = still_missing;
            }

            if missing_chars.is_empty() {
                return;
            }

            // 2. Try fallbacks
            let paths_to_try = self.fallback_paths.clone();
            for path in &paths_to_try {
                if missing_chars.is_empty() {
                    break;
                }
                if let Ok(face) = lib.new_face(path, 0) {
                    let is_color_font = face.has_fixed_sizes();
                    let mut still_missing = Vec::new();

                    for c in missing_chars {
                        let is_color = is_color_font || is_emoji(c);
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

                        let mut found = false;
                        if let Some(idx) = face.get_char_index(c as usize) {
                            if idx != 0 {
                                let load_flags = if is_color {
                                    LoadFlag::RENDER | LoadFlag::COLOR
                                } else {
                                    LoadFlag::RENDER
                                };
                                if face.load_glyph(idx, load_flags).is_ok() {
                                    let _ = self.rasterize_freetype_glyph_key(device, queue, GlyphKey::Char(c), &face.glyph(), is_color);
                                    found = true;
                                }
                            }
                        }

                        if !found {
                            still_missing.push(c);
                        }
                    }
                    missing_chars = still_missing;
                }
            }

            // For anything remaining, insert a zero-sized entry in entries
            for c in missing_chars {
                let dummy = AtlasEntry {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                    left: 0.0,
                    top: 0.0,
                    is_color: false,
                    is_block: false,
                };
                self.entries.insert(GlyphKey::Char(c), dummy);
            }
        });
    }

    pub fn get_or_rasterize(&mut self, c: char, device: &Device, queue: &Queue) -> Option<AtlasEntry> {
        if let Some(entry) = self.entries.get(&GlyphKey::Char(c)) {
            if entry.width > 0.0 || entry.height > 0.0 {
                return Some(*entry);
            }
            return None;
        }

        let entry = FT_LIB.with(|lib| {
            let cp = c as u32;
            let is_emoji_codepoint = cp >= 0x1F000 && cp <= 0x1FFFF;

            // 1. Try primary font — only accept non-empty bitmaps.
            //    Placeholder glyphs in text fonts return empty bitmaps for
            //    color emoji codepoints; fall through to fallbacks.
            if let Ok(face) = lib.new_face(&self.primary_path, 0) {
                let physical_size = self.font_size * self.scale_factor;
                let _ = face.set_pixel_sizes(0, physical_size as u32);
                if let Some(idx) = face.get_char_index(c as usize) {
                    if idx != 0 {
                        let is_color = face.has_fixed_sizes() || is_emoji(c);
                        let load_flags = if is_color {
                            LoadFlag::RENDER | LoadFlag::COLOR
                        } else {
                            LoadFlag::RENDER
                        };

                        if face.load_glyph(idx, load_flags).is_ok() {
                            let _ = self.rasterize_freetype_glyph_key(device, queue, GlyphKey::Char(c), &face.glyph(), is_color);
                            if let Some(entry) = self.entries.get(&GlyphKey::Char(c)) {
                                if entry.width > 0.0 && entry.height > 0.0 {
                                    return Some(*entry);
                                }
                            }
                        }
                    }
                }
            }

            // 2. Try fallbacks. For emoji codepoints, prioritize color-capable
            //    fonts (NotoColorEmoji and similar) so they are tried first.
            let mut paths_to_try: Vec<String> = self.fallback_paths.clone();
            if is_emoji_codepoint {
                paths_to_try.sort_by_key(|p| !Self::is_color_font_path(p));
            }

            for path in &paths_to_try {
                if let Ok(face) = lib.new_face(path, 0) {
                    let physical_size = self.font_size * self.scale_factor;
                    let is_color = face.has_fixed_sizes() || is_emoji(c);

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
                                let _ = self.rasterize_freetype_glyph_key(device, queue, GlyphKey::Char(c), &face.glyph(), is_color);
                                if let Some(entry) = self.entries.get(&GlyphKey::Char(c)) {
                                    if entry.width > 0.0 && entry.height > 0.0 {
                                        return Some(*entry);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            None
        });

        if let Some(entry) = entry {
            // Cache the entry so we don't re-rasterize
            self.entries.insert(GlyphKey::Char(c), entry);
            Some(entry)
        } else {
            // Cache a zero-sized entry to remember this glyph is missing.
            // Callers must check width/height before drawing — returns None
            // here so future lookups also skip without re-rasterizing.
            let dummy = AtlasEntry {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
                left: 0.0,
                top: 0.0,
                is_color: false,
                is_block: false,
            };
            self.entries.insert(GlyphKey::Char(c), dummy);
            None
        }
    }

    pub fn get_or_rasterize_glyph(
        &mut self,
        glyph_id: u32,
        device: &Device,
        queue: &Queue,
    ) -> Option<AtlasEntry> {
        if let Some(entry) = self.entries.get(&GlyphKey::GlyphId(glyph_id)) {
            return Some(*entry);
        }

        FT_LIB.with(|lib| {
            if let Ok(face) = lib.new_face(&self.primary_path, 0) {
                let physical_size = self.font_size * self.scale_factor;
                let _ = face.set_pixel_sizes(0, physical_size as u32);
                let is_color = face.has_fixed_sizes();
                let load_flags = if is_color {
                    LoadFlag::RENDER | LoadFlag::COLOR
                } else {
                    LoadFlag::RENDER
                };

                if face.load_glyph(glyph_id, load_flags).is_ok() {
                    let _ = self.rasterize_freetype_glyph_key(device, queue, GlyphKey::GlyphId(glyph_id), &face.glyph(), is_color);
                    return self.entries.get(&GlyphKey::GlyphId(glyph_id)).copied();
                }
            }
            None
        })
    }

    pub fn primary_path(&self) -> &str {
        &self.primary_path
    }

    #[allow(dead_code)]
    pub fn primary_font_bytes(&self) -> Option<&[u8]> {
        self.primary_font_bytes.as_deref()
    }

    pub fn hb_face(&self) -> Option<&rustybuzz::Face<'static>> {
        self.hb_face.as_ref()
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

    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    pub fn atlas_size(&self) -> (u32, u32) {
        (self.atlas_width, self.atlas_height)
    }

    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    pub fn texture(&self) -> &wgpu::Texture {
        self.texture.as_ref()
    }

    #[allow(dead_code)]
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
                is_block: false,
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

    #[allow(dead_code)]
    pub fn load_svg_icon(
        &mut self,
        path: &str,
        target_size: u32,
        queue: &Queue,
    ) -> anyhow::Result<AtlasEntry> {
        let svg_data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read SVG file at {:?}", path))?;
        
        // Convert currentColor to white for stencil rendering
        let svg_data = svg_data.replace("currentColor", "white");

        let opt = usvg::Options::default();
        let tree = Tree::from_str(&svg_data, &opt)
            .context("Failed to parse SVG data")?;

        let mut pixmap = tiny_skia::Pixmap::new(target_size, target_size)
            .context("Failed to create tiny-skia pixmap")?;

        // Fill with transparent
        pixmap.fill(tiny_skia::Color::TRANSPARENT);

        let size = tree.size();
        let scale_x = target_size as f32 / size.width();
        let scale_y = target_size as f32 / size.height();
        let scale = scale_x.min(scale_y);
        
        let transform = tiny_skia::Transform::from_scale(scale, scale);

        resvg::render(&tree, transform, &mut pixmap.as_mut());

        let rgba = pixmap.data();
        let w = target_size;
        let h = target_size;

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
                is_color: false, // We use it as stencil (is_color = false)
                is_block: false,
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
                rgba,
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
            anyhow::bail!("Failed to allocate space in atlas for SVG icon")
        }
    }

    #[allow(dead_code)]
    pub fn load_custom_image_from_memory(
        &mut self,
        bytes: &[u8],
        target_size: u32,
        queue: &Queue,
    ) -> anyhow::Result<AtlasEntry> {
        let img = image::load_from_memory(bytes).context("Failed to parse image from memory")?;
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
                is_block: false,
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

    pub fn load_raw_rgba_image(
        &mut self,
        rgba: &[u8],
        w: u32,
        h: u32,
        is_color: bool,
        queue: &Queue,
    ) -> anyhow::Result<AtlasEntry> {
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
                is_color,
                is_block: false,
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
                rgba,
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

    #[allow(dead_code)]
    pub fn load_svg_icon_from_memory(
        &mut self,
        svg_data: &str,
        target_size: u32,
        queue: &Queue,
    ) -> anyhow::Result<AtlasEntry> {
        let svg_data = svg_data.replace("currentColor", "white");

        let opt = usvg::Options::default();
        let tree = Tree::from_str(&svg_data, &opt)
            .context("Failed to parse SVG data")?;

        let mut pixmap = tiny_skia::Pixmap::new(target_size, target_size)
            .context("Failed to create tiny-skia pixmap")?;

        pixmap.fill(tiny_skia::Color::TRANSPARENT);

        let size = tree.size();
        let scale_x = target_size as f32 / size.width();
        let scale_y = target_size as f32 / size.height();
        let scale = scale_x.min(scale_y);
        
        let transform = tiny_skia::Transform::from_scale(scale, scale);

        resvg::render(&tree, transform, &mut pixmap.as_mut());

        let rgba = pixmap.data();
        let w = target_size;
        let h = target_size;

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
                is_color: false,
                is_block: false,
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
                rgba,
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
            anyhow::bail!("Failed to allocate space in atlas for SVG icon")
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

pub fn is_block_element(ch: char) -> bool {
    matches!(ch as u32,
        0x2580..=0x259F  // Block Elements: ▀▁▂▃▄▅▆▇█▉▊▋▌▍▎▏▐░▒▓
    )
}

pub fn is_emoji(ch: char) -> bool {
    matches!(ch as u32,
        0x1F300..=0x1F9FF |  // Misc symbols and pictographs
        0x1F000..=0x1F02F |  // Mahjong tiles
        0x1F0A0..=0x1F0FF |  // Playing cards
        0x1FA00..=0x1FA6F |  // Chess, other symbols
        0x2600..=0x26FF      // Misc symbols
    )
}