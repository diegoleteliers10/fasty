use gpui::*;
use icons::common::{IconType, StaticSvgElement, icon_registry_getter::get_icon_elements};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::OnceLock;

static ICON_CACHE: OnceLock<Mutex<HashMap<IconType, &'static [u8]>>> = OnceLock::new();

fn get_cache() -> &'static Mutex<HashMap<IconType, &'static [u8]>> {
    ICON_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn get_icon_svg_bytes(icon: IconType) -> &'static [u8] {
    let mut cache = get_cache().lock();
    if let Some(&bytes) = cache.get(&icon) {
        return bytes;
    }

    let elements = get_icon_elements(icon).unwrap_or(&[]);
    let mut s = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"#
    );
    for el in elements {
        match el {
            StaticSvgElement::Path { d } => {
                s.push_str(&format!(r#"<path d="{d}"/>"#));
            }
            StaticSvgElement::Circle { cx, cy, r } => {
                s.push_str(&format!(r#"<circle cx="{cx}" cy="{cy}" r="{r}"/>"#));
            }
            StaticSvgElement::Rect { x, y, width, height, rx, ry } => {
                let rx_str = rx.map(|v| format!(r#" rx="{v}""#)).unwrap_or_default();
                let ry_str = ry.map(|v| format!(r#" ry="{v}""#)).unwrap_or_default();
                s.push_str(&format!(r#"<rect x="{x}" y="{y}" width="{width}" height="{height}"{rx_str}{ry_str}/>"#));
            }
            StaticSvgElement::Line { x1, y1, x2, y2 } => {
                s.push_str(&format!(r#"<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}"/>"#));
            }
            StaticSvgElement::Polyline { points } => {
                s.push_str(&format!(r#"<polyline points="{points}"/>"#));
            }
            StaticSvgElement::Polygon { points } => {
                s.push_str(&format!(r#"<polygon points="{points}"/>"#));
            }
            StaticSvgElement::Ellipse { cx, cy, rx, ry } => {
                s.push_str(&format!(r#"<ellipse cx="{cx}" cy="{cy}" rx="{rx}" ry="{ry}"/>"#));
            }
        }
    }
    s.push_str("</svg>");
    let leaked: &'static [u8] = Box::leak(s.into_bytes().into_boxed_slice());
    cache.insert(icon, leaked);
    leaked
}

pub fn render_icon(icon: IconType, color: Hsla, size_px: f32) -> impl IntoElement {
    let bytes = get_icon_svg_bytes(icon);
    svg()
        .data(bytes)
        .text_color(color)
        .w(px(size_px))
        .h(px(size_px))
        .flex_shrink_0()
}

pub fn get_app_logo_image() -> std::sync::Arc<RenderImage> {
    static LOGO: OnceLock<std::sync::Arc<RenderImage>> = OnceLock::new();
    LOGO.get_or_init(|| {
        let bytes = include_bytes!("../../assets/fasttySmallIcon.png");
        let decoded = image::load_from_memory(bytes).expect("failed to decode fasttySmallIcon.png");
        let mut data = decoded.into_rgba8();
        for pixel in data.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let frame = image::Frame::new(data);
        std::sync::Arc::new(RenderImage::new(smallvec::smallvec![frame]))
    })
    .clone()
}

pub fn render_app_logo(size_px: f32) -> impl IntoElement {
    img(get_app_logo_image())
        .w(px(size_px))
        .h(px(size_px))
        .flex_shrink_0()
}

pub fn render_sidebar_icon(color: Hsla, size_px: f32) -> impl IntoElement {
    svg()
        .data(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/></svg>"#
        )
        .text_color(color)
        .w(px(size_px))
        .h(px(size_px))
        .flex_shrink_0()
}

pub fn render_spinner(color: Hsla, size_px: f32) -> impl IntoElement {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let frame = ((now_ms / 100) % 8) as usize;

    static SPINNER_FRAMES: [&[u8]; 8] = [
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M12 3a9 9 0 0 1 9 9"/></svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M18.36 5.64a9 9 0 0 1 2.64 8.72"/></svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M21 12a9 9 0 0 1-9 9"/></svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M18.36 18.36a9 9 0 0 1-8.72 2.64"/></svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M12 21a9 9 0 0 1-9-9"/></svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M5.64 18.36a9 9 0 0 1-2.64-8.72"/></svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M3 12a9 9 0 0 1 9-9"/></svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M5.64 5.64a9 9 0 0 1 8.72-2.64"/></svg>"#,
    ];

    svg()
        .data(SPINNER_FRAMES[frame])
        .text_color(color)
        .w(px(size_px))
        .h(px(size_px))
        .flex_shrink_0()
}

pub fn get_deck_process_icon(process_name: &str) -> (IconType, &'static str) {
    let lower = process_name.to_lowercase();
    let base = std::path::Path::new(&lower)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&lower);

    match base {
        "nvim" | "vim" | "vi" | "nano" | "helix" | "hx" | "emacs" => (IconType::FileCode, "edit"),
        "node" | "bun" | "deno" | "npm" | "pnpm" | "yarn" | "npx" | "ts-node" => (IconType::Code, "js"),
        "cargo" | "rustc" | "rustup" => (IconType::Layers, "rust"),
        "python" | "python3" | "py" | "ipython" | "uv" | "pip" => (IconType::Terminal, "py"),
        "git" | "gh" | "lazygit" | "gitui" => (IconType::GitBranch, "git"),
        "docker" | "docker-compose" | "podman" | "kubectl" | "k9s" | "helm" => (IconType::Server, "ops"),
        "ssh" => (IconType::Server, "ssh"),
        "go" => (IconType::Code, "go"),
        "ruby" | "irb" | "rails" => (IconType::Code, "rb"),
        "htop" | "btop" | "top" => (IconType::Cpu, "sys"),
        "zsh" | "bash" | "fish" | "sh" => (IconType::Terminal, "sh"),
        _ => (IconType::Terminal, "term"),
    }
}


