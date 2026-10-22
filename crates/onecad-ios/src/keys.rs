//! USB HID keyboard usages (`UIKeyboardHIDUsage`, page 0x07) to egui keys, and
//! the ABI modifier mask to egui modifiers. ⌘ is egui's `command` on Apple
//! platforms, exactly as winit reports it on macOS.
use crate::ffi::{MOD_COMMAND, MOD_CONTROL, MOD_OPTION, MOD_SHIFT};
use egui::Key;

const LETTERS: [Key; 26] = [
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::R,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
];

/// HID 0x1E..=0x27 are 1..9 then 0.
const DIGITS: [Key; 10] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
    Key::Num0,
];

const FUNCTION: [Key; 12] = [
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
    Key::F9,
    Key::F10,
    Key::F11,
    Key::F12,
];

pub fn from_hid(usage: u32) -> Option<Key> {
    let index = |base: u32| usize::try_from(usage - base).ok();
    match usage {
        0x04..=0x1D => LETTERS.get(index(0x04)?).copied(),
        0x1E..=0x27 => DIGITS.get(index(0x1E)?).copied(),
        0x3A..=0x45 => FUNCTION.get(index(0x3A)?).copied(),
        _ => named(usage),
    }
}

fn named(usage: u32) -> Option<Key> {
    Some(match usage {
        0x28 | 0x58 => Key::Enter,
        0x29 => Key::Escape,
        0x2A => Key::Backspace,
        0x2B => Key::Tab,
        0x2C => Key::Space,
        0x2D | 0x56 => Key::Minus,
        0x2E => Key::Equals,
        0x2F => Key::OpenBracket,
        0x30 => Key::CloseBracket,
        0x31 => Key::Backslash,
        0x33 => Key::Semicolon,
        0x34 => Key::Quote,
        0x35 => Key::Backtick,
        0x36 => Key::Comma,
        0x37 | 0x63 => Key::Period,
        0x38 | 0x54 => Key::Slash,
        0x49 => Key::Insert,
        0x4A => Key::Home,
        0x4B => Key::PageUp,
        0x4C => Key::Delete,
        0x4D => Key::End,
        0x4E => Key::PageDown,
        0x4F => Key::ArrowRight,
        0x50 => Key::ArrowLeft,
        0x51 => Key::ArrowDown,
        0x52 => Key::ArrowUp,
        0x57 => Key::Plus,
        _ => return None,
    })
}

pub fn modifiers(mask: u32) -> egui::Modifiers {
    let command = mask & MOD_COMMAND != 0;
    egui::Modifiers {
        alt: mask & MOD_OPTION != 0,
        ctrl: mask & MOD_CONTROL != 0,
        shift: mask & MOD_SHIFT != 0,
        mac_cmd: command,
        command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hid_usages_map_to_the_keys_onecad_shortcuts_use() {
        assert_eq!(from_hid(0x04), Some(Key::A));
        assert_eq!(from_hid(0x16), Some(Key::S));
        assert_eq!(from_hid(0x1D), Some(Key::Z));
        assert_eq!(from_hid(0x1E), Some(Key::Num1));
        assert_eq!(from_hid(0x27), Some(Key::Num0));
        assert_eq!(from_hid(0x29), Some(Key::Escape));
        assert_eq!(from_hid(0x3A), Some(Key::F1));
        assert_eq!(from_hid(0x45), Some(Key::F12));
        assert_eq!(from_hid(0xE3), None, "modifier keys are not egui keys");
        let cmd_shift = modifiers(MOD_COMMAND | MOD_SHIFT);
        assert!(cmd_shift.command && cmd_shift.mac_cmd && cmd_shift.shift);
        assert!(!modifiers(MOD_CONTROL).command);
    }
}
