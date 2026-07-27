#![no_std]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use sunlight_libc as libc;

pub const BUILTIN_WALLPAPER_DIR: &str = "/system/share/wallpapers";
pub const LEGACY_WALLPAPER_DIR: &str = "/var/sunlightos/wallpapers";
pub const USER_WALLPAPER_DIR: &str = "/root/.local/share/sunlight/wallpapers";
pub const CONFIG_PATH: &str = "/root/.config/sunlight/desktop.toml";
pub const CONFIG_TMP_PATH: &str = "/root/.config/sunlight/desktop.toml.tmp";
/// Empty path means solid desktop color (no image). Avoids loading the multi-MiB
/// default TGA into the shell process at every login.
pub const DEFAULT_WALLPAPER_PATH: &str = "";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallpaperMode {
    Cover,
}

impl WallpaperMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cover => "cover",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopConfig {
    pub wallpaper: String,
    pub wallpaper_mode: WallpaperMode,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            wallpaper: String::from(DEFAULT_WALLPAPER_PATH),
            wallpaper_mode: WallpaperMode::Cover,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WallpaperEntry {
    /// Listed/source asset (e.g. a `.tga`, or a `.jpg`/`.png` that has a TGA sidecar).
    pub path: String,
    /// Path written to the desktop config and rendered by the shell.
    /// Always a render-ready TGA (the desktop renderer is TGA-only); for a
    /// `.jpg`/`.png` source this points at its converted `.tga` sidecar.
    pub apply_path: String,
    /// Path the settings preview loads. Always a TGA (preview is TGA-only).
    pub preview_path: String,
    pub label: String,
    pub source: WallpaperSource,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallpaperSource {
    Builtin,
    User,
    Legacy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WallpaperError {
    Io,
    InvalidConfig,
    UnsupportedImage,
    CorruptImage,
}

pub fn load_desktop_config() -> DesktopConfig {
    let Some(bytes) = read_file_bytes(CONFIG_PATH.as_bytes(), 1024) else {
        return DesktopConfig::default();
    };
    parse_desktop_config(&bytes).unwrap_or_default()
}

pub fn parse_desktop_config(bytes: &[u8]) -> Result<DesktopConfig, WallpaperError> {
    let text = core::str::from_utf8(bytes).map_err(|_| WallpaperError::InvalidConfig)?;
    let mut cfg = DesktopConfig::default();
    let mut in_desktop = false;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[desktop]" {
            in_desktop = true;
            continue;
        }
        if line.starts_with('[') {
            in_desktop = false;
            continue;
        }
        if !in_desktop {
            continue;
        }
        if let Some(value) = line.strip_prefix("wallpaper =") {
            if let Some(parsed) = parse_toml_string(value.trim()) {
                cfg.wallpaper = parsed;
            }
        } else if let Some(value) = line.strip_prefix("wallpaper_mode =") {
            if let Some(parsed) = parse_toml_string(value.trim()) {
                if parsed == "cover" {
                    cfg.wallpaper_mode = WallpaperMode::Cover;
                }
            }
        }
    }
    Ok(cfg)
}

pub fn save_desktop_config(cfg: &DesktopConfig) -> Result<(), WallpaperError> {
    let mut out = String::from("[desktop]\nwallpaper = \"");
    out.push_str(&cfg.wallpaper);
    out.push_str("\"\nwallpaper_mode = \"");
    out.push_str(cfg.wallpaper_mode.as_str());
    out.push_str("\"\n");

    libc::mkdir_recursive(b"/root/.config/sunlight").map_err(|_| WallpaperError::Io)?;
    let fd = libc::open_with_flags(
        CONFIG_TMP_PATH.as_bytes(),
        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
    )
    .map_err(|_| WallpaperError::Io)?;
    let write_res = libc::write_all(fd, out.as_bytes()).map_err(|_| WallpaperError::Io);
    let _ = libc::close(fd);
    write_res?;
    libc::rename(CONFIG_TMP_PATH.as_bytes(), CONFIG_PATH.as_bytes())
        .map_err(|_| WallpaperError::Io)?;
    Ok(())
}

pub fn scan_wallpapers(active_path: &str) -> Vec<WallpaperEntry> {
    let mut out = Vec::new();
    // The local Wallpaper Settings MVP lists the bundled wallpapers from the
    // wallpaper directory. Builtin/User directories can be re-enabled once
    // additional render-ready assets are staged there.
    scan_dir(
        LEGACY_WALLPAPER_DIR,
        WallpaperSource::Legacy,
        active_path,
        &mut out,
    );
    // Stable name order so tiles do not jump when a different one is selected.
    // The current wallpaper is indicated by `selected`, not by grid position.
    out.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    out
}

pub fn is_supported_wallpaper(bytes: &[u8]) -> bool {
    if bytes.len() < 18 {
        return false;
    }
    let id_len = bytes[0] as usize;
    let has_color_map = bytes[1] != 0;
    let image_type = bytes[2];
    let color_map_len = u16::from_le_bytes([bytes[5], bytes[6]]) as usize;
    let color_map_entry_bits = bytes[7] as usize;
    let width = u16::from_le_bytes([bytes[12], bytes[13]]) as u32;
    let height = u16::from_le_bytes([bytes[14], bytes[15]]) as u32;
    let bit_depth = bytes[16];
    if width == 0 || height == 0 || image_type != 2 || !matches!(bit_depth, 24 | 32) {
        return false;
    }
    let color_map_bytes = if has_color_map {
        color_map_len.saturating_mul((color_map_entry_bits + 7) / 8)
    } else {
        0
    };
    let bytes_per_pixel = (bit_depth as usize) / 8;
    let data_offset = 18usize
        .saturating_add(id_len)
        .saturating_add(color_map_bytes);
    let pixel_bytes = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(bytes_per_pixel);
    data_offset
        .checked_add(pixel_bytes)
        .map(|needed| bytes.len() >= needed)
        .unwrap_or(false)
}

pub fn read_file_bytes(path: &[u8], limit: usize) -> Option<Vec<u8>> {
    let fd = libc::open(path).ok()?;
    let reserve = libc::fstat(fd)
        .ok()
        .map(|stat| (stat.size as usize).min(limit))
        .unwrap_or(0);
    let mut out = Vec::with_capacity(reserve);
    let mut buf = [0u8; 256];
    loop {
        let n = match libc::read(fd, &mut buf) {
            Ok(n) => n,
            Err(libc::sys::Errno::Again) => {
                continue;
            }
            Err(_) => {
                let _ = libc::close(fd);
                return None;
            }
        };
        if n == 0 {
            break;
        }
        let take = (limit - out.len()).min(n);
        out.extend_from_slice(&buf[..take]);
        if out.len() >= limit || take < n {
            break;
        }
    }
    let _ = libc::close(fd);
    Some(out)
}

fn scan_dir(dir: &str, source: WallpaperSource, active_path: &str, out: &mut Vec<WallpaperEntry>) {
    let mut entries = [libc::DirEntry::zeroed(); 64];
    let Ok(count) = libc::read_dir(dir.as_bytes(), &mut entries) else {
        return;
    };
    for entry in entries.iter().take(count) {
        let name = sanitize_ascii(entry.name_bytes());
        if !is_wallpaper_candidate(&name) {
            continue;
        }
        // The desktop renderer is TGA-only. `.jpg`/`.jpeg`/`.png` are recognised
        // as wallpaper asset types (is_wallpaper_candidate), but only render-ready
        // `.tga` entries are surfaced here: a bundled image is expected to ship
        // alongside its converted `.tga` sidecar, which is what gets listed and
        // applied. This keeps `apply_path`/`preview_path` always TGA-safe.
        if !has_ext(&name, "tga") {
            continue;
        }
        let path = join_path(dir, &name);
        out.push(WallpaperEntry {
            label: wallpaper_label(&name),
            selected: path == active_path,
            apply_path: path.clone(),
            preview_path: path.clone(),
            path,
            source,
        });
    }
}

fn is_wallpaper_candidate(name: &str) -> bool {
    if name.is_empty() || name.starts_with('.') {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tmp") || lower.ends_with(".part") {
        return false;
    }
    lower.ends_with(".tga")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".png")
}

/// Case-insensitive check that `name` ends with `.{ext}` (ext without the dot).
fn has_ext(name: &str, ext: &str) -> bool {
    if name.len() <= ext.len() + 1 {
        return false;
    }
    let dot = name.len() - ext.len() - 1;
    name.as_bytes()[dot] == b'.' && name[dot + 1..].eq_ignore_ascii_case(ext)
}

fn wallpaper_label(name: &str) -> String {
    let stem = wallpaper_stem(name);
    let mut out = String::with_capacity(stem.len() + 4);
    let mut upper = true;
    let mut prev_was_letter = false;
    for ch in stem.chars() {
        if ch == '-' || ch == '_' {
            out.push(' ');
            upper = true;
            prev_was_letter = false;
        } else if ch.is_ascii_digit() {
            // Separate a trailing number from the preceding word:
            // "wallpaper1" -> "Wallpaper 1".
            if prev_was_letter {
                out.push(' ');
            }
            out.push(ch);
            upper = false;
            prev_was_letter = false;
        } else {
            let c = if upper {
                upper = false;
                ch.to_ascii_uppercase()
            } else {
                ch
            };
            out.push(c);
            prev_was_letter = ch.is_ascii_alphabetic();
        }
    }
    out
}

/// Strip a recognised image extension (case-insensitive) to get the display stem.
fn wallpaper_stem(name: &str) -> &str {
    const EXTS: [&str; 4] = [".tga", ".jpg", ".jpeg", ".png"];
    for &ext in EXTS.iter() {
        if name.len() >= ext.len() && name[name.len() - ext.len()..].eq_ignore_ascii_case(ext) {
            return &name[..name.len() - ext.len()];
        }
    }
    name
}

fn join_path(base: &str, leaf: &str) -> String {
    let mut out = String::with_capacity(base.len() + leaf.len() + 1);
    out.push_str(base);
    if !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(leaf);
    out
}

fn parse_toml_string(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if !(trimmed.starts_with('"') && trimmed.ends_with('"')) || trimmed.len() < 2 {
        return None;
    }
    Some(String::from(&trimmed[1..trimmed.len() - 1]))
}

fn sanitize_ascii(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b == 0 {
            break;
        }
        if matches!(b, 0x20..=0x7E) {
            out.push(b as char);
        }
    }
    out
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_defaults() {
        assert_eq!(DesktopConfig::default().wallpaper, "");
        assert_eq!(DesktopConfig::default().wallpaper, DEFAULT_WALLPAPER_PATH);
    }

    #[test]
    fn malformed_config_falls_back_safely() {
        let cfg = parse_desktop_config(b"not toml").unwrap();
        assert_eq!(cfg, DesktopConfig::default());
    }

    #[test]
    fn invalid_utf8_config_is_rejected() {
        assert_eq!(
            parse_desktop_config(&[0xff, 0xfe]).unwrap_err(),
            WallpaperError::InvalidConfig
        );
    }

    #[test]
    fn parse_config_reads_wallpaper_path() {
        let cfg = parse_desktop_config(
            b"[desktop]\nwallpaper = \"/system/share/wallpapers/dark.tga\"\nwallpaper_mode = \"cover\"\n",
        )
        .unwrap();
        assert_eq!(cfg.wallpaper, "/system/share/wallpapers/dark.tga");
        assert_eq!(cfg.wallpaper_mode, WallpaperMode::Cover);
    }

    #[test]
    fn parse_config_ignores_unknown_mode_and_keeps_default() {
        let cfg = parse_desktop_config(
            b"[desktop]\nwallpaper = \"/system/share/wallpapers/default.tga\"\nwallpaper_mode = \"stretch\"\n",
        )
        .unwrap();
        assert_eq!(cfg.wallpaper_mode, WallpaperMode::Cover);
    }

    #[test]
    fn save_config_renders_expected_toml() {
        let cfg = DesktopConfig {
            wallpaper: String::from("/system/share/wallpapers/dark.tga"),
            wallpaper_mode: WallpaperMode::Cover,
        };
        let rendered = {
            let mut out = String::from("[desktop]\nwallpaper = \"");
            out.push_str(&cfg.wallpaper);
            out.push_str("\"\nwallpaper_mode = \"");
            out.push_str(cfg.wallpaper_mode.as_str());
            out.push_str("\"\n");
            out
        };
        assert_eq!(
            rendered,
            "[desktop]\nwallpaper = \"/system/share/wallpapers/dark.tga\"\nwallpaper_mode = \"cover\"\n"
        );
    }

    #[test]
    fn scan_filters_names() {
        assert!(is_wallpaper_candidate("default.tga"));
        assert!(is_wallpaper_candidate("DEFAULT.TGA"));
        assert!(is_wallpaper_candidate("wallpaper1.jpg"));
        assert!(is_wallpaper_candidate("wallpaper2.JPEG"));
        assert!(is_wallpaper_candidate("wallpaper3.png"));
        assert!(!is_wallpaper_candidate("bad.tmp"));
        assert!(!is_wallpaper_candidate("default.tga.tmp"));
        assert!(!is_wallpaper_candidate("download.part"));
        assert!(!is_wallpaper_candidate(".hidden.tga"));
        assert!(!is_wallpaper_candidate("readme.txt"));
        assert!(!is_wallpaper_candidate(""));
    }

    #[test]
    fn wallpaper_label_humanizes_names() {
        assert_eq!(wallpaper_label("default.tga"), "Default");
        assert_eq!(
            wallpaper_label("sunlight-login-background.tga"),
            "Sunlight Login Background"
        );
        assert_eq!(wallpaper_label("dark_mode.tga"), "Dark Mode");
        assert_eq!(wallpaper_label("wallpaper.tga"), "Wallpaper");
        assert_eq!(wallpaper_label("wallpaper1.tga"), "Wallpaper 1");
        assert_eq!(wallpaper_label("wallpaper6.tga"), "Wallpaper 6");
    }

    #[test]
    fn wallpaper_sort_keeps_original_before_numbered() {
        // Byte comparison puts '.' (0x2E) before '1' (0x31), so the original
        // wallpaper.tga sorts ahead of wallpaper1.tga..wallpaper6.tga.
        let mut paths = [
            String::from("/var/sunlightos/wallpapers/wallpaper1.tga"),
            String::from("/var/sunlightos/wallpapers/wallpaper2.tga"),
            String::from("/var/sunlightos/wallpapers/wallpaper3.tga"),
            String::from("/var/sunlightos/wallpapers/wallpaper4.tga"),
            String::from("/var/sunlightos/wallpapers/wallpaper5.tga"),
            String::from("/var/sunlightos/wallpapers/wallpaper6.tga"),
            String::from("/var/sunlightos/wallpapers/wallpaper.tga"),
        ];
        paths.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        let expected = [
            "/var/sunlightos/wallpapers/wallpaper.tga",
            "/var/sunlightos/wallpapers/wallpaper1.tga",
            "/var/sunlightos/wallpapers/wallpaper2.tga",
            "/var/sunlightos/wallpapers/wallpaper3.tga",
            "/var/sunlightos/wallpapers/wallpaper4.tga",
            "/var/sunlightos/wallpapers/wallpaper5.tga",
            "/var/sunlightos/wallpapers/wallpaper6.tga",
        ];
        assert_eq!(paths, expected);
    }

    #[test]
    fn supported_wallpaper_requires_type2_and_dimensions() {
        let valid = [
            0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 24, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0,
        ];
        assert!(is_supported_wallpaper(&valid));

        let mut rle = valid;
        rle[2] = 10;
        assert!(!is_supported_wallpaper(&rle));

        let mut zero_width = valid;
        zero_width[12] = 0;
        zero_width[13] = 0;
        assert!(!is_supported_wallpaper(&zero_width));
    }

    #[test]
    fn active_wallpaper_selection_marks_selected() {
        let entries = [
            WallpaperEntry {
                path: String::from("/system/share/wallpapers/dark.tga"),
                apply_path: String::from("/system/share/wallpapers/dark.tga"),
                preview_path: String::from("/system/share/wallpapers/dark.tga"),
                label: String::from("Dark"),
                source: WallpaperSource::Builtin,
                selected: false,
            },
            WallpaperEntry {
                path: String::from("/system/share/wallpapers/default.tga"),
                apply_path: String::from("/system/share/wallpapers/default.tga"),
                preview_path: String::from("/system/share/wallpapers/default.tga"),
                label: String::from("Default"),
                source: WallpaperSource::Builtin,
                selected: true,
            },
        ];
        assert!(entries.iter().any(|entry| entry.selected));
        assert_eq!(
            entries.iter().find(|entry| entry.selected).unwrap().path,
            "/system/share/wallpapers/default.tga"
        );
    }
}
