//! Small PCM WAV writer. The file owns data; only a batch lives in application RAM.

use crate::storage::{Error, File};

pub const HEADER_BYTES: u64 = 44;
pub const MAX_DATA_BYTES: u64 = (u32::MAX as u64 - 36) & !3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: u8,
    pub bits: u8,
}

impl Format {
    pub fn validate(self) -> Result<Self, Error> {
        if !(8000..=192000).contains(&self.rate)
            || !matches!(self.channels, 1 | 2)
            || self.bits != 16
        {
            return Err(Error::Format);
        }
        Ok(self)
    }
    pub fn frame_bytes(self) -> u64 {
        u64::from(self.channels) * 2
    }
}

pub fn header(format: Format, data: u64) -> Result<[u8; 44], Error> {
    format.validate()?;
    if data > MAX_DATA_BYTES || data % format.frame_bytes() != 0 {
        return Err(Error::Length);
    }
    let mut bytes = [0u8; 44];
    bytes[0..4].copy_from_slice(b"RIFF");
    bytes[4..8].copy_from_slice(&(36 + data as u32).to_le_bytes());
    bytes[8..16].copy_from_slice(b"WAVEfmt ");
    bytes[16..20].copy_from_slice(&16u32.to_le_bytes());
    bytes[20..22].copy_from_slice(&1u16.to_le_bytes());
    bytes[22..24].copy_from_slice(&u16::from(format.channels).to_le_bytes());
    bytes[24..28].copy_from_slice(&format.rate.to_le_bytes());
    bytes[28..32].copy_from_slice(&(format.rate * format.frame_bytes() as u32).to_le_bytes());
    bytes[32..34].copy_from_slice(&(format.frame_bytes() as u16).to_le_bytes());
    bytes[34..36].copy_from_slice(&u16::from(format.bits).to_le_bytes());
    bytes[36..40].copy_from_slice(b"data");
    bytes[40..44].copy_from_slice(&(data as u32).to_le_bytes());
    Ok(bytes)
}

pub struct Writer<F: File> {
    pub file: F,
    pub format: Format,
    data_bytes: u64,
    finalized: bool,
    failed: bool,
}

impl<F: File> Writer<F> {
    pub fn new(mut file: F, format: Format) -> Result<Self, Error> {
        let bytes = header(format, 0)?;
        file.write_all(&bytes)?;
        Ok(Self {
            file,
            format,
            data_bytes: 0,
            finalized: false,
            failed: false,
        })
    }
    pub fn frames(&self) -> u64 {
        self.data_bytes / self.format.frame_bytes()
    }
    pub fn append(&mut self, pcm: &[u8]) -> Result<(), Error> {
        if self.finalized || self.failed {
            return Err(Error::State);
        }
        let total = self
            .data_bytes
            .checked_add(pcm.len() as u64)
            .ok_or(Error::Length)?;
        if total > MAX_DATA_BYTES || pcm.len() as u64 % self.format.frame_bytes() != 0 {
            return Err(Error::Length);
        }
        if let Err(e) = self.file.write_all(pcm) {
            self.failed = true;
            return Err(e);
        }
        self.data_bytes = total;
        Ok(())
    }
    pub fn finalize(&mut self) -> Result<(), Error> {
        if self.finalized || self.failed {
            return Err(Error::State);
        }
        // A failed seek/header/sync permanently invalidates this candidate.
        self.failed = true;
        self.file.seek(0)?;
        self.file
            .write_all(&header(self.format, self.data_bytes)?)?;
        self.file.sync()?;
        self.finalized = true;
        self.failed = false;
        Ok(())
    }
}
