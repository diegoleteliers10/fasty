use std::sync::Arc;
use gpui::RenderImage;
use image::{Frame, RgbaImage};
use smallvec::smallvec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyAction {
    TransmitAndDisplay, // 't' (default)
    ImmediateDisplay,   // 'T'
    Query,              // 'q'
    Put,                // 'p'
    Delete,             // 'd'
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyFormat {
    Rgba32, // 32 (default)
    Rgb24,  // 24
    Png,    // 100
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyTransmission {
    Direct,    // 'd' (default)
    File,      // 'f'
    TempFile,  // 't'
    SharedMem, // 's'
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyDeleteAction {
    All,
    ById(u32),
    ByPlacement(u32),
    AtCursor,
}

#[derive(Debug, Clone)]
pub struct KittyControl {
    pub action: KittyAction,
    pub format: KittyFormat,
    pub transmission: KittyTransmission,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    pub image_id: Option<u32>,
    pub image_number: Option<u32>,
    pub placement_id: Option<u32>,
    pub columns: Option<usize>,
    pub rows: Option<usize>,
    pub z_index: i32,
    pub quiet: u8,
    pub more_chunks: bool,
    pub cursor_movement: bool,
    pub delete_action: Option<KittyDeleteAction>,
}

impl Default for KittyControl {
    fn default() -> Self {
        Self {
            action: KittyAction::TransmitAndDisplay,
            format: KittyFormat::Rgba32,
            transmission: KittyTransmission::Direct,
            width_px: None,
            height_px: None,
            image_id: None,
            image_number: None,
            placement_id: None,
            columns: None,
            rows: None,
            z_index: 0,
            quiet: 0,
            more_chunks: false,
            cursor_movement: true,
            delete_action: None,
        }
    }
}

pub fn parse_kitty_control(header: &[u8]) -> Option<KittyControl> {
    let mut ctrl = KittyControl::default();
    let header_str = std::str::from_utf8(header).ok()?;

    for item in header_str.split(',') {
        if item.is_empty() {
            continue;
        }
        let mut parts = item.splitn(2, '=');
        let key = parts.next()?.trim();
        let val = parts.next().unwrap_or("").trim();

        match key {
            "a" => {
                ctrl.action = match val {
                    "t" => KittyAction::TransmitAndDisplay,
                    "T" => KittyAction::ImmediateDisplay,
                    "q" => KittyAction::Query,
                    "p" => KittyAction::Put,
                    "d" => KittyAction::Delete,
                    _ => KittyAction::TransmitAndDisplay,
                };
            }
            "f" => {
                ctrl.format = match val {
                    "100" => KittyFormat::Png,
                    "24" => KittyFormat::Rgb24,
                    "32" | _ => KittyFormat::Rgba32,
                };
            }
            "t" => {
                ctrl.transmission = match val {
                    "f" => KittyTransmission::File,
                    "t" => KittyTransmission::TempFile,
                    "s" => KittyTransmission::SharedMem,
                    "d" | _ => KittyTransmission::Direct,
                };
            }
            "s" => ctrl.width_px = val.parse::<u32>().ok(),
            "v" => ctrl.height_px = val.parse::<u32>().ok(),
            "i" => ctrl.image_id = val.parse::<u32>().ok(),
            "I" => ctrl.image_number = val.parse::<u32>().ok(),
            "p" => ctrl.placement_id = val.parse::<u32>().ok(),
            "c" => ctrl.columns = val.parse::<usize>().ok(),
            "r" => ctrl.rows = val.parse::<usize>().ok(),
            "z" => ctrl.z_index = val.parse::<i32>().ok().unwrap_or(0),
            "q" => ctrl.quiet = val.parse::<u8>().ok().unwrap_or(0),
            "m" => ctrl.more_chunks = val == "1",
            "C" => ctrl.cursor_movement = val != "0",
            "d" => {
                ctrl.action = KittyAction::Delete;
                ctrl.delete_action = match val {
                    "a" | "A" => Some(KittyDeleteAction::All),
                    "i" | "I" => ctrl.image_id.map(KittyDeleteAction::ById),
                    "p" | "P" => ctrl.placement_id.map(KittyDeleteAction::ByPlacement),
                    "c" | "C" => Some(KittyDeleteAction::AtCursor),
                    _ => Some(KittyDeleteAction::All),
                };
            }
            _ => {}
        }
    }

    Some(ctrl)
}

/// Decode base64 bytes ignoring ASCII whitespace.
pub fn base64_decode_bytes(input: &[u8]) -> Option<Vec<u8>> {
    let mut table = [255u8; 256];
    for (i, &c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
        table[c as usize] = i as u8;
    }

    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;

    for &b in input {
        if b == b'=' {
            break;
        }
        let val = table[b as usize];
        if val == 255 {
            continue; // Ignore whitespace / invalid characters
        }
        buffer = (buffer << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

/// Decodes image bytes into a GPUI-compatible `RenderImage` (BGRA ordered).
pub fn decode_image_data(
    format: KittyFormat,
    width_px: Option<u32>,
    height_px: Option<u32>,
    raw_bytes: &[u8],
) -> anyhow::Result<(Arc<RenderImage>, u32, u32)> {
    if raw_bytes.is_empty() {
        anyhow::bail!("empty image payload");
    }

    // Try universal auto-detection via `image::load_from_memory` first (supports PNG, JPEG, WebP, GIF, BMP)
    let (mut rgba_img, w, h) = if let Ok(img) = image::load_from_memory(raw_bytes) {
        let w = img.width();
        let h = img.height();
        (img.to_rgba8(), w, h)
    } else {
        match format {
            KittyFormat::Png => {
                let img = image::load_from_memory(raw_bytes)?;
                let w = img.width();
                let h = img.height();
                (img.to_rgba8(), w, h)
            }
            KittyFormat::Rgba32 => {
                let w = width_px.unwrap_or(0);
                let h = height_px.unwrap_or(0);
                if w == 0 || h == 0 || (raw_bytes.len() as u32) < w * h * 4 {
                    anyhow::bail!("invalid RGBA32 dimensions or insufficient bytes");
                }
                let img = RgbaImage::from_raw(w, h, raw_bytes[..(w * h * 4) as usize].to_vec())
                    .ok_or_else(|| anyhow::anyhow!("failed to create RgbaImage from raw RGBA32 bytes"))?;
                (img, w, h)
            }
            KittyFormat::Rgb24 => {
                let w = width_px.unwrap_or(0);
                let h = height_px.unwrap_or(0);
                if w == 0 || h == 0 || (raw_bytes.len() as u32) < w * h * 3 {
                    anyhow::bail!("invalid RGB24 dimensions or insufficient bytes");
                }
                let mut rgba_bytes = Vec::with_capacity((w * h * 4) as usize);
                for chunk in raw_bytes[..(w * h * 3) as usize].chunks_exact(3) {
                    rgba_bytes.push(chunk[0]);
                    rgba_bytes.push(chunk[1]);
                    rgba_bytes.push(chunk[2]);
                    rgba_bytes.push(255);
                }
                let img = RgbaImage::from_raw(w, h, rgba_bytes)
                    .ok_or_else(|| anyhow::anyhow!("failed to create RgbaImage from raw RGB24 bytes"))?;
                (img, w, h)
            }
        }
    };

    // GPUI expects BGRA channel ordering in `RenderImage`
    for pixel in rgba_img.chunks_exact_mut(4) {
        pixel.swap(0, 2); // Swap Red (0) and Blue (2)
    }

    let frame = Frame::new(rgba_img);
    let render_image = Arc::new(RenderImage::new(smallvec![frame]));
    Ok((render_image, w, h))
}

#[derive(Debug, Default)]
pub struct KittyReassembler {
    pub pending_control: Option<KittyControl>,
    pub chunks_buffer: Vec<u8>,
}

impl KittyReassembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_chunk(&mut self, control: KittyControl, payload_b64: &[u8]) -> Option<(KittyControl, Vec<u8>)> {
        if self.pending_control.is_none() {
            self.pending_control = Some(control.clone());
            self.chunks_buffer.clear();
        }

        self.chunks_buffer.extend_from_slice(payload_b64);

        if !control.more_chunks {
            let mut final_control = self.pending_control.take().unwrap_or(control);
            final_control.more_chunks = false;
            let payload = std::mem::take(&mut self.chunks_buffer);
            let decoded = base64_decode_bytes(&payload).unwrap_or_default();
            Some((final_control, decoded))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_kitty_control_keys() {
        let header = b"a=t,f=100,s=100,v=200,i=42,c=10,r=5,z=1,m=0";
        let ctrl = parse_kitty_control(header).unwrap();
        assert_eq!(ctrl.action, KittyAction::TransmitAndDisplay);
        assert_eq!(ctrl.format, KittyFormat::Png);
        assert_eq!(ctrl.width_px, Some(100));
        assert_eq!(ctrl.height_px, Some(200));
        assert_eq!(ctrl.image_id, Some(42));
        assert_eq!(ctrl.columns, Some(10));
        assert_eq!(ctrl.rows, Some(5));
        assert_eq!(ctrl.z_index, 1);
        assert_eq!(ctrl.more_chunks, false);
    }

    #[test]
    fn test_parse_kitty_query_and_delete() {
        let q = parse_kitty_control(b"a=q,i=12").unwrap();
        assert_eq!(q.action, KittyAction::Query);
        assert_eq!(q.image_id, Some(12));

        let d = parse_kitty_control(b"a=d,d=a").unwrap();
        assert_eq!(d.action, KittyAction::Delete);
        assert_eq!(d.delete_action, Some(KittyDeleteAction::All));
    }

    #[test]
    fn test_kitty_reassembler_multi_chunk() {
        let mut reassembler = KittyReassembler::new();

        let mut c1 = KittyControl::default();
        c1.more_chunks = true;
        let p1 = b"SGVsbG8g"; // "Hello "

        let mut c2 = KittyControl::default();
        c2.more_chunks = false;
        let p2 = b"V29ybGQ="; // "World"

        assert!(reassembler.push_chunk(c1, p1).is_none());
        let (final_ctrl, decoded) = reassembler.push_chunk(c2, p2).unwrap();
        assert!(!final_ctrl.more_chunks);
        assert_eq!(String::from_utf8(decoded).unwrap(), "Hello World");
    }

    #[test]
    fn test_base64_decode_bytes() {
        let input = b"QUJDREVGRw==";
        let decoded = base64_decode_bytes(input).unwrap();
        assert_eq!(decoded, b"ABCDEFG");
    }

    #[test]
    fn test_decode_rgb24_and_rgba32() {
        let raw_rgb = vec![255, 0, 0, 0, 255, 0]; // 2 pixels: Red, Green
        let res = decode_image_data(KittyFormat::Rgb24, Some(2), Some(1), &raw_rgb).unwrap();
        assert_eq!(res.1, 2);
        assert_eq!(res.2, 1);

        let raw_rgba = vec![255, 0, 0, 255, 0, 255, 0, 255]; // 2 pixels RGBA
        let res_rgba = decode_image_data(KittyFormat::Rgba32, Some(2), Some(1), &raw_rgba).unwrap();
        assert_eq!(res_rgba.1, 2);
        assert_eq!(res_rgba.2, 1);
    }
}
