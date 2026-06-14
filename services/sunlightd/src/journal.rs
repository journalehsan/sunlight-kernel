//! Journal logging - capture service stdout/stderr to VFS

use crate::unit::LogDest;

pub struct LogCapture {
    pub unit_name: heapless::String<64>,
    pub log_path: heapless::String<128>,
    pub buffer: heapless::Vec<u8, 4096>,
}

impl LogCapture {
    pub fn new(unit_name: &str) -> Self {
        let mut log_path = heapless::String::new();
        let _ = log_path.push_str("/var/log/");
        let _ = log_path.push_str(unit_name);
        let _ = log_path.push_str(".log");
        
        let mut unit_name_str = heapless::String::new();
        let _ = unit_name_str.push_str(unit_name);

        Self {
            unit_name: unit_name_str,
            log_path,
            buffer: heapless::Vec::new(),
        }
    }

    /// Append data to the log buffer
    pub fn append(&mut self, data: &[u8]) -> Result<(), &'static str> {
        for &byte in data {
            if self.buffer.push(byte).is_err() {
                // Buffer full, flush it
                self.flush()?;
                let _ = self.buffer.push(byte);
            }
        }
        Ok(())
    }

    /// Flush buffer to VFS
    /// TODO: requires pipe IPC (Phase pipes)
    pub fn flush(&mut self) -> Result<(), &'static str> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        // TODO: VFS write IPC to self.log_path
        // For now, just clear the buffer
        self.buffer.clear();
        Ok(())
    }

    /// Setup pipe for stdout/stderr capture
    /// TODO: requires pipe IPC
    pub fn setup_pipe(&mut self, _log_dest: LogDest) -> Option<(u32, u32)> {
        // TODO: Create pipe pair via IPC
        // Returns (read_fd, write_fd) - write_fd goes to child process
        None
    }
}

/// Drain data from a pipe and append to log
/// TODO: requires pipe IPC read operation
pub fn drain_pipe(_read_fd: u32, _log: &mut LogCapture) -> Result<usize, &'static str> {
    // TODO: Read from pipe and call log.append()
    Ok(0)
}
