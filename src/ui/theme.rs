use gpui::{Hsla, Rgba, rgb, transparent_black};
use crate::config::{self, ThemeFile};

/// Theme tokens for fastty GPUI interface.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub foreground: Hsla,
    pub background: Hsla,
    pub cursor: Hsla,
    pub black: Hsla,
    pub red: Hsla,
    pub green: Hsla,
    pub yellow: Hsla,
    pub blue: Hsla,
    pub magenta: Hsla,
    pub cyan: Hsla,
    pub white: Hsla,
    pub bright_black: Hsla,
    pub bright_red: Hsla,
    pub bright_green: Hsla,
    pub bright_yellow: Hsla,
    pub bright_blue: Hsla,
    pub bright_magenta: Hsla,
    pub bright_cyan: Hsla,
    pub bright_white: Hsla,

    // Chrome tokens
    pub tab_bar_bg: Hsla,
    pub tab_active_bg: Hsla,
    pub tab_inactive_bg: Hsla,
    pub status_bar_bg: Hsla,
    pub sidebar_bg: Hsla,
    pub main_bg: Hsla,
    pub surface: Hsla,
    pub surface_raised: Hsla,
    pub border: Hsla,
    pub muted: Hsla,
    pub muted_strong: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub accent: Hsla,
    pub opacity: f32,
}

impl Theme {
    /// Fastty default theme (crisp high-clarity palette).
    pub fn fastty_default() -> Self {
        let foreground = rgb_to_hsla(0xE5, 0xE9, 0xF0);
        let background = rgb_to_hsla(0x15, 0x15, 0x15);
        let bright_black = rgb_to_hsla(0x84, 0x8B, 0x98);
        let accent = rgb_to_hsla(0xFD, 0xA9, 0x06);

        Self {
            foreground,
            background,
            cursor: rgb_to_hsla(0xFD, 0xA9, 0x06),
            black: rgb_to_hsla(0x3B, 0x42, 0x52),
            red: rgb_to_hsla(0xF0, 0x4E, 0x4E),
            green: rgb_to_hsla(0x8E, 0xE0, 0x44),
            yellow: accent,
            blue: rgb_to_hsla(0x5A, 0xB0, 0xF0),
            magenta: rgb_to_hsla(0xD0, 0x7E, 0xE0),
            cyan: rgb_to_hsla(0x56, 0xE2, 0xDB),
            white: rgb_to_hsla(0xE5, 0xE9, 0xF0),
            bright_black,
            bright_red: rgb_to_hsla(0xFF, 0x6B, 0x6B),
            bright_green: rgb_to_hsla(0xA6, 0xF0, 0x5E),
            bright_yellow: rgb_to_hsla(0xFF, 0xE0, 0x4A),
            bright_blue: rgb_to_hsla(0x74, 0xCC, 0xFF),
            bright_magenta: rgb_to_hsla(0xE8, 0x96, 0xFA),
            bright_cyan: rgb_to_hsla(0x6A, 0xF5, 0xF0),
            bright_white: rgb_to_hsla(0xFF, 0xFF, 0xFF),

            tab_bar_bg: background,
            tab_active_bg: rgb_to_hsla(0x20, 0x20, 0x20),
            tab_inactive_bg: rgb_to_hsla(0x10, 0x10, 0x10),
            status_bar_bg: background,
            sidebar_bg: background,
            main_bg: background,
            surface: background,
            surface_raised: rgb_to_hsla(0x20, 0x20, 0x20),
            border: rgb_to_hsla(0x2A, 0x2A, 0x2A),
            muted: bright_black,
            muted_strong: rgb_to_hsla(0xA0, 0xA8, 0xB6),
            hover: rgb_to_hsla(0x26, 0x26, 0x26),
            selected: rgb_to_hsla(0x30, 0x30, 0x30),
            accent,
            opacity: 1.0,
        }
    }

    pub fn catppuccin() -> Self {
        let foreground = rgb_to_hsla(0xCA, 0xD3, 0xF5);
        let background = rgb_to_hsla(0x24, 0x27, 0x3A);
        let bright_black = rgb_to_hsla(0x5B, 0x60, 0x78);
        let blue = rgb_to_hsla(0x8A, 0xAD, 0xF4);

        Self {
            foreground,
            background,
            cursor: rgb_to_hsla(0xF4, 0xDB, 0xD6),
            black: rgb_to_hsla(0x49, 0x4D, 0x64),
            red: rgb_to_hsla(0xED, 0x87, 0x96),
            green: rgb_to_hsla(0xA6, 0xDA, 0x95),
            yellow: rgb_to_hsla(0xEE, 0xDD, 0xB2),
            blue,
            magenta: rgb_to_hsla(0xF5, 0xB8, 0x95),
            cyan: rgb_to_hsla(0x91, 0xD7, 0xE3),
            white: rgb_to_hsla(0xB8, 0xC0, 0xE0),
            bright_black,
            bright_red: rgb_to_hsla(0xED, 0x87, 0x96),
            bright_green: rgb_to_hsla(0xA6, 0xDA, 0x95),
            bright_yellow: rgb_to_hsla(0xEE, 0xDD, 0xB2),
            bright_blue: rgb_to_hsla(0x8A, 0xAD, 0xF4),
            bright_magenta: rgb_to_hsla(0xF4, 0xBD, 0xD8),
            bright_cyan: rgb_to_hsla(0x91, 0xD7, 0xE3),
            bright_white: rgb_to_hsla(0xA5, 0xAD, 0xCB),

            tab_bar_bg: background,
            tab_active_bg: rgb_to_hsla(0x2a, 0x2e, 0x48),
            tab_inactive_bg: rgb_to_hsla(0x1e, 0x20, 0x30),
            status_bar_bg: background,
            sidebar_bg: background,
            main_bg: background,
            surface: rgb_to_hsla(0x2a, 0x2e, 0x48),
            surface_raised: rgb_to_hsla(0x36, 0x3a, 0x5e),
            border: rgb_to_hsla(0x36, 0x3a, 0x5e),
            muted: bright_black,
            muted_strong: rgb_to_hsla(0x93, 0x9A, 0xB7),
            hover: rgb_to_hsla(0x36, 0x3a, 0x5e),
            selected: rgb_to_hsla(0x49, 0x4d, 0x64),
            accent: rgb_to_hsla(0xFD, 0xA9, 0x06),
            opacity: 1.0,
        }
    }

    pub fn one_dark() -> Self {
        let foreground = rgb_to_hsla(0xAB, 0xB2, 0xBF);
        let background = rgb_to_hsla(0x28, 0x2C, 0x34);
        let bright_black = rgb_to_hsla(0x5C, 0x63, 0x70);
        let blue = rgb_to_hsla(0x61, 0xAF, 0xEF);

        Self {
            foreground,
            background,
            cursor: rgb_to_hsla(0x52, 0x8B, 0xFF),
            black: rgb_to_hsla(0x28, 0x2C, 0x34),
            red: rgb_to_hsla(0xE0, 0x6C, 0x75),
            green: rgb_to_hsla(0x98, 0xC3, 0x79),
            yellow: rgb_to_hsla(0xD1, 0x9A, 0x66),
            blue,
            magenta: rgb_to_hsla(0xC6, 0x78, 0xDD),
            cyan: rgb_to_hsla(0x56, 0xB6, 0xC2),
            white: rgb_to_hsla(0xAB, 0xB2, 0xBF),
            bright_black,
            bright_red: rgb_to_hsla(0xE0, 0x6C, 0x75),
            bright_green: rgb_to_hsla(0x98, 0xC3, 0x79),
            bright_yellow: rgb_to_hsla(0xD1, 0x9A, 0x66),
            bright_blue: rgb_to_hsla(0x61, 0xAF, 0xEF),
            bright_magenta: rgb_to_hsla(0xC6, 0x78, 0xDD),
            bright_cyan: rgb_to_hsla(0x56, 0xB6, 0xC2),
            bright_white: rgb_to_hsla(0xFF, 0xFF, 0xFF),

            tab_bar_bg: background,
            tab_active_bg: rgb_to_hsla(0x28, 0x2c, 0x34),
            tab_inactive_bg: rgb_to_hsla(0x21, 0x25, 0x2b),
            status_bar_bg: background,
            sidebar_bg: background,
            main_bg: background,
            surface: rgb_to_hsla(0x2c, 0x31, 0x3a),
            surface_raised: rgb_to_hsla(0x35, 0x3b, 0x45),
            border: rgb_to_hsla(0x3e, 0x44, 0x51),
            muted: bright_black,
            muted_strong: rgb_to_hsla(0x82, 0x89, 0x97),
            hover: rgb_to_hsla(0x35, 0x3b, 0x45),
            selected: rgb_to_hsla(0x3e, 0x44, 0x51),
            accent: rgb_to_hsla(0xFD, 0xA9, 0x06),
            opacity: 1.0,
        }
    }

    pub fn solarized_dark() -> Self {
        let foreground = rgb_to_hsla(0x83, 0x94, 0x96);
        let background = rgb_to_hsla(0x00, 0x2B, 0x36);
        let bright_black = rgb_to_hsla(0x58, 0x6E, 0x75);
        let blue = rgb_to_hsla(0x26, 0x8B, 0xD2);

        Self {
            foreground,
            background,
            cursor: rgb_to_hsla(0x93, 0xA1, 0xA1),
            black: rgb_to_hsla(0x07, 0x36, 0x42),
            red: rgb_to_hsla(0xDC, 0x32, 0x2F),
            green: rgb_to_hsla(0x85, 0x99, 0x00),
            yellow: rgb_to_hsla(0xB5, 0x89, 0x00),
            blue,
            magenta: rgb_to_hsla(0xD3, 0x36, 0x82),
            cyan: rgb_to_hsla(0x2A, 0xA1, 0x98),
            white: rgb_to_hsla(0xEE, 0xE8, 0xD5),
            bright_black,
            bright_red: rgb_to_hsla(0xCB, 0x4B, 0x16),
            bright_green: rgb_to_hsla(0x58, 0x6E, 0x75),
            bright_yellow: rgb_to_hsla(0x65, 0x7B, 0x83),
            bright_blue: rgb_to_hsla(0x83, 0x94, 0x96),
            bright_magenta: rgb_to_hsla(0x6C, 0x71, 0xC4),
            bright_cyan: rgb_to_hsla(0x93, 0xA1, 0xA1),
            bright_white: rgb_to_hsla(0xFD, 0xF6, 0xE3),

            tab_bar_bg: background,
            tab_active_bg: rgb_to_hsla(0x07, 0x36, 0x42),
            tab_inactive_bg: rgb_to_hsla(0x00, 0x21, 0x2b),
            status_bar_bg: background,
            sidebar_bg: background,
            main_bg: background,
            surface: rgb_to_hsla(0x07, 0x36, 0x42),
            surface_raised: rgb_to_hsla(0x09, 0x43, 0x52),
            border: rgb_to_hsla(0x58, 0x6e, 0x75),
            muted: bright_black,
            muted_strong: rgb_to_hsla(0x65, 0x7B, 0x83),
            hover: rgb_to_hsla(0x09, 0x43, 0x52),
            selected: rgb_to_hsla(0x58, 0x6e, 0x75),
            accent: rgb_to_hsla(0xFD, 0xA9, 0x06),
            opacity: 1.0,
        }
    }

    pub fn high_contrast() -> Self {
        let foreground = rgb_to_hsla(0xFF, 0xFF, 0xFF);
        let background = rgb_to_hsla(0x00, 0x00, 0x00);
        let bright_black = rgb_to_hsla(0x7F, 0x7F, 0x7F);
        let blue = rgb_to_hsla(0x62, 0xD6, 0xFF);

        Self {
            foreground,
            background,
            cursor: rgb_to_hsla(0xFF, 0xFF, 0xFF),
            black: rgb_to_hsla(0x00, 0x00, 0x00),
            red: rgb_to_hsla(0xFF, 0x55, 0x55),
            green: rgb_to_hsla(0x50, 0xFA, 0x7B),
            yellow: rgb_to_hsla(0xFF, 0xF0, 0x5A),
            blue,
            magenta: rgb_to_hsla(0xFF, 0x79, 0xC6),
            cyan: rgb_to_hsla(0x8B, 0xEC, 0xFF),
            white: rgb_to_hsla(0xFF, 0xFF, 0xFF),
            bright_black,
            bright_red: rgb_to_hsla(0xFF, 0x55, 0x55),
            bright_green: rgb_to_hsla(0x50, 0xFA, 0x7B),
            bright_yellow: rgb_to_hsla(0xFF, 0xF0, 0x5A),
            bright_blue: rgb_to_hsla(0x62, 0xD6, 0xFF),
            bright_magenta: rgb_to_hsla(0xFF, 0x79, 0xC6),
            bright_cyan: rgb_to_hsla(0x8B, 0xEC, 0xFF),
            bright_white: rgb_to_hsla(0xFF, 0xFF, 0xFF),

            tab_bar_bg: background,
            tab_active_bg: rgb_to_hsla(0x1a, 0x1a, 0x1a),
            tab_inactive_bg: rgb_to_hsla(0x05, 0x05, 0x05),
            status_bar_bg: background,
            sidebar_bg: background,
            main_bg: background,
            surface: rgb_to_hsla(0x1f, 0x1f, 0x1f),
            surface_raised: rgb_to_hsla(0x2e, 0x2e, 0x2e),
            border: rgb_to_hsla(0x55, 0x55, 0x55),
            muted: bright_black,
            muted_strong: rgb_to_hsla(0xAA, 0xAA, 0xAA),
            hover: rgb_to_hsla(0x2e, 0x2e, 0x2e),
            selected: rgb_to_hsla(0x44, 0x44, 0x44),
            accent: rgb_to_hsla(0xFD, 0xA9, 0x06),
            opacity: 1.0,
        }
    }

    pub fn from_name(name: &str) -> Self {
        if let Some(custom) = config::CUSTOM_THEMES.get() {
            let map = custom.read();
            if let Some(tf) = map.get(name) {
                return Self::from_theme_file(tf);
            }
        }
        match name.to_lowercase().as_str() {
            "catppuccin" | "catppuccin-mocha" => Self::catppuccin(),
            "one-dark" | "onedark" => Self::one_dark(),
            "solarized-dark" | "solarized" => Self::solarized_dark(),
            "high-contrast" => Self::high_contrast(),
            _ => Self::fastty_default(),
        }
    }

    pub fn from_theme_file(tf: &ThemeFile) -> Self {
        let parse_h = |s: &str| -> Hsla {
            if let Some((r, g, b)) = config::parse_hex_color(s) {
                rgb_to_hsla(r, g, b)
            } else {
                rgb_to_hsla(0xFF, 0xFF, 0xFF)
            }
        };

        let mut t = Self::fastty_default();
        t.foreground = parse_h(&tf.foreground);
        t.background = parse_h(&tf.background);
        t.main_bg = t.background;
        t.tab_bar_bg = t.background;
        t.status_bar_bg = t.background;
        t.sidebar_bg = t.background;
        t.black = parse_h(&tf.black);
        t.red = parse_h(&tf.red);
        t.green = parse_h(&tf.green);
        t.yellow = parse_h(&tf.yellow);
        t.blue = parse_h(&tf.blue);
        t.magenta = parse_h(&tf.magenta);
        t.cyan = parse_h(&tf.cyan);
        t.white = parse_h(&tf.white);
        t.bright_black = parse_h(&tf.bright_black);
        t.bright_red = parse_h(&tf.bright_red);
        t.bright_green = parse_h(&tf.bright_green);
        t.bright_yellow = parse_h(&tf.bright_yellow);
        t.bright_blue = parse_h(&tf.bright_blue);
        t.bright_magenta = parse_h(&tf.bright_magenta);
        t.bright_cyan = parse_h(&tf.bright_cyan);
        t.bright_white = parse_h(&tf.bright_white);
        t.cursor = t.foreground;
        t.accent = rgb_to_hsla(0xFD, 0xA9, 0x06);
        t
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.1, 1.0);
        self.background.a = self.opacity;
        self.main_bg.a = self.opacity;
        self.tab_bar_bg.a = self.opacity;
        self.status_bar_bg.a = self.opacity;
        self.sidebar_bg.a = self.opacity;
        self.surface.a = (self.surface.a * self.opacity).clamp(0.0, 1.0);
        self.surface_raised.a = (self.surface_raised.a * self.opacity).clamp(0.0, 1.0);
        self.tab_active_bg.a = (self.tab_active_bg.a * self.opacity).clamp(0.0, 1.0);
        self.tab_inactive_bg.a = (self.tab_inactive_bg.a * self.opacity).clamp(0.0, 1.0);
        self.hover.a = (self.hover.a * self.opacity).clamp(0.0, 1.0);
        self.border.a = (self.border.a * self.opacity).clamp(0.0, 1.0);
        self
    }

    pub fn window_fill(self) -> Hsla {
        if self.opacity < 1.0 {
            transparent_black()
        } else {
            self.background
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::fastty_default()
    }
}

pub fn rgb_to_hsla(r: u8, g: u8, b: u8) -> Hsla {
    let u = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
    let Rgba { r, g, b, a } = rgb(u);
    Rgba { r, g, b, a }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_with_opacity_scales_components() {
        let theme = Theme::fastty_default().with_opacity(0.8);
        assert_eq!(theme.opacity, 0.8);
        assert!((theme.background.a - 0.8).abs() < 1e-4);
        assert!((theme.surface.a - 0.8).abs() < 1e-4);
        assert!((theme.surface_raised.a - 0.8).abs() < 1e-4);
        assert!((theme.tab_bar_bg.a - 0.8).abs() < 1e-4);
        assert!((theme.tab_active_bg.a - 0.8).abs() < 1e-4);
        assert!((theme.tab_inactive_bg.a - 0.8).abs() < 1e-4);
        assert!((theme.status_bar_bg.a - 0.8).abs() < 1e-4);
        assert!((theme.sidebar_bg.a - 0.8).abs() < 1e-4);
        assert!((theme.hover.a - 0.8).abs() < 1e-4);
        assert!((theme.border.a - 0.8).abs() < 1e-4);
    }
}
