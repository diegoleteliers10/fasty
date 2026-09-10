use std::io::Cursor;
use std::path::Path;
use base64::prelude::*;
use image::GenericImageView;
use crate::ai::model::ContentPart;

const MAX_IMAGE_DIMENSION: u32 = 1568;

/// Loads an image from disk, downscales if necessary (max edge 1568px),
/// re-encodes to PNG/JPEG, and converts to a base64 `ContentPart::Image`.
pub fn load_and_prepare_image(path: &Path) -> Result<ContentPart, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("Failed to read image file '{}': {}", path.display(), e))?;

    prepare_image_from_bytes(&bytes, path)
}

/// Prepares an image from raw file bytes with path context for extension fallback.
pub fn prepare_image_from_bytes(bytes: &[u8], path: &Path) -> Result<ContentPart, String> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| format!("Failed to decode image from '{}': {}", path.display(), e))?;

    let (w, h) = img.dimensions();

    let resized = if w > MAX_IMAGE_DIMENSION || h > MAX_IMAGE_DIMENSION {
        let ratio = (MAX_IMAGE_DIMENSION as f32) / (w.max(h) as f32);
        let target_w = ((w as f32 * ratio).round() as u32).max(1);
        let target_h = ((h as f32 * ratio).round() as u32).max(1);
        img.resize(target_w, target_h, image::imageops::FilterType::Triangle)
    } else {
        img
    };

    let has_alpha = resized.color().has_alpha();
    let mut out_bytes = Vec::new();
    let media_type = if has_alpha {
        resized
            .write_to(&mut Cursor::new(&mut out_bytes), image::ImageFormat::Png)
            .map_err(|e| format!("Failed to encode image to PNG: {}", e))?;
        "image/png"
    } else {
        resized
            .write_to(&mut Cursor::new(&mut out_bytes), image::ImageFormat::Jpeg)
            .map_err(|e| format!("Failed to encode image to JPEG: {}", e))?;
        "image/jpeg"
    };

    let base64_data = BASE64_STANDARD.encode(&out_bytes);

    Ok(ContentPart::Image {
        media_type: media_type.to_string(),
        data: base64_data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn test_image_downscaling_and_encoding() {
        // Create an in-memory 2000x1000 image
        let mut img = RgbaImage::new(2000, 1000);
        for pixel in img.pixels_mut() {
            *pixel = Rgba([255, 0, 0, 255]);
        }

        let mut png_bytes = Vec::new();
        img.write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
            .unwrap();

        let part = prepare_image_from_bytes(&png_bytes, Path::new("test.png")).unwrap();
        match part {
            ContentPart::Image { media_type, data } => {
                assert_eq!(media_type, "image/png");
                assert!(!data.is_empty());
                // Verify decoding the base64 output
                let decoded = BASE64_STANDARD.decode(&data).unwrap();
                let loaded = image::load_from_memory(&decoded).unwrap();
                let (w, h) = loaded.dimensions();
                assert_eq!(w, 1568);
                assert_eq!(h, 784);
            }
            _ => panic!("Expected ContentPart::Image"),
        }
    }
}

