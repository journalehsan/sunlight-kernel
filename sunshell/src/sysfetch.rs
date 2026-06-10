use core::fmt::{self, Write};

/// Ultra-compact stack buffer for formatting strings without heap allocation
pub struct CompactWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> CompactWriter<'a> {
    pub fn new(slice: &'a mut [u8]) -> Self {
        Self { buf: slice, pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.pos
    }
}

impl<'a> Write for CompactWriter<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = s.as_bytes();
        if self.pos + bytes.len() > self.buf.len() {
            return Err(fmt::Error);
        }
        self.buf[self.pos..self.pos + bytes.len()].copy_from_slice(bytes);
        self.pos += bytes.len();
        Ok(())
    }
}

/// Renders system stats cleanly without breaking the 512-byte payload threshold
pub fn render_sysfetch_to_buffer(
    username: &str,
    kernel_version: &str,
    uptime_secs: u64,
    mem_used: u32,
    mem_total: u32,
    out: &mut [u8],
) -> usize {
    let mut w = CompactWriter::new(out);
    
    // Minimal inline ANSI: only cyan for labels + reset
    let c = "\x1B[36m";
    let r = "\x1B[0m";

    let h = uptime_secs / 3600;
    let m = (uptime_secs / 60) % 60;
    let s = uptime_secs % 60;

    let _ = writeln!(w, "{}{}@sunlightos{}", c, username, r);
    let _ = writeln!(w, "{}OS:{} SunlightOS", c, r);
    let _ = writeln!(w, "{}Kernel:{} {}", c, r, kernel_version);

    let _ = write!(w, "{}Uptime:{} ", c, r);
    if h > 0 {
        let _ = writeln!(w, "{}h {}m", h, m);
    } else if m > 0 {
        let _ = writeln!(w, "{}m {}s", m, s);
    } else {
        let _ = writeln!(w, "{}s", s);
    }

    let _ = writeln!(w, "{}Memory:{} {}MB/{}MB", c, r, mem_used, mem_total);

    // Ultra-short color blocks: 1 space instead of 3
    let _ = write!(w, "Palette: ");
    for i in 0..8 {
        let _ = write!(w, "\x1B[4{}m \x1B[0m", i);
    }
    let _ = writeln!(w);
    
    w.len()
}
