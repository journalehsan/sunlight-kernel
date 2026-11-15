//! Successful reply bodies and operation-specific admission/completion checks.
//! All fields are little endian. These bytes carry no credential authority.
use super::codec::{Reader, Request, Writer};
use super::{Completion, ProtocolError, MAX_TRANSFER};

/// Transport envelope plus the copied, immutable initialized output snapshot.
/// No reply capabilities are permitted in v1. Buffer authority is retained by
/// the kernel pending operation, never accepted back from the service.
pub struct ReplyMessage<'a> {
    pub label: u64,
    pub words: [u64; 4],
    pub word_count: usize,
    pub cap_count: usize,
    pub output: &'a [u8],
}

pub const METADATA_BYTES: usize = 56;
pub const OPEN_BYTES: usize = 8 + METADATA_BYTES;
pub const DIRECTORY_HEADER_BYTES: usize = 16;
pub const MAX_NAME_BYTES: usize = 255;

/// Mount and object generations are distinct from service registration epochs.
/// A retained object must match all four fields, even after rename/unlink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub mount_id: u64,
    pub mount_generation: u64,
    pub object_id: u64,
    pub object_generation: u64,
}
impl Provenance {
    fn decode(r: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let value = Self {
            mount_id: r.u64()?,
            mount_generation: r.u64()?,
            object_id: r.u64()?,
            object_generation: r.u64()?,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(self) -> Result<(), ProtocolError> {
        if [
            self.mount_id,
            self.mount_generation,
            self.object_id,
            self.object_generation,
        ]
        .contains(&0)
        {
            return Err(ProtocolError::Encoding);
        }
        Ok(())
    }
    fn encode(self, w: &mut Writer<'_>) -> Result<(), ProtocolError> {
        self.validate()?;
        for value in [
            self.mount_id,
            self.mount_generation,
            self.object_id,
            self.object_generation,
        ] {
            w.u64(value)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File = 1,
    Directory = 2,
}
impl Kind {
    fn decode(r: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        match r.u16()? {
            1 => Ok(Self::File),
            2 => Ok(Self::Directory),
            _ => Err(ProtocolError::Encoding),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub provenance: Provenance,
    pub kind: Kind,
    /// Permission and special bits only; kind is carried separately.
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    /// Zero links is valid for a retained, unlinked object.
    pub links: u32,
    pub size: u64,
}
impl Metadata {
    fn decode(r: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let value = Self {
            provenance: Provenance::decode(r)?,
            kind: Kind::decode(r)?,
            mode: r.u16()?,
            uid: r.u32()?,
            gid: r.u32()?,
            links: r.u32()?,
            size: r.u64()?,
        };
        if value.mode & !0o7777 != 0 {
            return Err(ProtocolError::Encoding);
        }
        Ok(value)
    }
    fn encode(self, w: &mut Writer<'_>) -> Result<(), ProtocolError> {
        if self.mode & !0o7777 != 0 {
            return Err(ProtocolError::Encoding);
        }
        self.provenance.encode(w)?;
        w.u16(self.kind as u16)?;
        w.u16(self.mode)?;
        w.u32(self.uid)?;
        w.u32(self.gid)?;
        w.u32(self.links)?;
        w.u64(self.size)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Open,
    Read(u32),
    Write(u32),
    Metadata,
    Directory { cookie: u64, capacity: u32 },
    Empty,
}

/// Retain this small, owned value with a pending call; never borrow the input
/// path/buffer across a wait. Input and output reuse the same transfer range in
/// successive phases. Its allocation must fit max(input_bytes, output_capacity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplyContract {
    shape: Shape,
    pub input_bytes: usize,
    pub output_capacity: usize,
}
impl ReplyContract {
    pub fn complete(
        self,
        transfer: super::Transfer,
        reply: ReplyMessage<'_>,
    ) -> Result<Completion, ProtocolError> {
        if reply.label != super::REPLY || reply.cap_count != 0 {
            return Err(ProtocolError::Encoding);
        }
        let completion = super::decode_reply(transfer, reply.words, reply.word_count)?;
        self.validate(completion, reply.output)?;
        Ok(completion)
    }

    pub fn for_request(request: Request<'_>) -> Result<Self, ProtocolError> {
        request.validate()?;
        let (shape, output_capacity) = match request {
            Request::Open { .. } => (Shape::Open, OPEN_BYTES),
            Request::Read { count, .. } => (Shape::Read(count), count as usize),
            Request::Write { data, .. } => (Shape::Write(data.len() as u32), 4),
            Request::Stat { .. } | Request::Fstat { .. } => (Shape::Metadata, METADATA_BYTES),
            Request::ReadDir { cookie, count, .. } => {
                if (count as usize) < DIRECTORY_HEADER_BYTES {
                    return Err(ProtocolError::Bounds);
                }
                (
                    Shape::Directory {
                        cookie,
                        capacity: count,
                    },
                    count as usize,
                )
            }
            _ => (Shape::Empty, 0),
        };
        Ok(Self {
            shape,
            input_bytes: request.encoded_len(),
            output_capacity,
        })
    }

    pub fn buffer_bytes(self) -> usize {
        self.input_bytes.max(self.output_capacity)
    }

    /// The caller provides an immutable kernel snapshot of initialized output,
    /// not a concurrently writable service mapping. Error replies carry no body.
    /// Check label/capability count at the transport boundary as well.
    pub fn validate<'a>(
        self,
        completion: Completion,
        bytes: &'a [u8],
    ) -> Result<Reply<'a>, ProtocolError> {
        if completion.errno > 4095
            || completion.output_bytes != bytes.len() as u64
            || bytes.len() > self.output_capacity
        {
            return Err(ProtocolError::Bounds);
        }
        if completion.errno != 0 {
            return if bytes.is_empty() {
                Ok(Reply::Empty)
            } else {
                Err(ProtocolError::Status)
            };
        }
        let mut r = Reader(bytes);
        let reply = match self.shape {
            Shape::Open => {
                let handle = r.u64()?;
                if handle == 0 {
                    return Err(ProtocolError::Encoding);
                }
                Reply::Opened {
                    handle,
                    metadata: Metadata::decode(&mut r)?,
                }
            }
            Shape::Read(_) => {
                r.take(bytes.len())?;
                Reply::Read(bytes)
            }
            Shape::Write(limit) => {
                let count = r.u32()?;
                if count > limit {
                    return Err(ProtocolError::Bounds);
                }
                Reply::Written(count)
            }
            Shape::Metadata => Reply::Metadata(Metadata::decode(&mut r)?),
            Shape::Directory { cookie, .. } => {
                let page = DirectoryPage::decode(bytes, cookie)?;
                r.take(bytes.len())?;
                Reply::Directory(page)
            }
            Shape::Empty => Reply::Empty,
        };
        if !r.0.is_empty() {
            return Err(ProtocolError::Encoding);
        }
        Ok(reply)
    }

    /// Encode only bodies valid for this operation. No payload length or native
    /// Rust struct layout is trusted on decode.
    pub fn encode(self, reply: Reply<'_>, output: &mut [u8]) -> Result<usize, ProtocolError> {
        let mut w = Writer { output, used: 0 };
        match reply {
            Reply::Opened { handle, metadata } => {
                w.u64(handle)?;
                metadata.encode(&mut w)?;
            }
            Reply::Read(bytes) => w.put(bytes)?,
            Reply::Written(count) => w.u32(count)?,
            Reply::Metadata(value) => value.encode(&mut w)?,
            Reply::Directory(page) => w.put(page.bytes)?,
            Reply::Empty => {}
        }
        let decoded = self.validate(
            Completion {
                errno: 0,
                output_bytes: w.used as u64,
            },
            &w.output[..w.used],
        )?;
        // Equal-length payloads of different operation shapes are not synonyms.
        if core::mem::discriminant(&decoded) != core::mem::discriminant(&reply) {
            return Err(ProtocolError::Operation);
        }
        Ok(w.used)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply<'a> {
    Opened { handle: u64, metadata: Metadata },
    Read(&'a [u8]),
    Written(u32),
    Metadata(Metadata),
    Directory(DirectoryPage<'a>),
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryEntry<'a> {
    pub provenance: Provenance,
    /// A monotonic opaque position, scoped to this open directory handle.
    pub cookie: u64,
    pub kind: Kind,
    pub name: &'a str,
}
impl<'a> DirectoryEntry<'a> {
    fn decode(r: &mut Reader<'a>) -> Result<Self, ProtocolError> {
        let provenance = Provenance::decode(r)?;
        let cookie = r.u64()?;
        let kind = Kind::decode(r)?;
        let len = r.u16()? as usize;
        let name = core::str::from_utf8(r.take(len)?).map_err(|_| ProtocolError::Path)?;
        validate_name(name)?;
        Ok(Self {
            provenance,
            cookie,
            kind,
            name,
        })
    }
}
fn validate_name(name: &str) -> Result<(), ProtocolError> {
    if name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || name.as_bytes().contains(&0)
        || name.contains('/')
    {
        return Err(ProtocolError::Path);
    }
    Ok(())
}

/// Header: next cookie u64, EOF u16 (0/1), entry count u16, reserved u32 (0).
/// Entries: provenance, cookie u64, kind u16, name length u16, UTF-8 name.
/// Cookies increase strictly, never wrap, and the last equals next_cookie.
/// An empty page must be EOF and retain the request cookie. Mutation that
/// invalidates a cursor returns ESTALE; servers must not restart it silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryPage<'a> {
    bytes: &'a [u8],
    pub next_cookie: u64,
    pub eof: bool,
    pub entry_count: u16,
}
impl<'a> DirectoryPage<'a> {
    pub fn decode(bytes: &'a [u8], start_cookie: u64) -> Result<Self, ProtocolError> {
        if bytes.len() as u64 > MAX_TRANSFER {
            return Err(ProtocolError::Bounds);
        }
        let mut r = Reader(bytes);
        let next_cookie = r.u64()?;
        let eof = match r.u16()? {
            0 => false,
            1 => true,
            _ => return Err(ProtocolError::Encoding),
        };
        let entry_count = r.u16()?;
        if r.u32()? != 0 {
            return Err(ProtocolError::Encoding);
        }
        let mut previous = start_cookie;
        for _ in 0..entry_count {
            let entry = DirectoryEntry::decode(&mut r)?;
            if entry.cookie <= previous {
                return Err(ProtocolError::Encoding);
            }
            previous = entry.cookie;
        }
        if !r.0.is_empty() || next_cookie != previous || (entry_count == 0 && !eof) {
            return Err(ProtocolError::Encoding);
        }
        Ok(Self {
            bytes,
            next_cookie,
            eof,
            entry_count,
        })
    }

    pub fn encode(
        entries: &[DirectoryEntry<'_>],
        start_cookie: u64,
        eof: bool,
        output: &'a mut [u8],
    ) -> Result<Self, ProtocolError> {
        let mut w = Writer { output, used: 0 };
        w.u64(entries.last().map_or(start_cookie, |e| e.cookie))?;
        w.u16(u16::from(eof))?;
        w.u16(u16::try_from(entries.len()).map_err(|_| ProtocolError::Bounds)?)?;
        w.u32(0)?;
        for entry in entries {
            validate_name(entry.name)?;
            entry.provenance.encode(&mut w)?;
            w.u64(entry.cookie)?;
            w.u16(entry.kind as u16)?;
            w.u16(entry.name.len() as u16)?;
            w.put(entry.name.as_bytes())?;
        }
        Self::decode(&w.output[..w.used], start_cookie)
    }

    /// Full validation precedes iteration; errors are still surfaced rather
    /// than silently treating malformed entries as end-of-directory.
    pub fn entries(self) -> impl Iterator<Item = Result<DirectoryEntry<'a>, ProtocolError>> {
        let mut r = Reader(&self.bytes[DIRECTORY_HEADER_BYTES..]);
        (0..self.entry_count).map(move |_| DirectoryEntry::decode(&mut r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROVENANCE: Provenance = Provenance {
        mount_id: 1,
        mount_generation: 2,
        object_id: 3,
        object_generation: 4,
    };
    const METADATA: Metadata = Metadata {
        provenance: PROVENANCE,
        kind: Kind::File,
        mode: 0o640,
        uid: 1000,
        gid: 100,
        links: 0,
        size: u64::MAX,
    };
    fn success(bytes: &[u8]) -> Completion {
        Completion {
            errno: 0,
            output_bytes: bytes.len() as u64,
        }
    }
    fn contract(request: Request<'_>) -> ReplyContract {
        ReplyContract::for_request(request).unwrap()
    }
    fn entry(cookie: u64, name: &str) -> DirectoryEntry<'_> {
        DirectoryEntry {
            provenance: PROVENANCE,
            cookie,
            kind: Kind::File,
            name,
        }
    }

    #[test]
    fn response_shapes_roundtrip_and_fixed_shapes_reject_all_truncations() {
        for (request, reply) in [
            (
                Request::Open {
                    path: "/",
                    flags: 0,
                    mode: 0,
                },
                Reply::Opened {
                    handle: 7,
                    metadata: METADATA,
                },
            ),
            (Request::Stat { path: "/" }, Reply::Metadata(METADATA)),
            (Request::Fstat { handle: 7 }, Reply::Metadata(METADATA)),
            (
                Request::Write {
                    handle: 7,
                    offset: 0,
                    data: b"abc",
                },
                Reply::Written(2),
            ),
        ] {
            let c = contract(request);
            let mut buffer = [0; 128];
            let size = c.encode(reply, &mut buffer).unwrap();
            assert_eq!(size, c.output_capacity);
            assert_eq!(
                c.validate(success(&buffer[..size]), &buffer[..size]),
                Ok(reply)
            );
            for end in 0..size {
                assert!(c.validate(success(&buffer[..end]), &buffer[..end]).is_err());
            }
            assert!(c
                .validate(success(&buffer[..size + 1]), &buffer[..size + 1])
                .is_err());
        }
    }

    #[test]
    fn metadata_provenance_and_shape_are_not_native_layout() {
        let c = contract(Request::Stat { path: "/" });
        let mut buffer = [0; METADATA_BYTES];
        c.encode(Reply::Metadata(METADATA), &mut buffer).unwrap();
        assert_eq!(&buffer[..8], &1u64.to_le_bytes());
        assert_eq!(&buffer[32..36], &[1, 0, 0xa0, 1]);
        for start in [0, 8, 16, 24] {
            let mut bad = buffer;
            bad[start..start + 8].fill(0);
            assert!(c.validate(success(&bad), &bad).is_err());
        }
        for (start, value) in [(32, 0u16), (32, 3), (34, 0o100000)] {
            let mut bad = buffer;
            bad[start..start + 2].copy_from_slice(&value.to_le_bytes());
            assert!(c.validate(success(&bad), &bad).is_err());
        }
        let c = contract(Request::Open {
            path: "/",
            flags: 0,
            mode: 0,
        });
        let mut buffer = [0; OPEN_BYTES];
        assert!(c
            .encode(
                Reply::Opened {
                    handle: 0,
                    metadata: METADATA
                },
                &mut buffer
            )
            .is_err());
    }

    #[test]
    fn input_and_output_admission_are_separate_for_every_operation() {
        for (request, capacity) in [
            (
                Request::Open {
                    path: "/",
                    flags: 0,
                    mode: 0,
                },
                OPEN_BYTES,
            ),
            (Request::Close { handle: 1 }, 0),
            (
                Request::Read {
                    handle: 1,
                    offset: 2,
                    count: 7,
                },
                7,
            ),
            (
                Request::Write {
                    handle: 1,
                    offset: 2,
                    data: b"hello",
                },
                4,
            ),
            (Request::Stat { path: "/" }, METADATA_BYTES),
            (Request::Fstat { handle: 1 }, METADATA_BYTES),
            (
                Request::ReadDir {
                    handle: 1,
                    cookie: 9,
                    count: 128,
                },
                128,
            ),
            (
                Request::Mkdir {
                    path: "/a",
                    mode: 0o700,
                },
                0,
            ),
            (Request::Unlink { path: "/a" }, 0),
            (
                Request::Rename {
                    old: "/a",
                    new: "/b",
                },
                0,
            ),
            (
                Request::Chmod {
                    path: "/a",
                    mode: 0o600,
                },
                0,
            ),
            (
                Request::Chown {
                    path: "/a",
                    uid: 1,
                    gid: 2,
                },
                0,
            ),
            (Request::Sync { handle: 1 }, 0),
            (Request::Truncate { handle: 1, size: 2 }, 0),
        ] {
            let c = contract(request);
            let mut input = [0; 128];
            assert_eq!(request.encode(&mut input).unwrap(), c.input_bytes);
            assert_eq!(c.output_capacity, capacity);
            assert_eq!(c.buffer_bytes(), c.input_bytes.max(capacity));
            assert_eq!(
                c.validate(
                    Completion {
                        errno: 5,
                        output_bytes: 0
                    },
                    &[]
                ),
                Ok(Reply::Empty)
            );
            assert!(c
                .validate(
                    Completion {
                        errno: 5,
                        output_bytes: 1
                    },
                    &[0]
                )
                .is_err());
            if capacity == 0 {
                assert_eq!(c.validate(success(&[]), &[]), Ok(Reply::Empty));
                assert!(c.validate(success(&[0]), &[0]).is_err());
            }
        }
    }

    #[test]
    fn short_io_has_operation_limits_and_write_counts_are_not_output_lengths() {
        let c = contract(Request::Read {
            handle: 1,
            offset: 0,
            count: 3,
        });
        for data in [&b""[..], b"a", b"abc"] {
            assert_eq!(c.validate(success(data), data), Ok(Reply::Read(data)));
        }
        assert!(c.validate(success(b"abcd"), b"abcd").is_err());
        assert!(c
            .validate(
                Completion {
                    errno: 0,
                    output_bytes: 2
                },
                b"a"
            )
            .is_err());
        let c = contract(Request::Write {
            handle: 1,
            offset: 0,
            data: b"abc",
        });
        let mut buffer = [0; 4];
        for count in [0, 1, 3] {
            assert_eq!(c.encode(Reply::Written(count), &mut buffer), Ok(4));
        }
        assert!(c.encode(Reply::Written(4), &mut buffer).is_err());
        assert!(c.encode(Reply::Read(&[0; 4]), &mut buffer).is_err());
    }

    #[test]
    fn request_validation_precedes_admission_and_preserves_transfer_ceiling() {
        for request in [
            Request::Open {
                path: "/",
                flags: 0x4000,
                mode: 0,
            },
            Request::Open {
                path: "/",
                flags: 3,
                mode: 0,
            },
            Request::Open {
                path: "/",
                flags: 1 << 31,
                mode: 0,
            },
            Request::Open {
                path: "/",
                flags: 0,
                mode: 0o100000,
            },
            Request::Chmod {
                path: "/",
                mode: 0o100000,
            },
            Request::Close { handle: 0 },
            Request::ReadDir {
                handle: 1,
                cookie: 0,
                count: 15,
            },
        ] {
            assert!(ReplyContract::for_request(request).is_err());
        }
        // Write overhead is part of the transport ceiling, unlike Read output.
        let data = [0; MAX_TRANSFER as usize];
        let c = contract(Request::Write {
            handle: 1,
            offset: 0,
            data: &data[..data.len() - 22],
        });
        assert_eq!(c.buffer_bytes(), MAX_TRANSFER as usize);
        assert!(ReplyContract::for_request(Request::Write {
            handle: 1,
            offset: 0,
            data: &data[..data.len() - 21]
        })
        .is_err());
        assert_eq!(
            contract(Request::Read {
                handle: 1,
                offset: 0,
                count: MAX_TRANSFER as u32
            })
            .buffer_bytes(),
            MAX_TRANSFER as usize
        );
        // Existing O_LARGEFILE and other accepted compatibility bits survive.
        assert!(ReplyContract::for_request(Request::Open {
            path: "/",
            flags: 0xb8fc2,
            mode: 0o1777
        })
        .is_ok());
    }

    #[test]
    fn directory_pages_roundtrip_with_handle_scoped_monotonic_cookies() {
        let entries = [entry(10, "."), entry(19, "hello"), entry(u64::MAX, "آخر")];
        let mut buffer = [0; 256];
        let page = DirectoryPage::encode(&entries, 9, true, &mut buffer).unwrap();
        assert_eq!(page.next_cookie, u64::MAX);
        assert!(page.eof);
        assert_eq!(page.entry_count, 3);
        for (got, expected) in page.entries().zip(entries) {
            assert_eq!(got.unwrap(), expected);
        }
        let c = contract(Request::ReadDir {
            handle: 1,
            cookie: 9,
            count: 256,
        });
        assert_eq!(
            c.validate(success(page.bytes), page.bytes),
            Ok(Reply::Directory(page))
        );
        for end in 0..page.bytes.len() {
            assert!(DirectoryPage::decode(&page.bytes[..end], 9).is_err());
        }
        let c = contract(Request::ReadDir {
            handle: 1,
            cookie: 10,
            count: 256,
        });
        assert!(c.validate(success(page.bytes), page.bytes).is_err());
        let mut empty = [0; DIRECTORY_HEADER_BYTES];
        assert!(DirectoryPage::encode(&[], 9, false, &mut empty).is_err());
        let end = DirectoryPage::encode(&[], u64::MAX, true, &mut empty).unwrap();
        assert_eq!(end.next_cookie, u64::MAX);
    }

    #[test]
    fn directory_rejects_bad_names_cookies_headers_and_trailing_data() {
        let mut buffer = [0; 256];
        for name in ["", "a/b", "a\0b"] {
            assert!(DirectoryPage::encode(&[entry(1, name)], 0, true, &mut buffer).is_err());
        }
        for entries in [
            [entry(1, "a"), entry(1, "b")],
            [entry(2, "a"), entry(1, "b")],
        ] {
            assert!(DirectoryPage::encode(&entries, 0, true, &mut buffer).is_err());
        }
        assert!(DirectoryPage::encode(&[entry(0, "a")], u64::MAX, true, &mut buffer).is_err());
        let size = DirectoryPage::encode(&[entry(1, "a")], 0, false, &mut buffer)
            .unwrap()
            .bytes
            .len();
        for (at, value) in [(0, 2), (8, 2), (10, 2), (12, 1), (size - 1, 0xff)] {
            let mut bad = buffer;
            bad[at] = value;
            assert!(DirectoryPage::decode(&bad[..size], 0).is_err());
        }
        assert!(DirectoryPage::decode(&buffer[..size + 1], 0).is_err());
        let long_name = [b'a'; MAX_NAME_BYTES + 1];
        assert!(DirectoryPage::encode(
            &[entry(1, core::str::from_utf8(&long_name).unwrap())],
            0,
            true,
            &mut buffer
        )
        .is_err());
    }
}
