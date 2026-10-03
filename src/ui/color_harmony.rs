//! Automatic contrast correction in Oklab space.
//!
//! Terminal programs bias to dark mode: TUIs request explicit colors
//! (truecolor `Spec` or 256-color cube/grayscale indexes) tuned for dark
//! backgrounds, and those same values turn invisible on light themes — and
//! vice versa. When an explicit color's WCAG contrast against what it sits
//! on is too low, we keep its Oklab hue/chroma and binary-search the
//! lightness channel until the pair reads. Theme-owned named ANSI colors
//! are never touched. Inspired by Superlogical's Automatic Contrast
//! Correction (docs/plans/ideas-superlogical.md item 13).

use std::collections::HashMap;
use std::sync::Mutex;

/// Minimum contrast ratio before a color is left alone (WCAG "large text"
/// AA level; terminal glyphs are chunky enough for it).
const LEAVE_UNTOUCHED_RATIO: f32 = 3.0;
/// Ratio the correction aims for once it intervenes.
const TARGET_RATIO: f32 = 4.5;
/// Ratio targeted for background corrections (large areas, less critical).
const TARGET_RATIO_BG: f32 = 3.0;

/// Oklab lightness bounds used by the search, away from the extremes so
/// the result stays a color and not black/white mush.
const L_MIN: f32 = 0.08;
const L_MAX: f32 = 0.97;

type Rgb = (u8, u8, u8);

// Correction inputs are (color, other-color) pairs from a small practical
// set, but the render loop asks per cell per frame: memoize packed results.
// Key: color << 18 | other, value: packed corrected rgb.
static FG_CACHE: Mutex<Option<HashMap<u32, u32>>> = Mutex::new(None);
static BG_CACHE: Mutex<Option<HashMap<u32, u32>>> = Mutex::new(None);
const CACHE_CLEAR_AT: usize = 8192;

fn cache_get(cache: &Mutex<Option<HashMap<u32, u32>>>, key: u32) -> Option<Rgb> {
    let guard = cache.lock().unwrap();
    let packed = guard.as_ref()?.get(&key)?;
    Some(unpack_rgb(*packed))
}

fn cache_put(cache: &Mutex<Option<HashMap<u32, u32>>>, key: u32, value: Rgb) {
    let mut guard = cache.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    if map.len() >= CACHE_CLEAR_AT {
        map.clear();
    }
    map.insert(key, pack_rgb(value));
}

fn pack_rgb(c: Rgb) -> u32 {
    ((c.0 as u32) << 16) | ((c.1 as u32) << 8) | c.2 as u32
}

fn unpack_rgb(p: u32) -> Rgb {
    (((p >> 16) & 0xff) as u8, ((p >> 8) & 0xff) as u8, (p & 0xff) as u8)
}

fn pair_key(color: Rgb, other: Rgb) -> u32 {
    (pack_rgb(color) << 12) | (pack_rgb(other) >> 12)
}

/// Corrects an explicit foreground color against the background it sits on.
/// Returns the input unchanged when the pair already contrasts enough.
pub fn correct_fg(color: Rgb, bg: Rgb) -> Rgb {
    let key = pair_key(color, bg);
    if let Some(hit) = cache_get(&FG_CACHE, key) {
        return hit;
    }
    let corrected = correct_against(color, bg, TARGET_RATIO);
    cache_put(&FG_CACHE, key, corrected);
    corrected
}

/// Corrects an explicit background color against the theme foreground that
/// will be drawn over it.
pub fn correct_bg(color: Rgb, fg: Rgb) -> Rgb {
    let key = pair_key(color, fg);
    if let Some(hit) = cache_get(&BG_CACHE, key) {
        return hit;
    }
    let corrected = correct_against(color, fg, TARGET_RATIO_BG);
    cache_put(&BG_CACHE, key, corrected);
    corrected
}

/// The shared Oklab "magic": keep hue and chroma (`a`, `b`), search the
/// lightness that restores contrast against `other`, moving away from the
/// other color's luminance.
fn correct_against(color: Rgb, other: Rgb, target: f32) -> Rgb {
    if contrast_ratio(color, other) >= LEAVE_UNTOUCHED_RATIO {
        return color;
    }
    let (l0, a, b) = rgb_to_oklab(color);
    let other_light = relative_luminance(other) > 0.5;
    // Search direction: darken the color on light surroundings, brighten it
    // on dark ones. Luminance rises monotonically with Oklab L in practice,
    // so a plain bisection finds the boundary.
    let (mut fail, mut pass) = if other_light {
        (l0.max(L_MIN + 1e-3), L_MIN)
    } else {
        (l0.min(L_MAX - 1e-3), L_MAX)
    };
    // Invariant: `fail` never reaches `target`, `pass` always does. Bisect
    // toward the boundary and keep the last passing candidate: the SMALLEST
    // move away from the background that restores contrast. Correcting any
    // further blows dim TUI grays out to near-white (dark themes) or
    // near-black (light themes) and erases the app's visual hierarchy.
    let mut best = oklab_to_rgb(pass, a, b);
    for _ in 0..18 {
        let mid = (fail + pass) / 2.0;
        let candidate = oklab_to_rgb(mid, a, b);
        if contrast_ratio(candidate, other) >= target {
            pass = mid;
            best = candidate;
        } else {
            fail = mid;
        }
    }
    best
}

/// sRGB 0..255 to Oklab (Björn Ottosson's reference conversion).
fn rgb_to_oklab(c: Rgb) -> (f32, f32, f32) {
    let r = srgb_to_linear(c.0);
    let g = srgb_to_linear(c.1);
    let b = srgb_to_linear(c.2);
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    (
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    )
}

/// Oklab back to sRGB 0..255, clamped into gamut.
fn oklab_to_rgb(l: f32, a: f32, b: f32) -> Rgb {
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
    let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;
    (
        linear_to_srgb(r),
        linear_to_srgb(g),
        linear_to_srgb(b),
    )
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> u8 {
    let c = if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// WCAG relative luminance.
fn relative_luminance(c: Rgb) -> f32 {
    let r = srgb_to_linear(c.0);
    let g = srgb_to_linear(c.1);
    let b = srgb_to_linear(c.2);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG contrast ratio (1..21).
fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// RGB for a 256-color index >= 16, matching `convert_color`'s cube
/// (`*51` steps) and grayscale ramp. Returns None for 0..=15, which are
/// theme-owned named colors and never candidates for correction.
pub fn indexed_to_rgb(idx: u8) -> Option<Rgb> {
    if idx < 16 {
        return None;
    }
    if idx < 232 {
        let i = idx - 16;
        Some((
            ((i / 36) % 6) * 51,
            ((i / 6) % 6) * 51,
            (i % 6) * 51,
        ))
    } else {
        let gray = (idx - 232) * 10 + 8;
        Some((gray, gray, gray))
    }
}

/// sRGB-space "over" blend, matching how the GPU composites the quad
/// overlays painted under text (selection tint, search highlights).
pub fn alpha_blend(base: Rgb, over: Rgb, alpha: f32) -> Rgb {
    let mix = |b: u8, o: u8| (b as f32 * (1.0 - alpha) + o as f32 * alpha).round() as u8;
    (mix(base.0, over.0), mix(base.1, over.1), mix(base.2, over.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ratio(a: Rgb, b: Rgb) -> f32 {
        contrast_ratio(a, b)
    }

    #[test]
    fn oklab_roundtrip_is_stable() {
        for &(r, g, b) in &[(0u8, 0, 0), (255, 255, 255), (10, 200, 30), (128, 64, 192), (250, 200, 12)] {
            let (l, a, bb) = rgb_to_oklab((r, g, b));
            let (r2, g2, b2) = oklab_to_rgb(l, a, bb);
            assert!((r as i32 - r2 as i32).abs() <= 1, "r {r} vs {r2}");
            assert!((g as i32 - g2 as i32).abs() <= 1, "g {g} vs {g2}");
            assert!((b as i32 - b2 as i32).abs() <= 1, "b {b} vs {b2}");
        }
    }

    #[test]
    fn alpha_blend_matches_quad_compositing() {
        assert_eq!(alpha_blend((0, 0, 0), (255, 255, 255), 0.35), (89, 89, 89));
        assert_eq!(alpha_blend((10, 20, 30), (200, 100, 0), 0.5), (105, 60, 15));
        // alpha 0 and 1 are the endpoints
        assert_eq!(alpha_blend((1, 2, 3), (9, 9, 9), 0.0), (1, 2, 3));
        assert_eq!(alpha_blend((1, 2, 3), (9, 9, 9), 1.0), (9, 9, 9));
    }

    #[test]
    fn corrected_fg_stays_readable_under_selection_tint() {
        // Fastty default theme: background #151515, accent #FDA906. Selected
        // cells paint the accent at SELECTION_OVERLAY_ALPHA over the cell
        // background, under the glyphs — so the background a corrected fg
        // must read against is the blend, not the plain cell background.
        let bg = (0x15, 0x15, 0x15);
        let accent = (0xFD, 0xA9, 0x06);
        let tint = alpha_blend(bg, accent, 0.35);

        // TUI dim grays (Claude Code / opencode use these constantly).
        for dim in [(0x5A, 0x5A, 0x5A), (0x58, 0x58, 0x58), (0x76, 0x76, 0x76)] {
            let fixed = correct_fg(dim, tint);
            assert!(
                contrast_ratio(fixed, tint) >= LEAVE_UNTOUCHED_RATIO,
                "corrected {dim:?} must read on the selection tint, got {fixed:?} at {:?}",
                contrast_ratio(fixed, tint)
            );
        }

        // The invisible-selection repro, pinned: #767676 passes the ≥3:1
        // check against the plain background (so correction lets it
        // through untouched) but sinks below readable once the selection
        // tint lands on top of it — the span builder must correct against
        // the tint instead (see `render_pane_tree_node`).
        let passthrough = correct_fg((0x76, 0x76, 0x76), bg);
        assert_eq!(passthrough, (0x76, 0x76, 0x76)); // ≥3:1 on bg → untouched
        assert!(contrast_ratio(passthrough, tint) < LEAVE_UNTOUCHED_RATIO);

        // And correcting that same color against the tint rescues it —
        // minimally, not by blowing it out to near-white.
        let rescued = correct_fg((0x76, 0x76, 0x76), tint);
        assert!(contrast_ratio(rescued, tint) >= LEAVE_UNTOUCHED_RATIO);
        assert!(contrast_ratio(rescued, tint) <= TARGET_RATIO + 1.0);
    }

    #[test]
    fn correction_is_minimal_not_blown_out() {
        // A dim TUI gray on the dark default background: brightened just
        // enough to read, staying a mid gray. The old bisection converged
        // to the passing extreme instead of the boundary and returned
        // near-white, erasing dim/bright hierarchy in TUIs.
        let bg = (0x15, 0x15, 0x15);
        let dim = (0x5A, 0x5A, 0x5A);
        let fixed = correct_fg(dim, bg);
        let r = contrast_ratio(fixed, bg);
        assert!(r >= TARGET_RATIO - 0.3, "must reach the target, got {r}");
        assert!(r <= TARGET_RATIO + 1.0, "must stop near the target, got {r} for {fixed:?}");
        assert!(relative_luminance(fixed) < 0.5, "stays a mid gray, got {fixed:?}");
    }

    #[test]
    fn already_contrasting_colors_pass_through() {
        let c = (200u8, 30, 40);
        assert_eq!(correct_fg(c, (255, 255, 255)), c);
        let c2 = (230u8, 230, 90);
        assert_eq!(correct_fg(c2, (18, 18, 24)), c2);
    }

    #[test]
    fn light_truecolor_on_white_gets_darkened() {
        // The actual light-mode pain: bright "designed for dark bg" tones.
        let pale = (188u8, 188, 195);
        assert!(ratio(pale, (255, 255, 255)) < 2.0);
        let fixed = correct_fg(pale, (255, 255, 255));
        assert!(ratio(fixed, (255, 255, 255)) >= TARGET_RATIO - 0.3);
    }

    #[test]
    fn bright_truecolor_on_white_gets_darkened() {
        let yellow = (255u8, 255, 40);
        assert!(ratio(yellow, (255, 255, 255)) < 2.0);
        let fixed = correct_fg(yellow, (255, 255, 255));
        assert!(ratio(fixed, (255, 255, 255)) >= TARGET_RATIO - 0.3);
        // Still yellow-ish: red and green stay close together.
        assert!((fixed.0 as i32 - fixed.1 as i32).abs() <= 24);
    }

    #[test]
    fn near_black_on_dark_theme_gets_brightened() {
        let dim = (25u8, 25, 30);
        let bg = (24u8, 24, 32); // One Dark-ish background
        assert!(ratio(dim, bg) < 1.2);
        let fixed = correct_fg(dim, bg);
        assert!(ratio(fixed, bg) >= TARGET_RATIO - 0.3);
    }

    #[test]
    fn explicit_background_gets_corrected_too() {
        // A TUI status bar with a dark explicit bg under a light theme's
        // dark foreground: push the bar away from the text.
        let bar = (30u8, 34, 90);
        let fg = (40u8, 42, 48);
        assert!(ratio(bar, fg) < 3.0);
        let fixed = correct_bg(bar, fg);
        assert!(ratio(fixed, fg) >= TARGET_RATIO_BG - 0.3);
    }

    #[test]
    fn grayscale_stays_neutral() {
        let fixed = correct_fg((90u8, 90, 90), (252, 252, 252));
        let (r, g, b) = fixed;
        assert!((r as i32 - g as i32).abs() <= 2 && (g as i32 - b as i32).abs() <= 2);
    }

    #[test]
    fn indexed_cube_matches_convert_color_math() {
        assert_eq!(indexed_to_rgb(16), Some((0, 0, 0)));
        assert_eq!(indexed_to_rgb(231), Some((255, 255, 255)));
        assert_eq!(indexed_to_rgb(196), Some((255, 0, 0)));
        assert_eq!(indexed_to_rgb(244), Some((128, 128, 128)));
        assert_eq!(indexed_to_rgb(15), None);
    }
}

