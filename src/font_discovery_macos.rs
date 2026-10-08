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
///
/// Only families whose glyphs share one advance width are returned. A
/// proportional font cannot be drawn on a terminal's fixed cell grid, so
/// offering it as a choice only produces corrupted text.
pub fn available_monospace_fonts() -> Vec<String> {
    filter_monospace(all_system_font_families())
}

/// Narrows any family list to the monospace ones, preserving its order.
pub fn filter_monospace(families: Vec<String>) -> Vec<String> {
    families
        .into_iter()
        .filter(|family| is_monospace_cached(family))
        .collect()
}

/// Advance width of a single glyph at 12pt, or `None` if it cannot be shaped.
fn glyph_advance(family: &str, ch: char) -> Option<f64> {
    let font = new_from_name(family, 12.0).ok()?;
    let codes = [ch as u16];
    let mut glyphs = [0u16];
    if !unsafe { font.get_glyphs_for_characters(codes.as_ptr(), glyphs.as_mut_ptr(), 1) }
        || glyphs[0] == 0
    {
        return None;
    }
    let mut advance = CGSize::default();
    unsafe {
        CTFontGetAdvancesForGlyphs(
            font.as_concrete_TypeRef(),
            0,
            glyphs.as_ptr(),
            &mut advance,
            1,
        );
    }
    (advance.width > 0.0).then_some(advance.width)
}

/// Whether common ASCII glyphs in `family` share one advance width.
pub fn is_monospace(family: &str) -> bool {
    let advances = ['i', 'W', 'm', '0', '@', '.']
        .into_iter()
        .map(|ch| glyph_advance(family, ch));
    let Some(advances) = advances.collect::<Option<Vec<_>>>() else {
        return false;
    };
    let Some(first) = advances.first() else {
        return false;
    };
    advances.iter().all(|advance| (advance - first).abs() < 0.01)
}

/// Memoised `is_monospace`. Each probe hits CoreText and the system exposes
/// hundreds of families, so the verdict is cached per family for the process.
pub fn is_monospace_cached(family: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(family).copied() {
        return cached;
    }
    let verdict = is_monospace(family);
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(family.to_string(), verdict);
    verdict
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

/// Monospace families to fall back to, in preference order. Every macOS install
/// ships at least one of these, so this always resolves to something real.
const FALLBACK_MONOSPACE: [&str; 3] = ["Menlo", "Monaco", "Courier New"];

/// Whether `family` names a font the system can actually load.
///
/// The generic `monospace` keyword is deliberately not treated as available:
/// callers substitute a concrete family for it before asking.
pub fn font_is_available(family: &str) -> bool {
    if family.is_empty() {
        return false;
    }
    // create_for_family is strict and returns None for an unknown family.
    // new_from_name is not: it happily returns a substituted font for any input,
    // so it cannot answer this question.
    create_for_family(family).is_some()
}

/// Resolves a configured family to one that is actually installed.
///
/// A family that cannot be loaded used to be passed straight through to the
/// text system, which silently substituted a fallback font while cell metrics
/// were still computed as if it were the requested monospace font. The grid then
/// allocated cells at one width and drew glyphs at another, so text overlapped
/// or scrambled with nothing on screen to explain why.
///
/// Returning a concrete installed family keeps measurement and rendering
/// describing the same font.
pub fn resolve_font_family(preferred: &str) -> String {
    if font_is_available(preferred) {
        return preferred.to_string();
    }
    for candidate in FALLBACK_MONOSPACE {
        if font_is_available(candidate) {
            return candidate.to_string();
        }
    }
    // Nothing installed even in the fallback list; let the text system decide.
    preferred.to_string()
}

/// Measure exact monospace character cell metrics (advance width and line height) for a font.
pub fn measure_font_metrics(family: &str, size: f32) -> (f32, f32) {
    // Measure whichever family will actually be used for painting, so the cell
    // geometry and the glyphs always describe the same font.
    let resolved = resolve_font_family(family);
    if let Ok(font) = new_from_name(&resolved, size as f64) {
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
    // Only reached when even the fallback list is unavailable.
    (size * 0.60, size * 1.32)
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_family_is_returned_unchanged() {
        // Menlo ships with macOS, so it must survive resolution untouched or the
        // fallback would quietly override a perfectly good configured font.
        assert_eq!(resolve_font_family("Menlo"), "Menlo");
    }

    #[test]
    fn missing_family_falls_back_to_an_installed_monospace() {
        let resolved = resolve_font_family("Definitely Not Installed 12345");
        assert!(
            font_is_available(&resolved),
            "fallback {resolved:?} must itself be installed"
        );
        assert!(
            FALLBACK_MONOSPACE.contains(&resolved.as_str()),
            "fallback {resolved:?} should come from the known monospace list"
        );
    }

    #[test]
    fn empty_family_is_not_treated_as_available() {
        // Callers substitute a concrete family for the empty case, so reporting
        // it as available here would suppress the fallback.
        assert!(!font_is_available(""));
    }

    #[test]
    fn generic_monospace_keyword_resolves_to_a_real_family() {
        // The default config stores "monospace"; it must not be handed to the
        // text system as-is.
        let resolved = resolve_font_family("monospace");
        assert!(
            font_is_available(&resolved),
            "expected a concrete installed family, got {resolved:?}"
        );
    }

    #[test]
    fn metrics_never_report_a_zero_or_tiny_advance() {
        // The pre-fix bug was metrics describing one font while another was
        // drawn. Assert the measurable invariant: whatever family is asked for,
        // the returned cell width matches a real installed font.
        for family in ["Menlo", "Definitely Not Installed 12345", ""] {
            let (w, h) = measure_font_metrics(family, 12.0);
            assert!(
                w >= 3.0 && h >= 5.0,
                "implausible metrics for {family:?}: {w}x{h}"
            );
        }
    }

    #[test]
    fn metrics_for_a_missing_family_equal_its_fallback() {
        // The invariant that matters: a family that cannot be loaded must
        // produce exactly the metrics of the family that will actually be drawn.
        // Previously the two paths each picked their own substitute, so cell
        // geometry could describe a different font than the one painted.
        for missing in ["Definitely Not Installed 12345", "monospace", ""] {
            let fallback = resolve_font_family(missing);
            assert!(
                font_is_available(&fallback),
                "{missing:?} resolved to uninstalled {fallback:?}"
            );
            assert_eq!(
                measure_font_metrics(missing, 12.0),
                measure_font_metrics(&fallback, 12.0),
                "{missing:?} metrics must match its fallback {fallback:?}"
            );
        }
    }

    #[test]
    fn metrics_are_monospace_proportional_to_size() {
        // Cell width must scale with the font size, otherwise the grid and the
        // glyphs drift apart at any size other than the one measured.
        let (small, _) = measure_font_metrics("Menlo", 12.0);
        let (large, _) = measure_font_metrics("Menlo", 24.0);
        assert!(
            (large / small - 2.0).abs() < 0.02,
            "expected doubling, got {small} -> {large}"
        );
    }
}



#[cfg(test)]
mod mono_tests {
    use super::*;

    #[test]
    fn monospace_families_are_detected() {
        // Menlo ships with macOS and is the codebase's own default.
        assert!(is_monospace("Menlo"), "Menlo must be recognised as monospace");
    }

    #[test]
    fn a_proportional_family_is_rejected() {
        // Arial ships with macOS and uses proportional ASCII advances.
        assert!(!is_monospace("Arial"), "Arial must not pass the terminal font filter");
    }

    #[test]
    fn unavailable_font_is_not_monospace() {
        assert!(!is_monospace("Definitely Not Installed 12345"));
    }

    #[test]
    fn monospace_verdicts_are_cached_and_stable() {
        let first = is_monospace_cached("Menlo");
        let second = is_monospace_cached("Menlo");
        assert_eq!(first, second, "repeated probes must agree");
        assert!(first);
    }

    #[test]
    fn available_monospace_fonts_excludes_proportional_ones() {
        let list = available_monospace_fonts();
        assert!(
            list.iter().any(|f| f == "Menlo"),
            "the default monospace family must survive the filter"
        );
        assert!(
            list.iter().all(|f| is_monospace_cached(f)),
            "every returned family must be monospace"
        );
        assert!(
            !list.iter().any(|f| f.eq_ignore_ascii_case("Arial")),
            "the proportional Arial family must not appear"
        );
    }

    #[test]
    fn filtering_is_a_subset_of_all_families() {
        let all = all_system_font_families();
        let mono = available_monospace_fonts();
        assert!(mono.len() <= all.len(), "filter must not grow the list");
        for family in &mono {
            assert!(all.contains(family), "{family:?} was not in the full list");
        }
    }
}
