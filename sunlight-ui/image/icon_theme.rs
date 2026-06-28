//! SunlightOS icon theme constants and path helpers.
//!
//! The SunlightOS icon theme lives at:
//!   `/var/sunlightos/icons/SunlightOS`
//!
//! Directory layout (Breeze-inspired, TGA format):
//!   apps/{size}/      — application icons
//!   places/{size}/    — places, folders
//!   mimetypes/{size}/ — file-type icons
//!   devices/{size}/   — hardware devices
//!   actions/{size}/   — toolbar / menu actions
//!   status/{size}/    — status indicators
//!   preferences/{size}/ — settings / preferences
//!
//! All icons are stored as uncompressed 32-bit BGRA TGA type-2 images.
//! The canonical size for dock / desktop use is 48 or 64 px, but the actual
//! pixel dimensions are always 256×256 (scaled on blit).
//!
//! Fallback order for `resolve_icon`:
//!   1. Exact category/size/name match
//!   2. Alternate category ("places" for folder-style names)
//!   3. Generic fallback (folder, file)
//!
//! For the current release, icons are embedded directly in service binaries
//! using `include_bytes!` (same approach as the wallpaper).  Future releases
//! will support runtime loading from the VFS path above.

/// Root path of the SunlightOS icon theme inside the VFS.
pub const ICON_THEME_ROOT: &str = "/var/sunlightos/icons/SunlightOS";

/// Icon category sub-directories.
pub mod category {
    pub const APPS: &str = "apps";
    pub const PLACES: &str = "places";
    pub const MIMETYPES: &str = "mimetypes";
    pub const DEVICES: &str = "devices";
    pub const ACTIONS: &str = "actions";
    pub const STATUS: &str = "status";
    pub const PREFERENCES: &str = "preferences";
}

/// Write the VFS path for an icon into `buf`.  Returns the byte length written,
/// or 0 if the buffer is too small.
///
/// Path format: `{ICON_THEME_ROOT}/{category}/{size}/{name}.tga`
pub fn icon_path(category: &str, size: u32, name: &str, buf: &mut [u8]) -> usize {
    let mut pos = 0usize;

    macro_rules! append {
        ($s:expr) => {{
            let s: &[u8] = $s;
            if pos + s.len() > buf.len() {
                return 0;
            }
            buf[pos..pos + s.len()].copy_from_slice(s);
            pos += s.len();
        }};
    }

    // u32 → decimal in a stack buffer (no alloc)
    let mut size_buf = [0u8; 10];
    let (size_start, _) = {
        let mut n = size;
        let mut i = 10usize;
        loop {
            i -= 1;
            size_buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        (i, 10 - i)
    };

    append!(ICON_THEME_ROOT.as_bytes());
    append!(b"/");
    append!(category.as_bytes());
    append!(b"/");
    append!(&size_buf[size_start..]);
    append!(b"/");
    append!(name.as_bytes());
    append!(b".tga");

    pos
}

/// Well-known icon names used by SunlightOS services.
pub mod name {
    // Apps
    pub const TERMINAL: &str = "utilities-terminal";
    pub const CALCULATOR: &str = "accessories-calculator";
    pub const FILE_MANAGER: &str = "system-file-manager";
    pub const SETTINGS: &str = "preferences-system";

    // Places / folders
    pub const FOLDER: &str = "folder";
    pub const FOLDER_HOME: &str = "folder_home";
    pub const FOLDER_DOCUMENTS: &str = "folder-documents";
    pub const FOLDER_DOWNLOADS: &str = "folder-downloads";
    pub const FOLDER_MUSIC: &str = "folder-music";
    pub const FOLDER_PICTURES: &str = "folder-pictures";
    pub const FOLDER_VIDEOS: &str = "folder-videos";
    pub const FOLDER_DESKTOP: &str = "folder-desktop";
    pub const USER_HOME: &str = "user-home";
    pub const USER_TRASH: &str = "user-trash";

    // Mimetypes
    pub const TEXT_GENERIC: &str = "text-x-generic";
    pub const IMAGE_GENERIC: &str = "image-x-generic";
    pub const VIDEO_GENERIC: &str = "video-x-generic";
    pub const AUDIO_GENERIC: &str = "audio-x-generic";
    pub const APP_EXECUTABLE: &str = "application-x-executable";
    pub const APP_ARCHIVE: &str = "application-gzip";

    // Devices
    pub const COMPUTER: &str = "computer";
    pub const DRIVE: &str = "drive-harddisk";
    pub const INPUT_MOUSE: &str = "input-mouse";
    pub const VIDEO_DISPLAY: &str = "video-display";

    // Preferences
    pub const PREF_MOUSE: &str = "preferences-desktop-mouse";
    pub const PREF_DISPLAY: &str = "preferences-desktop-display";
}
