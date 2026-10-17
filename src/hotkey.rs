//! Global hotkey parsing/registration helpers.

use global_hotkey::hotkey::{Code, HotKey, Modifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeySpec {
    pub modifiers: Modifiers,
    pub code: Code,
}

impl HotkeySpec {
    pub fn to_hotkey(self) -> HotKey {
        HotKey::new(Some(self.modifiers), self.code)
    }
}

pub const PRESETS: &[&str] = &[
    "Ctrl+Alt+A",
    "Ctrl+Shift+A",
    "Ctrl+Alt+S",
    "Ctrl+Alt+Z",
    "Alt+Shift+A",
    "Ctrl+F1",
    "F1",
    "F9",
    "PrintScreen",
];

pub fn is_valid(text: &str) -> bool {
    parse(text).is_some()
}

pub fn parse(text: &str) -> Option<HotkeySpec> {
    let mut modifiers = Modifiers::empty();
    let mut code: Option<Code> = None;
    let parts: Vec<&str> = text.split('+').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    for (i, part) in parts.iter().enumerate() {
        let lower = part.to_ascii_lowercase();
        let is_last = i + 1 == parts.len();
        match lower.as_str() {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "alt" | "option" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            "win" | "super" | "meta" | "cmd" => modifiers |= Modifiers::SUPER,
            _ => {
                if !is_last || code.is_some() {
                    return None;
                }
                code = Some(parse_code(part)?);
            }
        }
    }
    let code = code?;
    // A bare modifier row is useless as a capture shortcut.
    if modifiers.is_empty() && !matches!(code, Code::PrintScreen | Code::F1 | Code::F9) {
        // Allow bare function keys only.
        if !matches!(
            code,
            Code::F1
                | Code::F2
                | Code::F3
                | Code::F4
                | Code::F5
                | Code::F6
                | Code::F7
                | Code::F8
                | Code::F9
                | Code::F10
                | Code::F11
                | Code::F12
        ) {
            return None;
        }
    }
    Some(HotkeySpec { modifiers, code })
}

fn parse_code(token: &str) -> Option<Code> {
    let t = token.trim();
    let upper = t.to_ascii_uppercase();
    // Single letters / digits.
    if t.chars().count() == 1 {
        let c = t.chars().next().unwrap().to_ascii_uppercase();
        if c.is_ascii_alphabetic() {
            return letter_code(c);
        }
        if c.is_ascii_digit() {
            return digit_code(c);
        }
    }
    if upper.starts_with('F') && upper.len() <= 3 {
        if let Ok(n) = upper[1..].parse::<u8>() {
            if (1..=24).contains(&n) {
                return Some(FUNCTION_KEYS[(n - 1) as usize]);
            }
        }
    }
    let named = match upper.as_str() {
        "PRINTSCREEN" | "PRTSC" | "PRTSCN" => Code::PrintScreen,
        "SPACE" => Code::Space,
        "ENTER" | "RETURN" => Code::Enter,
        "TAB" => Code::Tab,
        "BACKSPACE" => Code::Backspace,
        "INSERT" | "INS" => Code::Insert,
        "DELETE" | "DEL" => Code::Delete,
        "HOME" => Code::Home,
        "END" => Code::End,
        "PAGEUP" | "PGUP" => Code::PageUp,
        "PAGEDOWN" | "PGDN" => Code::PageDown,
        "UP" | "ARROWUP" => Code::ArrowUp,
        "DOWN" | "ARROWDOWN" => Code::ArrowDown,
        "LEFT" | "ARROWLEFT" => Code::ArrowLeft,
        "RIGHT" | "ARROWRIGHT" => Code::ArrowRight,
        "MINUS" => Code::Minus,
        "EQUAL" => Code::Equal,
        "QUOTE" => Code::Quote,
        "SEMICOLON" => Code::Semicolon,
        "COMMA" => Code::Comma,
        "PERIOD" => Code::Period,
        "SLASH" => Code::Slash,
        "BACKSLASH" => Code::Backslash,
        "BACKQUOTE" => Code::Backquote,
        "BRACKETLEFT" => Code::BracketLeft,
        "BRACKETRIGHT" => Code::BracketRight,
        _ => return None,
    };
    Some(named)
}

const FUNCTION_KEYS: [Code; 24] = [
    Code::F1,
    Code::F2,
    Code::F3,
    Code::F4,
    Code::F5,
    Code::F6,
    Code::F7,
    Code::F8,
    Code::F9,
    Code::F10,
    Code::F11,
    Code::F12,
    Code::F13,
    Code::F14,
    Code::F15,
    Code::F16,
    Code::F17,
    Code::F18,
    Code::F19,
    Code::F20,
    Code::F21,
    Code::F22,
    Code::F23,
    Code::F24,
];

fn letter_code(c: char) -> Option<Code> {
    let table: [Code; 26] = [
        Code::KeyA,
        Code::KeyB,
        Code::KeyC,
        Code::KeyD,
        Code::KeyE,
        Code::KeyF,
        Code::KeyG,
        Code::KeyH,
        Code::KeyI,
        Code::KeyJ,
        Code::KeyK,
        Code::KeyL,
        Code::KeyM,
        Code::KeyN,
        Code::KeyO,
        Code::KeyP,
        Code::KeyQ,
        Code::KeyR,
        Code::KeyS,
        Code::KeyT,
        Code::KeyU,
        Code::KeyV,
        Code::KeyW,
        Code::KeyX,
        Code::KeyY,
        Code::KeyZ,
    ];
    let idx = (c as u8).wrapping_sub(b'A') as usize;
    table.get(idx).copied()
}

fn digit_code(c: char) -> Option<Code> {
    let table: [Code; 10] = [
        Code::Digit0,
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ];
    let idx = (c as u8).wrapping_sub(b'0') as usize;
    table.get(idx).copied()
}

/// Human readable, canonical form, e.g. `Ctrl+Alt+A`.
pub fn format(spec: HotkeySpec) -> String {
    let mut out = String::new();
    if spec.modifiers.contains(Modifiers::CONTROL) {
        out.push_str("Ctrl+");
    }
    if spec.modifiers.contains(Modifiers::ALT) {
        out.push_str("Alt+");
    }
    if spec.modifiers.contains(Modifiers::SHIFT) {
        out.push_str("Shift+");
    }
    if spec.modifiers.contains(Modifiers::SUPER) {
        out.push_str("Win+");
    }
    out.push_str(&code_name(spec.code));
    out
}

pub fn code_name(code: Code) -> String {
    let letters = [
        Code::KeyA,
        Code::KeyB,
        Code::KeyC,
        Code::KeyD,
        Code::KeyE,
        Code::KeyF,
        Code::KeyG,
        Code::KeyH,
        Code::KeyI,
        Code::KeyJ,
        Code::KeyK,
        Code::KeyL,
        Code::KeyM,
        Code::KeyN,
        Code::KeyO,
        Code::KeyP,
        Code::KeyQ,
        Code::KeyR,
        Code::KeyS,
        Code::KeyT,
        Code::KeyU,
        Code::KeyV,
        Code::KeyW,
        Code::KeyX,
        Code::KeyY,
        Code::KeyZ,
    ];
    if let Some(i) = letters.iter().position(|c| *c == code) {
        return ((b'A' + i as u8) as char).to_string();
    }
    let digits = [
        Code::Digit0,
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ];
    if let Some(i) = digits.iter().position(|c| *c == code) {
        return ((b'0' + i as u8) as char).to_string();
    }
    if let Some(i) = FUNCTION_KEYS.iter().position(|c| *c == code) {
        return format!("F{}", i + 1);
    }
    match code {
        Code::PrintScreen => "PrintScreen".into(),
        Code::Space => "Space".into(),
        Code::Enter => "Enter".into(),
        Code::Tab => "Tab".into(),
        Code::Insert => "Insert".into(),
        Code::Delete => "Delete".into(),
        Code::Home => "Home".into(),
        Code::End => "End".into(),
        Code::PageUp => "PageUp".into(),
        Code::PageDown => "PageDown".into(),
        Code::ArrowUp => "Up".into(),
        Code::ArrowDown => "Down".into(),
        Code::ArrowLeft => "Left".into(),
        Code::ArrowRight => "Right".into(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_presets() {
        for p in PRESETS {
            assert!(parse(p).is_some(), "preset should parse: {p}");
        }
    }

    #[test]
    fn roundtrips() {
        let spec = parse("Ctrl+Alt+A").unwrap();
        assert_eq!(format(spec), "Ctrl+Alt+A");
        let spec = parse("PrintScreen").unwrap();
        assert_eq!(format(spec), "PrintScreen");
        let spec = parse("F9").unwrap();
        assert_eq!(format(spec), "F9");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("").is_none());
        assert!(parse("Ctrl+Alt").is_none());
        assert!(parse("Hyper+Q").is_none());
        assert!(parse("Ctrl+Alt+QQ").is_none());
    }
}
