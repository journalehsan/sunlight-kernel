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

/// A Unix stream pair uses two of the existing pipe rings, one in each
/// direction. Endpoints retain their own status descriptions in FdTable.
struct SocketPair {
    incoming: [u32; 2],
    refs: [usize; 2],
}

static SOCKET_PAIRS: spin::Mutex<Vec<Option<SocketPair>>> = spin::Mutex::new(Vec::new());

pub fn socket_stream(index: u32, endpoint: usize) -> Option<(u32, u32)> {
    SOCKET_PAIRS.lock().get(index as usize)?.as_ref().map(|pair| {
        (pair.incoming[endpoint], pair.incoming[1 - endpoint])
    })
}

pub fn socket_retain(index: u32, endpoint: usize) {
    let streams = {
        let mut pool = SOCKET_PAIRS.lock();
        let pair = pool[index as usize].as_mut().expect("live socket pair");
        pair.refs[endpoint] += 1;
        (pair.incoming[endpoint], pair.incoming[1 - endpoint])
    };
    let mut pool = PIPE_POOL.lock();
    pool[streams.0 as usize].as_mut().expect("live read ring").add_reader();
    pool[streams.1 as usize].as_mut().expect("live write ring").add_writer();
}

pub fn socket_close(index: u32, endpoint: usize) -> (u32, u32) {
    let streams = {
        let mut pool = SOCKET_PAIRS.lock();
        let pair = pool[index as usize].as_mut().expect("live socket pair");
        pair.refs[endpoint] -= 1;
        let streams = (pair.incoming[endpoint], pair.incoming[1 - endpoint]);
        if pair.refs == [0, 0] {
            pool[index as usize] = None;
        }
        streams
    };
    pipe_close_end(streams.0, false);
    pipe_close_end(streams.1, true);
    streams
}

pub fn create_socketpair(sched: &mut crate::sched::Scheduler, flags: u32) -> Result<(i32, i32), PipeError> {
    use crate::process::fd_table::{CapRights, FileHandle};
    let first = alloc_pipe();
    let second = alloc_pipe();
    let index = {
        let mut pool = SOCKET_PAIRS.lock();
        let item = SocketPair { incoming: [first, second], refs: [1, 1] };
        if let Some((idx, slot)) = pool.iter_mut().enumerate().find(|(_, s)| s.is_none()) {
            *slot = Some(item);
            idx as u32
        } else {
            let idx = pool.len();
            pool.push(Some(item));
            idx as u32
        }
    };
    let rights = CapRights::new(CapRights::READ | CapRights::WRITE);
    let (a, b) = {
        let table = &mut sched.current_shared_process_mut().fd_table;
        let a = match table.open(FileHandle::socket(index, 0), rights, flags | 2) {
            Ok(fd) => fd,
            Err(_) => {
                socket_close(index, 0);
                socket_close(index, 1);
                return Err(PipeError::BadFd);
            }
        };
        match table.open(FileHandle::socket(index, 1), rights, flags | 2) {
            Ok(fd) => (a, fd),
            Err(_) => {
                let _ = table.take(a);
                socket_close(index, 0);
                socket_close(index, 1);
                return Err(PipeError::BadFd);
            }
        }
    };
    Ok((a, b))
}

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

/// True if a non-blocking read would not return `WouldBlock`
/// (data available, or EOF because all writers closed).
pub fn pipe_has_data_or_eof(pool_idx: u32) -> bool {
    let pool = PIPE_POOL.lock();
    match pool.get(pool_idx as usize).and_then(|s| s.as_ref()) {
        Some(pipe) => pipe.data_len > 0 || pipe.writers == 0,
        None => true, // closed → treat as readable EOF
    }
}

pub fn pipe_has_space_or_broken(pool_idx: u32) -> bool {
    let pool = PIPE_POOL.lock();
    pool.get(pool_idx as usize).and_then(Option::as_ref)
        .is_none_or(|pipe| pipe.readers == 0 || pipe.data_len < PIPE_BUFFER_SIZE)
}

pub fn pipe_state(pool_idx: u32) -> Option<(usize, u32, u32)> {
    let pool = PIPE_POOL.lock();
    pool.get(pool_idx as usize).and_then(Option::as_ref)
        .map(|pipe| (pipe.data_len, pipe.readers, pipe.writers))
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
        if available == 0 || (buf.len() <= PIPE_BUFFER_SIZE && available < buf.len()) {
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

pub fn pipe_retain_end(pool_idx: u32, is_write: bool) {
    let mut pool = PIPE_POOL.lock();
    if let Some(Some(pipe)) = pool.get_mut(pool_idx as usize) {
        if is_write { pipe.add_writer(); } else { pipe.add_reader(); }
    }
}

/// Create a new pipe and return (read_fd, write_fd)
pub fn create_pipe(
    _pmm: &mut crate::memory::pmm::PhysicalMemoryManager,
    sched: &mut crate::sched::Scheduler,
) -> Result<(i32, i32), PipeError> {
    use crate::process::fd_table::{CapRights, FileHandle};

    let pipe_idx = alloc_pipe();

    let process = sched.current_shared_process_mut();

    let read_handle = FileHandle(PIPE_FLAG | pipe_idx);
    let write_handle = FileHandle(PIPE_FLAG | PIPE_WRITE_FLAG | pipe_idx);

    let read_fd = process
        .fd_table
        .open(read_handle, CapRights::new(CapRights::READ), 0)
        .map_err(|_| {
            // `Pipe::new` owns one reader and one writer.  No descriptor was
            // published, so release both references rather than leaking the
            // pool entry on descriptor-table exhaustion.
            pipe_close_end(pipe_idx, false);
            pipe_close_end(pipe_idx, true);
            PipeError::BadFd
        })?;

    let write_fd = match process
        .fd_table
        .open(write_handle, CapRights::new(CapRights::WRITE), 0)
    {
        Ok(fd) => fd,
        Err(_) => {
            // Undo the published read end as well as both pool-side refs.
            let _ = process.fd_table.close(read_fd);
            pipe_close_end(pipe_idx, false);
            pipe_close_end(pipe_idx, true);
            return Err(PipeError::BadFd);
        }
    };

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
