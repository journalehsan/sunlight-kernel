//! Single-line ANSI progress bar for SunlightTTY.
//!
//! Design principles:
//! - Single `\r` rewrite, no line spam
//! - Non-blocking: pure computation, no I/O blocking
//! - SunlightTTY-friendly: respects terminal width, clean on Ctrl+C

use std::string::String;
use std::fmt::Write;

/// Track download progress across all chunks
pub struct ProgressTracker {
    total_bytes: usize,
    downloaded_bytes: usize,
    start_tick: u64,
    last_render_tick: u64,
    term_width: usize,
    finished: bool,
}

/// Snapshot for rendering — avoids holding mutable borrow during I/O
pub struct ProgressSnapshot {
    pub total: usize,
    pub downloaded: usize,
    pub elapsed_ms: u64,
}

impl ProgressTracker {
    /// Create a new tracker.
    /// `total_bytes`: 0 means unknown size (indeterminate mode).
    /// `term_width`: terminal column count (default 80).
    pub fn new(total_bytes: usize, term_width: usize) -> Self {
        Self {
            total_bytes,
            downloaded_bytes: 0,
            start_tick: current_tick_ms(),
            last_render_tick: 0,
            term_width: if term_width > 20 { term_width } else { 80 },
            finished: false,
        }
    }

    /// Update the downloaded byte count (atomic add).
    /// Returns true if the display should be redrawn (throttled to ~20fps).
    pub fn update(&mut self, additional_bytes: usize) -> bool {
        self.downloaded_bytes += additional_bytes;

        let now = current_tick_ms();
        let should_redraw = now - self.last_render_tick >= 50; // 20 fps cap
        if should_redraw {
            self.last_render_tick = now;
        }
        should_redraw
    }

    /// Mark download as finished
    pub fn finish(&mut self) {
        self.downloaded_bytes = self.total_bytes;
        self.finished = true;
    }

    /// Take a snapshot for rendering
    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            total: self.total_bytes,
            downloaded: self.downloaded_bytes,
            elapsed_ms: current_tick_ms() - self.start_tick,
        }
    }

    /// Render the progress bar into a string buffer.
    ///
    /// Format: `\r[=====>     ] 45% | 1.2MB / 2.6MB | 1.5MB/s`
    ///
    /// When total is unknown: `\r[<=>        ] 1.2MB | 1.5MB/s`
    pub fn render(&self, buf: &mut String) {
        buf.clear();

        let snap = self.snapshot();
        let downloaded_str = format_bytes(snap.downloaded);
        let speed = if snap.elapsed_ms > 0 {
            (snap.downloaded as u64 * 1000) / snap.elapsed_ms
        } else {
            0
        };
        let speed_str = format_bytes(speed as usize);

        // Build the right-side info string first to know how much room the bar gets
        let mut info = String::with_capacity(64);

        if snap.total > 0 {
            let pct = if snap.total > 0 {
                (snap.downloaded * 100) / snap.total
            } else {
                0
            };
            let total_str = format_bytes(snap.total);
            let _ = write!(&mut info, " {}% | {} / {} | {}/s", pct, downloaded_str, total_str, speed_str);
        } else {
            let _ = write!(&mut info, " {} | {}/s", downloaded_str, speed_str);
        }

        // Bar width = term_width - info_len - brackets - \r - margin
        let bar_overhead = 3 + info.len() + 1; // "[\r" + "] " + info
        let bar_width = if self.term_width > bar_overhead + 10 {
            self.term_width - bar_overhead
        } else {
            20 // minimum bar width
        };

        // Write \r to return to start of line
        buf.push('\r');

        if snap.total > 0 {
            // Determinate mode
            let filled = if snap.total > 0 {
                (snap.downloaded * bar_width) / snap.total
            } else {
                0
            };
            let filled = core::cmp::min(filled, bar_width);

            buf.push('[');
            for i in 0..bar_width {
                if i < filled {
                    buf.push('=');
                } else if i == filled && filled < bar_width {
                    buf.push('>');
                } else {
                    buf.push(' ');
                }
            }
            buf.push(']');
        } else {
            // Indeterminate mode — bouncing indicator
            let pos = if snap.elapsed_ms > 0 {
                ((snap.elapsed_ms / 100) as usize) % (bar_width * 2)
            } else {
                0
            };
            let pos = if pos >= bar_width {
                bar_width * 2 - pos - 1
            } else {
                pos
            };

            buf.push('[');
            for i in 0..bar_width {
                if i >= pos && i < pos + 3 {
                    buf.push('=');
                } else {
                    buf.push(' ');
                }
            }
            buf.push(']');
        }

        buf.push_str(&info);

        // If finished, add newline
        if self.finished {
            buf.push('\n');
        }
    }
}

/// Format byte count into human-readable string
pub fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2}GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Get current monotonic time in milliseconds.
use std::time::{SystemTime, UNIX_EPOCH};

fn current_tick_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500B");
        assert_eq!(format_bytes(1536), "1.5KB");
        assert_eq!(format_bytes(2_621_440), "2.5MB");
    }

    #[test]
    fn test_render_determinate() {
        let mut tracker = ProgressTracker::new(1000, 80);
        tracker.downloaded_bytes = 500;
        let mut buf = String::new();
        tracker.render(&mut buf);
        assert!(buf.contains("50%"));
        assert!(buf.contains('['));
        assert!(buf.contains(']'));
    }

    #[test]
    fn test_render_indeterminate() {
        let tracker = ProgressTracker::new(0, 80);
        let mut buf = String::new();
        tracker.render(&mut buf);
        assert!(buf.contains('['));
        assert!(!buf.contains('%'));
    }
}
