//! Little-endian checked serialization helpers.
//!
//! No raw Rust layouts. All multi-byte fields are explicit LE.

use alloc::vec::Vec;

use crate::error::DbError;

/// Writer that fails on capacity overflow.
#[derive(Debug, Default)]
pub struct BufWriter {
    buf: Vec<u8>,
    max: usize,
}

impl BufWriter {
    pub fn with_capacity(max: usize) -> Self {
        Self {
            buf: Vec::new(),
            max,
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    fn ensure(&self, extra: usize) -> Result<(), DbError> {
        let next = self
            .buf
            .len()
            .checked_add(extra)
            .ok_or(DbError::Internal("writer overflow"))?;
        if next > self.max {
            return Err(DbError::PayloadTooLarge {
                size: next as u32,
                max: self.max as u32,
            });
        }
        Ok(())
    }

    pub fn write_u8(&mut self, v: u8) -> Result<(), DbError> {
        self.ensure(1)?;
        self.buf.push(v);
        Ok(())
    }

    pub fn write_u16(&mut self, v: u16) -> Result<(), DbError> {
        self.ensure(2)?;
        self.buf.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    pub fn write_u32(&mut self, v: u32) -> Result<(), DbError> {
        self.ensure(4)?;
        self.buf.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    pub fn write_u64(&mut self, v: u64) -> Result<(), DbError> {
        self.ensure(8)?;
        self.buf.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    pub fn write_i64(&mut self, v: i64) -> Result<(), DbError> {
        self.write_u64(v as u64)
    }

    pub fn write_bytes(&mut self, data: &[u8]) -> Result<(), DbError> {
        self.ensure(data.len())?;
        self.buf.extend_from_slice(data);
        Ok(())
    }

    pub fn write_bytes_len_u16(&mut self, data: &[u8]) -> Result<(), DbError> {
        if data.len() > u16::MAX as usize {
            return Err(DbError::InvalidRequest("bytes too long for u16 len"));
        }
        self.write_u16(data.len() as u16)?;
        self.write_bytes(data)
    }

    pub fn write_bytes_len_u32(&mut self, data: &[u8]) -> Result<(), DbError> {
        if data.len() > u32::MAX as usize {
            return Err(DbError::Internal("bytes too long for u32 len"));
        }
        self.write_u32(data.len() as u32)?;
        self.write_bytes(data)
    }
}

/// Reader with bounds checks on every access.
#[derive(Debug, Clone)]
pub struct BufReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BufReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DbError> {
        let end = self.pos.checked_add(n).ok_or(DbError::Corrupt {
            reason: "reader overflow",
        })?;
        if end > self.data.len() {
            return Err(DbError::Corrupt {
                reason: "truncated buffer",
            });
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub fn read_u8(&mut self) -> Result<u8, DbError> {
        Ok(self.take(1)?[0])
    }

    pub fn read_u16(&mut self) -> Result<u16, DbError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn read_u32(&mut self) -> Result<u32, DbError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn read_u64(&mut self) -> Result<u64, DbError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    pub fn read_i64(&mut self) -> Result<i64, DbError> {
        Ok(self.read_u64()? as i64)
    }

    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], DbError> {
        self.take(n)
    }

    pub fn read_bytes_len_u16(&mut self) -> Result<&'a [u8], DbError> {
        let n = self.read_u16()? as usize;
        self.read_bytes(n)
    }

    pub fn read_bytes_len_u32(&mut self) -> Result<&'a [u8], DbError> {
        let n = self.read_u32()? as usize;
        self.read_bytes(n)
    }

    pub fn read_opt_u64(&mut self) -> Result<Option<u64>, DbError> {
        let tag = self.read_u8()?;
        match tag {
            0 => Ok(None),
            1 => Ok(Some(self.read_u64()?)),
            _ => Err(DbError::InvalidValue("option tag")),
        }
    }

    pub fn write_opt_u64(w: &mut BufWriter, v: Option<u64>) -> Result<(), DbError> {
        match v {
            None => w.write_u8(0),
            Some(x) => {
                w.write_u8(1)?;
                w.write_u64(x)
            }
        }
    }
}

/// FNV-1a 64-bit (stable, non-cryptographic). Used for content hashes.
pub fn fnv1a64(data: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x1000_0000_01b3;
    let mut hash = OFFSET;
    for &b in data {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_primitives() {
        let mut w = BufWriter::with_capacity(64);
        w.write_u8(7).unwrap();
        w.write_u16(0xABCD).unwrap();
        w.write_u32(0x1122_3344).unwrap();
        w.write_u64(0x0102_0304_0506_0708).unwrap();
        w.write_bytes_len_u16(b"hi").unwrap();
        let mut r = BufReader::new(w.as_slice());
        assert_eq!(r.read_u8().unwrap(), 7);
        assert_eq!(r.read_u16().unwrap(), 0xABCD);
        assert_eq!(r.read_u32().unwrap(), 0x1122_3344);
        assert_eq!(r.read_u64().unwrap(), 0x0102_0304_0506_0708);
        assert_eq!(r.read_bytes_len_u16().unwrap(), b"hi");
    }

    #[test]
    fn fnv_stable() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_ne!(fnv1a64(b"a"), fnv1a64(b"b"));
    }

    #[test]
    fn writer_capacity() {
        let mut w = BufWriter::with_capacity(2);
        w.write_u8(1).unwrap();
        w.write_u8(2).unwrap();
        assert!(w.write_u8(3).is_err());
    }
}
