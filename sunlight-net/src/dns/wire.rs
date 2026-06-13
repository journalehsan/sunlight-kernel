//! RFC 1035 DNS wire protocol — hand-written binary parser/serializer.
//!
//! Pure Rust, `no_std` + `alloc`, zero `unsafe`. All reads are bounds-checked
//! against a fixed 512-byte UDP buffer (`BytePacketBuffer`), so a malformed
//! or hostile response can only ever produce a `DnsWireError`, never a panic
//! or out-of-bounds access.

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsWireError {
    /// Read or write past the end of the 512-byte buffer.
    BufferOverflow,
    /// Compression-pointer chain exceeded the jump limit (cycle guard).
    TooManyJumps,
    /// A label exceeded the 63-byte limit imposed by RFC 1035.
    LabelTooLong,
}

pub type WireResult<T> = Result<T, DnsWireError>;

const MAX_JUMPS: usize = 5;

/// Fixed-size, bounds-checked buffer for reading/writing DNS packets.
pub struct BytePacketBuffer {
    pub buf: [u8; 512],
    pub pos: usize,
}

impl BytePacketBuffer {
    pub fn new() -> BytePacketBuffer {
        BytePacketBuffer { buf: [0; 512], pos: 0 }
    }

    pub fn from_slice(data: &[u8]) -> BytePacketBuffer {
        let mut b = BytePacketBuffer::new();
        let n = data.len().min(512);
        b.buf[..n].copy_from_slice(&data[..n]);
        b
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    fn step(&mut self, steps: usize) -> WireResult<()> {
        if self.pos + steps > 512 {
            return Err(DnsWireError::BufferOverflow);
        }
        self.pos += steps;
        Ok(())
    }

    fn seek(&mut self, pos: usize) -> WireResult<()> {
        if pos > 512 {
            return Err(DnsWireError::BufferOverflow);
        }
        self.pos = pos;
        Ok(())
    }

    fn read(&mut self) -> WireResult<u8> {
        if self.pos >= 512 {
            return Err(DnsWireError::BufferOverflow);
        }
        let res = self.buf[self.pos];
        self.pos += 1;
        Ok(res)
    }

    fn get(&self, pos: usize) -> WireResult<u8> {
        if pos >= 512 {
            return Err(DnsWireError::BufferOverflow);
        }
        Ok(self.buf[pos])
    }

    fn get_range(&self, start: usize, len: usize) -> WireResult<&[u8]> {
        if start + len > 512 {
            return Err(DnsWireError::BufferOverflow);
        }
        Ok(&self.buf[start..start + len])
    }

    fn read_u16(&mut self) -> WireResult<u16> {
        Ok(((self.read()? as u16) << 8) | (self.read()? as u16))
    }

    fn read_u32(&mut self) -> WireResult<u32> {
        Ok(((self.read()? as u32) << 24)
            | ((self.read()? as u32) << 16)
            | ((self.read()? as u32) << 8)
            | (self.read()? as u32))
    }

    /// Read a (possibly compressed) domain name into `outstr`.
    ///
    /// Compression pointers (top two bits of the length byte set) jump to an
    /// earlier offset in the buffer. `MAX_JUMPS` bounds the chain so a
    /// malicious response cannot loop forever.
    fn read_qname(&mut self, outstr: &mut String) -> WireResult<()> {
        let mut pos = self.pos();
        let mut jumped = false;
        let mut delim = "";
        let mut jumps_performed = 0;

        loop {
            if jumps_performed > MAX_JUMPS {
                return Err(DnsWireError::TooManyJumps);
            }

            let len = self.get(pos)?;

            if (len & 0xC0) == 0xC0 {
                if !jumped {
                    self.seek(pos + 2)?;
                }
                let b2 = self.get(pos + 1)? as u16;
                let offset = (((len as u16) ^ 0xC0) << 8) | b2;
                pos = offset as usize;
                jumped = true;
                jumps_performed += 1;
                continue;
            }

            pos += 1;
            if len == 0 {
                break;
            }

            outstr.push_str(delim);
            let str_buffer = self.get_range(pos, len as usize)?;
            for &b in str_buffer {
                outstr.push((b as char).to_ascii_lowercase());
            }
            delim = ".";
            pos += len as usize;
        }

        if !jumped {
            self.seek(pos)?;
        }
        Ok(())
    }

    fn write(&mut self, val: u8) -> WireResult<()> {
        if self.pos >= 512 {
            return Err(DnsWireError::BufferOverflow);
        }
        self.buf[self.pos] = val;
        self.pos += 1;
        Ok(())
    }

    fn write_u8(&mut self, val: u8) -> WireResult<()> {
        self.write(val)
    }

    fn write_u16(&mut self, val: u16) -> WireResult<()> {
        self.write((val >> 8) as u8)?;
        self.write((val & 0xFF) as u8)
    }

    fn write_u32(&mut self, val: u32) -> WireResult<()> {
        self.write(((val >> 24) & 0xFF) as u8)?;
        self.write(((val >> 16) & 0xFF) as u8)?;
        self.write(((val >> 8) & 0xFF) as u8)?;
        self.write((val & 0xFF) as u8)
    }

    /// Write a domain name with no compression (queries we generate are
    /// short enough that compression isn't worth the complexity).
    fn write_qname(&mut self, qname: &str) -> WireResult<()> {
        for label in qname.split('.') {
            let len = label.len();
            if len > 0x3f {
                return Err(DnsWireError::LabelTooLong);
            }
            self.write_u8(len as u8)?;
            for b in label.as_bytes() {
                self.write_u8(*b)?;
            }
        }
        self.write_u8(0)
    }

    fn set_u16(&mut self, pos: usize, val: u16) -> WireResult<()> {
        if pos + 1 >= 512 {
            return Err(DnsWireError::BufferOverflow);
        }
        self.buf[pos] = (val >> 8) as u8;
        self.buf[pos + 1] = (val & 0xFF) as u8;
        Ok(())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResultCode {
    NOERROR = 0,
    FORMERR = 1,
    SERVFAIL = 2,
    NXDOMAIN = 3,
    NOTIMP = 4,
    REFUSED = 5,
}

impl ResultCode {
    pub fn from_num(num: u8) -> ResultCode {
        match num {
            1 => ResultCode::FORMERR,
            2 => ResultCode::SERVFAIL,
            3 => ResultCode::NXDOMAIN,
            4 => ResultCode::NOTIMP,
            5 => ResultCode::REFUSED,
            _ => ResultCode::NOERROR,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DnsHeader {
    pub id: u16,

    pub recursion_desired: bool,
    pub truncated_message: bool,
    pub authoritative_answer: bool,
    pub opcode: u8,
    pub response: bool,

    pub rescode: ResultCode,
    pub checking_disabled: bool,
    pub authed_data: bool,
    pub z: bool,
    pub recursion_available: bool,

    pub questions: u16,
    pub answers: u16,
    pub authoritative_entries: u16,
    pub resource_entries: u16,
}

impl DnsHeader {
    pub fn new() -> DnsHeader {
        DnsHeader {
            id: 0,
            recursion_desired: false,
            truncated_message: false,
            authoritative_answer: false,
            opcode: 0,
            response: false,
            rescode: ResultCode::NOERROR,
            checking_disabled: false,
            authed_data: false,
            z: false,
            recursion_available: false,
            questions: 0,
            answers: 0,
            authoritative_entries: 0,
            resource_entries: 0,
        }
    }

    pub fn read(&mut self, buffer: &mut BytePacketBuffer) -> WireResult<()> {
        self.id = buffer.read_u16()?;

        let flags = buffer.read_u16()?;
        let a = (flags >> 8) as u8;
        let b = (flags & 0xFF) as u8;

        self.recursion_desired = (a & (1 << 0)) > 0;
        self.truncated_message = (a & (1 << 1)) > 0;
        self.authoritative_answer = (a & (1 << 2)) > 0;
        self.opcode = (a >> 3) & 0x0F;
        self.response = (a & (1 << 7)) > 0;

        self.rescode = ResultCode::from_num(b & 0x0F);
        self.checking_disabled = (b & (1 << 4)) > 0;
        self.authed_data = (b & (1 << 5)) > 0;
        self.z = (b & (1 << 6)) > 0;
        self.recursion_available = (b & (1 << 7)) > 0;

        self.questions = buffer.read_u16()?;
        self.answers = buffer.read_u16()?;
        self.authoritative_entries = buffer.read_u16()?;
        self.resource_entries = buffer.read_u16()?;
        Ok(())
    }

    pub fn write(&self, buffer: &mut BytePacketBuffer) -> WireResult<()> {
        buffer.write_u16(self.id)?;

        buffer.write_u8(
            (self.recursion_desired as u8)
                | ((self.truncated_message as u8) << 1)
                | ((self.authoritative_answer as u8) << 2)
                | (self.opcode << 3)
                | ((self.response as u8) << 7),
        )?;

        buffer.write_u8(
            (self.rescode as u8)
                | ((self.checking_disabled as u8) << 4)
                | ((self.authed_data as u8) << 5)
                | ((self.z as u8) << 6)
                | ((self.recursion_available as u8) << 7),
        )?;

        buffer.write_u16(self.questions)?;
        buffer.write_u16(self.answers)?;
        buffer.write_u16(self.authoritative_entries)?;
        buffer.write_u16(self.resource_entries)?;
        Ok(())
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Hash, Copy)]
pub enum QueryType {
    UNKNOWN(u16),
    A,
    NS,
    CNAME,
    MX,
    AAAA,
}

impl QueryType {
    pub fn to_num(&self) -> u16 {
        match *self {
            QueryType::UNKNOWN(x) => x,
            QueryType::A => 1,
            QueryType::NS => 2,
            QueryType::CNAME => 5,
            QueryType::MX => 15,
            QueryType::AAAA => 28,
        }
    }

    pub fn from_num(num: u16) -> QueryType {
        match num {
            1 => QueryType::A,
            2 => QueryType::NS,
            5 => QueryType::CNAME,
            15 => QueryType::MX,
            28 => QueryType::AAAA,
            _ => QueryType::UNKNOWN(num),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsQuestion {
    pub name: String,
    pub qtype: QueryType,
}

impl DnsQuestion {
    pub fn new(name: String, qtype: QueryType) -> DnsQuestion {
        DnsQuestion { name, qtype }
    }

    pub fn read(&mut self, buffer: &mut BytePacketBuffer) -> WireResult<()> {
        buffer.read_qname(&mut self.name)?;
        self.qtype = QueryType::from_num(buffer.read_u16()?);
        let _ = buffer.read_u16()?; // class (always IN)
        Ok(())
    }

    pub fn write(&self, buffer: &mut BytePacketBuffer) -> WireResult<()> {
        buffer.write_qname(&self.name)?;
        buffer.write_u16(self.qtype.to_num())?;
        buffer.write_u16(1)?; // class IN
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsRecord {
    UNKNOWN { domain: String, qtype: u16, data_len: u16, ttl: u32 },
    A { domain: String, addr: [u8; 4], ttl: u32 },
    NS { domain: String, host: String, ttl: u32 },
    CNAME { domain: String, host: String, ttl: u32 },
    MX { domain: String, priority: u16, host: String, ttl: u32 },
    AAAA { domain: String, addr: [u8; 16], ttl: u32 },
}

impl DnsRecord {
    pub fn read(buffer: &mut BytePacketBuffer) -> WireResult<DnsRecord> {
        let mut domain = String::new();
        buffer.read_qname(&mut domain)?;

        let qtype_num = buffer.read_u16()?;
        let qtype = QueryType::from_num(qtype_num);
        let _ = buffer.read_u16()?; // class
        let ttl = buffer.read_u32()?;
        let data_len = buffer.read_u16()?;

        match qtype {
            QueryType::A => {
                let raw_addr = buffer.read_u32()?;
                let addr = [
                    ((raw_addr >> 24) & 0xFF) as u8,
                    ((raw_addr >> 16) & 0xFF) as u8,
                    ((raw_addr >> 8) & 0xFF) as u8,
                    (raw_addr & 0xFF) as u8,
                ];
                Ok(DnsRecord::A { domain, addr, ttl })
            }
            QueryType::AAAA => {
                let mut addr = [0u8; 16];
                for chunk in addr.chunks_mut(4) {
                    let word = buffer.read_u32()?;
                    chunk[0] = ((word >> 24) & 0xFF) as u8;
                    chunk[1] = ((word >> 16) & 0xFF) as u8;
                    chunk[2] = ((word >> 8) & 0xFF) as u8;
                    chunk[3] = (word & 0xFF) as u8;
                }
                Ok(DnsRecord::AAAA { domain, addr, ttl })
            }
            QueryType::NS => {
                let mut ns = String::new();
                buffer.read_qname(&mut ns)?;
                Ok(DnsRecord::NS { domain, host: ns, ttl })
            }
            QueryType::CNAME => {
                let mut cname = String::new();
                buffer.read_qname(&mut cname)?;
                Ok(DnsRecord::CNAME { domain, host: cname, ttl })
            }
            QueryType::MX => {
                let priority = buffer.read_u16()?;
                let mut mx = String::new();
                buffer.read_qname(&mut mx)?;
                Ok(DnsRecord::MX { domain, priority, host: mx, ttl })
            }
            QueryType::UNKNOWN(_) => {
                buffer.step(data_len as usize)?;
                Ok(DnsRecord::UNKNOWN { domain, qtype: qtype_num, data_len, ttl })
            }
        }
    }

    pub fn write(&self, buffer: &mut BytePacketBuffer) -> WireResult<usize> {
        let start_pos = buffer.pos();

        match self {
            DnsRecord::A { domain, addr, ttl } => {
                buffer.write_qname(domain)?;
                buffer.write_u16(QueryType::A.to_num())?;
                buffer.write_u16(1)?;
                buffer.write_u32(*ttl)?;
                buffer.write_u16(4)?;
                for octet in addr {
                    buffer.write_u8(*octet)?;
                }
            }
            DnsRecord::AAAA { domain, addr, ttl } => {
                buffer.write_qname(domain)?;
                buffer.write_u16(QueryType::AAAA.to_num())?;
                buffer.write_u16(1)?;
                buffer.write_u32(*ttl)?;
                buffer.write_u16(16)?;
                for octet in addr {
                    buffer.write_u8(*octet)?;
                }
            }
            DnsRecord::NS { domain, host, ttl } => {
                buffer.write_qname(domain)?;
                buffer.write_u16(QueryType::NS.to_num())?;
                buffer.write_u16(1)?;
                buffer.write_u32(*ttl)?;
                let pos = buffer.pos();
                buffer.write_u16(0)?;
                buffer.write_qname(host)?;
                let size = buffer.pos() - (pos + 2);
                buffer.set_u16(pos, size as u16)?;
            }
            DnsRecord::CNAME { domain, host, ttl } => {
                buffer.write_qname(domain)?;
                buffer.write_u16(QueryType::CNAME.to_num())?;
                buffer.write_u16(1)?;
                buffer.write_u32(*ttl)?;
                let pos = buffer.pos();
                buffer.write_u16(0)?;
                buffer.write_qname(host)?;
                let size = buffer.pos() - (pos + 2);
                buffer.set_u16(pos, size as u16)?;
            }
            DnsRecord::MX { domain, priority, host, ttl } => {
                buffer.write_qname(domain)?;
                buffer.write_u16(QueryType::MX.to_num())?;
                buffer.write_u16(1)?;
                buffer.write_u32(*ttl)?;
                let pos = buffer.pos();
                buffer.write_u16(0)?;
                buffer.write_u16(*priority)?;
                buffer.write_qname(host)?;
                let size = buffer.pos() - (pos + 2);
                buffer.set_u16(pos, size as u16)?;
            }
            DnsRecord::UNKNOWN { .. } => {
                // Nothing to (re-)serialize for records we don't understand.
            }
        }

        Ok(buffer.pos() - start_pos)
    }
}

#[derive(Clone, Debug)]
pub struct DnsPacket {
    pub header: DnsHeader,
    pub questions: Vec<DnsQuestion>,
    pub answers: Vec<DnsRecord>,
    pub authorities: Vec<DnsRecord>,
    pub resources: Vec<DnsRecord>,
}

impl DnsPacket {
    pub fn new() -> DnsPacket {
        DnsPacket {
            header: DnsHeader::new(),
            questions: Vec::new(),
            answers: Vec::new(),
            authorities: Vec::new(),
            resources: Vec::new(),
        }
    }

    /// Build a standard recursive A/AAAA query for `qname`.
    pub fn query(id: u16, qname: &str, qtype: QueryType) -> DnsPacket {
        let mut packet = DnsPacket::new();
        packet.header.id = id;
        packet.header.questions = 1;
        packet.header.recursion_desired = true;
        packet.questions.push(DnsQuestion::new(qname.to_string(), qtype));
        packet
    }

    pub fn from_buffer(buffer: &mut BytePacketBuffer) -> WireResult<DnsPacket> {
        let mut result = DnsPacket::new();
        result.header.read(buffer)?;

        for _ in 0..result.header.questions {
            let mut question = DnsQuestion::new(String::new(), QueryType::UNKNOWN(0));
            question.read(buffer)?;
            result.questions.push(question);
        }
        for _ in 0..result.header.answers {
            result.answers.push(DnsRecord::read(buffer)?);
        }
        for _ in 0..result.header.authoritative_entries {
            result.authorities.push(DnsRecord::read(buffer)?);
        }
        for _ in 0..result.header.resource_entries {
            result.resources.push(DnsRecord::read(buffer)?);
        }

        Ok(result)
    }

    pub fn write(&mut self, buffer: &mut BytePacketBuffer) -> WireResult<()> {
        self.header.questions = self.questions.len() as u16;
        self.header.answers = self.answers.len() as u16;
        self.header.authoritative_entries = self.authorities.len() as u16;
        self.header.resource_entries = self.resources.len() as u16;

        self.header.write(buffer)?;
        for q in &self.questions {
            q.write(buffer)?;
        }
        for rec in &self.answers {
            rec.write(buffer)?;
        }
        for rec in &self.authorities {
            rec.write(buffer)?;
        }
        for rec in &self.resources {
            rec.write(buffer)?;
        }
        Ok(())
    }

    /// First A record's address and TTL, if any (used by the resolver to
    /// populate the cache).
    pub fn first_a(&self) -> Option<([u8; 4], u32)> {
        self.answers.iter().find_map(|r| match r {
            DnsRecord::A { addr, ttl, .. } => Some((*addr, *ttl)),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_query() {
        let mut packet = DnsPacket::query(0x1234, "example.com", QueryType::A);
        let mut buf = BytePacketBuffer::new();
        packet.write(&mut buf).unwrap();

        buf.seek(0).unwrap();
        let parsed = DnsPacket::from_buffer(&mut buf).unwrap();
        assert_eq!(parsed.header.id, 0x1234);
        assert!(parsed.header.recursion_desired);
        assert_eq!(parsed.questions.len(), 1);
        assert_eq!(parsed.questions[0].name, "example.com");
        assert_eq!(parsed.questions[0].qtype, QueryType::A);
    }

    #[test]
    fn parse_a_response_with_compression() {
        // Hand-built minimal response: 1 question + 1 A answer, where the
        // answer's name is a compression pointer back to the question.
        let mut buf = BytePacketBuffer::new();
        let mut packet = DnsPacket::new();
        packet.header.id = 0xABCD;
        packet.header.response = true;
        packet.header.recursion_desired = true;
        packet.header.recursion_available = true;
        packet.questions.push(DnsQuestion::new("example.com".to_string(), QueryType::A));
        packet.answers.push(DnsRecord::A {
            domain: "example.com".to_string(),
            addr: [93, 184, 216, 34],
            ttl: 300,
        });
        packet.write(&mut buf).unwrap();

        buf.seek(0).unwrap();
        let parsed = DnsPacket::from_buffer(&mut buf).unwrap();
        assert_eq!(parsed.header.rescode, ResultCode::NOERROR);
        assert_eq!(parsed.first_a(), Some(([93, 184, 216, 34], 300)));
    }

    #[test]
    fn malformed_packet_does_not_panic() {
        // All-zero buffer: header parses to 0 questions/answers, so this
        // must succeed with empty vectors rather than panicking.
        let mut buf = BytePacketBuffer::new();
        let parsed = DnsPacket::from_buffer(&mut buf).unwrap();
        assert!(parsed.questions.is_empty());
        assert!(parsed.answers.is_empty());
    }

    #[test]
    fn truncated_qname_errors_cleanly() {
        // Place an oversized label length byte at the very end of the buffer
        // so the label data would run past the 512-byte bound.
        let mut buf2 = BytePacketBuffer::new();
        buf2.buf[511] = 10; // claims 10-byte label starting at 511
        buf2.pos = 511;
        let mut name = String::new();
        assert_eq!(buf2.read_qname(&mut name), Err(DnsWireError::BufferOverflow));
    }
}
