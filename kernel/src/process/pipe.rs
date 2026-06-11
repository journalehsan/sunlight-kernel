use alloc::sync::Arc;
use spin::Mutex;

const PIPE_BUFFER_SIZE: usize = 4096;

/// A kernel pipe with ring buffer
pub struct Pipe {
    buffer: [u8; PIPE_BUFFER_SIZE],
    read_pos: usize,
    write_pos: usize,
    data_len: usize,
    readers: u32,   // reference count for read side
    writers: u32,   // reference count for write side
}

impl Pipe {
    pub fn new() -> Self {
        Self {
            buffer: [0u8; PIPE_BUFFER_SIZE],
            read_pos: 0,
            write_pos: 0,
            data_len: 0,
            readers: 1,
            writers: 1,
        }
    }

    /// Read data from pipe (non-blocking for now)
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, PipeError> {
        if self.data_len == 0 {
            if self.writers == 0 {
                // EOF: no data and no writers
                return Ok(0);
            }
            // No data available yet (would block in real implementation)
            return Ok(0);
        }

        let to_read = core::cmp::min(buf.len(), self.data_len);

        // Copy data from pipe buffer to user buffer
        for i in 0..to_read {
            buf[i] = self.buffer[(self.read_pos + i) % PIPE_BUFFER_SIZE];
        }

        self.read_pos = (self.read_pos + to_read) % PIPE_BUFFER_SIZE;
        self.data_len -= to_read;

        Ok(to_read)
    }

    /// Write data to pipe (non-blocking for now)
    pub fn write(&mut self, buf: &[u8]) -> Result<usize, PipeError> {
        if self.readers == 0 {
            // EPIPE: no readers
            return Err(PipeError::BrokenPipe);
        }

        let available = PIPE_BUFFER_SIZE - self.data_len;
        if available == 0 {
            // Pipe full (would block in real implementation)
            return Ok(0);
        }

        let to_write = core::cmp::min(buf.len(), available);

        // Copy data from user buffer to pipe buffer
        for i in 0..to_write {
            self.buffer[(self.write_pos + i) % PIPE_BUFFER_SIZE] = buf[i];
        }

        self.write_pos = (self.write_pos + to_write) % PIPE_BUFFER_SIZE;
        self.data_len += to_write;

        Ok(to_write)
    }

    pub fn add_reader(&mut self) {
        self.readers += 1;
    }

    pub fn add_writer(&mut self) {
        self.writers += 1;
    }

    pub fn remove_reader(&mut self) {
        if self.readers > 0 {
            self.readers -= 1;
        }
    }

    pub fn remove_writer(&mut self) {
        if self.writers > 0 {
            self.writers -= 1;
        }
    }

    pub fn has_readers(&self) -> bool {
        self.readers > 0
    }

    pub fn has_writers(&self) -> bool {
        self.writers > 0
    }
}

impl Default for Pipe {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeError {
    BrokenPipe,  // Write to pipe with no readers
    BadFd,       // Invalid file descriptor
    NotAPipe,    // FD is not a pipe
}

/// Pipe handle (index into global pipe table)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipeHandle(pub u32);

// Global pipe storage (simplified — in production would use proper pool)
// For now, we'll store pipes in the fd_table's FileHandle space
// A FileHandle with type bits indicating it's a pipe.

/// Create a new pipe and return (read_fd, write_fd)
pub fn create_pipe(
    _pmm: &mut crate::memory::pmm::PhysicalMemoryManager,
    sched: &mut crate::sched::Scheduler,
) -> Result<(i32, i32), PipeError> {
    use crate::process::fd_table::{CapRights, FileHandle};

    let process = sched.current_process_mut();

    // For now, we'll use a simplified approach:
    // Open the read fd and write fd with special FileHandle values
    // In a real implementation, we'd need a global pipe pool

    // Create FileHandle for the pipe (simplified: use 0xDEADBEEF as marker)
    // In production, this would point to an actual Pipe struct
    let pipe_handle = FileHandle(0xDEADBEEF);

    // Open read side (read-only)
    let read_fd = process
        .fd_table
        .open(pipe_handle, CapRights::new(CapRights::READ), 0)
        .map_err(|_| PipeError::BadFd)?;

    // Open write side (write-only)
    let write_fd = process
        .fd_table
        .open(pipe_handle, CapRights::new(CapRights::WRITE), 0)
        .map_err(|_| PipeError::BadFd)?;

    crate::serial_println!("[PIPE] created pipe: read_fd={}, write_fd={}", read_fd, write_fd);

    Ok((read_fd, write_fd))
}
