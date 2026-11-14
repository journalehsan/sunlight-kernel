//! Linux hosted disposable image safety. All ancestor components must be literal
//! canonical directories; creation is relative to an already opened directory fd.
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path},
};
use sunlight_sunfs::{block::*, Error};
pub const MAX_IMAGE: u64 = 128 * 1024 * 1024;
pub struct Image {
    file: File,
    info: DeviceInfo,
}
pub fn parse_size(value: &str) -> Result<u64, String> {
    let (digits, mul) = if let Some(n) = value.strip_suffix("MiB") {
        (n, 1024 * 1024)
    } else if let Some(n) = value.strip_suffix("GiB") {
        (n, 1024 * 1024 * 1024)
    } else {
        (value, 1)
    };
    if digits.is_empty() || !digits.bytes().all(|v| v.is_ascii_digit()) {
        return Err("expected integer bytes or MiB/GiB".into());
    }
    let size = digits
        .parse::<u64>()
        .map_err(|_| "size overflow")?
        .checked_mul(mul)
        .ok_or("size overflow")?;
    if size == 0 || size > MAX_IMAGE || !size.is_multiple_of(4096) {
        return Err("size must be 4KiB aligned, positive, at most 128MiB for this adapter".into());
    }
    Ok(size)
}
impl Image {
    pub fn create(path: &Path, size: u64, confirm: bool) -> io::Result<Self> {
        if !confirm || size == 0 || size > MAX_IMAGE || !size.is_multiple_of(4096) {
            return Err(io::Error::other("disposable confirmation/size required"));
        }
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        if absolute
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(io::Error::other("path aliases refused"));
        }
        let parent = absolute
            .parent()
            .ok_or_else(|| io::Error::other("missing parent"))?;
        let canonical = parent.canonicalize()?;
        if parent != canonical {
            return Err(io::Error::other("symlink or noncanonical ancestor refused"));
        }
        // O_DIRECTORY|O_NOFOLLOW. Pin the resolved parent, then create through fd.
        use std::os::fd::AsRawFd;
        let mut directory = OpenOptions::new()
            .read(true)
            .custom_flags(0x10000 | 0x20000)
            .open("/")?;
        for component in canonical.components() {
            if let Component::Normal(name) = component {
                let step = Path::new("/proc/self/fd")
                    .join(directory.as_raw_fd().to_string())
                    .join(name);
                directory = OpenOptions::new()
                    .read(true)
                    .custom_flags(0x10000 | 0x20000)
                    .open(step)?;
            }
        }
        let dm = directory.metadata()?;
        let current = std::fs::metadata(&canonical)?;
        if dm.dev() != current.dev() || dm.ino() != current.ino() {
            return Err(io::Error::other("directory changed"));
        }
        let name = absolute
            .file_name()
            .ok_or_else(|| io::Error::other("missing filename"))?;
        let target = Path::new("/proc/self/fd")
            .join(directory.as_raw_fd().to_string())
            .join(name);
        // create_new is exclusive and implies no final symlink following on Linux.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .custom_flags(0x20000)
            .open(target)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.nlink() != 1 {
            return Err(io::Error::other("regular exclusive file required"));
        }
        file.set_len(size)?;
        // Persist the name and initial length as well as future image contents.
        file.sync_all()?;
        directory.sync_all()?;
        Ok(Self {
            file,
            info: DeviceInfo {
                capacity: size,
                logical_block: 512,
                physical_block: None,
                alignment: 512,
                max_transfer: 4096,
                identity: None,
                incarnation: 1,
                read_only: false,
                flush: true,
                fua: false,
            },
        })
    }
}
impl BlockDevice for Image {
    fn info(&self) -> DeviceInfo {
        self.info
    }
    fn read(&mut self, offset: u64, output: &mut [u8]) -> IoResult<()> {
        self.info.range(offset, output.len())?;
        let mut temp = vec![0; output.len()];
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.read_exact(&mut temp))
            .map_err(|_| IoFailure::known(IoKind::Read, 0))?;
        output.copy_from_slice(&temp);
        Ok(())
    }
    fn write(&mut self, offset: u64, input: &[u8]) -> IoResult<WriteCompletion> {
        self.info.range(offset, input.len())?;
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.write_all(input))
            .map_err(|_| IoFailure::uncertain(IoKind::Write))?;
        Ok(WriteCompletion::Completed)
    }
    fn flush(&mut self) -> IoResult<()> {
        self.file
            .sync_all()
            .map_err(|_| IoFailure::uncertain(IoKind::Flush))
    }
}
pub fn random_uuid() -> io::Result<[u8; 16]> {
    let mut uuid = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut uuid)?;
    if uuid == [0; 16] {
        return Err(io::Error::other("zero random identity"));
    }
    Ok(uuid)
}
pub fn core_error(e: Error) -> io::Error {
    io::Error::other(format!("{e:?}"))
}
