//! macOS font discovery via CoreText.
//!
//! Resolves font family names to file paths using the system's native font
//! database, replacing hardcoded paths that break across macOS versions.

use std::path::PathBuf;

use core_foundation::base::{CFType, TCFType};
use core_foundation::number::{CFNumber, CFNumberRef};
use core_foundation::string::CFString;
use core_text::font::{cascade_list_for_languages as ct_cascade_list_for_languages, new_from_name};
use core_text::font_collection::create_for_family;
use core_text::font_descriptor::{self, kCTFontEnabledAttribute, CTFontDescriptor};

/// Resolve a font family name to its file path on disk.
///
/// Returns `None` if the family cannot be found or has no on-disk file
/// (e.g. in-memory system fonts like `.AppleSymbolsFB`).
pub fn resolve_font_path(family: &str) -> Option<PathBuf> {
    let collection = create_for_family(family)?;
    let descriptors = collection.get_descriptors()?;

    for desc in descriptors.iter() {
        if let Some(path) = desc.font_path() {
            if !path.as_os_str().is_empty() {
                return Some(path);
            }
        }
    }

    None
}

/// Get the system's default font cascade list as file paths.
///
/// This is the order macOS uses for fallback glyph resolution. Color/emoji
/// fonts like `Apple Color Emoji` appear in their system-determined position
/// rather than being hardcoded at the end.
pub fn cascade_list() -> Vec<PathBuf> {
    let font = match new_from_name("Menlo", 12.0) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let languages = vec![CFString::new("en")];
    let langarr = core_foundation::array::CFArray::from_CFTypes(&languages);

    let list = ct_cascade_list_for_languages(&font, &langarr);

    list.into_iter()
        .filter(|desc| is_enabled(desc))
        .filter_map(|desc| desc.font_path())
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
}

/// Check if a font family is a color/emoji font (has color glyph tables).
pub fn is_color_font(family: &str) -> bool {
    use core_text::font_descriptor::kCTFontColorGlyphsTrait;

    let font = match new_from_name(family, 12.0) {
        Ok(f) => f,
        Err(_) => return false,
    };

    (font.symbolic_traits() & kCTFontColorGlyphsTrait) != 0
}

/// Check if a font descriptor is enabled (not a synthetic/disabled fallback).
fn is_enabled(fontdesc: &core_foundation::base::ItemRef<'_, CTFontDescriptor>) -> bool {
    unsafe {
        let descriptor = fontdesc.as_concrete_TypeRef();
        let attr_val =
            font_descriptor::CTFontDescriptorCopyAttribute(descriptor, kCTFontEnabledAttribute);

        if attr_val.is_null() {
            return false;
        }

        let attr_val = CFType::wrap_under_create_rule(attr_val);
        let attr_val = CFNumber::wrap_under_get_rule(attr_val.as_CFTypeRef() as CFNumberRef);

        attr_val.to_i32().unwrap_or(0) != 0
    }
}
