//! Application-owned candidates and bounded atomic Save As. Failure preserves the source.

use crate::wav::{Format, Writer};
use alloc::string::String;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Io(i32),
    Format,
    Length,
    State,
    Storage,
    Capture,
    Busy,
    Interrupted,
    Empty,
    Playback,
    Cancelled,
}

impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Self::Io(28) => "Recording storage is full",
            Self::Io(13) | Self::Io(1) => "Permission denied for this location",
            Self::Io(2) => "The selected folder does not exist",
            Self::Io(_) => "Storage operation failed; check free space and disk availability",
            Self::Format => "The microphone format is unsupported",
            Self::Length => "The PCM WAV size limit has been reached",
            Self::State => "This action is currently unavailable",
            Self::Storage => {
                "Recording storage is unavailable; choose a writable home or /tmp folder"
            }
            Self::Capture => "Microphone unavailable or capture service failed",
            Self::Busy => "Microphone is busy; stop input monitoring in Sound settings",
            Self::Interrupted => "Capture was interrupted; this incomplete recording was discarded",
            Self::Empty => "No audio was recorded",
            Self::Playback => "Could not play the recording",
            Self::Cancelled => "Cancelled",
        }
    }
}

pub trait File {
    fn read(&mut self, dst: &mut [u8]) -> Result<usize, Error>;
    fn write_all(&mut self, src: &[u8]) -> Result<(), Error>;
    fn seek(&mut self, offset: u64) -> Result<(), Error>;
    fn sync(&mut self) -> Result<(), Error>;
    fn close(&mut self) -> Result<(), Error>;
}

pub trait FileSystem: Clone {
    type Handle: File;
    fn create(&self, path: &str) -> Result<Self::Handle, Error>;
    fn open(&self, path: &str) -> Result<Self::Handle, Error>;
    fn remove(&self, path: &str) -> Result<(), Error>;
    fn publish(&self, staging: &str, target: &str, replace: bool) -> Result<(), Error>;
    fn sync_dir(&self, directory: &str) -> Result<(), Error>;
}

pub struct Recording<S: FileSystem> {
    fs: S,
    pub path: String,
    pub writer: Option<Writer<S::Handle>>,
    pub frames: u64,
    pub format: Format,
    pub finalized: bool,
    pub saved: bool,
}

impl<S: FileSystem> Recording<S> {
    pub fn create(fs: S, path: String, format: Format) -> Result<Self, Error> {
        let file = fs.create(&path)?;
        match Writer::new(file, format) {
            Ok(writer) => Ok(Self {
                fs,
                path,
                writer: Some(writer),
                frames: 0,
                format,
                finalized: false,
                saved: false,
            }),
            Err(e) => {
                let _ = fs.remove(&path);
                Err(e)
            }
        }
    }
    pub fn append(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let writer = self.writer.as_mut().ok_or(Error::State)?;
        writer.append(bytes)?;
        self.frames = writer.frames();
        Ok(())
    }
    pub fn finalize(&mut self) -> Result<(), Error> {
        let writer = self.writer.as_mut().ok_or(Error::State)?;
        writer.finalize()?;
        writer.file.close()?;
        self.writer = None;
        self.finalized = true;
        Ok(())
    }
    pub fn save_as(&mut self, target: &str, staging: &str, replace: bool) -> Result<(), Error> {
        self.save_as_with_cancel(target, staging, replace, || false)
    }
    pub fn save_as_with_cancel(
        &mut self,
        target: &str,
        staging: &str,
        replace: bool,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(), Error> {
        if !self.finalized
            || self.frames == 0
            || target == self.path
            || staging == self.path
            || staging == target
        {
            return Err(Error::State);
        }
        let parent = target
            .rsplit_once('/')
            .map(|(p, _)| if p.is_empty() { "/" } else { p })
            .ok_or(Error::State)?;
        let stage_parent = staging
            .rsplit_once('/')
            .map(|(p, _)| if p.is_empty() { "/" } else { p })
            .ok_or(Error::State)?;
        if parent != stage_parent {
            return Err(Error::State);
        }
        let mut source = self.fs.open(&self.path)?;
        let mut output = self.fs.create(staging)?;
        let result = (|| {
            let mut chunk = [0u8; 32768];
            let mut bytes = 0u64;
            loop {
                if cancelled() {
                    return Err(Error::Cancelled);
                }
                let count = source.read(&mut chunk)?;
                if count == 0 {
                    break;
                }
                output.write_all(&chunk[..count])?;
                bytes += count as u64;
            }
            if bytes != 44 + self.frames * self.format.frame_bytes() {
                return Err(Error::Length);
            }
            output.sync()?;
            output.close()?;
            source.close()?;
            self.fs.publish(staging, target, replace)?;
            self.fs.sync_dir(parent)?;
            Ok(())
        })();
        let _ = output.close();
        let _ = source.close();
        if result.is_err() {
            let _ = self.fs.remove(staging);
        }
        if result.is_ok() {
            self.saved = true;
        }
        result
    }
}

impl<S: FileSystem> Drop for Recording<S> {
    fn drop(&mut self) {
        self.writer = None;
        let _ = self.fs.remove(&self.path);
    }
}

// These namespaces are RAMFS in live mode. Their contents are immediately
// visible but cannot be flushed to disk; keep sync mandatory for /state.
#[cfg(any(target_os = "none", test))]
fn volatile_path(path: &str) -> bool {
    ["/home", "/root", "/tmp"]
        .iter()
        .any(|base| path == *base || path.strip_prefix(base).is_some_and(|s| s.starts_with('/')))
}

#[cfg(target_os = "none")]
pub mod native {
    use super::*;
    use sunlight_libc as libc;
    #[derive(Clone, Copy)]
    pub struct NativeFs;
    pub struct NativeFile(Option<libc::Fd>, bool);
    pub fn io(e: libc::sys::Errno) -> Error {
        use libc::sys::Errno;
        Error::Io(match e {
            Errno::NoEntry => 2,
            Errno::Access => 13,
            Errno::Again => 11,
            Errno::Inval => 22,
            Errno::TooBig => 36,
            Errno::IsDir => 21,
            Errno::NotDir => 20,
            Errno::BadFd => 9,
            Errno::Failed => 5,
        })
    }
    impl File for NativeFile {
        fn read(&mut self, dst: &mut [u8]) -> Result<usize, Error> {
            libc::read(self.0.ok_or(Error::State)?, dst).map_err(io)
        }
        fn write_all(&mut self, src: &[u8]) -> Result<(), Error> {
            libc::write_all(self.0.ok_or(Error::State)?, src).map_err(io)
        }
        fn seek(&mut self, offset: u64) -> Result<(), Error> {
            libc::lseek(self.0.ok_or(Error::State)?, offset as i64, libc::SEEK_SET)
                .map(|_| ())
                .map_err(io)
        }
        fn sync(&mut self) -> Result<(), Error> {
            if self.1 {
                self.0.ok_or(Error::State)?;
                return Ok(());
            }
            libc::file_sync(self.0.ok_or(Error::State)?).map_err(io)
        }
        fn close(&mut self) -> Result<(), Error> {
            if let Some(fd) = self.0.take() {
                libc::close(fd).map_err(io)
            } else {
                Ok(())
            }
        }
    }
    impl Drop for NativeFile {
        fn drop(&mut self) {
            let _ = self.close();
        }
    }
    impl FileSystem for NativeFs {
        type Handle = NativeFile;
        fn create(&self, path: &str) -> Result<NativeFile, Error> {
            libc::open_with_flags_mode(
                path.as_bytes(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW,
                0o600,
            )
            .map(|fd| NativeFile(Some(fd), volatile_path(path)))
            .map_err(io)
        }
        fn open(&self, path: &str) -> Result<NativeFile, Error> {
            libc::open(path.as_bytes())
                .map(|fd| NativeFile(Some(fd), volatile_path(path)))
                .map_err(io)
        }
        fn remove(&self, path: &str) -> Result<(), Error> {
            libc::unlink(path.as_bytes()).map_err(io)
        }
        fn publish(&self, staging: &str, target: &str, replace: bool) -> Result<(), Error> {
            if replace {
                libc::rename(staging.as_bytes(), target.as_bytes()).map_err(io)
            } else {
                libc::rename_no_replace(staging.as_bytes(), target.as_bytes()).map_err(io)
            }
        }
        fn sync_dir(&self, directory: &str) -> Result<(), Error> {
            if volatile_path(directory) {
                return Ok(());
            }
            libc::dir_sync(directory.as_bytes()).map_err(io)
        }
    }
}

#[cfg(test)]
mod tests;
