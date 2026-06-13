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

/// Renders system stats cleanly without breaking the 512-byte payload threshold.
/// net_ip: optional [a,b,c,d] for "IP: a.b.c.d" line (Phase 5+).
pub fn render_sysfetch_to_buffer(
    username: &str,
    kernel_version: &str,
    cpu: &str,
    uptime_secs: u64,
    mem_used: u32,
    mem_total: u32,
    net_ip: Option<[u8; 4]>,
    out: &mut [u8],
) -> usize {
    let mut w = CompactWriter::new(out);

    // Minimal inline ANSI: cyan for labels, green for good usage, yellow for warn, red for danger
    let c = "\x1B[36m"; // cyan
    let g = "\x1B[32m"; // green (good - low usage)
    let y = "\x1B[33m"; // yellow (warning - medium usage)
    let r_color = "\x1B[31m"; // red (danger - high usage)
    let r = "\x1B[0m"; // reset

    let h = uptime_secs / 3600;
    let m = (uptime_secs / 60) % 60;
    let s = uptime_secs % 60;

    // Calculate memory percentage for color coding
    let mem_percent = if mem_total > 0 {
        (mem_used as u32 * 100) / mem_total
    } else {
        0
    };

    let _ = writeln!(w, "{}{}@sunlightos{}", c, username, r);
    let _ = writeln!(w, "{}OS:{} SunlightOS", c, r);
    let _ = writeln!(w, "{}Kernel:{} {}", c, r, kernel_version);
    if !cpu.is_empty() {
        let _ = writeln!(w, "{}CPU:{} {}", c, r, cpu);
    }

    let _ = write!(w, "{}Uptime:{} ", c, r);
    if h > 0 {
        let _ = writeln!(w, "{}h {}m", h, m);
    } else if m > 0 {
        let _ = writeln!(w, "{}m {}s", m, s);
    } else {
        let _ = writeln!(w, "{}s", s);
    }

    // Color-coded memory display
    let mem_color = if mem_percent < 50 {
        g // Green if < 50%
    } else if mem_percent < 80 {
        y // Yellow if 50-80%
    } else {
        r_color // Red if > 80%
    };

    let _ = writeln!(
        w,
        "{}Memory:{} {}{}MB{}/{}MB ({}%)",
        c, r, mem_color, mem_used, r, mem_total, mem_percent
    );

    // Memory bar: 10 blocks = 10% each
    let _ = write!(w, "{}Bar:{} ", c, r);
    let blocks = (mem_percent / 10) as u32;
    for i in 0..10 {
        if i < blocks {
            let _ = write!(w, "{}█{}", mem_color, r);
        } else {
            let _ = write!(w, "░");
        }
    }
    let _ = writeln!(w);

    if let Some(ip) = net_ip {
        let _ = write!(w, "{}IP:{} {}.{}.{}.{}/24 (eth0)\n", c, r, ip[0], ip[1], ip[2], ip[3]);
    }

    // Minimal palette: 8 color blocks
    let _ = write!(w, "Palette: ");
    for i in 0..8 {
        let _ = write!(w, "\x1B[4{}m \x1B[0m", i);
    }
    let _ = writeln!(w);

    w.len()
}
