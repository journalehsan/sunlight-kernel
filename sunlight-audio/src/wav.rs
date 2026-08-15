//! Strict, allocation-free PCM WAV validation for packaged system sounds.

use crate::pcm::{AudioFormat, NATIVE_CHANNELS, NATIVE_RATE_HZ, NATIVE_SAMPLE_BITS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavError {
    Truncated,
    BadContainer,
    MissingFormat,
    MissingData,
    UnsupportedFormat,
    MisalignedData,
}

#[derive(Clone, Copy, Debug)]
pub struct WavPcm<'a> {
    pub pcm: &'a [u8],
    pub format: AudioFormat,
}

pub fn parse_pcm_wav(bytes: &[u8]) -> Result<WavPcm<'_>, WavError> {
    if bytes.len() < 12 {
        return Err(WavError::Truncated);
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::BadContainer);
    }
    let declared = read_u32(bytes, 4)? as usize;
    if declared.saturating_add(8) > bytes.len() {
        return Err(WavError::Truncated);
    }

    let mut cursor = 12usize;
    let mut format = None;
    let mut pcm = None;
    while cursor.saturating_add(8) <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let len = read_u32(bytes, cursor + 4)? as usize;
        let start = cursor + 8;
        let end = start.checked_add(len).ok_or(WavError::Truncated)?;
        if end > bytes.len() {
            return Err(WavError::Truncated);
        }
        if id == b"fmt " {
            if len < 16 {
                return Err(WavError::Truncated);
            }
            let encoding = read_u16(bytes, start)?;
            let channels = read_u16(bytes, start + 2)?;
            let sample_rate = read_u32(bytes, start + 4)?;
            let block_align = read_u16(bytes, start + 12)?;
            let bits = read_u16(bytes, start + 14)?;
            if encoding != 1
                || channels != NATIVE_CHANNELS as u16
                || sample_rate != NATIVE_RATE_HZ
                || bits != NATIVE_SAMPLE_BITS as u16
                || block_align != 4
            {
                return Err(WavError::UnsupportedFormat);
            }
            format = Some(AudioFormat::NATIVE);
        } else if id == b"data" {
            pcm = Some(&bytes[start..end]);
        }
        cursor = end.saturating_add(len & 1);
    }

    let format = format.ok_or(WavError::MissingFormat)?;
    let pcm = pcm.ok_or(WavError::MissingData)?;
    if pcm.is_empty() || pcm.len() % format.frame_bytes() != 0 {
        return Err(WavError::MisalignedData);
    }
    Ok(WavPcm { pcm, format })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, WavError> {
    let value = bytes.get(offset..offset + 2).ok_or(WavError::Truncated)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, WavError> {
    let value = bytes.get(offset..offset + 4).ok_or(WavError::Truncated)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_wav(format: u16, channels: u16, rate: u32, bits: u16) -> std::vec::Vec<u8> {
        let pcm = [1u8, 0, 1, 0];
        let mut out = std::vec::Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36u32 + pcm.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&format.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32 * bits as u32 / 8).to_le_bytes());
        out.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        out.extend_from_slice(&pcm);
        out
    }

    #[test]
    fn accepts_only_native_pcm() {
        let wav = tiny_wav(1, 2, 48_000, 16);
        let parsed = parse_pcm_wav(&wav).unwrap();
        assert_eq!(parsed.pcm.len(), 4);
        assert!(parsed.format.is_native());
        assert_eq!(
            parse_pcm_wav(&tiny_wav(3, 2, 48_000, 16)).unwrap_err(),
            WavError::UnsupportedFormat
        );
        assert_eq!(
            parse_pcm_wav(&tiny_wav(1, 1, 48_000, 16)).unwrap_err(),
            WavError::UnsupportedFormat
        );
    }

    #[test]
    fn rejects_missing_and_truncated_chunks() {
        assert_eq!(parse_pcm_wav(b"short").unwrap_err(), WavError::Truncated);
        let mut wav = tiny_wav(1, 2, 48_000, 16);
        wav.truncate(wav.len() - 2);
        assert_eq!(parse_pcm_wav(&wav).unwrap_err(), WavError::Truncated);
    }
}
