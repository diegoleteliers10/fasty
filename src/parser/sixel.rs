//! Sixel graphics decoder (DEC DCS `ESC P ... q ... ESC \`).
//!
//! Programs like `chafa --format sixel`, `lsix`, and older `vim` builds
//! draw inline images with this protocol. The decoder turns the sixel
//! stream into an RGBA image that lands in the same `ImageStore` the
//! kitty graphics path uses, so rendering and scrolling come for free.
//!
//! Scope notes:
//! - Raster attributes (`" pan;pad;Ph;Pv`) are parsed but the aspect ratio
//!   is ignored; the image renders at one pixel per sixel pixel.
//! - The cursor does not move after a sixel image, matching the kitty
//!   graphics behavior.
//! - The gateway cannot show sixel: FST1 snapshots carry cells only.

use std::sync::Arc;

use gpui::RenderImage;
use image::Frame;

/// Refuse absurd streams before they consume memory.
pub const MAX_SIXEL_BYTES: usize = 16 * 1024 * 1024;
const MAX_PIXELS: usize = 64 * 1024 * 1024 / 4;

/// True when a DCS payload introduces a sixel stream: parameters then `q`.
pub fn is_sixel(buf: &[u8]) -> bool {
    buf.contains(&b'q')
}

#[derive(Debug, Clone, Copy)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

/// Framebuffer that grows to the right and down as the stream draws.
#[derive(Default)]
struct Canvas {
    /// One RGBA row per pixel row; unused pixels are fully transparent.
    rows: Vec<Vec<u8>>,
    width: usize,
    height: usize,
}

impl Canvas {
    fn ensure(&mut self, x: usize, y: usize) {
        if x + 1 > self.width {
            self.width = x + 1;
        }
        if y + 1 > self.height {
            self.height = y + 1;
        }
        self.rows.resize_with(self.height, Vec::new);
        for row in &mut self.rows {
            row.resize(self.width * 4, 0);
        }
    }

    fn put(&mut self, x: usize, y: usize, color: Rgb) {
        self.ensure(x, y);
        let row = &mut self.rows[y];
        let offset = x * 4;
        row[offset] = color.r;
        row[offset + 1] = color.g;
        row[offset + 2] = color.b;
        row[offset + 3] = 255;
    }
}

fn hls_to_rgb(hue: u32, lightness: u32, saturation: u32) -> Rgb {
    // Sixel HLS runs 0..=100 on all three axes; hue is a fraction of 360
    // degrees. The standard HSL-to-RGB formulas apply with the usual
    // scaling.
    let h = (f32::from(hue as u16) / 100.0 * 360.0).rem_euclid(360.0) / 60.0;
    let l = f32::from(lightness as u16) / 100.0;
    let s = f32::from(saturation as u16) / 100.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Rgb {
        r: ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        g: ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        b: ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    }
}

fn percent_to_rgb(r: u32, g: u32, b: u32) -> Rgb {
    Rgb {
        r: (r.min(100) * 255 / 100) as u8,
        g: (g.min(100) * 255 / 100) as u8,
        b: (b.min(100) * 255 / 100) as u8,
    }
}

struct SixelParser<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> SixelParser<'a> {
    fn number(&mut self) -> u32 {
        let mut value: u32 = 0;
        while let Some(&byte) = self.data.get(self.pos) {
            if !byte.is_ascii_digit() {
                break;
            }
            self.pos += 1;
            value = value.saturating_mul(10).saturating_add(u32::from(byte - b'0'));
        }
        value
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }
}

/// Decode a full DCS payload (`params;params;qdata`). Returns the image in
/// BGRA form, ready for `RenderImage`, plus its pixel size.
pub fn decode(buf: &[u8]) -> anyhow::Result<(Arc<RenderImage>, u32, u32)> {
    if buf.len() > MAX_SIXEL_BYTES {
        anyhow::bail!("sixel payload too large");
    }
    let Some(q_pos) = buf.iter().position(|&byte| byte == b'q') else {
        anyhow::bail!("missing sixel introducer");
    };
    let mut parser = SixelParser {
        data: buf,
        pos: q_pos + 1,
    };

    // Every color register starts white; programs that care define their
    // own colors before drawing.
    let mut palette = [Rgb { r: 255, g: 255, b: 255 }; 256];
    let mut current = palette[0];
    let mut canvas = Canvas::default();
    let mut x = 0usize;
    let mut band = 0usize;

    while let Some(byte) = parser.peek() {
        parser.pos += 1;
        match byte {
            b'"' => {
                // Raster attributes: aspect;aspect;width;height. Parsed and
                // ignored: one pixel per sixel pixel.
                let _pan = parser.number();
                if parser.peek() == Some(b';') {
                    parser.pos += 1;
                    let _pad = parser.number();
                    if parser.peek() == Some(b';') {
                        parser.pos += 1;
                        let _width = parser.number();
                        if parser.peek() == Some(b';') {
                            parser.pos += 1;
                            let _height = parser.number();
                        }
                    }
                }
            }
            b'#' => {
                let register = parser.number().min(255) as usize;
                if parser.peek() == Some(b';') {
                    parser.pos += 1;
                    let kind = parser.number();
                    if parser.peek() == Some(b';') {
                        parser.pos += 1;
                        let a = parser.number();
                        if parser.peek() == Some(b';') {
                            parser.pos += 1;
                            let b = parser.number();
                            if parser.peek() == Some(b';') {
                                parser.pos += 1;
                                let c = parser.number();
                                palette[register] = if kind == 1 {
                                    hls_to_rgb(a, b, c)
                                } else {
                                    percent_to_rgb(a, b, c)
                                };
                            }
                        }
                    }
                }
                current = palette[register];
            }
            b'!' => {
                let count = parser.number().max(1) as usize;
                let Some(glyph) = parser.peek() else { break };
                parser.pos += 1;
                draw_glyph(&mut canvas, &mut x, band, glyph, count, current);
            }
            b'$' => x = 0,
            b'-' => {
                band += 1;
                x = 0;
            }
            0x20..=0x7E => {
                draw_glyph(&mut canvas, &mut x, band, byte, 1, current);
            }
            _ => {
                // Anything outside printable range (including ESC, which
                // the checkpoint strips) ends the parse.
                break;
            }
        }
        if canvas.width * canvas.height > MAX_PIXELS {
            anyhow::bail!("sixel image too large");
        }
    }

    if canvas.width == 0 || canvas.height == 0 {
        anyhow::bail!("empty sixel image");
    }
    let (w, h) = (canvas.width as u32, canvas.height as u32);
    let mut bgra = Vec::with_capacity(canvas.rows.len() * canvas.width * 4);
    for row in &canvas.rows {
        bgra.extend_from_slice(row);
    }
    // GPUI expects BGRA channel ordering in `RenderImage`.
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let frame = Frame::new(
        image::RgbaImage::from_raw(w, h, bgra).ok_or_else(|| anyhow::anyhow!("bad sixel size"))?,
    );
    Ok((Arc::new(RenderImage::new(smallvec::smallvec![frame])), w, h))
}

/// Draw one sixel glyph `count` times. Each glyph carries six vertical
/// pixels; bit 0 is the top row of the band.
fn draw_glyph(
    canvas: &mut Canvas,
    x: &mut usize,
    band: usize,
    glyph: u8,
    count: usize,
    color: Rgb,
) {
    if !(0x3F..=0x7E).contains(&glyph) {
        return;
    }
    let value = glyph - 0x3F;
    for _ in 0..count {
        for bit in 0..6u8 {
            if value & (1 << bit) != 0 {
                canvas.put(*x, band * 6 + bit as usize, color);
            }
        }
        *x += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_payloads_without_an_introducer() {
        assert!(!is_sixel(b"\x1bP1;1;0#0;2;0;0;0"));
        assert!(decode(b"no-introducer").is_err());
    }

    #[test]
    fn draws_one_glyph_with_the_right_pixels() {
        // '@' is 0x40 → value 0b000001 → only the top pixel of the band.
        let (image, w, h) = decode(b"0;0;0q@").expect("decodes");
        assert_eq!((w, h), (1, 1));
        let bytes = image.as_bytes(0).expect("frame bytes");
        // BGRA, opaque.
        assert_eq!([bytes[0], bytes[1], bytes[2], bytes[3]], [255, 255, 255, 255]);
    }

    #[test]
    fn a_zero_glyph_draws_nothing() {
        // '?' is 0x3F → value 0 → no bits set → no image at all.
        assert!(decode(b"0;0;0q?").is_err());
    }

    #[test]
    fn color_register_and_repeat_apply() {
        // Define color 1 as red (kind 2, RGB percents), then draw '!' 2×
        // ('~' = 0x7E → all six pixels of the band).
        let (image, w, h) = decode(b"0;0;0q#1;2;100;0;0!2~").expect("decodes");
        assert_eq!((w, h), (2, 6));
        let bytes = image.as_bytes(0).expect("frame bytes");
        // Every pixel is red, BGRA order.
        for pixel in bytes.chunks_exact(4) {
            assert_eq!([pixel[0], pixel[1], pixel[2], pixel[3]], [0, 0, 255, 255]);
        }
    }

    #[test]
    fn newline_and_carriage_return_shape_the_image() {
        // First band draws one pixel at x=0 ('@'), then '-' drops a band;
        // the second '@' draws at x=0 again.
        let (image, w, h) = decode(b"0;0;0q@-@").expect("decodes");
        assert_eq!((w, h), (1, 7));
        let bytes = image.as_bytes(0).expect("frame bytes");
        // Row 0 opaque, rows 1..=5 transparent, row 6 opaque.
        assert_eq!(bytes[3], 255);
        assert_eq!(bytes[4 * 2 + 3], 0);
        assert_eq!(bytes[6 * 4 + 3], 255);
    }

    #[test]
    fn hls_conversion_hits_known_colors() {
        // Saturation zero is neutral gray at any lightness below white.
        let gray = hls_to_rgb(0, 50, 0);
        assert_eq!((gray.r, gray.g, gray.b), (128, 128, 128));
        let red = hls_to_rgb(0, 50, 100);
        assert_eq!((red.r, red.g, red.b), (255, 0, 0));
        let white = hls_to_rgb(0, 100, 100);
        assert_eq!((white.r, white.g, white.b), (255, 255, 255));
    }
}
