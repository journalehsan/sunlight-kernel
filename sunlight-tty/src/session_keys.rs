//! Debounce the two native session chords before foreground input routing.
//! Call only for hardware-driver events authenticated by the TTY service.
#[derive(Default)]
pub struct SessionKeys {
    down: [bool; 2],
    consumed: [bool; 2],
}

impl SessionKeys {
    /// True means suppress a repeated chord or its release. The first Ctrl+Fn
    /// press is left for the existing session transition handler.
    pub fn suppress(&mut self, key: u8, pressed: bool, ctrl: bool) -> bool {
        let index = match key {
            0x3b => 0,
            0x3c => 1,
            _ => return false,
        };
        let was_down = self.down[index];
        self.down[index] = pressed;
        if !pressed {
            let consumed = self.consumed[index];
            self.consumed[index] = false;
            return consumed;
        }
        if self.consumed[index] {
            return true;
        }
        if ctrl {
            if was_down {
                // Its original bare press reached the application. Suppress
                // this repeat without swallowing that application's release.
                return true;
            }
            self.consumed[index] = true;
        }
        false
    }
}

/// Decode local line-editor input only while native TTY owns the seat.
/// Forwarding a desktop event must never also execute it in a background shell.
pub fn native_ascii(tty_active: bool, packed: u64) -> Option<u8> {
    if !tty_active || sunlight_ipc::is_text_commit(packed) {
        return None;
    }
    let (_, pressed, _, ctrl, _, _, ascii) = sunlight_ipc::unpack_key_event(packed);
    if pressed && !ctrl {
        ascii
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_typing_never_reaches_preserved_native_line_editor() {
        use sunlight_ipc::{pack_key_event, pack_text_commit};
        let text = pack_key_event(0x12, true, false, false, false, false, Some(b'e'));
        let enter = pack_key_event(0x1c, true, false, false, false, false, Some(b'\n'));
        let control = pack_key_event(0x14, true, false, true, false, false, Some(b't'));
        let release = pack_key_event(0x12, false, false, false, false, false, None);
        for event in [text, enter, control, release, pack_text_commit('e')] {
            assert_eq!(native_ascii(false, event), None);
        }
        assert_eq!(native_ascii(true, text), Some(b'e'));
        assert_eq!(native_ascii(true, enter), Some(b'\n'));
        for event in [control, release, pack_text_commit('e')] {
            assert_eq!(native_ascii(true, event), None);
        }
    }

    #[test]
    fn plain_function_keys_pass_through() {
        let mut keys = SessionKeys::default();
        for key in [0x3b, 0x3c] {
            assert!(!keys.suppress(key, true, false));
            assert!(!keys.suppress(key, false, false));
        }
    }

    #[test]
    fn chord_repeats_and_release_never_reach_new_foreground() {
        let mut keys = SessionKeys::default();
        for _ in 0..20 {
            for key in [0x3b, 0x3c] {
                assert!(!keys.suppress(key, true, true));
                assert!(keys.suppress(key, true, true));
                assert!(keys.suppress(key, true, false));
                assert!(keys.suppress(key, false, false));
            }
        }
    }

    #[test]
    fn pressing_ctrl_after_held_function_key_does_not_switch() {
        let mut keys = SessionKeys::default();
        assert!(!keys.suppress(0x3b, true, false));
        assert!(keys.suppress(0x3b, true, true));
        assert!(!keys.suppress(0x3b, false, true));
    }
}
