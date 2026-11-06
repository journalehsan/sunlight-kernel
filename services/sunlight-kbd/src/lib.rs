#![no_std]
/// PS/2 Scancode Set 1 keycodes
mod keycode {
    pub const A: u8 = 0x1E;
    pub const B: u8 = 0x30;
    pub const C: u8 = 0x2E;
    pub const D: u8 = 0x20;
    pub const E: u8 = 0x12;
    pub const F: u8 = 0x21;
    pub const G: u8 = 0x22;
    pub const H: u8 = 0x23;
    pub const I: u8 = 0x17;
    pub const J: u8 = 0x24;
    pub const K: u8 = 0x25;
    pub const L: u8 = 0x26;
    pub const M: u8 = 0x32;
    pub const N: u8 = 0x31;
    pub const O: u8 = 0x18;
    pub const P: u8 = 0x19;
    pub const Q: u8 = 0x10;
    pub const R: u8 = 0x13;
    pub const S: u8 = 0x1F;
    pub const T: u8 = 0x14;
    pub const U: u8 = 0x16;
    pub const V: u8 = 0x2F;
    pub const W: u8 = 0x11;
    pub const X: u8 = 0x2D;
    pub const Y: u8 = 0x15;
    pub const Z: u8 = 0x2C;
    pub const NUM0: u8 = 0x0B;
    pub const NUM1: u8 = 0x02;
    pub const NUM2: u8 = 0x03;
    pub const NUM3: u8 = 0x04;
    pub const NUM4: u8 = 0x05;
    pub const NUM5: u8 = 0x06;
    pub const NUM6: u8 = 0x07;
    pub const NUM7: u8 = 0x08;
    pub const NUM8: u8 = 0x09;
    pub const NUM9: u8 = 0x0A;
    pub const ENTER: u8 = 0x1C;
    pub const ESC: u8 = 0x01;
    pub const BACKSPACE: u8 = 0x0E;
    pub const TAB: u8 = 0x0F;
    pub const SPACE: u8 = 0x39;
    pub const MINUS: u8 = 0x0C;
    pub const EQUALS: u8 = 0x0D;
    pub const LEFT_BRACKET: u8 = 0x1A;
    pub const RIGHT_BRACKET: u8 = 0x1B;
    pub const BACKSLASH: u8 = 0x2B;
    pub const SEMICOLON: u8 = 0x27;
    pub const QUOTE: u8 = 0x28;
    pub const TILDE: u8 = 0x29;
    pub const COMMA: u8 = 0x33;
    pub const PERIOD: u8 = 0x34;
    pub const SLASH: u8 = 0x35;
    pub const LEFT_CTRL: u8 = 0x1D;
    pub const LEFT_SHIFT: u8 = 0x2A;
    pub const LEFT_ALT: u8 = 0x38;
    pub const RIGHT_SHIFT: u8 = 0x36;
    pub const LEFT_SUPER: u8 = 0x5B;
    pub const RIGHT_SUPER: u8 = 0x5C;
}

/// Modifier state tracker
#[derive(Default, Clone, Copy)]
struct Modifiers {
    sides: u8,
    ctrl: bool,
    alt: bool,
    shift: bool,
    super_key: bool,
}

/// Translate PS/2 scancode to ASCII (US QWERTY layout)
fn scancode_to_ascii(scancode: u8, mods: &Modifiers) -> Option<u8> {
    let base: u8 = match scancode {
        keycode::A => b'a',
        keycode::B => b'b',
        keycode::C => b'c',
        keycode::D => b'd',
        keycode::E => b'e',
        keycode::F => b'f',
        keycode::G => b'g',
        keycode::H => b'h',
        keycode::I => b'i',
        keycode::J => b'j',
        keycode::K => b'k',
        keycode::L => b'l',
        keycode::M => b'm',
        keycode::N => b'n',
        keycode::O => b'o',
        keycode::P => b'p',
        keycode::Q => b'q',
        keycode::R => b'r',
        keycode::S => b's',
        keycode::T => b't',
        keycode::U => b'u',
        keycode::V => b'v',
        keycode::W => b'w',
        keycode::X => b'x',
        keycode::Y => b'y',
        keycode::Z => b'z',
        keycode::NUM1 => b'1',
        keycode::NUM2 => b'2',
        keycode::NUM3 => b'3',
        keycode::NUM4 => b'4',
        keycode::NUM5 => b'5',
        keycode::NUM6 => b'6',
        keycode::NUM7 => b'7',
        keycode::NUM8 => b'8',
        keycode::NUM9 => b'9',
        keycode::NUM0 => b'0',
        keycode::MINUS => b'-',
        keycode::EQUALS => b'=',
        keycode::LEFT_BRACKET => b'[',
        keycode::RIGHT_BRACKET => b']',
        keycode::BACKSLASH => b'\\',
        keycode::SEMICOLON => b';',
        keycode::QUOTE => b'\'',
        keycode::TILDE => b'`',
        keycode::COMMA => b',',
        keycode::PERIOD => b'.',
        keycode::SLASH => b'/',
        keycode::SPACE => b' ',
        keycode::TAB => b'\t',
        keycode::ENTER => b'\n',
        keycode::ESC => 0x1b,
        keycode::BACKSPACE => 0x08,
        _ => return None,
    };

    if base >= b'a' && base <= b'z' {
        if mods.shift {
            Some(base - 32)
        } else {
            Some(base)
        }
    } else if base >= b'1' && base <= b'9' && mods.shift {
        let shifted = match base {
            b'1' => b'!',
            b'2' => b'@',
            b'3' => b'#',
            b'4' => b'$',
            b'5' => b'%',
            b'6' => b'^',
            b'7' => b'&',
            b'8' => b'*',
            b'9' => b'(',
            _ => base,
        };
        Some(shifted)
    } else if base == b'0' && mods.shift {
        Some(b')')
    } else if mods.shift {
        let shifted = match base {
            b'-' => b'_',
            b'=' => b'+',
            b'[' => b'{',
            b']' => b'}',
            b'\\' => b'|',
            b';' => b':',
            b'\'' => b'"',
            b'`' => b'~',
            b',' => b'<',
            b'.' => b'>',
            b'/' => b'?',
            _ => base,
        };
        Some(shifted)
    } else {
        Some(base)
    }
}

/// Native scalar composition: Ctrl+Shift+U, one to six hex digits, Enter.
/// Preedit stays private here; Escape cancels and Backspace removes a digit.
/// No partial text reaches an application or diagnostic output.
#[derive(Default)]
pub struct Keyboard {
    mods: Modifiers,
    composition: Option<(u32, u8)>,
}
impl Keyboard {
    pub fn process(&mut self, scancode: u8, extended: bool) -> (u64, Option<u64>) {
        let pressed = scancode & 0x80 == 0;
        let code = scancode & 0x7f;
        let side = match (extended, code) {
            (false, 0x2a) => 1,
            (false, 0x36) => 2,
            (false, 0x1d) => 4,
            (true, 0x1d) => 8,
            (false, 0x38) => 16,
            (true, 0x38) => 32,
            (true, 0x5b) => 64,
            (true, 0x5c) => 128,
            _ => 0,
        };
        if pressed {
            self.mods.sides |= side;
        } else {
            self.mods.sides &= !side;
        }
        self.mods.shift = self.mods.sides & 3 != 0;
        self.mods.ctrl = self.mods.sides & 12 != 0;
        self.mods.alt = self.mods.sides & 48 != 0;
        self.mods.super_key = self.mods.sides & 192 != 0;
        let mut ascii = if pressed && !extended {
            scancode_to_ascii(code, &self.mods)
        } else {
            None
        };
        let mut commit = None;
        let was_composing = self.composition.is_some();
        // Preserve every physical transition, including releases during preedit.
        if pressed && code == 0x16 && self.mods.ctrl && self.mods.shift {
            self.composition = Some((0, 0));
            ascii = None;
        } else if let Some((value, digits)) = self.composition {
            ascii = None;
            if pressed {
                match code {
                    0x01 => self.composition = None,
                    0x0e => self.composition = Some((value >> 4, digits.saturating_sub(1))),
                    0x1c | 0x39 => {
                        commit = char::from_u32(value)
                            .filter(|c| digits > 0 && !c.is_control())
                            .map(sunlight_ipc::pack_text_commit);
                        self.composition = None;
                    }
                    _ => {
                        if let Some(digit) = scancode_to_ascii(code, &Modifiers::default())
                            .and_then(|c| (c as char).to_digit(16))
                        {
                            if digits < 6 {
                                self.composition = Some(((value << 4) | digit, digits + 1));
                            }
                        }
                    }
                }
            }
        }
        // Printable input commits separately from physical identifiers. Legacy
        // control payloads keep existing TTY/application command behavior.
        if !self.mods.ctrl && !self.mods.alt && !self.mods.super_key {
            if let Some(c) = ascii.filter(|c| *c >= 0x20) {
                commit = Some(sunlight_ipc::pack_text_commit(c as char));
                // ASCII remains on physical events for old TTY consumers; the
                // native UI recognizes bit 56 and uses only the commit event.
            }
        }
        let mut raw = sunlight_ipc::pack_key_event(
            code,
            pressed,
            self.mods.shift,
            self.mods.ctrl,
            self.mods.alt,
            self.mods.super_key,
            ascii,
        );
        if extended {
            raw |= sunlight_ipc::KEY_EXTENDED;
        }
        raw |= sunlight_ipc::KEY_SEPARATE_TEXT;
        // Composition command keys must not execute editing/submission defaults.
        if side == 0 && (was_composing || self.composition.is_some()) {
            raw |= sunlight_ipc::KEY_COMPOSING;
        }
        (raw, commit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn unicode(k: &mut Keyboard, digits: &[u8]) -> Option<char> {
        k.process(0x1d, false);
        k.process(0x2a, false);
        k.process(0x16, false);
        k.process(0x96, false);
        let shift_up = k.process(0xaa, false).0;
        let ctrl_up = k.process(0x9d, false).0;
        assert_eq!(shift_up & sunlight_ipc::KEY_COMPOSING, 0);
        assert_eq!(ctrl_up & sunlight_ipc::KEY_COMPOSING, 0);
        for &code in digits {
            assert!(k.process(code, false).1.is_none());
            k.process(code | 0x80, false);
        }
        let (raw, commit) = k.process(0x1c, false);
        assert_ne!(raw & (1 << 58), 0, "composition Enter must not submit");
        commit.and_then(sunlight_ipc::unpack_text_commit)
    }
    #[test]
    fn native_unicode_composition_and_invalid_scalars() {
        let mut k = Keyboard::default();
        assert_eq!(unicode(&mut k, &[0x12, 0x0a]), Some('é')); // e9
        assert_eq!(unicode(&mut k, &[0x02, 0x21, 0x07, 0x03, 0x0b]), Some('😠')); // 1f620
        assert_eq!(unicode(&mut k, &[0x20, 0x09, 0x0b, 0x0b]), None); // d800
        assert_eq!(unicode(&mut k, &[0x02, 0x02, 0x0b, 0x0b, 0x0b, 0x0b]), None);
    }
    #[test]
    fn overlapping_modifier_releases_and_shift_tab() {
        let mut k = Keyboard::default();
        k.process(0x2a, false);
        k.process(0x36, false);
        k.process(0xaa, false);
        let (tab, _) = k.process(0x0f, false);
        assert!(sunlight_ipc::unpack_key_event(tab).2);
        k.process(0xb6, false);
        assert!(!sunlight_ipc::unpack_key_event(k.process(0x0f, false).0).2);
        for code in [0x1d, 0x38, 0x5b] {
            k.process(code, code == 0x5b);
            k.process(if code == 0x5b { 0x5c } else { code }, true);
            let (release, _) = k.process(code | 0x80, code == 0x5b);
            let (_, _, _, ctrl, alt, meta, _) = sunlight_ipc::unpack_key_event(release);
            assert!(match code {
                0x1d => ctrl,
                0x38 => alt,
                _ => meta,
            });
        }
    }
    #[test]
    fn composition_cancel_and_backspace_are_private() {
        let mut k = Keyboard::default();
        k.process(0x1d, false);
        k.process(0x2a, false);
        k.process(0x16, false);
        k.process(0x9d, false);
        k.process(0xaa, false);
        assert!(k.process(0x12, false).1.is_none());
        assert!(k.process(0x0e, false).1.is_none());
        let (escape, commit) = k.process(0x01, false);
        assert!(commit.is_none());
        assert_ne!(escape & (1 << 58), 0);
        assert_eq!(
            k.process(0x1e, false)
                .1
                .and_then(sunlight_ipc::unpack_text_commit),
            Some('a')
        );
    }
}
