//! Codec-private decoder selection and Ogg Vorbis implementation.

use alloc::vec::Vec;

use crate::{
    error::{MediaError, MediaErrorKind},
    types::{AudioStreamInfo, MediaTime, PcmFormat},
};

pub const MAX_COMPRESSED_BYTES: usize = 4 * 1024 * 1024;

pub struct DecodeChunk {
    pub frames: usize,
    pub end_of_stream: bool,
}

pub trait AudioDecoder {
    fn stream_info(&self) -> AudioStreamInfo;
    fn decode(&mut self, output: &mut [i16]) -> Result<DecodeChunk, MediaError>;
    fn seek(&mut self, position: MediaTime) -> Result<MediaTime, MediaError>;
}

pub struct VorbisDecoder<'a> {
    source: &'a [u8],
    reader: lewton::inside_ogg::OggStreamReader<'a>,
    info: AudioStreamInfo,
    pending: Vec<i16>,
    pending_offset: usize,
    frame_position: u64,
}

impl<'a> VorbisDecoder<'a> {
    pub fn open(source: &'a [u8]) -> Result<Self, MediaError> {
        validate_ogg_vorbis(source)?;
        let reader = lewton::inside_ogg::OggStreamReader::new(source)
            .map_err(|_| MediaError::new(MediaErrorKind::MalformedMedia, 1))?;
        let rate = reader.ident_hdr.audio_sample_rate;
        let channels = reader.ident_hdr.audio_channels;
        let duration = last_granule_position(source)
            .filter(|_| rate != 0)
            .map(|frames| MediaTime::from_frames(frames, rate));
        let info = AudioStreamInfo {
            sample_rate_hz: rate,
            channels,
            sample_format: PcmFormat::Signed16LeInterleaved,
            duration,
            seekable: duration.is_some(),
        };
        Ok(Self {
            source,
            reader,
            info,
            pending: Vec::new(),
            pending_offset: 0,
            frame_position: 0,
        })
    }

    fn reset(&mut self) -> Result<(), MediaError> {
        self.reader = lewton::inside_ogg::OggStreamReader::new(self.source)
            .map_err(|_| MediaError::new(MediaErrorKind::Seek, 1))?;
        self.pending.clear();
        self.pending_offset = 0;
        self.frame_position = 0;
        Ok(())
    }
}

impl AudioDecoder for VorbisDecoder<'_> {
    fn stream_info(&self) -> AudioStreamInfo {
        self.info
    }

    fn decode(&mut self, output: &mut [i16]) -> Result<DecodeChunk, MediaError> {
        let channels = self.info.channels as usize;
        if channels == 0 || output.len() < channels {
            return Err(MediaError::new(MediaErrorKind::Decode, 2));
        }
        let capacity = output.len() - output.len() % channels;
        let mut written = 0usize;
        let mut eos = false;
        while written < capacity {
            if self.pending_offset < self.pending.len() {
                let count = (capacity - written).min(self.pending.len() - self.pending_offset);
                output[written..written + count].copy_from_slice(
                    &self.pending[self.pending_offset..self.pending_offset + count],
                );
                self.pending_offset += count;
                written += count;
                continue;
            }
            self.pending.clear();
            self.pending_offset = 0;
            match self
                .reader
                .read_dec_packet_itl()
                .map_err(|_| MediaError::new(MediaErrorKind::Decode, 3))?
            {
                Some(packet) => self.pending = packet,
                None => {
                    eos = true;
                    break;
                }
            }
        }
        let frames = written / channels;
        self.frame_position = self.frame_position.saturating_add(frames as u64);
        Ok(DecodeChunk {
            frames,
            end_of_stream: eos,
        })
    }

    fn seek(&mut self, position: MediaTime) -> Result<MediaTime, MediaError> {
        let bounded = self
            .info
            .duration
            .map(|duration| position.min(duration))
            .unwrap_or(position);
        let target = bounded.frames_at(self.info.sample_rate_hz);
        self.reset()?;
        let channels = self.info.channels as usize;
        let mut scratch = [0i16; 4096];
        while self.frame_position < target {
            let remaining = (target - self.frame_position) as usize;
            let samples = remaining
                .saturating_mul(channels)
                .min(scratch.len() - scratch.len() % channels);
            let decoded = self.decode(&mut scratch[..samples])?;
            if decoded.frames == 0 || decoded.end_of_stream {
                break;
            }
        }
        Ok(MediaTime::from_frames(
            self.frame_position,
            self.info.sample_rate_hz,
        ))
    }
}

fn validate_ogg_vorbis(source: &[u8]) -> Result<(), MediaError> {
    if source.len() < 35 || source.get(..4) != Some(b"OggS") {
        return Err(MediaError::new(MediaErrorKind::UnsupportedContainer, 1));
    }
    let segments = source[26] as usize;
    let header_end = 27usize
        .checked_add(segments)
        .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 4))?;
    if header_end > source.len() {
        return Err(MediaError::new(MediaErrorKind::MalformedMedia, 5));
    }
    let first_packet = source[27..header_end]
        .iter()
        .try_fold(0usize, |sum, value| sum.checked_add(*value as usize))
        .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 6))?;
    let packet_end = header_end
        .checked_add(first_packet)
        .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 7))?;
    let packet = source
        .get(header_end..packet_end)
        .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 8))?;
    if packet.get(..7) != Some(b"\x01vorbis") {
        return Err(MediaError::new(MediaErrorKind::UnsupportedCodec, 1));
    }
    Ok(())
}

fn last_granule_position(source: &[u8]) -> Option<u64> {
    let mut offset = 0usize;
    let mut last = None;
    while offset.checked_add(27)? <= source.len() {
        if source.get(offset..offset + 4)? != b"OggS" {
            return None;
        }
        let segments = source[offset + 26] as usize;
        let table_end = offset.checked_add(27)?.checked_add(segments)?;
        let body = source.get(offset + 27..table_end)?;
        let body_len = body
            .iter()
            .try_fold(0usize, |sum, value| sum.checked_add(*value as usize))?;
        let page_end = table_end.checked_add(body_len)?;
        if page_end > source.len() {
            return None;
        }
        let bytes: [u8; 8] = source.get(offset + 6..offset + 14)?.try_into().ok()?;
        let granule = u64::from_le_bytes(bytes);
        if granule != u64::MAX {
            last = Some(granule);
        }
        offset = page_end;
    }
    (offset == source.len()).then_some(last).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_OGG: &[u8] = include_bytes!("../../assets/sounds/melody-mina-test-48k-stereo.ogg");

    #[test]
    fn rejects_non_ogg_and_non_vorbis_without_panicking() {
        let error = match VorbisDecoder::open(b"not ogg") {
            Ok(_) => panic!("non-Ogg input was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind, MediaErrorKind::UnsupportedContainer);
        let mut ogg = [0u8; 35];
        ogg[..4].copy_from_slice(b"OggS");
        ogg[26] = 1;
        ogg[27] = 7;
        ogg[28..35].copy_from_slice(b"\x01theora");
        let error = match VorbisDecoder::open(&ogg) {
            Ok(_) => panic!("non-Vorbis Ogg input was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind, MediaErrorKind::UnsupportedCodec);
    }

    #[test]
    fn decodes_known_48k_stereo_fixture_to_eof() {
        let mut decoder = VorbisDecoder::open(TEST_OGG).unwrap();
        let info = decoder.stream_info();
        assert_eq!(info.sample_rate_hz, 48_000);
        assert_eq!(info.channels, 2);
        assert_eq!(info.duration.unwrap().as_millis(), 2_000);
        let mut pcm = [0i16; 2048];
        let mut frames = 0usize;
        loop {
            let chunk = decoder.decode(&mut pcm).unwrap();
            frames += chunk.frames;
            if chunk.end_of_stream {
                break;
            }
        }
        assert_eq!(frames, 96_000);
    }

    #[test]
    fn seek_rebuilds_decoder_and_clamps_to_duration() {
        let mut decoder = VorbisDecoder::open(TEST_OGG).unwrap();
        let actual = decoder.seek(MediaTime::from_millis(1_500)).unwrap();
        assert_eq!(actual.as_millis(), 1_500);
        let end = decoder.seek(MediaTime::from_millis(9_000)).unwrap();
        assert_eq!(end.as_millis(), 2_000);
    }
}
