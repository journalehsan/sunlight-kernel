//! Bounded MPEG Layer III decoding over an immutable, complete local file.

use crate::{
    decoder::{AudioDecoder, DecodeChunk},
    error::{MediaError, MediaErrorKind},
    types::{AudioStreamInfo, MediaTime, PcmFormat},
};

const MAX_SAMPLES: usize = nanomp3::MAX_SAMPLES_PER_FRAME;

#[derive(Clone, Copy)]
struct Header {
    rate: u32,
    channels: u8,
    bytes: usize,
    frames: usize,
}

fn header(bytes: &[u8]) -> Option<Header> {
    let h = u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?);
    let version = (h >> 19) & 3;
    let bitrate_index = ((h >> 12) & 15) as usize;
    let rate_index = ((h >> 10) & 3) as usize;
    if h >> 21 != 0x7ff
        || version == 1
        || (h >> 17) & 3 != 1
        || bitrate_index == 0
        || bitrate_index == 15
        || rate_index == 3
    {
        return None;
    }
    let rate = [44_100, 48_000, 32_000][rate_index]
        / match version {
            3 => 1,
            2 => 2,
            _ => 4,
        };
    let bitrates = if version == 3 {
        [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ]
    } else {
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160]
    };
    let bytes = (if version == 3 { 144_000 } else { 72_000 }) * bitrates[bitrate_index] / rate
        + ((h >> 9) & 1);
    Some(Header {
        rate,
        channels: if (h >> 6) & 3 == 3 { 1 } else { 2 },
        bytes: bytes as usize,
        frames: if version == 3 { 1152 } else { 576 },
    })
}

pub fn is_mp3(source: &[u8]) -> bool {
    source.get(..3) == Some(b"ID3") || header(source).is_some()
}

fn audio_start(source: &[u8]) -> Result<usize, MediaError> {
    if source.get(..3) != Some(b"ID3") {
        return Ok(0);
    }
    let invalid = || MediaError::new(MediaErrorKind::MalformedMedia, 50);
    let tag = source.get(..10).ok_or_else(invalid)?;
    if !(2..=4).contains(&tag[3]) || tag[6..10].iter().any(|byte| byte & 0x80 != 0) {
        return Err(invalid());
    }
    let size = tag[6..10]
        .iter()
        .fold(0usize, |size, byte| size * 128 + *byte as usize);
    let end = 10usize
        .checked_add(size)
        .and_then(|end| {
            end.checked_add(if tag[3] == 4 && tag[5] & 0x10 != 0 {
                10
            } else {
                0
            })
        })
        .ok_or_else(invalid)?;
    if end > source.len() {
        return Err(invalid());
    }
    Ok(end)
}

pub struct Mp3Decoder<'a> {
    source: &'a [u8],
    start: usize,
    end: usize,
    cursor: usize,
    decoder: nanomp3::Decoder,
    pending: [i16; MAX_SAMPLES],
    pending_len: usize,
    pending_offset: usize,
    position: u64,
    info: AudioStreamInfo,
}

impl<'a> Mp3Decoder<'a> {
    pub fn open(source: &'a [u8]) -> Result<Self, MediaError> {
        let invalid = || MediaError::new(MediaErrorKind::MalformedMedia, 51);
        let start = audio_start(source)?;
        let first = header(source.get(start..).ok_or_else(invalid)?)
            .ok_or_else(|| MediaError::new(MediaErrorKind::UnsupportedCodec, 2))?;
        let mut cursor = start;
        let mut frames = 0u64;
        while let Some(frame) = source.get(cursor..).and_then(header) {
            if frame.rate != first.rate || frame.channels != first.channels {
                return Err(MediaError::new(MediaErrorKind::UnsupportedSampleFormat, 52));
            }
            cursor = cursor.checked_add(frame.bytes).ok_or_else(invalid)?;
            if cursor > source.len() {
                return Err(invalid());
            }
            frames = frames
                .checked_add(frame.frames as u64)
                .ok_or_else(invalid)?;
        }
        // ID3v1 and APEv2 metadata can follow the audio frames. Reject other
        // trailing bytes and truncated frames before accepting a local file.
        let mut tail = &source[cursor..];
        if tail.get(..8) == Some(b"APETAGEX") {
            let ape = tail.get(..32).ok_or_else(invalid)?;
            let size = u32::from_le_bytes(ape[12..16].try_into().unwrap()) as usize;
            // The files use a 32-byte header followed by the APE body/footer.
            tail = tail.get(32 + size..).ok_or_else(invalid)?;
        }
        if tail.len() == 128 && tail.get(..3) == Some(b"TAG") {
            tail = &[];
        }
        if frames == 0 || !tail.is_empty() {
            return Err(invalid());
        }
        let info = AudioStreamInfo {
            sample_rate_hz: first.rate,
            channels: first.channels,
            sample_format: PcmFormat::Signed16LeInterleaved,
            duration: Some(MediaTime::from_frames(frames, first.rate)),
            seekable: true,
        };
        Ok(Self {
            source,
            start,
            end: cursor,
            cursor: start,
            decoder: nanomp3::Decoder::new(),
            pending: [0; MAX_SAMPLES],
            pending_len: 0,
            pending_offset: 0,
            position: 0,
            info,
        })
    }
}

impl AudioDecoder for Mp3Decoder<'_> {
    fn stream_info(&self) -> AudioStreamInfo {
        self.info
    }

    fn decode(&mut self, output: &mut [i16]) -> Result<DecodeChunk, MediaError> {
        let channels = self.info.channels as usize;
        let capacity = output.len() / channels * channels;
        if capacity == 0 {
            return Err(MediaError::new(MediaErrorKind::Decode, 2));
        }
        let mut written = 0;
        while written < capacity {
            if self.pending_offset < self.pending_len {
                let count = (capacity - written).min(self.pending_len - self.pending_offset);
                output[written..written + count].copy_from_slice(
                    &self.pending[self.pending_offset..self.pending_offset + count],
                );
                self.pending_offset += count;
                written += count;
                continue;
            }
            if self.cursor == self.end {
                break;
            }
            let mut pcm = [0.0f32; MAX_SAMPLES];
            let (consumed, frame) = self
                .decoder
                .decode(&self.source[self.cursor..self.end], &mut pcm);
            if consumed == 0 || consumed > self.end - self.cursor {
                return Err(MediaError::new(MediaErrorKind::Decode, 53));
            }
            self.cursor += consumed;
            let frame = frame.ok_or_else(|| MediaError::new(MediaErrorKind::Decode, 54))?;
            if frame.sample_rate != self.info.sample_rate_hz
                || frame.channels.num() != self.info.channels
            {
                return Err(MediaError::new(MediaErrorKind::UnsupportedSampleFormat, 52));
            }
            self.pending_len = frame.samples_produced * channels;
            self.pending_offset = 0;
            if self.pending_len == 0 || self.pending_len > MAX_SAMPLES {
                return Err(MediaError::new(MediaErrorKind::Decode, 54));
            }
            for (sample, value) in self.pending[..self.pending_len].iter_mut().zip(pcm.iter()) {
                *sample = (value.clamp(-1.0, 1.0) * 32768.0) as i16;
            }
        }
        self.position += (written / channels) as u64;
        Ok(DecodeChunk {
            frames: written / channels,
            end_of_stream: self.cursor == self.end && self.pending_offset == self.pending_len,
        })
    }

    fn seek(&mut self, position: MediaTime) -> Result<MediaTime, MediaError> {
        let target = position.frames_at(self.info.sample_rate_hz).min(
            self.info
                .duration
                .unwrap()
                .frames_at(self.info.sample_rate_hz),
        );
        self.cursor = self.start;
        self.decoder = nanomp3::Decoder::new();
        self.pending_len = 0;
        self.pending_offset = 0;
        self.position = 0;
        let mut scratch = [0i16; 2048];
        while self.position < target {
            let samples = ((target - self.position) as usize)
                .saturating_mul(self.info.channels as usize)
                .min(scratch.len());
            let chunk = self.decode(&mut scratch[..samples])?;
            if chunk.frames == 0 {
                break;
            }
        }
        Ok(MediaTime::from_frames(
            self.position,
            self.info.sample_rate_hz,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_mp3_files_decode_and_seek_with_bounded_storage() {
        for (name, expected_rate) in [
            ("002-Zamfir-EinsamerHirte.mp3", 48_000),
            ("014-SecretGarden-SongFromASecretGarden.mp3", 48_000),
        ] {
            let path = std::format!("{}/../assets/sounds/{name}", env!("CARGO_MANIFEST_DIR"));
            let bytes = std::fs::read(path).unwrap();
            assert!(bytes.len() <= crate::decoder::MAX_COMPRESSED_BYTES);
            let mut decoder = Mp3Decoder::open(&bytes).unwrap();
            assert_eq!(decoder.stream_info().sample_rate_hz, expected_rate);
            assert_eq!(decoder.stream_info().channels, 2);
            let mut pcm = [0i16; 2048];
            let first = decoder.decode(&mut pcm).unwrap();
            assert!(first.frames > 0);
            let seeked = decoder.seek(MediaTime::from_millis(1_000)).unwrap();
            assert_eq!(seeked.as_millis(), 1_000);
            let mut frames = seeked.frames_at(expected_rate);
            let mut nonzero = false;
            loop {
                let chunk = decoder.decode(&mut pcm).unwrap();
                nonzero |= pcm[..chunk.frames * 2].iter().any(|value| *value != 0);
                frames += chunk.frames as u64;
                if chunk.end_of_stream {
                    break;
                }
                assert!(chunk.frames > 0);
            }
            assert!(nonzero);
            assert_eq!(
                frames,
                decoder
                    .stream_info()
                    .duration
                    .unwrap()
                    .frames_at(expected_rate)
            );
        }
    }

    #[test]
    fn invalid_tags_and_truncated_audio_are_rejected() {
        assert_eq!(
            Mp3Decoder::open(b"ID3").err().unwrap().kind,
            MediaErrorKind::MalformedMedia
        );
        let path = std::format!(
            "{}/../assets/sounds/014-SecretGarden-SongFromASecretGarden.mp3",
            env!("CARGO_MANIFEST_DIR")
        );
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(
            Mp3Decoder::open(&bytes[..bytes.len() - 200])
                .err()
                .unwrap()
                .kind,
            MediaErrorKind::MalformedMedia
        );
    }
}
