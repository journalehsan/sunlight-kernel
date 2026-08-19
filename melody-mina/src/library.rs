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

fn supported_extension(path: &str) -> Option<MediaFormat> {
    let ext = path.rsplit_once('.')?.1;
    if ext.eq_ignore_ascii_case("ogg") || ext.eq_ignore_ascii_case("oga") {
        Some(MediaFormat::OggVorbis)
    } else if ext.eq_ignore_ascii_case("wav") {
        Some(MediaFormat::WavPcm)
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
            let Some(size) = size.filter(|size| *size > 0) else {
                continue;
            };
            let Ok(fd) = sunlight_libc::open_with_flags(path.as_bytes(), O_RDONLY) else {
                continue;
            };
            let read_limit = size.min(MAX_COMPRESSED_BYTES);
            let mut bytes = Vec::with_capacity(read_limit);
            bytes.resize(read_limit, 0);
            let mut offset = 0;
            while offset < read_limit {
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
            result.push(MediaEntry {
                display_title: title_from_path(&path),
                path,
                artist: None,
                album: None,
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
            paths.push(MediaEntry {
                display_title: title_from_path(&normalized),
                path: normalized,
                artist: None,
                album: None,
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
}
