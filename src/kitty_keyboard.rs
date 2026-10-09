use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

#[derive(Clone, Copy, Debug)]
pub struct PressedKey {
    code: u32,
    alternate: Option<u32>,
    modifiers: u16,
}

pub fn encode_key_down(
    keystroke: &Keystroke,
    mode: TermMode,
    repeat: bool,
) -> Option<(Vec<u8>, Option<PressedKey>)> {
    let modifiers = modifier_code(keystroke);
    let disambiguate = mode.contains(TermMode::DISAMBIGUATE_ESC_CODES);
    let report_events = mode.contains(TermMode::REPORT_EVENT_TYPES);
    let report_alternates = mode.contains(TermMode::REPORT_ALTERNATE_KEYS);
    let all_as_escape = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let report_text = mode.contains(TermMode::REPORT_ASSOCIATED_TEXT) && all_as_escape;
    let key_generates_text = keystroke.key_char.is_some() && !is_functional_key(&keystroke.key);

    let should_encode =
        all_as_escape || (report_events && !key_generates_text) || (disambiguate && modifiers > 1);
    if !should_encode {
        return None;
    }
    let code = key_code(&keystroke.key, keystroke.key_char.as_deref())
        .or_else(|| (all_as_escape && key_generates_text).then_some(0))?;
    let alternate = (report_alternates && keystroke.modifiers.shift)
        .then(|| keystroke.key_char.as_deref())
        .flatten()
        .filter(|text| text.chars().count() == 1)
        .and_then(|text| text.chars().next())
        .map(u32::from)
        .filter(|alternate| *alternate != code);

    let event = if report_events {
        if repeat {
            2
        } else {
            1
        }
    } else {
        0
    };
    let can_report_release = all_as_escape
        || !matches!(
            keystroke.key.to_ascii_lowercase().as_str(),
            "enter" | "return" | "tab" | "backspace"
        );
    let text = report_text
        .then_some(keystroke.key_char.as_deref())
        .flatten();
    Some((
        encode(code, alternate, modifiers, event, text),
        (report_events && can_report_release).then_some(PressedKey {
            code,
            alternate,
            modifiers,
        }),
    ))
}

pub fn encode_key_up(key: PressedKey) -> Vec<u8> {
    encode(key.code, key.alternate, key.modifiers, 3, None)
}

fn encode(
    code: u32,
    alternate: Option<u32>,
    modifiers: u16,
    event: u8,
    text: Option<&str>,
) -> Vec<u8> {
    let text = text
        .map(|text| {
            text.chars()
                .filter(|c| !c.is_control())
                .map(|c| u32::from(c).to_string())
                .collect::<Vec<_>>()
                .join(":")
        })
        .filter(|text| !text.is_empty());
    let code = alternate.map_or_else(|| code.to_string(), |alt| format!("{code}:{alt}"));
    let sequence = if event == 0 {
        if modifiers == 1 {
            match text {
                Some(text) => format!("\x1b[{code};1;{text}u"),
                None => format!("\x1b[{code}u"),
            }
        } else {
            match text {
                Some(text) => format!("\x1b[{code};{modifiers};{text}u"),
                None => format!("\x1b[{code};{modifiers}u"),
            }
        }
    } else {
        match text {
            Some(text) => format!("\x1b[{code};{modifiers}:{event};{text}u"),
            None => format!("\x1b[{code};{modifiers}:{event}u"),
        }
    };
    sequence.into_bytes()
}

fn modifier_code(keystroke: &Keystroke) -> u16 {
    1 + u16::from(keystroke.modifiers.shift)
        + 2 * u16::from(keystroke.modifiers.alt)
        + 4 * u16::from(keystroke.modifiers.control)
        + 8 * u16::from(keystroke.modifiers.platform)
}

fn key_code(key: &str, key_char: Option<&str>) -> Option<u32> {
    let lower = key.to_ascii_lowercase();
    let special = match lower.as_str() {
        "escape" | "esc" => Some(27),
        "enter" | "return" => Some(13),
        "tab" => Some(9),
        "backspace" => Some(127),
        "insert" => Some(57348),
        "delete" | "del" => Some(57349),
        "left" | "arrowleft" => Some(57350),
        "right" | "arrowright" => Some(57351),
        "up" | "arrowup" => Some(57352),
        "down" | "arrowdown" => Some(57353),
        "pageup" => Some(57354),
        "pagedown" => Some(57355),
        "home" => Some(57356),
        "end" => Some(57357),
        "capslock" => Some(57358),
        "numlock" => Some(57360),
        "printscreen" => Some(57361),
        "pause" => Some(57362),
        "menu" => Some(57363),
        "shift" => Some(57441),
        "control" | "ctrl" => Some(57442),
        "alt" => Some(57443),
        "super" | "command" | "cmd" => Some(57444),
        _ => function_key_code(&lower),
    };
    special.or_else(|| {
        (key.chars().count() == 1)
            .then_some(lower.as_str())
            .or_else(|| key_char.filter(|text| text.chars().count() == 1))
            .and_then(|text| text.chars().next())
            .map(u32::from)
    })
}

fn function_key_code(key: &str) -> Option<u32> {
    let number = key.strip_prefix('f')?.parse::<u32>().ok()?;
    (1..=35).contains(&number).then_some(57363 + number)
}

fn is_functional_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "escape"
            | "esc"
            | "enter"
            | "return"
            | "tab"
            | "backspace"
            | "insert"
            | "delete"
            | "del"
            | "left"
            | "arrowleft"
            | "right"
            | "arrowright"
            | "up"
            | "arrowup"
            | "down"
            | "arrowdown"
            | "pageup"
            | "pagedown"
            | "home"
            | "end"
            | "capslock"
            | "numlock"
            | "printscreen"
            | "pause"
            | "menu"
            | "shift"
            | "control"
            | "ctrl"
            | "alt"
            | "super"
            | "command"
            | "cmd"
    ) || function_key_code(&key).is_some()
}
