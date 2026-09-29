//! Bounded, best-effort tags for the audio formats Melody Mina can play.

extern crate alloc;

use alloc::{string::String, vec::Vec};

use crate::library::MediaFormat;

pub const MAX_TAG_BYTES: usize = 2 * 1024 * 1024;
const MAX_COVER_BYTES: usize = 1024 * 1024;

#[derive(Default)]
pub struct SongMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub cover: Option<Vec<u8>>,
}

pub fn parse(format: MediaFormat, bytes: &[u8]) -> SongMetadata {
    parse_with_cover(format, bytes, true)
}

pub fn parse_labels(format: MediaFormat, bytes: &[u8]) -> SongMetadata {
    parse_with_cover(format, bytes, false)
}

fn parse_with_cover(format: MediaFormat, bytes: &[u8], include_cover: bool) -> SongMetadata {
    let mut result = SongMetadata::default();
    match format {
        MediaFormat::Mp3 => {
            parse_id3(bytes, &mut result, include_cover);
            parse_id3v1(bytes, &mut result);
        }
        MediaFormat::OggVorbis => parse_ogg(bytes, &mut result, include_cover),
        MediaFormat::WavPcm => parse_wav(bytes, &mut result, include_cover),
    }
    result
}

fn clean_text(text: String) -> Option<String> {
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    (!trimmed.is_empty() && trimmed.len() <= 256 && !trimmed.chars().any(char::is_control))
        .then(|| String::from(trimmed))
}

fn id3_text(bytes: &[u8]) -> Option<String> {
    if bytes.len() > 1024 {
        return None;
    }
    let (&encoding, body) = bytes.split_first()?;
    let text = match encoding {
        0 => body
            .iter()
            .take_while(|b| **b != 0)
            .map(|b| char::from(*b))
            .collect(),
        3 => String::from(core::str::from_utf8(body.split(|b| *b == 0).next()?).ok()?),
        1 | 2 => {
            let (little, data) = if encoding == 1 && body.starts_with(&[0xff, 0xfe]) {
                (true, &body[2..])
            } else if encoding == 1 && body.starts_with(&[0xfe, 0xff]) {
                (false, &body[2..])
            } else {
                (false, body)
            };
            let units: Vec<u16> = data
                .chunks_exact(2)
                .map(|c| {
                    if little {
                        u16::from_le_bytes([c[0], c[1]])
                    } else {
                        u16::from_be_bytes([c[0], c[1]])
                    }
                })
                .take_while(|unit| *unit != 0)
                .collect();
            String::from_utf16(&units).ok()?
        }
        _ => return None,
    };
    clean_text(text)
}

fn synchsafe(bytes: &[u8]) -> Option<usize> {
    if bytes.len() != 4 || bytes.iter().any(|b| b & 0x80 != 0) {
        return None;
    }
    Some(bytes.iter().fold(0usize, |n, b| n * 128 + *b as usize))
}

fn parse_id3(bytes: &[u8], result: &mut SongMetadata, include_cover: bool) {
    let Some(header) = bytes.get(..10).filter(|h| &h[..3] == b"ID3") else {
        return;
    };
    if !matches!(header[3], 3 | 4) || header[5] & 0x80 != 0 {
        return;
    }
    let Some(size) = synchsafe(&header[6..10]).filter(|s| *s <= MAX_TAG_BYTES) else {
        return;
    };
    let Some(end) = 10usize.checked_add(size).filter(|end| *end <= bytes.len()) else {
        return;
    };
    let mut cursor = if header[5] & 0x40 != 0 {
        let Some(extra) = bytes.get(10..14) else {
            return;
        };
        let length = if header[3] == 4 {
            synchsafe(extra)
        } else {
            Some(u32::from_be_bytes(extra.try_into().unwrap()) as usize + 4)
        };
        let Some(next) = length
            .and_then(|length| 10usize.checked_add(length))
            .filter(|next| *next <= end)
        else {
            return;
        };
        next
    } else {
        10
    };
    while cursor + 10 <= end {
        let frame = &bytes[cursor..cursor + 10];
        if frame[..4] == [0; 4] {
            break;
        }
        let length = if header[3] == 4 {
            synchsafe(&frame[4..8])
        } else {
            Some(u32::from_be_bytes(frame[4..8].try_into().unwrap()) as usize)
        };
        let Some(next) = length
            .and_then(|n| (cursor + 10).checked_add(n))
            .filter(|n| *n <= end)
        else {
            break;
        };
        let payload = &bytes[cursor + 10..next];
        if frame[8..10] == [0; 2] {
            match &frame[..4] {
                b"TIT2" => result.title = id3_text(payload).or(result.title.take()),
                b"TPE1" => result.artist = id3_text(payload).or(result.artist.take()),
                b"TALB" => result.album = id3_text(payload).or(result.album.take()),
                b"APIC" if include_cover && result.cover.is_none() => {
                    // Encoding, MIME, picture type, description, then raw image.
                    if let Some(mime_end) = payload
                        .get(1..)
                        .and_then(|s| s.iter().position(|b| *b == 0))
                    {
                        let kind = mime_end + 2;
                        if payload.len() > kind + 1 {
                            let description = &payload[kind + 1..];
                            let terminator = if matches!(payload[0], 1 | 2) { 2 } else { 1 };
                            let offset = if terminator == 1 {
                                description.iter().position(|b| *b == 0).map(|n| n + 1)
                            } else {
                                description
                                    .windows(2)
                                    .position(|w| w == [0, 0])
                                    .map(|n| n + 2)
                            };
                            if let Some(image) = offset.and_then(|n| description.get(n..)) {
                                if !image.is_empty() && image.len() <= MAX_COVER_BYTES {
                                    result.cover = Some(image.to_vec());
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        cursor = next;
    }
}

fn parse_id3v1(bytes: &[u8], result: &mut SongMetadata) {
    let Some(tag) = bytes
        .get(bytes.len().saturating_sub(128)..)
        .filter(|tag| tag.len() == 128 && &tag[..3] == b"TAG")
    else {
        return;
    };
    for (field, destination) in [
        (&tag[3..33], &mut result.title),
        (&tag[33..63], &mut result.artist),
        (&tag[63..93], &mut result.album),
    ] {
        if destination.is_none() {
            *destination = clean_text(field.iter().map(|b| char::from(*b)).collect());
        }
    }
}

fn parse_ogg(bytes: &[u8], result: &mut SongMetadata, include_cover: bool) {
    let mut offset = 0;
    let mut packet = Vec::new();
    while offset + 27 <= bytes.len() && packet.len() <= MAX_TAG_BYTES {
        if &bytes[offset..offset + 4] != b"OggS" || bytes[offset + 4] != 0 {
            break;
        }
        let segments = bytes[offset + 26] as usize;
        let Some(table) = bytes.get(offset + 27..offset + 27 + segments) else {
            break;
        };
        let length: usize = table.iter().map(|n| *n as usize).sum();
        let start = offset + 27 + segments;
        let Some(data) = bytes.get(start..start + length) else {
            break;
        };
        let mut pos = 0;
        for &segment in table {
            let len = segment as usize;
            if packet.len() + len > MAX_TAG_BYTES {
                return;
            }
            packet.extend_from_slice(&data[pos..pos + len]);
            pos += len;
            if segment < 255 {
                if packet.starts_with(b"\x03vorbis") {
                    parse_vorbis_comments(&packet[7..], result, include_cover);
                    return;
                }
                packet.clear();
            }
        }
        offset = start + length;
    }
}

fn take_le32<'a>(bytes: &mut &'a [u8]) -> Option<usize> {
    let head = bytes.get(..4)?;
    *bytes = &bytes[4..];
    Some(u32::from_le_bytes(head.try_into().ok()?) as usize)
}

fn parse_vorbis_comments(mut bytes: &[u8], result: &mut SongMetadata, include_cover: bool) {
    let Some(vendor) = take_le32(&mut bytes) else {
        return;
    };
    let Some(rest) = bytes.get(vendor..) else {
        return;
    };
    bytes = rest;
    let Some(count) = take_le32(&mut bytes).filter(|count| *count <= 4096) else {
        return;
    };
    for _ in 0..count {
        let Some(length) = take_le32(&mut bytes) else {
            return;
        };
        let Some(comment) = bytes.get(..length) else {
            return;
        };
        bytes = &bytes[length..];
        let Ok(text) = core::str::from_utf8(comment) else {
            continue;
        };
        let Some((key, value)) = text.split_once('=') else {
            continue;
        };
        if key.eq_ignore_ascii_case("TITLE") {
            if value.len() <= 256 {
                result.title = clean_text(String::from(value));
            }
        } else if key.eq_ignore_ascii_case("ARTIST") {
            if value.len() <= 256 {
                result.artist = clean_text(String::from(value));
            }
        } else if key.eq_ignore_ascii_case("ALBUM") {
            if value.len() <= 256 {
                result.album = clean_text(String::from(value));
            }
        } else if include_cover
            && key.eq_ignore_ascii_case("METADATA_BLOCK_PICTURE")
            && result.cover.is_none()
        {
            result.cover = decode_flac_picture_base64(value.as_bytes());
        } else if include_cover && key.eq_ignore_ascii_case("COVERART") && result.cover.is_none() {
            result.cover = decode_base64(value.as_bytes(), MAX_COVER_BYTES);
        }
    }
}

fn decode_base64(encoded: &[u8], max_decoded: usize) -> Option<Vec<u8>> {
    if encoded.len() > MAX_COVER_BYTES * 2 {
        return None;
    }
    let mut decoded = Vec::with_capacity(encoded.len() * 3 / 4);
    let mut bits = 0u32;
    let mut count = 0;
    for &byte in encoded {
        if byte == b'=' {
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | value as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            decoded.push((bits >> count) as u8);
        }
    }
    (decoded.len() <= max_decoded).then_some(decoded)
}

fn decode_flac_picture_base64(encoded: &[u8]) -> Option<Vec<u8>> {
    let decoded = decode_base64(encoded, MAX_COVER_BYTES + 4096)?;
    let mut picture = decoded.as_slice();
    let _kind = take_be32(&mut picture)?;
    let mime_length = take_be32(&mut picture)?;
    picture = picture.get(mime_length..)?;
    let description_length = take_be32(&mut picture)?;
    picture = picture.get(description_length..)?;
    for _ in 0..4 {
        take_be32(&mut picture)?;
    }
    let data_length = take_be32(&mut picture)?;
    let image = picture
        .get(..data_length)
        .filter(|image| !image.is_empty() && image.len() <= MAX_COVER_BYTES)?;
    Some(image.to_vec())
}

fn take_be32(bytes: &mut &[u8]) -> Option<usize> {
    let head = bytes.get(..4)?;
    *bytes = &bytes[4..];
    Some(u32::from_be_bytes(head.try_into().ok()?) as usize)
}

fn parse_wav(bytes: &[u8], result: &mut SongMetadata, include_cover: bool) {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return;
    }
    let mut offset = 12;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let Some(end) = (offset + 8)
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
        else {
            break;
        };
        if id == b"LIST" && bytes.get(offset + 8..offset + 12) == Some(b"INFO") {
            let mut item = offset + 12;
            while item + 8 <= end {
                let field = &bytes[item..item + 4];
                let len =
                    u32::from_le_bytes(bytes[item + 4..item + 8].try_into().unwrap()) as usize;
                let Some(next) = (item + 8).checked_add(len).filter(|next| *next <= end) else {
                    break;
                };
                if let Ok(value) =
                    core::str::from_utf8(bytes[item + 8..next].split(|b| *b == 0).next().unwrap())
                {
                    match field {
                        b"INAM" => result.title = clean_text(String::from(value)),
                        b"IART" => result.artist = clean_text(String::from(value)),
                        b"IPRD" => result.album = clean_text(String::from(value)),
                        _ => {}
                    }
                }
                item = next + (len & 1);
            }
        } else if id == b"id3 " || id == b"ID3 " {
            parse_id3(&bytes[offset + 8..end], result, include_cover);
        }
        offset = end + (size & 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    fn id3_frame(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut frame = Vec::from(*id);
        frame.extend_from_slice(&(data.len() as u32).to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(data);
        frame
    }

    #[test]
    fn mp3_reads_text_and_embedded_cover() {
        let mut frames = id3_frame(b"TIT2", b"\x03Song");
        frames.extend(id3_frame(b"TPE1", b"\x03Author"));
        frames.extend(id3_frame(b"TALB", b"\x03Album"));
        frames.extend(id3_frame(b"APIC", b"\x00image/png\0\x03\0image-data"));
        let size = frames.len();
        let mut tag = vec![
            b'I',
            b'D',
            b'3',
            3,
            0,
            0,
            ((size >> 21) & 127) as u8,
            ((size >> 14) & 127) as u8,
            ((size >> 7) & 127) as u8,
            (size & 127) as u8,
        ];
        tag.extend(frames);
        let tags = parse(MediaFormat::Mp3, &tag);
        assert_eq!(tags.title.as_deref(), Some("Song"));
        assert_eq!(tags.artist.as_deref(), Some("Author"));
        assert_eq!(tags.album.as_deref(), Some("Album"));
        assert_eq!(tags.cover.as_deref(), Some(b"image-data".as_slice()));
        assert!(parse_labels(MediaFormat::Mp3, &tag).cover.is_none());
    }

    #[test]
    fn ogg_reads_vorbis_comments() {
        let mut comments = Vec::new();
        comments.extend_from_slice(&0u32.to_le_bytes());
        comments.extend_from_slice(&3u32.to_le_bytes());
        for comment in [
            b"TITLE=Track".as_slice(),
            b"ARTIST=Composer",
            b"ALBUM=Record",
        ] {
            comments.extend_from_slice(&(comment.len() as u32).to_le_bytes());
            comments.extend_from_slice(comment);
        }
        let mut ogg = vec![0u8; 28];
        ogg[..4].copy_from_slice(b"OggS");
        ogg[26] = 1;
        ogg[27] = (comments.len() + 7) as u8;
        ogg.extend_from_slice(b"\x03vorbis");
        ogg.extend(comments);
        let tags = parse(MediaFormat::OggVorbis, &ogg);
        assert_eq!(tags.title.as_deref(), Some("Track"));
        assert_eq!(tags.artist.as_deref(), Some("Composer"));
        assert_eq!(tags.album.as_deref(), Some("Record"));
    }

    #[test]
    fn vorbis_picture_block_yields_cover_bytes() {
        let mut block = Vec::new();
        block.extend_from_slice(&3u32.to_be_bytes());
        block.extend_from_slice(&9u32.to_be_bytes());
        block.extend_from_slice(b"image/png");
        block.extend_from_slice(&0u32.to_be_bytes());
        for value in [2u32, 2, 24, 0] {
            block.extend_from_slice(&value.to_be_bytes());
        }
        block.extend_from_slice(&4u32.to_be_bytes());
        block.extend_from_slice(b"COVR");
        const BASE64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut encoded = Vec::new();
        for chunk in block.chunks(3) {
            let a = chunk[0];
            let b = *chunk.get(1).unwrap_or(&0);
            let c = *chunk.get(2).unwrap_or(&0);
            encoded.push(BASE64[(a >> 2) as usize]);
            encoded.push(BASE64[(((a & 3) << 4) | (b >> 4)) as usize]);
            encoded.push(if chunk.len() > 1 {
                BASE64[(((b & 15) << 2) | (c >> 6)) as usize]
            } else {
                b'='
            });
            encoded.push(if chunk.len() > 2 {
                BASE64[(c & 63) as usize]
            } else {
                b'='
            });
        }
        assert_eq!(
            decode_flac_picture_base64(&encoded).as_deref(),
            Some(b"COVR".as_slice())
        );
    }

    #[test]
    fn wav_reads_info_and_ignores_missing_fields() {
        let mut list = Vec::from(b"INFO".as_slice());
        list.extend_from_slice(b"INAM");
        list.extend_from_slice(&6u32.to_le_bytes());
        list.extend_from_slice(b"Piece\0");
        let mut wav = Vec::from(b"RIFF\0\0\0\0WAVE".as_slice());
        wav.extend_from_slice(b"LIST");
        wav.extend_from_slice(&(list.len() as u32).to_le_bytes());
        wav.extend(list);
        let tags = parse(MediaFormat::WavPcm, &wav);
        assert_eq!(tags.title.as_deref(), Some("Piece"));
        assert!(tags.artist.is_none());
        assert!(tags.album.is_none());
        assert!(tags.cover.is_none());
    }

    #[test]
    fn damaged_tags_do_not_produce_labels() {
        let tags = parse(MediaFormat::Mp3, b"ID3\x03\0\0\x7f\x7f\x7f\x7f");
        assert!(tags.title.is_none());
        assert!(tags.cover.is_none());
    }
}
