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


