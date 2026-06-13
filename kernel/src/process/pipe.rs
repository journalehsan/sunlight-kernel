use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

const PIPE_BUFFER_SIZE: usize = 4096;
const PIPE_FLAG: u32 = 0x8000_0000;
const PIPE_WRITE_FLAG: u32 = 0x4000_0000;

/// A kernel pipe with ring buffer
pub struct Pipe {
    buffer: [u8; PIPE_BUFFER_SIZE],
    read_pos: usize,
    write_pos: usize,
    data_len: usize,
    readers: u32, // reference count for read side
    writers: u32, // reference count for write side
}

/// Global pipe pool (slot table, None = free slot)
static PIPE_POOL: spin::Mutex<Vec<Option<Pipe>>> = spin::Mutex::new(Vec::new());

/// Result type for pipe operations
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeResult {
    Ok(usize),  // bytes read/written
    WouldBlock, // no data/space available
    Eof,        // no writers (on read)
    BrokenPipe, // no readers (on write)
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

/// Allocate a new pipe and return the pool index
fn alloc_pipe() -> u32 {
    let mut pool = PIPE_POOL.lock();
    let new_pipe = Pipe::new();

    for (idx, slot) in pool.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(new_pipe);
            return idx as u32;
        }
    }

    let idx = pool.len() as u32;
    pool.push(Some(new_pipe));
    idx
}

/// Free a pipe slot (if both reader and writer counts are 0)
fn free_pipe_if_done(idx: u32) {
    let mut pool = PIPE_POOL.lock();
    if let Some(Some(pipe)) = pool.get(idx as usize) {
        if pipe.readers == 0 && pipe.writers == 0 {
            pool[idx as usize] = None;
        }
    }
}

/// Read from a pipe (non-blocking)
pub fn pipe_read(pool_idx: u32, buf: &mut [u8]) -> PipeResult {
    let mut pool = PIPE_POOL.lock();

    if let Some(Some(pipe)) = pool.get_mut(pool_idx as usize) {
        if pipe.data_len == 0 {
            if pipe.writers == 0 {
                return PipeResult::Eof;
            }
            return PipeResult::WouldBlock;
        }

        let to_read = core::cmp::min(buf.len(), pipe.data_len);

        for i in 0..to_read {
            buf[i] = pipe.buffer[(pipe.read_pos + i) % PIPE_BUFFER_SIZE];
        }

        pipe.read_pos = (pipe.read_pos + to_read) % PIPE_BUFFER_SIZE;
        pipe.data_len -= to_read;

        PipeResult::Ok(to_read)
    } else {
        PipeResult::Eof
    }
}

/// Write to a pipe (non-blocking)
pub fn pipe_write(pool_idx: u32, buf: &[u8]) -> PipeResult {
    let mut pool = PIPE_POOL.lock();

    if let Some(Some(pipe)) = pool.get_mut(pool_idx as usize) {
        if pipe.readers == 0 {
            return PipeResult::BrokenPipe;
        }

        let available = PIPE_BUFFER_SIZE - pipe.data_len;
        if available == 0 {
            return PipeResult::WouldBlock;
        }

        let to_write = core::cmp::min(buf.len(), available);

        for i in 0..to_write {
            pipe.buffer[(pipe.write_pos + i) % PIPE_BUFFER_SIZE] = buf[i];
        }

        pipe.write_pos = (pipe.write_pos + to_write) % PIPE_BUFFER_SIZE;
        pipe.data_len += to_write;

        PipeResult::Ok(to_write)
    } else {
        PipeResult::BrokenPipe
    }
}

/// Close one end of a pipe (decrement reader/writer count)
pub fn pipe_close_end(pool_idx: u32, is_write: bool) {
    let mut pool = PIPE_POOL.lock();

    if let Some(Some(pipe)) = pool.get_mut(pool_idx as usize) {
        if is_write {
            if pipe.writers > 0 {
                pipe.writers -= 1;
            }
        } else {
            if pipe.readers > 0 {
                pipe.readers -= 1;
            }
        }

        if pipe.readers == 0 && pipe.writers == 0 {
            pool[pool_idx as usize] = None;
        }
    }
}

/// Create a new pipe and return (read_fd, write_fd)
pub fn create_pipe(
    _pmm: &mut crate::memory::pmm::PhysicalMemoryManager,
    sched: &mut crate::sched::Scheduler,
) -> Result<(i32, i32), PipeError> {
    use crate::process::fd_table::{CapRights, FileHandle};

    let pipe_idx = alloc_pipe();

    let process = sched.current_process_mut();

    let read_handle = FileHandle(PIPE_FLAG | pipe_idx);
    let write_handle = FileHandle(PIPE_FLAG | PIPE_WRITE_FLAG | pipe_idx);

    let read_fd = process
        .fd_table
        .open(read_handle, CapRights::new(CapRights::READ), 0)
        .map_err(|_| PipeError::BadFd)?;

    let write_fd = process
        .fd_table
        .open(write_handle, CapRights::new(CapRights::WRITE), 0)
        .map_err(|_| PipeError::BadFd)?;

    crate::serial_println!(
        "[PIPE] created pipe: read_fd={}, write_fd={}, pool_idx={}",
        read_fd,
        write_fd,
        pipe_idx
    );

    Ok((read_fd, write_fd))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeError {
    BrokenPipe, // Write to pipe with no readers
    BadFd,      // Invalid file descriptor
    NotAPipe,   // FD is not a pipe
}

/// Pipe handle (index into global pipe table)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipeHandle(pub u32);
