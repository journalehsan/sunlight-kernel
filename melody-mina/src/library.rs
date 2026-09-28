//! Lightweight Melody Mina media discovery, independent of decoding/playback.

extern crate alloc;

use alloc::{string::String, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaSource {
    BuiltIn,
    UserMusic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaFormat {
    Mp3,
    OggVorbis,
    WavPcm,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaEntry {
    pub path: String,
    pub display_title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub format: MediaFormat,
    pub source: MediaSource,
}

pub const BUILTIN_SAMPLE_PATH: &str = "/usr/share/sunlightos/media/melody-mina-sample.wav";

pub fn builtin_entry() -> MediaEntry {
    MediaEntry {
        path: String::from(BUILTIN_SAMPLE_PATH),
        display_title: String::from("Sunlight Audio Sample"),
        artist: None,
        album: None,
        duration_ms: Some(6_000),
        format: MediaFormat::WavPcm,
        source: MediaSource::BuiltIn,
    }
}

fn title_from_path(path: &str) -> String {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let stem = filename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(filename);
    String::from(if stem.is_empty() { filename } else { stem })
}

// Read only the small, plain-text ID3v2 frames used for playlist labels.
// Unsupported encodings and damaged tags simply fall back to the filename.
fn mp3_labels(bytes: &[u8]) -> (Option<String>, Option<String>, Option<String>) {
    let mut labels = (None, None, None);
    let Some(tag) = bytes.get(..10).filter(|tag| &tag[..3] == b"ID3") else {
        return labels;
    };
    if !matches!(tag[3], 3 | 4) || tag[5] & 0xc0 != 0 || tag[6..10].iter().any(|b| b & 0x80 != 0) {
        return labels;
    }
    let size = tag[6..10]
        .iter()
        .fold(0usize, |size, b| size * 128 + *b as usize);
    let Some(end) = 10usize
        .checked_add(size)
        .filter(|end| *end <= bytes.len() && size <= 64 * 1024)
    else {
        return labels;
    };
    let mut cursor = 10;
    while cursor + 10 <= end {
        let frame = &bytes[cursor..cursor + 10];
        if frame[..4] == [0; 4] {
            break;
        }
        let len = if tag[3] == 4 {
            if frame[4..8].iter().any(|b| b & 0x80 != 0) {
                break;
            }
            frame[4..8]
                .iter()
                .fold(0usize, |size, b| size * 128 + *b as usize)
        } else {
            u32::from_be_bytes(frame[4..8].try_into().unwrap()) as usize
        };
        let Some(next) = (cursor + 10).checked_add(len).filter(|next| *next <= end) else {
            break;
        };
        let destination = match &frame[..4] {
            b"TIT2" => &mut labels.0,
            b"TPE1" => &mut labels.1,
            b"TALB" => &mut labels.2,
            _ => {
                cursor = next;
                continue;
            }
        };
        let payload = &bytes[cursor + 10..next];
        if frame[8..10] == [0; 2] && payload.len() > 1 {
            let text = payload[1..].split(|byte| *byte == 0).next().unwrap();
            if !text.is_empty() && text.len() <= 128 {
                let label = match payload[0] {
                    0 => Some(
                        text.iter()
                            .map(|byte| char::from(*byte))
                            .collect::<String>(),
                    ),
                    3 => core::str::from_utf8(text).ok().map(String::from),
                    _ => None,
                };
                *destination = label.filter(|label| !label.chars().any(char::is_control));
            }
        }
        cursor = next;
    }
    labels
}

fn labels_for(
    path: &str,
    format: MediaFormat,
    bytes: &[u8],
) -> (String, Option<String>, Option<String>) {
    let (title, artist, album) = if format == MediaFormat::Mp3 {
        mp3_labels(bytes)
    } else {
        (None, None, None)
    };
    (
        title.unwrap_or_else(|| title_from_path(path)),
        artist,
        album,
    )
}

fn supported_extension(path: &str) -> Option<MediaFormat> {
    let ext = path.rsplit_once('.')?.1;
    if ext.eq_ignore_ascii_case("ogg") || ext.eq_ignore_ascii_case("oga") {
        Some(MediaFormat::OggVorbis)
    } else if ext.eq_ignore_ascii_case("wav") {
        Some(MediaFormat::WavPcm)
    } else if ext.eq_ignore_ascii_case("mp3") {
        Some(MediaFormat::Mp3)
    } else {
        None
    }
}

#[cfg(target_os = "none")]
pub fn scan_music_directory(root: &str) -> Vec<MediaEntry> {
    use sunlight_libc::{self, DirEntry, FT_DIR, MAX_PATH, O_RDONLY};
    use sunlight_media::decoder::MAX_COMPRESSED_BYTES;
    let mut result = Vec::new();
    let mut pending = Vec::new();
    pending.push(String::from(root));
    while let Some(directory) = pending.pop() {
        let mut entries = [DirEntry::zeroed(); 64];
        let Ok(count) = sunlight_libc::read_dir(directory.as_bytes(), &mut entries) else {
            continue;
        };
        for entry in entries.iter().take(count) {
            let name = core::str::from_utf8(entry.name_bytes()).ok();
            let Some(name) = name.filter(|name| !name.is_empty() && *name != "." && *name != "..")
            else {
                continue;
            };
            let mut path = directory.clone();
            if !path.ends_with('/') {
                path.push('/');
            }
            path.push_str(name);
            if path.len() >= MAX_PATH {
                continue;
            }
            if entry.file_type == FT_DIR {
                if path.matches('/').count() < 32 {
                    pending.push(path);
                }
                continue;
            }
            let Some(format) = supported_extension(&path) else {
                continue;
            };
            let Ok(stat) = sunlight_libc::stat(path.as_bytes()) else {
                continue;
            };
            let size = usize::try_from(stat.size).ok();
            let Some(size) = size.filter(|size| *size > 0 && *size <= MAX_COMPRESSED_BYTES) else {
                continue;
            };
            let Ok(fd) = sunlight_libc::open_with_flags(path.as_bytes(), O_RDONLY) else {
                continue;
            };
            let mut bytes = Vec::with_capacity(size);
            bytes.resize(size, 0);
            let mut offset = 0;
            while offset < size {
                let Ok(read) = sunlight_libc::read(fd, &mut bytes[offset..]) else {
                    break;
                };
                if read == 0 {
                    break;
                }
                offset += read;
            }
            let _ = sunlight_libc::close(fd);
            if offset == 0 {
                continue;
            }
            let info = match sunlight_media::decoder::probe(&bytes[..offset]) {
                Ok(info) => info,
                Err(_)
                    if format == MediaFormat::WavPcm
                        && bytes[..offset].get(..12) == Some(b"RIFF\x00\x00\x00\x00WAVE") =>
                {
                    sunlight_media::AudioStreamInfo {
                        sample_rate_hz: 0,
                        channels: 0,
                        sample_format: sunlight_media::PcmFormat::Signed16LeInterleaved,
                        duration: None,
                        seekable: false,
                    }
                }
                Err(_) => continue,
            };
            if result.iter().any(|entry: &MediaEntry| entry.path == path) {
                continue;
            }
            let (display_title, artist, album) = labels_for(&path, format, &bytes[..offset]);
            result.push(MediaEntry {
                display_title,
                path,
                artist,
                album,
                duration_ms: info.duration.map(|duration| duration.as_millis()),
                format,
                source: MediaSource::UserMusic,
            });
            if result.len() >= 256 {
                break;
            }
        }
    }
    result.sort_by(|a, b| {
        a.display_title
            .cmp(&b.display_title)
            .then_with(|| a.path.cmp(&b.path))
    });
    result
}

#[cfg(test)]
pub fn scan_music_directory(root: &std::path::Path) -> Vec<MediaEntry> {
    use std::fs;
    use std::vec;
    let mut paths = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(path);
                continue;
            }
            let path_string = path.to_string_lossy().into_owned();
            let Some(format) = supported_extension(&path_string) else {
                continue;
            };
            let Ok(bytes) = fs::read(&path) else { continue };
            let Ok(info) = sunlight_media::decoder::probe(&bytes) else {
                continue;
            };
            let normalized = path_string;
            let (display_title, artist, album) = labels_for(&normalized, format, &bytes);
            paths.push(MediaEntry {
                display_title,
                path: normalized,
                artist,
                album,
                duration_ms: info.duration.map(|duration| duration.as_millis()),
                format,
                source: MediaSource::UserMusic,
            });
        }
    }
    paths.sort_by(|a, b| {
        a.display_title
            .to_ascii_lowercase()
            .cmp(&b.display_title.to_ascii_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    paths.dedup_by(|a, b| a.path == b.path);
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{format, fs, path::Path};

    #[test]
    fn missing_and_empty_directories_are_harmless() {
        let root =
            std::env::temp_dir().join(format!("melody-library-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        assert!(scan_music_directory(&root).is_empty());
        fs::create_dir_all(&root).unwrap();
        assert!(scan_music_directory(&root).is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recursively_discovers_valid_media_and_sorts_deterministically() {
        let root =
            std::env::temp_dir().join(format!("melody-library-recursive-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Album")).unwrap();
        let ogg = include_bytes!("../../assets/sounds/melody-mina-test-48k-stereo.ogg");
        let wav = include_bytes!("../../assets/sounds/melody-mina-sample-48k-stereo.wav");
        fs::write(root.join("zeta.ogg"), ogg).unwrap();
        fs::write(root.join("Album/alpha.WAV"), wav).unwrap();
        fs::write(root.join("notes.txt"), b"ignore").unwrap();
        fs::write(root.join("fake.ogg"), b"not ogg").unwrap();
        let entries = scan_music_directory(Path::new(&root));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].display_title, "alpha");
        assert_eq!(entries[1].display_title, "zeta");
        assert_eq!(entries[0].format, MediaFormat::WavPcm);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn living_sunlight_preview_is_player_compatible() {
        let wav = include_bytes!("../../assets/sounds/the-living-sunlight-48k.wav");
        assert!(wav.len() <= sunlight_media::decoder::MAX_COMPRESSED_BYTES);
        let info = sunlight_media::decoder::probe(wav).unwrap();
        assert_eq!(info.sample_rate_hz, 48_000);
        assert_eq!(info.channels, 2);
        assert_eq!(info.duration.unwrap().as_millis(), 18_000);
    }

    #[test]
    fn bundled_mp3_examples_appear_in_music_scan() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/sounds");
        let entries = scan_music_directory(&dir);
        for (name, title, artist) in [
            ("002-Zamfir-EinsamerHirte.mp3", "Einsamer Hirte", "Zamfir"),
            (
                "014-SecretGarden-SongFromASecretGarden.mp3",
                "Song From a Secret Garden",
                "Secret Garden",
            ),
        ] {
            let entry = entries
                .iter()
                .find(|entry| entry.path.ends_with(name))
                .unwrap();
            assert_eq!(entry.display_title, title);
            assert_eq!(entry.artist.as_deref(), Some(artist));
            assert_eq!(entry.format, MediaFormat::Mp3);
            assert!(entry.duration_ms.unwrap() > 200_000);
        }
    }
}
