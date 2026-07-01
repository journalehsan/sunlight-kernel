//! No-alloc VT100 / ANSI escape sequence parser.
//!
//! State machine that reads one byte at a time and produces discrete output
//! events. Designed for embedded use — zero heap, fixed parameter buffer.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VtOutput {
    Char(u8),
    MoveCursor { row: i16, col: i16 },
    SetCursor { row: u16, col: u16 },
    ClearScreen { mode: u16 },
    ClearLine { mode: u16 },
    Sgr { params: [u16; 8], count: usize },
    DecPrivateMode { mode: u16, enabled: bool },
    SaveCursor,
    RestoreCursor,
    CarriageReturn,
    Newline,
    Backspace,
    Tab,
    Bell,
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VtState {
    Ground,
    Escape,
    Csi,
}

pub struct Vt100Parser {
    state: VtState,
    params: [u16; 8],
    param_count: usize,
    current_param: u16,
    has_digit: bool,
    private_mode: bool,
}

impl Vt100Parser {
    pub const fn new() -> Self {
        Self {
            state: VtState::Ground,
            params: [0; 8],
            param_count: 0,
            current_param: 0,
            has_digit: false,
            private_mode: false,
        }
    }

    /// Feed one byte into the parser. Returns the parsed output event.
    pub fn feed(&mut self, byte: u8) -> VtOutput {
        match self.state {
            VtState::Ground => self.handle_ground(byte),
            VtState::Escape => self.handle_escape(byte),
            VtState::Csi => self.handle_csi(byte),
        }
    }

    fn handle_ground(&mut self, byte: u8) -> VtOutput {
        match byte {
            0x1B => {
                self.state = VtState::Escape;
                VtOutput::Nothing
            }
            b'\r' => VtOutput::CarriageReturn,
            b'\n' => VtOutput::Newline,
            0x08 => VtOutput::Backspace,
            b'\t' => VtOutput::Tab,
            0x07 => VtOutput::Bell,
            _ => VtOutput::Char(byte),
        }
    }

    fn handle_escape(&mut self, byte: u8) -> VtOutput {
        match byte {
            b'[' => {
                self.state = VtState::Csi;
                self.param_count = 0;
                self.current_param = 0;
                self.has_digit = false;
                self.private_mode = false;
                VtOutput::Nothing
            }
            b'7' => {
                self.state = VtState::Ground;
                VtOutput::SaveCursor
            }
            b'8' => {
                self.state = VtState::Ground;
                VtOutput::RestoreCursor
            }
            _ => {
                self.state = VtState::Ground;
                VtOutput::Nothing
            }
        }
    }

    fn handle_csi(&mut self, byte: u8) -> VtOutput {
        match byte {
            b'0'..=b'9' => {
                self.current_param = self
                    .current_param
                    .wrapping_mul(10)
                    .wrapping_add((byte - b'0') as u16);
                self.has_digit = true;
                VtOutput::Nothing
            }
            b';' => {
                self.store_param();
                VtOutput::Nothing
            }
            b'?' => {
                self.private_mode = true;
                VtOutput::Nothing
            }
            b'A' => {
                self.store_param();
                self.state = VtState::Ground;
                let n = self.param(0, 1) as i16;
                VtOutput::MoveCursor { row: -n, col: 0 }
            }
            b'B' => {
                self.store_param();
                self.state = VtState::Ground;
                let n = self.param(0, 1) as i16;
                VtOutput::MoveCursor { row: n, col: 0 }
            }
            b'C' => {
                self.store_param();
                self.state = VtState::Ground;
                let n = self.param(0, 1) as i16;
                VtOutput::MoveCursor { row: 0, col: n }
            }
            b'D' => {
                self.store_param();
                self.state = VtState::Ground;
                let n = self.param(0, 1) as i16;
                VtOutput::MoveCursor { row: 0, col: -n }
            }
            b'H' | b'f' => {
                self.store_param();
                self.state = VtState::Ground;
                let row = self.param(0, 1);
                let col = self.param(1, 1);
                VtOutput::SetCursor {
                    row: row.saturating_sub(1),
                    col: col.saturating_sub(1),
                }
            }
            b'J' => {
                self.store_param();
                self.state = VtState::Ground;
                VtOutput::ClearScreen {
                    mode: self.param(0, 0),
                }
            }
            b'K' => {
                self.store_param();
                self.state = VtState::Ground;
                VtOutput::ClearLine {
                    mode: self.param(0, 0),
                }
            }
            b'm' => {
                self.store_param();
                self.state = VtState::Ground;
                VtOutput::Sgr {
                    params: self.params,
                    count: self.param_count,
                }
            }
            b'h' | b'l' => {
                self.store_param();
                self.state = VtState::Ground;
                if self.private_mode {
                    VtOutput::DecPrivateMode {
                        mode: self.param(0, 0),
                        enabled: byte == b'h',
                    }
                } else {
                    VtOutput::Nothing
                }
            }
            b's' => {
                self.store_param();
                self.state = VtState::Ground;
                VtOutput::SaveCursor
            }
            b'u' => {
                self.store_param();
                self.state = VtState::Ground;
                VtOutput::RestoreCursor
            }
            _ => {
                self.state = VtState::Ground;
                VtOutput::Nothing
            }
        }
    }

    fn store_param(&mut self) {
        if self.param_count < 8 {
            self.params[self.param_count] = if self.has_digit {
                self.current_param
            } else {
                0
            };
            self.param_count += 1;
        }
        self.current_param = 0;
        self.has_digit = false;
    }

    fn param(&self, idx: usize, default: u16) -> u16 {
        if idx < self.param_count {
            self.params[idx]
        } else {
            default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_chars_pass_through() {
        let mut p = Vt100Parser::new();
        assert_eq!(p.feed(b'a'), VtOutput::Char(b'a'));
        assert_eq!(p.feed(b'Z'), VtOutput::Char(b'Z'));
    }

    #[test]
    fn newline_and_cr() {
        let mut p = Vt100Parser::new();
        assert_eq!(p.feed(b'\n'), VtOutput::Newline);
        assert_eq!(p.feed(b'\r'), VtOutput::CarriageReturn);
    }

    #[test]
    fn cursor_up() {
        let mut p = Vt100Parser::new();
        assert_eq!(p.feed(0x1B), VtOutput::Nothing);
        assert_eq!(p.feed(b'['), VtOutput::Nothing);
        assert_eq!(p.feed(b'5'), VtOutput::Nothing);
        assert_eq!(p.feed(b'A'), VtOutput::MoveCursor { row: -5, col: 0 });
    }

    #[test]
    fn cursor_home() {
        let mut p = Vt100Parser::new();
        assert_eq!(p.feed(0x1B), VtOutput::Nothing);
        assert_eq!(p.feed(b'['), VtOutput::Nothing);
        assert_eq!(p.feed(b'H'), VtOutput::SetCursor { row: 0, col: 0 });
    }

    #[test]
    fn clear_screen_mode_two() {
        let mut p = Vt100Parser::new();
        assert_eq!(p.feed(0x1B), VtOutput::Nothing);
        assert_eq!(p.feed(b'['), VtOutput::Nothing);
        assert_eq!(p.feed(b'2'), VtOutput::Nothing);
        assert_eq!(p.feed(b'J'), VtOutput::ClearScreen { mode: 2 });
    }

    #[test]
    fn clear_line_mode_two() {
        let mut p = Vt100Parser::new();
        assert_eq!(p.feed(0x1B), VtOutput::Nothing);
        assert_eq!(p.feed(b'['), VtOutput::Nothing);
        assert_eq!(p.feed(b'2'), VtOutput::Nothing);
        assert_eq!(p.feed(b'K'), VtOutput::ClearLine { mode: 2 });
    }

    #[test]
    fn sgr_keeps_multiple_params() {
        let mut p = Vt100Parser::new();
        assert_eq!(p.feed(0x1B), VtOutput::Nothing);
        assert_eq!(p.feed(b'['), VtOutput::Nothing);
        assert_eq!(p.feed(b'3'), VtOutput::Nothing);
        assert_eq!(p.feed(b'1'), VtOutput::Nothing);
        assert_eq!(p.feed(b';'), VtOutput::Nothing);
        assert_eq!(p.feed(b'1'), VtOutput::Nothing);
        assert_eq!(
            p.feed(b'm'),
            VtOutput::Sgr {
                params: [31, 1, 0, 0, 0, 0, 0, 0],
                count: 2,
            }
        );
    }

    #[test]
    fn alt_screen_private_mode_is_reported() {
        let mut p = Vt100Parser::new();
        assert_eq!(p.feed(0x1B), VtOutput::Nothing);
        assert_eq!(p.feed(b'['), VtOutput::Nothing);
        assert_eq!(p.feed(b'?'), VtOutput::Nothing);
        assert_eq!(p.feed(b'1'), VtOutput::Nothing);
        assert_eq!(p.feed(b'0'), VtOutput::Nothing);
        assert_eq!(p.feed(b'4'), VtOutput::Nothing);
        assert_eq!(p.feed(b'9'), VtOutput::Nothing);
        assert_eq!(
            p.feed(b'h'),
            VtOutput::DecPrivateMode {
                mode: 1049,
                enabled: true,
            }
        );
    }
}
