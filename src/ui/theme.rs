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

/// Compact palette spec for the bundled theme families: background,
/// foreground, an optional cursor color, the 16 ANSI slots in order
/// (black..white, bright_black..bright_white), and an accent. Chrome
/// tokens are derived by [`Theme::from_spec`] from background lightness.
pub struct ThemeSpec {
    pub background: (u8, u8, u8),
    pub foreground: (u8, u8, u8),
    pub cursor: Option<(u8, u8, u8)>,
    pub ansi: [(u8, u8, u8); 16],
    pub accent: (u8, u8, u8),
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
            "catppuccin-frappe" | "frappe" => Self::catppuccin_frappe(),
            "catppuccin-macchiato" | "macchiato" => Self::catppuccin_macchiato(),
            "catppuccin-latte" | "latte" => Self::catppuccin_latte(),
            "dracula" => Self::dracula(),
            "nord" => Self::nord(),
            "tokyo-night" => Self::tokyo_night(),
            "tokyo-night-storm" | "tokyo-storm" => Self::tokyo_night_storm(),
            "gruvbox-dark" | "gruvbox" => Self::gruvbox_dark(),
            "gruvbox-light" => Self::gruvbox_light(),
            "rose-pine" | "rosepine" => Self::rose_pine(),
            "rose-pine-moon" | "rosepine-moon" => Self::rose_pine_moon(),
            "rose-pine-dawn" | "rosepine-dawn" => Self::rose_pine_dawn(),
            "one-light" => Self::one_light(),
            "solarized-light" => Self::solarized_light(),
            "kanagawa" | "kanagawa-wave" => Self::kanagawa(),
            _ => Self::fastty_default(),
        }
    }

    pub fn from_theme_file(tf: &ThemeFile) -> Self {
        let parse = |s: &str| -> (u8, u8, u8) {
            config::parse_hex_color(s).unwrap_or((0xFF, 0xFF, 0xFF))
        };
        let spec = ThemeSpec {
            background: parse(&tf.background),
            foreground: parse(&tf.foreground),
            cursor: None,
            ansi: [
                parse(&tf.black),
                parse(&tf.red),
                parse(&tf.green),
                parse(&tf.yellow),
                parse(&tf.blue),
                parse(&tf.magenta),
                parse(&tf.cyan),
                parse(&tf.white),
                parse(&tf.bright_black),
                parse(&tf.bright_red),
                parse(&tf.bright_green),
                parse(&tf.bright_yellow),
                parse(&tf.bright_blue),
                parse(&tf.bright_magenta),
                parse(&tf.bright_cyan),
                parse(&tf.bright_white),
            ],
            // Custom files carry no accent; amber stays the house accent.
            accent: (0xFD, 0xA9, 0x06),
        };
        Self::from_spec(&spec)
    }

    /// Builds a theme from a compact palette spec. Chrome tokens (surfaces,
    /// borders, hover, selection) are derived from the background's HSL
    /// lightness, so light families get light chrome and dark families
    /// dark chrome — the fix that makes light themes usable at all.
    pub fn from_spec(spec: &ThemeSpec) -> Self {
        let h = |c: (u8, u8, u8)| rgb_to_hsla(c.0, c.1, c.2);
        let foreground = h(spec.foreground);
        let background = h(spec.background);
        let bright_black = h(spec.ansi[8]);
        let light = background.l > 0.5;
        let step = |delta: f32| {
            let mut c = background;
            c.l = (c.l + if light { -delta } else { delta }).clamp(0.0, 1.0);
            c
        };
        let surface = step(0.03);
        let surface_raised = step(0.055);
        let border = step(0.085);
        let hover = step(0.04);
        let selected = step(0.07);
        Self {
            foreground,
            background,
            cursor: spec.cursor.map(h).unwrap_or(foreground),
            black: h(spec.ansi[0]),
            red: h(spec.ansi[1]),
            green: h(spec.ansi[2]),
            yellow: h(spec.ansi[3]),
            blue: h(spec.ansi[4]),
            magenta: h(spec.ansi[5]),
            cyan: h(spec.ansi[6]),
            white: h(spec.ansi[7]),
            bright_black,
            bright_red: h(spec.ansi[9]),
            bright_green: h(spec.ansi[10]),
            bright_yellow: h(spec.ansi[11]),
            bright_blue: h(spec.ansi[12]),
            bright_magenta: h(spec.ansi[13]),
            bright_cyan: h(spec.ansi[14]),
            bright_white: h(spec.ansi[15]),
            tab_bar_bg: background,
            tab_active_bg: surface_raised,
            tab_inactive_bg: surface,
            status_bar_bg: background,
            sidebar_bg: background,
            main_bg: background,
            surface,
            surface_raised,
            border,
            muted: bright_black,
            muted_strong: {
                let mut c = bright_black;
                c.l = (c.l + if light { -0.08 } else { 0.10 }).clamp(0.0, 1.0);
                c
            },
            hover,
            selected,
            accent: h(spec.accent),
            opacity: 1.0,
        }
    }

    pub fn catppuccin_frappe() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0x30, 0x34, 0x46),
            foreground: (0xC6, 0xD0, 0xF5),
            cursor: Some((0xF2, 0xD5, 0xCF)),
            ansi: [
                (0x51, 0x57, 0x6D), (0xE7, 0x82, 0x84), (0xA6, 0xD1, 0x89), (0xE5, 0xC8, 0x90),
                (0x8C, 0xAA, 0xEE), (0xCA, 0x9E, 0xE6), (0x81, 0xC8, 0xBE), (0xB5, 0xBF, 0xE2),
                (0x62, 0x68, 0x80), (0xE7, 0x82, 0x84), (0xA6, 0xD1, 0x89), (0xE5, 0xC8, 0x90),
                (0x8C, 0xAA, 0xEE), (0xF4, 0xB8, 0xE4), (0x99, 0xD1, 0xDB), (0xA5, 0xAD, 0xCE),
            ],
            accent: (0xCA, 0x9E, 0xE6),
        })
    }

    pub fn catppuccin_macchiato() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0x24, 0x27, 0x3A),
            foreground: (0xCA, 0xD3, 0xF5),
            cursor: Some((0xF4, 0xDB, 0xD6)),
            ansi: [
                (0x49, 0x4D, 0x64), (0xED, 0x87, 0x96), (0xA6, 0xDA, 0x95), (0xEE, 0xD4, 0x9F),
                (0x8A, 0xAD, 0xF4), (0xC6, 0xA0, 0xF6), (0x8B, 0xD5, 0xCA), (0xB8, 0xC0, 0xE0),
                (0x62, 0x68, 0x80), (0xED, 0x87, 0x96), (0xA6, 0xDA, 0x95), (0xEE, 0xD4, 0x9F),
                (0x8A, 0xAD, 0xF4), (0xF5, 0xBD, 0xE6), (0x91, 0xD7, 0xE3), (0xA5, 0xAD, 0xCB),
            ],
            accent: (0xC6, 0xA0, 0xF6),
        })
    }

    pub fn catppuccin_latte() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0xEF, 0xF1, 0xF5),
            foreground: (0x4C, 0x4F, 0x69),
            cursor: Some((0xDC, 0x8A, 0x78)),
            ansi: [
                (0x5C, 0x5F, 0x77), (0xD2, 0x0F, 0x39), (0x40, 0xA0, 0x2B), (0xDF, 0x8E, 0x1D),
                (0x1E, 0x66, 0xF5), (0x88, 0x39, 0xEF), (0x17, 0x92, 0x99), (0xAC, 0xB0, 0xBE),
                (0x8C, 0x8F, 0xA1), (0xD2, 0x0F, 0x39), (0x40, 0xA0, 0x2B), (0xDF, 0x8E, 0x1D),
                (0x1E, 0x66, 0xF5), (0xEA, 0x76, 0xCB), (0x17, 0x92, 0x99), (0x6C, 0x6F, 0x85),
            ],
            accent: (0x88, 0x39, 0xEF),
        })
    }

    pub fn dracula() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0x28, 0x2A, 0x36),
            foreground: (0xF8, 0xF8, 0xF2),
            cursor: Some((0xF8, 0xF8, 0xF2)),
            ansi: [
                (0x21, 0x22, 0x2C), (0xFF, 0x55, 0x55), (0x50, 0xFA, 0x7B), (0xF1, 0xFA, 0x8C),
                (0xBD, 0x93, 0xF9), (0xFF, 0x79, 0xC6), (0x8B, 0xE9, 0xFD), (0xF8, 0xF8, 0xF2),
                (0x62, 0x72, 0xA4), (0xFF, 0x6E, 0x6E), (0x69, 0xFF, 0x94), (0xFF, 0xFF, 0xA5),
                (0xD6, 0xAC, 0xFF), (0xFF, 0x92, 0xDF), (0xA4, 0xFF, 0xFF), (0xFF, 0xFF, 0xFF),
            ],
            accent: (0xBD, 0x93, 0xF9),
        })
    }

    pub fn nord() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0x2E, 0x34, 0x40),
            foreground: (0xD8, 0xDE, 0xE9),
            cursor: Some((0xD8, 0xDE, 0xE9)),
            ansi: [
                (0x3B, 0x42, 0x52), (0xBF, 0x61, 0x6A), (0xA3, 0xBE, 0x8C), (0xEB, 0xCB, 0x8B),
                (0x81, 0xA1, 0xC1), (0xB4, 0x8E, 0xAD), (0x88, 0xC0, 0xD0), (0xE5, 0xE9, 0xF0),
                (0x4C, 0x56, 0x6A), (0xBF, 0x61, 0x6A), (0xA3, 0xBE, 0x8C), (0xEB, 0xCB, 0x8B),
                (0x81, 0xA1, 0xC1), (0xB4, 0x8E, 0xAD), (0x8F, 0xBC, 0xBB), (0xEC, 0xEF, 0xF4),
            ],
            accent: (0x88, 0xC0, 0xD0),
        })
    }

    pub fn tokyo_night() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0x1A, 0x1B, 0x26),
            foreground: (0xC0, 0xCA, 0xF5),
            cursor: Some((0xC0, 0xCA, 0xF5)),
            ansi: [
                (0x15, 0x16, 0x1E), (0xF7, 0x76, 0x8E), (0x9E, 0xCE, 0x6A), (0xE0, 0xAF, 0x68),
                (0x7A, 0xA2, 0xF7), (0xBB, 0x9A, 0xF7), (0x7D, 0xCF, 0xFF), (0xA9, 0xB1, 0xD6),
                (0x41, 0x4B, 0x67), (0xFF, 0x7A, 0x93), (0xB9, 0xF2, 0x7C), (0xFF, 0x9E, 0x64),
                (0x7D, 0xA6, 0xFF), (0xBB, 0x9A, 0xF7), (0x0D, 0xB9, 0xD7), (0xC0, 0xCA, 0xF5),
            ],
            accent: (0x7A, 0xA2, 0xF7),
        })
    }

    pub fn tokyo_night_storm() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0x24, 0x28, 0x3B),
            foreground: (0xC0, 0xCA, 0xF5),
            cursor: Some((0xC0, 0xCA, 0xF5)),
            ansi: [
                (0x1F, 0x23, 0x35), (0xF7, 0x76, 0x8E), (0x9E, 0xCE, 0x6A), (0xE0, 0xAF, 0x68),
                (0x7A, 0xA2, 0xF7), (0xBB, 0x9A, 0xF7), (0x7D, 0xCF, 0xFF), (0xA9, 0xB1, 0xD6),
                (0x2F, 0x33, 0x4D), (0xFF, 0x7A, 0x93), (0xB9, 0xF2, 0x7C), (0xFF, 0x9E, 0x64),
                (0x7D, 0xA6, 0xFF), (0xBB, 0x9A, 0xF7), (0x0D, 0xB9, 0xD7), (0xC0, 0xCA, 0xF5),
            ],
            accent: (0x7A, 0xA2, 0xF7),
        })
    }

    pub fn gruvbox_dark() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0x28, 0x28, 0x28),
            foreground: (0xEB, 0xDB, 0xB2),
            cursor: Some((0xEB, 0xDB, 0xB2)),
            ansi: [
                (0x3C, 0x38, 0x36), (0xFB, 0x49, 0x34), (0xB8, 0xBB, 0x26), (0xFA, 0xBD, 0x2F),
                (0x83, 0xA5, 0x98), (0xD3, 0x86, 0x9B), (0x8E, 0xC0, 0x7C), (0xEB, 0xDB, 0xB2),
                (0x92, 0x83, 0x74), (0xFB, 0x49, 0x34), (0xB8, 0xBB, 0x26), (0xFA, 0xBD, 0x2F),
                (0x83, 0xA5, 0x98), (0xD3, 0x86, 0x9B), (0x8E, 0xC0, 0x7C), (0xFB, 0xF1, 0xC7),
            ],
            accent: (0xFA, 0xBD, 0x2F),
        })
    }

    pub fn gruvbox_light() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0xFB, 0xF1, 0xC7),
            foreground: (0x3C, 0x38, 0x36),
            cursor: Some((0x3C, 0x38, 0x36)),
            ansi: [
                (0x66, 0x5C, 0x54), (0x9D, 0x00, 0x06), (0x79, 0x74, 0x0E), (0xB5, 0x76, 0x14),
                (0x07, 0x66, 0x78), (0x8F, 0x3F, 0x71), (0x42, 0x7B, 0x58), (0xD5, 0xC4, 0xA1),
                (0x92, 0x83, 0x74), (0xAF, 0x3A, 0x03), (0x79, 0x74, 0x0E), (0xB5, 0x76, 0x14),
                (0x07, 0x66, 0x78), (0x8F, 0x3F, 0x71), (0x42, 0x7B, 0x58), (0xEB, 0xDB, 0xB2),
            ],
            accent: (0xB5, 0x76, 0x14),
        })
    }

    pub fn rose_pine() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0x19, 0x17, 0x24),
            foreground: (0xE0, 0xDE, 0xF4),
            cursor: Some((0xE0, 0xDE, 0xF4)),
            ansi: [
                (0x26, 0x23, 0x3A), (0xEB, 0x6F, 0x92), (0x9C, 0xCF, 0xD8), (0xF6, 0xC1, 0x77),
                (0x31, 0x74, 0x8F), (0xC4, 0xA7, 0xE7), (0xEB, 0xBC, 0xBE), (0xE0, 0xDE, 0xF4),
                (0x6E, 0x6A, 0x86), (0xEB, 0x6F, 0x92), (0x9C, 0xCF, 0xD8), (0xF6, 0xC1, 0x77),
                (0x31, 0x74, 0x8F), (0xC4, 0xA7, 0xE7), (0xEB, 0xBC, 0xBE), (0xF0, 0xEE, 0xEA),
            ],
            accent: (0xC4, 0xA7, 0xE7),
        })
    }

    pub fn rose_pine_moon() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0x23, 0x21, 0x36),
            foreground: (0xE0, 0xDE, 0xF4),
            cursor: Some((0xE0, 0xDE, 0xF4)),
            ansi: [
                (0x39, 0x35, 0x52), (0xEB, 0x6F, 0x92), (0x9C, 0xCF, 0xD8), (0xF6, 0xC1, 0x77),
                (0x3E, 0x8F, 0xB0), (0xC4, 0xA7, 0xE7), (0xEA, 0x9A, 0x97), (0xE0, 0xDE, 0xF4),
                (0x6E, 0x6A, 0x86), (0xEB, 0x6F, 0x92), (0x9C, 0xCF, 0xD8), (0xF6, 0xC1, 0x77),
                (0x3E, 0x8F, 0xB0), (0xC4, 0xA7, 0xE7), (0xEA, 0x9A, 0x97), (0xF0, 0xEE, 0xEA),
            ],
            accent: (0xC4, 0xA7, 0xE7),
        })
    }

    pub fn rose_pine_dawn() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0xFA, 0xF4, 0xED),
            foreground: (0x57, 0x52, 0x79),
            cursor: Some((0x57, 0x52, 0x79)),
            ansi: [
                (0x98, 0x93, 0xA5), (0xB4, 0x63, 0x7A), (0x56, 0x94, 0x9F), (0xEA, 0x9D, 0x34),
                (0x28, 0x69, 0x83), (0x90, 0x7A, 0xA9), (0xD7, 0x82, 0x7E), (0x79, 0x75, 0x93),
                (0x6E, 0x6A, 0x86), (0xB4, 0x63, 0x7A), (0x56, 0x94, 0x9F), (0xEA, 0x9D, 0x34),
                (0x28, 0x69, 0x83), (0x90, 0x7A, 0xA9), (0xD7, 0x82, 0x7E), (0x57, 0x52, 0x79),
            ],
            accent: (0x90, 0x7A, 0xA9),
        })
    }

    pub fn one_light() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0xFA, 0xFA, 0xFA),
            foreground: (0x38, 0x3A, 0x42),
            cursor: Some((0x38, 0x3A, 0x42)),
            ansi: [
                (0x38, 0x3A, 0x42), (0xE4, 0x56, 0x49), (0x50, 0xA1, 0x4F), (0xC1, 0x84, 0x01),
                (0x40, 0x78, 0xF2), (0xA6, 0x26, 0xA4), (0x01, 0x84, 0xBC), (0xA0, 0xA1, 0xA7),
                (0x90, 0x92, 0x97), (0xE4, 0x56, 0x49), (0x50, 0xA1, 0x4F), (0xC1, 0x84, 0x01),
                (0x40, 0x78, 0xF2), (0xA6, 0x26, 0xA4), (0x01, 0x84, 0xBC), (0x38, 0x3A, 0x42),
            ],
            accent: (0x40, 0x78, 0xF2),
        })
    }

    pub fn solarized_light() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0xFD, 0xF6, 0xE3),
            foreground: (0x58, 0x6E, 0x75),
            cursor: Some((0x58, 0x6E, 0x75)),
            ansi: [
                (0x07, 0x36, 0x42), (0xDC, 0x32, 0x2F), (0x85, 0x99, 0x00), (0xB5, 0x89, 0x00),
                (0x26, 0x8B, 0xD2), (0xD3, 0x36, 0x82), (0x2A, 0xA1, 0x98), (0xEE, 0xE8, 0xD5),
                (0x00, 0x2B, 0x36), (0xCB, 0x4B, 0x16), (0x58, 0x6E, 0x75), (0x65, 0x7B, 0x83),
                (0x83, 0x94, 0x96), (0x6C, 0x71, 0xC4), (0x93, 0xA1, 0xA1), (0xFD, 0xF6, 0xE3),
            ],
            accent: (0x26, 0x8B, 0xD2),
        })
    }

    pub fn kanagawa() -> Self {
        Self::from_spec(&ThemeSpec {
            background: (0x1F, 0x1F, 0x28),
            foreground: (0xDC, 0xD7, 0xBA),
            cursor: Some((0xDC, 0xD7, 0xBA)),
            ansi: [
                (0x09, 0x06, 0x18), (0xC3, 0x40, 0x43), (0x76, 0x94, 0x6A), (0xC0, 0xA3, 0x6E),
                (0x7E, 0x9C, 0xD8), (0x95, 0x7F, 0xB8), (0x6A, 0x95, 0x89), (0xC8, 0xC0, 0x93),
                (0x72, 0x71, 0x69), (0xE8, 0x24, 0x24), (0x98, 0xBB, 0x6C), (0xE6, 0xC3, 0x84),
                (0x7F, 0xB4, 0xCA), (0x93, 0x8A, 0xA9), (0xA3, 0xD4, 0xD5), (0xDC, 0xD7, 0xBA),
            ],
            accent: (0x7E, 0x9C, 0xD8),
        })
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

    #[test]
    fn every_builtin_theme_resolves_with_distinct_colors() {
        for name in crate::config::builtin_theme_names() {
            let t = Theme::from_name(name);
            assert_ne!(
                t.foreground, t.background,
                "theme {name} resolved to a blank palette"
            );
            assert!(t.opacity == 1.0);
        }
    }

    #[test]
    fn chrome_derivation_is_lightness_aware() {
        // Dark families: surfaces lighten away from the background.
        let dracula = Theme::from_name("dracula");
        assert!(dracula.surface.l > dracula.background.l);
        assert!(dracula.border.l > dracula.surface.l);
        // Light families: surfaces darken below the background instead of
        // inheriting the old hard-coded dark grays.
        let latte = Theme::from_name("catppuccin-latte");
        assert!(latte.surface.l < latte.background.l);
        assert!(latte.border.l < latte.surface.l);
        assert!(latte.surface.l > 0.5, "light theme must keep light surfaces");
    }

    #[test]
    fn theme_file_light_palette_gets_light_chrome() {
        let tf = crate::config::ThemeFile {
            background: "#fafafa".to_string(),
            foreground: "#383a42".to_string(),
            ..Default::default()
        };
        let t = Theme::from_theme_file(&tf);
        assert!(t.surface.l < t.background.l);
        assert!(t.surface.l > 0.5);
    }
}
