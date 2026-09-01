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
        .take(30)
        .collect()
}

/// Check if a font family is a color/emoji font (has color glyph tables).
#[allow(dead_code)]
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

/// Query all available font family names from the system.
pub fn all_system_font_families() -> Vec<String> {
    extern "C" {
        fn CTFontManagerCopyAvailableFontFamilyNames() -> core_foundation::array::CFArrayRef;
    }
    unsafe {
        let array_ref = CTFontManagerCopyAvailableFontFamilyNames();
        if array_ref.is_null() {
            return vec!["Menlo".to_string(), "Monaco".to_string(), "Courier New".to_string()];
        }
        let array: core_foundation::array::CFArray<CFString> = core_foundation::array::CFArray::wrap_under_create_rule(array_ref);
        let mut families: Vec<String> = array.iter().map(|s| s.to_string()).filter(|s| !s.starts_with('.')).collect();
        families.sort_by_key(|a| a.to_lowercase());
        families.dedup();
        families
    }
}

/// Query the system for available monospace / coding fonts.
pub fn available_monospace_fonts() -> Vec<String> {
    all_system_font_families()
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

extern "C" {
    fn CTFontGetAdvancesForGlyphs(
        font: core_text::font::CTFontRef,
        orientation: u32,
        glyphs: *const u16,
        advances: *mut CGSize,
        count: isize,
    ) -> f64;
}

/// Measure exact monospace character cell metrics (advance width and line height) for a font.
pub fn measure_font_metrics(family: &str, size: f32) -> (f32, f32) {
    if let Ok(font) = new_from_name(family, size as f64) {
        let chars = ['0' as u16];
        let mut glyphs = [0u16];
        let ok = unsafe { font.get_glyphs_for_characters(chars.as_ptr(), glyphs.as_mut_ptr(), 1) };
        if ok && glyphs[0] != 0 {
            let mut advance = CGSize::default();
            unsafe {
                CTFontGetAdvancesForGlyphs(font.as_concrete_TypeRef(), 0, glyphs.as_ptr(), &mut advance, 1);
            }
            let advance_w = advance.width as f32;
            let line_h = (font.ascent() + font.descent() + font.leading()) as f32;
            if advance_w >= 3.0 && line_h >= 5.0 {
                return (advance_w, line_h.max(size * 1.25));
            }
        }
    }
    // Fallback if font name is not found
    (size * 0.60, size * 1.32)
}


