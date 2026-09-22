/// No clipboard, selection, debug representation, heap allocation, or plaintext draw.
pub struct SecretInput {
    bytes: [u8; 128],
    len: usize,
}
impl SecretInput {
    pub const fn new() -> Self {
        Self {
            bytes: [0; 128],
            len: 0,
        }
    }
    pub fn clear(&mut self) {
        clear_secret(&mut self.bytes);
        self.len = 0;
    }
    pub fn value(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
    pub fn key(&mut self, c: char) {
        if c == '\u{8}' || c == '\u{7f}' {
            if self.len > 0 {
                let old = self.len;
                self.len -= 1;
                while self.len > 0 && self.bytes[self.len] & 0xc0 == 0x80 {
                    self.len -= 1;
                }
                clear_secret(&mut self.bytes[self.len..old]);
            }
        } else if !c.is_control() && self.len + c.len_utf8() <= self.bytes.len() {
            let n = c.len_utf8();
            c.encode_utf8(&mut self.bytes[self.len..self.len + n]);
            self.len += n;
        }
    }
}
impl Drop for SecretInput {
    fn drop(&mut self) {
        self.clear();
    }
}

fn clear_secret(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe {
            core::ptr::write_volatile(byte, 0);
        }
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_backspace_clears_removed_bytes() {
        let mut input = SecretInput::new();
        input.key('a');
        input.key('é');
        input.key('\u{8}');
        assert_eq!(input.value(), b"a");
        assert!(input.bytes[1..].iter().all(|b| *b == 0));
        input.clear();
        assert_eq!(input.value(), b"");
        assert_eq!(input.bytes, [0; 128]);
    }
    #[test]
    fn secret_input_is_bounded_and_ignores_control_characters() {
        let mut input = SecretInput::new();
        for _ in 0..200 {
            input.key('x');
        }
        assert_eq!(input.value().len(), 128);
        input.clear();
        for c in ['\n', '\r', '\t', '\0'] {
            input.key(c);
        }
        assert!(input.value().is_empty());
    }
}
