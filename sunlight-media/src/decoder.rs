//! Codec-private decoder selection and Ogg Vorbis/WAV implementations.

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

#[derive(Clone, Copy)]
struct WavLayout {
    data_offset: usize,
    data_len: usize,
    info: AudioStreamInfo,
}

pub struct WavPcmDecoder<'a> {
    source: &'a [u8],
    layout: WavLayout,
    frame_position: u64,
}

impl<'a> WavPcmDecoder<'a> {
    pub fn open(source: &'a [u8]) -> Result<Self, MediaError> {
        let layout = probe_wav(source)?;
        Ok(Self {
            source,
            layout,
            frame_position: 0,
        })
    }
}

impl AudioDecoder for WavPcmDecoder<'_> {
    fn stream_info(&self) -> AudioStreamInfo {
        self.layout.info
    }

    fn decode(&mut self, output: &mut [i16]) -> Result<DecodeChunk, MediaError> {
        let channels = self.layout.info.channels as usize;
        let capacity = output.len() - output.len() % channels;
        let remaining_frames =
            (self.layout.data_len / (channels * 2)).saturating_sub(self.frame_position as usize);
        let frames = remaining_frames.min(capacity / channels);
        let start = self
            .layout
            .data_offset
            .checked_add(self.frame_position as usize * channels * 2)
            .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 30))?;
        let bytes = frames
            .checked_mul(channels)
            .and_then(|samples| samples.checked_mul(2))
            .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 31))?;
        let input = self
            .source
            .get(start..start + bytes)
            .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 32))?;
        for (sample, bytes) in output[..frames * channels]
            .iter_mut()
            .zip(input.chunks_exact(2))
        {
            *sample = i16::from_le_bytes([bytes[0], bytes[1]]);
        }
        self.frame_position = self.frame_position.saturating_add(frames as u64);
        Ok(DecodeChunk {
            frames,
            end_of_stream: self.frame_position as usize >= self.layout.data_len / (channels * 2),
        })
    }

    fn seek(&mut self, position: MediaTime) -> Result<MediaTime, MediaError> {
        let target = position
            .frames_at(self.layout.info.sample_rate_hz)
            .min((self.layout.data_len / (self.layout.info.channels as usize * 2)) as u64);
        self.frame_position = target;
        Ok(MediaTime::from_frames(
            target,
            self.layout.info.sample_rate_hz,
        ))
    }
}

pub fn probe(source: &[u8]) -> Result<AudioStreamInfo, MediaError> {
    if source.get(..4) == Some(b"RIFF") {
        return Ok(probe_wav(source)?.info);
    }
    if source.get(..4) == Some(b"OggS") {
        let decoder = VorbisDecoder::open(source)?;
        return Ok(decoder.stream_info());
    }
    Err(MediaError::new(MediaErrorKind::UnsupportedContainer, 10))
}

pub enum ProbeDecoder<'a> {
    Vorbis(VorbisDecoder<'a>),
    Wav(WavPcmDecoder<'a>),
}

impl<'a> ProbeDecoder<'a> {
    pub fn open(source: &'a [u8]) -> Result<Self, MediaError> {
        if source.get(..4) == Some(b"RIFF") {
            return Ok(Self::Wav(WavPcmDecoder::open(source)?));
        }
        Ok(Self::Vorbis(VorbisDecoder::open(source)?))
    }
}

impl AudioDecoder for ProbeDecoder<'_> {
    fn stream_info(&self) -> AudioStreamInfo {
        match self {
            Self::Vorbis(decoder) => decoder.stream_info(),
            Self::Wav(decoder) => decoder.stream_info(),
        }
    }

    fn decode(&mut self, output: &mut [i16]) -> Result<DecodeChunk, MediaError> {
        match self {
            Self::Vorbis(decoder) => decoder.decode(output),
            Self::Wav(decoder) => decoder.decode(output),
        }
    }

    fn seek(&mut self, position: MediaTime) -> Result<MediaTime, MediaError> {
        match self {
            Self::Vorbis(decoder) => decoder.seek(position),
            Self::Wav(decoder) => decoder.seek(position),
        }
    }
}

fn probe_wav(source: &[u8]) -> Result<WavLayout, MediaError> {
    if source.len() < 12 {
        return Err(MediaError::new(MediaErrorKind::MalformedMedia, 10));
    }
    if source.get(..4) != Some(b"RIFF") || source.get(8..12) != Some(b"WAVE") {
        return Err(MediaError::new(MediaErrorKind::UnsupportedContainer, 11));
    }
    let riff_size = u32::from_le_bytes(source[4..8].try_into().unwrap()) as usize;
    let riff_end = 8usize
        .checked_add(riff_size)
        .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 12))?;
    if riff_end > source.len() || riff_end < 12 {
        return Err(MediaError::new(MediaErrorKind::MalformedMedia, 13));
    }
    let mut cursor = 12usize;
    let mut format = None;
    let mut data = None;
    while cursor < riff_end {
        let header_end = cursor
            .checked_add(8)
            .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 14))?;
        let header = source
            .get(cursor..header_end)
            .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 15))?;
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let payload_end = header_end
            .checked_add(len)
            .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 16))?;
        let padded_end = payload_end
            .checked_add(len & 1)
            .ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 17))?;
        if payload_end > riff_end || padded_end > riff_end {
            return Err(MediaError::new(MediaErrorKind::MalformedMedia, 18));
        }
        let payload = &source[header_end..payload_end];
        match &header[..4] {
            b"fmt " => {
                if payload.len() < 16 {
                    return Err(MediaError::new(MediaErrorKind::MalformedMedia, 19));
                }
                let encoding = u16::from_le_bytes(payload[0..2].try_into().unwrap());
                let channels = u16::from_le_bytes(payload[2..4].try_into().unwrap());
                let rate = u32::from_le_bytes(payload[4..8].try_into().unwrap());
                let block_align = u16::from_le_bytes(payload[12..14].try_into().unwrap());
                let bits = u16::from_le_bytes(payload[14..16].try_into().unwrap());
                if encoding != 1 || !matches!(channels, 1 | 2) || rate == 0 || bits != 16 {
                    return Err(MediaError::new(MediaErrorKind::UnsupportedSampleFormat, 20));
                }
                if block_align != channels.saturating_mul(2) {
                    return Err(MediaError::new(MediaErrorKind::MalformedMedia, 21));
                }
                format = Some((channels as u8, rate, block_align as usize));
            }
            b"data" => {
                data = Some((header_end, payload.len()));
            }
            _ => {}
        }
        cursor = padded_end;
    }
    let (channels, rate, frame_bytes) =
        format.ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 22))?;
    let (data_offset, data_len) =
        data.ok_or_else(|| MediaError::new(MediaErrorKind::MalformedMedia, 23))?;
    if data_len == 0 || data_len % frame_bytes != 0 {
        return Err(MediaError::new(MediaErrorKind::MalformedMedia, 24));
    }
    let frames = (data_len / frame_bytes) as u64;
    Ok(WavLayout {
        data_offset,
        data_len,
        info: AudioStreamInfo {
            sample_rate_hz: rate,
            channels,
            sample_format: PcmFormat::Signed16LeInterleaved,
            duration: Some(MediaTime::from_frames(frames, rate)),
            seekable: true,
        },
    })
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
    static TEST_WAV: &[u8] =
        include_bytes!("../../assets/sounds/melody-mina-sample-48k-stereo.wav");

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

    #[test]
    fn decodes_generated_wav_fixture() {
        let mut decoder = WavPcmDecoder::open(TEST_WAV).unwrap();
        let info = decoder.stream_info();
        assert_eq!(info.sample_rate_hz, 48_000);
        assert_eq!(info.channels, 2);
        assert_eq!(info.duration.unwrap().as_millis(), 6_000);
        let mut pcm = [0i16; 2048];
        let mut frames = 0usize;
        loop {
            let chunk = decoder.decode(&mut pcm).unwrap();
            frames += chunk.frames;
            if chunk.end_of_stream {
                break;
            }
        }
        assert_eq!(frames, 288_000);
    }

    #[test]
    fn wav_parser_skips_unknown_odd_padded_chunks() {
        let mut wav = TEST_WAV.to_vec();
        let insert = 12;
        let mut chunk = b"JUNK".to_vec();
        chunk.extend_from_slice(&3u32.to_le_bytes());
        chunk.extend_from_slice(b"abc");
        chunk.push(0);
        wav.splice(insert..insert, chunk);
        let riff_size = (wav.len() - 8) as u32;
        wav[4..8].copy_from_slice(&riff_size.to_le_bytes());
        assert!(WavPcmDecoder::open(&wav).is_ok());
    }

    #[test]
    fn wav_parser_rejects_truncation_and_unsupported_formats() {
        assert!(WavPcmDecoder::open(&TEST_WAV[..20]).is_err());
        let mut wav = TEST_WAV.to_vec();
        let fmt = wav.windows(4).position(|window| window == b"fmt ").unwrap() + 8;
        wav[fmt..fmt + 2].copy_from_slice(&3u16.to_le_bytes());
        let error = match WavPcmDecoder::open(&wav) {
            Ok(_) => panic!("unsupported PCM encoding was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind, MediaErrorKind::UnsupportedSampleFormat);
    }
}
