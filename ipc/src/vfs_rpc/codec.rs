//! Explicit little-endian request payloads. Credentials are deliberately absent:
//! they must arrive through the authenticated kernel dispatch context.
use super::{ProtocolError, MAX_PATH_BYTES, MAX_TRANSFER};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request<'a> {
    Open {
        path: &'a str,
        flags: u32,
        mode: u16,
    },
    Close {
        handle: u64,
    },
    Read {
        handle: u64,
        offset: u64,
        count: u32,
    },
    Write {
        handle: u64,
        offset: u64,
        data: &'a [u8],
    },
    Stat {
        path: &'a str,
    },
    Fstat {
        handle: u64,
    },
    ReadDir {
        handle: u64,
        cookie: u64,
        count: u32,
    },
    Mkdir {
        path: &'a str,
        mode: u16,
    },
    Unlink {
        path: &'a str,
    },
    Rename {
        old: &'a str,
        new: &'a str,
    },
    Chmod {
        path: &'a str,
        mode: u16,
    },
    Chown {
        path: &'a str,
        uid: u32,
        gid: u32,
    },
    Sync {
        handle: u64,
    },
    Truncate {
        handle: u64,
        size: u64,
    },
}

pub(super) struct Reader<'a>(pub(super) &'a [u8]);
impl<'a> Reader<'a> {
    pub(super) fn take(&mut self, count: usize) -> Result<&'a [u8], ProtocolError> {
        if count > self.0.len() {
            return Err(ProtocolError::Encoding);
        }
        let (head, tail) = self.0.split_at(count);
        self.0 = tail;
        Ok(head)
    }
    pub(super) fn u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub(super) fn u32(&mut self) -> Result<u32, ProtocolError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub(super) fn u64(&mut self) -> Result<u64, ProtocolError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn path(&mut self) -> Result<&'a str, ProtocolError> {
        let count = self.u16()? as usize;
        let path = core::str::from_utf8(self.take(count)?).map_err(|_| ProtocolError::Path)?;
        validate_path(path)?;
        Ok(path)
    }
    fn handle(&mut self) -> Result<u64, ProtocolError> {
        let value = self.u64()?;
        if value == 0 {
            return Err(ProtocolError::Encoding);
        }
        Ok(value)
    }
}

fn validate_path(path: &str) -> Result<(), ProtocolError> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.as_bytes().contains(&0) {
        return Err(ProtocolError::Path);
    }
    Ok(())
}
fn validate_range(offset: u64, count: u64) -> Result<(), ProtocolError> {
    if count > MAX_TRANSFER || offset.checked_add(count).is_none() {
        return Err(ProtocolError::Bounds);
    }
    Ok(())
}

impl<'a> Request<'a> {
    /// u16 opcode followed by operation fields. Unknown operations, trailing
    /// bytes and malformed fields fail closed. Offsets are explicit; seek and
    /// shared open-description offset serialization remain kernel work.
    pub fn decode(bytes: &'a [u8]) -> Result<Self, ProtocolError> {
        if bytes.len() as u64 > MAX_TRANSFER {
            return Err(ProtocolError::Bounds);
        }
        let mut r = Reader(bytes);
        let request = match r.u16()? {
            1 => Self::Open {
                flags: r.u32()?,
                mode: r.u16()?,
                path: r.path()?,
            },
            2 => Self::Close {
                handle: r.handle()?,
            },
            3 => Self::Read {
                handle: r.handle()?,
                offset: r.u64()?,
                count: r.u32()?,
            },
            4 => {
                let handle = r.handle()?;
                let offset = r.u64()?;
                let count = r.u32()? as usize;
                Self::Write {
                    handle,
                    offset,
                    data: r.take(count)?,
                }
            }
            5 => Self::Stat { path: r.path()? },
            6 => Self::Fstat {
                handle: r.handle()?,
            },
            7 => Self::ReadDir {
                handle: r.handle()?,
                cookie: r.u64()?,
                count: r.u32()?,
            },
            8 => Self::Mkdir {
                mode: r.u16()?,
                path: r.path()?,
            },
            9 => Self::Unlink { path: r.path()? },
            10 => Self::Rename {
                old: r.path()?,
                new: r.path()?,
            },
            11 => Self::Chmod {
                mode: r.u16()?,
                path: r.path()?,
            },
            12 => Self::Chown {
                uid: r.u32()?,
                gid: r.u32()?,
                path: r.path()?,
            },
            13 => Self::Sync {
                handle: r.handle()?,
            },
            14 => Self::Truncate {
                handle: r.handle()?,
                size: r.u64()?,
            },
            _ => return Err(ProtocolError::Operation),
        };
        if !r.0.is_empty() {
            return Err(ProtocolError::Encoding);
        }
        request.validate()?;
        Ok(request)
    }

    /// Validate before resource admission as well as after wire decoding.
    pub fn validate(self) -> Result<(), ProtocolError> {
        match self {
            Self::Open { path, flags, mode } => {
                validate_path(path)?;
                // Same bit values as the native/Linux open ABI. No direct I/O,
                // unknown flags, or reserved access mode 3 on this protocol.
                const SUPPORTED: u32 = 3
                    | 0x40
                    | 0x80
                    | 0x100
                    | 0x200
                    | 0x400
                    | 0x800
                    | 0x8000
                    | 0x10000
                    | 0x20000
                    | 0x80000;
                if flags & !SUPPORTED != 0 || flags & 3 == 3 || mode & !0o7777 != 0 {
                    return Err(ProtocolError::Encoding);
                }
            }
            Self::Stat { path } | Self::Unlink { path } => validate_path(path)?,
            Self::Mkdir { path, mode } | Self::Chmod { path, mode } => {
                validate_path(path)?;
                if mode & !0o7777 != 0 {
                    return Err(ProtocolError::Encoding);
                }
            }
            Self::Chown { path, .. } => validate_path(path)?,
            Self::Rename { old, new } => {
                validate_path(old)?;
                validate_path(new)?;
            }
            _ => {}
        }
        let handle = match self {
            Self::Close { handle }
            | Self::Read { handle, .. }
            | Self::Write { handle, .. }
            | Self::Fstat { handle }
            | Self::ReadDir { handle, .. }
            | Self::Sync { handle }
            | Self::Truncate { handle, .. } => Some(handle),
            _ => None,
        };
        if handle == Some(0) {
            return Err(ProtocolError::Encoding);
        }
        match self {
            Self::Read { offset, count, .. } => validate_range(offset, count as u64)?,
            Self::Write { offset, data, .. } => validate_range(offset, data.len() as u64)?,
            Self::ReadDir { count, .. } => validate_range(0, count as u64)?,
            _ => {}
        }
        if self.encoded_len() as u64 > MAX_TRANSFER {
            return Err(ProtocolError::Bounds);
        }
        Ok(())
    }

    /// Exact input length, distinct from the successful response capacity.
    /// Call validate() before using this value for admission.
    pub fn encoded_len(self) -> usize {
        match self {
            Self::Open { path, .. } => 10 + path.len(),
            Self::Close { .. } | Self::Fstat { .. } | Self::Sync { .. } => 10,
            Self::Read { .. } | Self::ReadDir { .. } => 22,
            Self::Write { data, .. } => 22 + data.len(),
            Self::Stat { path } | Self::Unlink { path } => 4 + path.len(),
            Self::Mkdir { path, .. } | Self::Chmod { path, .. } => 6 + path.len(),
            Self::Rename { old, new } => 6 + old.len() + new.len(),
            Self::Chown { path, .. } => 12 + path.len(),
            Self::Truncate { .. } => 18,
        }
    }

    pub fn encode(self, output: &mut [u8]) -> Result<usize, ProtocolError> {
        self.validate()?;
        let mut w = Writer { output, used: 0 };
        match self {
            Self::Open { path, flags, mode } => {
                w.u16(1)?;
                w.u32(flags)?;
                w.u16(mode)?;
                w.path(path)?;
            }
            Self::Close { handle } => {
                w.u16(2)?;
                w.u64(handle)?;
            }
            Self::Read {
                handle,
                offset,
                count,
            } => {
                w.u16(3)?;
                w.u64(handle)?;
                w.u64(offset)?;
                w.u32(count)?;
            }
            Self::Write {
                handle,
                offset,
                data,
            } => {
                w.u16(4)?;
                w.u64(handle)?;
                w.u64(offset)?;
                w.u32(u32::try_from(data.len()).map_err(|_| ProtocolError::Bounds)?)?;
                w.put(data)?;
            }
            Self::Stat { path } => {
                w.u16(5)?;
                w.path(path)?;
            }
            Self::Fstat { handle } => {
                w.u16(6)?;
                w.u64(handle)?;
            }
            Self::ReadDir {
                handle,
                cookie,
                count,
            } => {
                w.u16(7)?;
                w.u64(handle)?;
                w.u64(cookie)?;
                w.u32(count)?;
            }
            Self::Mkdir { path, mode } => {
                w.u16(8)?;
                w.u16(mode)?;
                w.path(path)?;
            }
            Self::Unlink { path } => {
                w.u16(9)?;
                w.path(path)?;
            }
            Self::Rename { old, new } => {
                w.u16(10)?;
                w.path(old)?;
                w.path(new)?;
            }
            Self::Chmod { path, mode } => {
                w.u16(11)?;
                w.u16(mode)?;
                w.path(path)?;
            }
            Self::Chown { path, uid, gid } => {
                w.u16(12)?;
                w.u32(uid)?;
                w.u32(gid)?;
                w.path(path)?;
            }
            Self::Sync { handle } => {
                w.u16(13)?;
                w.u64(handle)?;
            }
            Self::Truncate { handle, size } => {
                w.u16(14)?;
                w.u64(handle)?;
                w.u64(size)?;
            }
        }
        Request::decode(&w.output[..w.used])?;
        Ok(w.used)
    }
}

pub(super) struct Writer<'a> {
    pub(super) output: &'a mut [u8],
    pub(super) used: usize,
}
impl Writer<'_> {
    pub(super) fn put(&mut self, bytes: &[u8]) -> Result<(), ProtocolError> {
        let end = self
            .used
            .checked_add(bytes.len())
            .ok_or(ProtocolError::Bounds)?;
        if end > self.output.len() || end as u64 > MAX_TRANSFER {
            return Err(ProtocolError::Bounds);
        }
        self.output[self.used..end].copy_from_slice(bytes);
        self.used = end;
        Ok(())
    }
    pub(super) fn u16(&mut self, v: u16) -> Result<(), ProtocolError> {
        self.put(&v.to_le_bytes())
    }
    pub(super) fn u32(&mut self, v: u32) -> Result<(), ProtocolError> {
        self.put(&v.to_le_bytes())
    }
    pub(super) fn u64(&mut self, v: u64) -> Result<(), ProtocolError> {
        self.put(&v.to_le_bytes())
    }
    fn path(&mut self, path: &str) -> Result<(), ProtocolError> {
        validate_path(path)?;
        self.u16(path.len() as u16)?;
        self.put(path.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    #[test]
    fn operations_roundtrip_and_reject_every_truncation() {
        for request in [
            Request::Open {
                path: "/tmp/hello",
                flags: 0,
                mode: 0o600,
            },
            Request::Close { handle: 1 },
            Request::Read {
                handle: 1,
                offset: 12,
                count: 50,
            },
            Request::Write {
                handle: 2,
                offset: 0,
                data: b"abc\0def",
            },
            Request::Stat { path: "/" },
            Request::Fstat { handle: 2 },
            Request::ReadDir {
                handle: 3,
                cookie: 0,
                count: 1024,
            },
            Request::Mkdir {
                path: "relative",
                mode: 0o700,
            },
            Request::Unlink { path: "old" },
            Request::Rename {
                old: "old",
                new: "new",
            },
            Request::Chmod {
                path: "/tmp",
                mode: 0o1777,
            },
            Request::Chown {
                path: "/tmp",
                uid: 1000,
                gid: 1000,
            },
            Request::Sync { handle: 1 },
            Request::Truncate {
                handle: 1,
                size: 100,
            },
        ] {
            let mut buffer = [0u8; 256];
            let size = request.encode(&mut buffer).unwrap();
            assert_eq!(Request::decode(&buffer[..size]), Ok(request));
            for end in 0..size {
                assert!(Request::decode(&buffer[..end]).is_err());
            }
            assert_eq!(
                Request::decode(&buffer[..size + 1]),
                Err(ProtocolError::Encoding)
            );
        }
    }
    #[test]
    fn hostile_fields_fail_closed() {
        let mut buffer = [0; 128];
        for path in ["", "a\0b"] {
            assert!(Request::Stat { path }.encode(&mut buffer).is_err());
        }
        assert_eq!(
            Request::decode(&[5, 0, 1, 0, 0xff]),
            Err(ProtocolError::Path)
        );
        assert_eq!(
            Request::decode(&[0xff, 0xff]),
            Err(ProtocolError::Operation)
        );
        assert!(Request::Read {
            handle: 1,
            offset: u64::MAX,
            count: 1
        }
        .encode(&mut buffer)
        .is_err());
        assert!(Request::Close { handle: 0 }.encode(&mut buffer).is_err());
        assert!(Request::Close { handle: 1 }
            .encode(&mut buffer[..9])
            .is_err());
    }
    #[test]
    fn path_limit_is_bytes_and_little_endian_is_explicit() {
        let path = std::string::String::from_utf8(std::vec![b'a'; MAX_PATH_BYTES]).unwrap();
        let mut buffer = std::vec![0; MAX_PATH_BYTES + 16];
        let size = Request::Stat { path: &path }.encode(&mut buffer).unwrap();
        assert_eq!(&buffer[..4], &[5, 0, 0xff, 0x0f]);
        assert_eq!(size, MAX_PATH_BYTES + 4);
        let long = path + "a";
        assert_eq!(
            Request::Stat { path: &long }.encode(&mut buffer),
            Err(ProtocolError::Path)
        );
    }
}
