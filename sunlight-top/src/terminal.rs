pub fn write_stdout(buf: &[u8]) {
    if buf.is_empty() {
        return;
    }

    // SAFETY: passing a valid userspace pointer/len to write(1,...); kernel validates pointer range.
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 43u64,
            in("rdi") 1u64,
            in("rsi") buf.as_ptr() as u64,
            in("rdx") buf.len() as u64,
            lateout("rax") _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
}

pub struct Canvas {
    buf: [u8; 4096],
    pos: usize,
}

impl Canvas {
    pub const fn new() -> Self {
        Self {
            buf: [0; 4096],
            pos: 0,
        }
    }

    pub fn clear(&mut self) {
        self.pos = 0;
    }

    pub fn push(&mut self, b: u8) {
        if self.pos >= self.buf.len() {
            self.flush();
        }
        if self.pos < self.buf.len() {
            self.buf[self.pos] = b;
            self.pos += 1;
        }
    }

    pub fn push_str(&mut self, s: &str) {
        self.push_bytes(s.as_bytes());
    }

    pub fn push_bytes(&mut self, b: &[u8]) {
        for &byte in b {
            self.push(byte);
        }
    }

    pub fn push_u64(&mut self, mut v: u64) {
        if v == 0 {
            self.push(b'0');
            return;
        }
        let mut tmp = [0u8; 20];
        let mut i = tmp.len();
        while v > 0 {
            i -= 1;
            tmp[i] = b'0' + (v % 10) as u8;
            v /= 10;
        }
        self.push_bytes(&tmp[i..]);
    }

    pub fn flush(&mut self) {
        if self.pos > 0 {
            write_stdout(&self.buf[..self.pos]);
            self.pos = 0;
        }
    }

    pub fn enter_alt_screen(&mut self) {
        self.push_str("\x1b[?1049h");
    }

    pub fn exit_alt_screen(&mut self) {
        self.push_str("\x1b[?1049l");
    }

    pub fn hide_cursor(&mut self) {
        self.push_str("\x1b[?25l");
    }

    pub fn show_cursor(&mut self) {
        self.push_str("\x1b[?25h");
    }

    pub fn move_to(&mut self, row: u16, col: u16) {
        self.push_str("\x1b[");
        self.push_u64(row as u64);
        self.push(b';');
        self.push_u64(col as u64);
        self.push(b'H');
    }

    pub fn clear_eol(&mut self) {
        self.push_str("\x1b[K");
    }

    pub fn clear_line(&mut self) {
        self.push_str("\x1b[2K\r");
    }

    pub fn reset(&mut self) {
        self.push_str("\x1b[0m");
    }

    pub fn fg_orange(&mut self) {
        self.push_str("\x1b[38;2;255;110;0m");
    }

    pub fn fg_white(&mut self) {
        self.push_str("\x1b[38;2;238;238;238m");
    }

    pub fn fg_dim(&mut self) {
        self.push_str("\x1b[38;5;242m");
    }

    pub fn fg_green(&mut self) {
        self.push_str("\x1b[38;2;68;204;68m");
    }

    pub fn fg_red(&mut self) {
        self.push_str("\x1b[38;2;255;68;68m");
    }

    pub fn fg_yellow(&mut self) {
        self.push_str("\x1b[38;2;255;170;0m");
    }

    pub fn bg_surface(&mut self) {
        self.push_str("\x1b[48;2;17;17;17m");
    }

    pub fn bg_selected(&mut self) {
        self.push_str("\x1b[48;2;40;30;10m");
    }

    pub fn bold(&mut self) {
        self.push_str("\x1b[1m");
    }

    pub fn dim(&mut self) {
        self.push_str("\x1b[2m");
    }

    pub fn progress_bar(&mut self, filled_pct: u8, width: u16) {
        let pct = filled_pct.min(100);
        let filled = (pct as u32 * width as u32 / 100) as u16;

        if pct < 60 {
            self.fg_green();
        } else if pct < 85 {
            self.fg_yellow();
        } else {
            self.fg_red();
        }

        for _ in 0..filled {
            self.push_str("█");
        }

        self.fg_dim();
        for _ in filled..width {
            self.push_str("░");
        }

        self.reset();
    }

    pub fn push_padded(&mut self, s: &str, width: usize) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(width);
        self.push_bytes(&bytes[..len]);
        for _ in len..width {
            self.push(b' ');
        }
    }
}
