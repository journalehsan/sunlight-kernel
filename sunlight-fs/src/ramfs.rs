//! Static boot image and compatibility names for the page-backed Tmpfs.
pub use crate::tmpfs::Tmpfs as RamFs;
use crate::vfs::mode;
pub const RAMFS_MAX_HANDLES: usize = 32;
pub const RAMFS_MAX_ENTRIES: usize = 512;

pub struct RamEntry {
    pub path: &'static str,
    pub data: &'static [u8],
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
    pub is_dir: bool,
}

/// Exact sum of static INITRAMFS file payload bytes (directories contribute 0).
/// These payloads are embedded in the kernel image via `include_bytes!` and
/// remain resident there; there is no second physical copy after mount.
pub fn initramfs_static_file_data_bytes(entries: &[RamEntry]) -> u64 {
    let mut total: u64 = 0;
    for e in entries {
        if !e.is_dir {
            total = total.saturating_add(e.data.len() as u64);
        }
    }
    total
}

/// True when static RAMFS file data is embedded in the kernel/boot image
/// (include_bytes) rather than allocated as separate PMM frames.
pub const INITRAMFS_EMBEDDED_IN_KERNEL_IMAGE: bool = true;

impl RamEntry {
    pub const fn file(
        path: &'static str,
        uid: u32,
        gid: u32,
        file_mode: u16,
        data: &'static [u8],
    ) -> Self {
        Self {
            path,
            data,
            uid,
            gid,
            mode: file_mode,
            is_dir: false,
        }
    }

    pub const fn dir(path: &'static str, uid: u32, gid: u32, dir_mode: u16) -> Self {
        Self {
            path,
            data: b"",
            uid,
            gid,
            mode: dir_mode,
            is_dir: true,
        }
    }
}

/// The conventional per-user directory structure that the OS guarantees to
/// exist inside every home directory (`/root` and each `/home/<user>`).
///
/// This is an OS-level responsibility: the kernel/filesystem seeds it at boot
/// for `/root` and the default user (see `INITRAMFS`), and the user-creation
/// path (`sunshell` `useradd`) reproduces it for every new account. The file
/// manager (`sunlight-files`) deliberately does NOT create these; it only
/// assumes they exist and surfaces an error if navigation fails.
pub const STANDARD_HOME_DIRS: &[&str] = &[
    "Desktop",
    "Documents",
    "Downloads",
    "Pictures",
    "Music",
    "Videos",
];

static CAT_BIG_BYTES: [u8; 513] = [b'x'; 513];

static HEAD_MULTILINE: &[u8] = b"line1\nline2\nline3\nline4\nline5\n\
line6\nline7\nline8\nline9\nline10\n\
line11\nline12\nline13\nline14\nline15\n\
line16\nline17\nline18\nline19\nline20\n";

static HEAD_ONELINE: &[u8] = b"just one line\n";

static CMP_IDENTICAL: &[u8] = b"hello world\nthis is a test file\n";

static CMP_DIFF_A: &[u8] = b"hello world\nthis is file a\n";

static CMP_DIFF_B: &[u8] = b"hello world\nthis is file b\n";

// Phase 2B.3 test fixtures — wc, cut, fold, expand
static WC_TEXT: &[u8] = b"hello world\nline two\n  line three  \n";
static WC_EMPTY: &[u8] = b"";
static WC_NO_NL: &[u8] = b"no newline here";
static WC_SPACES: &[u8] = b"   \t  \n";
static WC_UTF8: &[u8] = &[
    0x68, 0xC3, 0xA9, 0x6C, 0x6C, 0x6F, 0x0A, 0x77, 0xC3, 0xB6, 0x72, 0x6C, 0x64, 0x0A,
];

static CUT_DELIM: &[u8] = b"a:b:c:d\n1:2:3:4\n";
static CUT_NO_DELIM: &[u8] = b"no delimiters here\n";
static CUT_BYTES: &[u8] = b"abcdefghij\n";

static FOLD_LONG: &[u8] =
    b"this is a very long line that should be folded at the specified width\n";
static FOLD_EXACT: &[u8] = b"1234567890\n";
static FOLD_OVER: &[u8] = b"12345678901\n";
static FOLD_TABS: &[u8] = b"hello\tworld\t!\n";

static EXPAND_TABS: &[u8] = b"a\tb\tc\n";
static EXPAND_CONSECUTIVE: &[u8] = b"\t\tx\n";
static EXPAND_NO_NL: &[u8] = b"a\tb";

static GREP_TEXT: &[u8] = b"sunlight kernel\nposix utilities\nhello world\n";
static SORT_DATA: &[u8] = b"z\na\nm\n";
static UNIQ_DATA: &[u8] = b"apple\napple\nbanana\ncherry\ncherry\n";
static COMM_A: &[u8] = b"apple\nbanana\ncherry\n";
static COMM_B: &[u8] = b"banana\ncherry\ndate\n";

// Phase 2B.5 fixtures — deliberately small, ordered, and byte-visible.
static PASTE_A: &[u8] = b"a1\na2\n";
static PASTE_B: &[u8] = b"b1\n";
static PASTE_SERIAL: &[u8] = b"one\ntwo";
static JOIN_A: &[u8] = b"a x\na y\nb z\n";
static JOIN_B: &[u8] = b"a p\na q\nc r\n";

pub static INITRAMFS: &[RamEntry] = &[
    // Directories
    RamEntry::dir("/", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc/sunlight", 0, 0, mode::DIR_755),
    RamEntry::dir("/bin", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/ChronosDosShell.sunapp", 0, 0, mode::DIR_755),
    RamEntry::dir(
        "/Applications/ChronosDosShell.sunapp/Program",
        0,
        0,
        mode::DIR_755,
    ),
    RamEntry::dir(
        "/Applications/ChronosDosShell.sunapp/Program/TESTS",
        0,
        0,
        mode::DIR_755,
    ),
    RamEntry::dir(
        "/Applications/ChronosDosShell.sunapp/Resources",
        0,
        0,
        mode::DIR_755,
    ),
    RamEntry::dir(
        "/Applications/ChronosDosShell.sunapp/Licenses",
        0,
        0,
        mode::DIR_755,
    ),

    RamEntry::dir("/Applications/SunlightMines.sunapp", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/SunlightMines.sunapp/Program", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/SunlightMines.sunapp/Resources", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/SunlightMines.sunapp/Defaults", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/SunlightMines.sunapp/Licenses", 0, 0, mode::DIR_755),

    RamEntry::dir("/Applications/ChronosFileLab.sunapp", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/ChronosFileLab.sunapp/Program", 0, 0, mode::DIR_755),
    RamEntry::dir(
        "/Applications/ChronosFileLab.sunapp/Dependencies",
        0,
        0,
        mode::DIR_755,
    ),
    RamEntry::dir("/Applications/ChronosFileLab.sunapp/Resources", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/ChronosFileLab.sunapp/Defaults", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/ChronosFileLab.sunapp/Licenses", 0, 0, mode::DIR_755),

    // -- Standard user home directory layout (OS responsibility) ------------
    // These directories are seeded by the OS so every home directory ships
    // with the conventional folder structure. This is intentionally done here
    // (and in the user-creation path), NOT in the file manager: the file
    // manager only navigates existing folders and must never be responsible
    // for creating the standard home layout.
    //
    // Keep this list in sync with `STANDARD_HOME_DIRS` below.
    RamEntry::dir("/root", 0, 0, mode::DIR_700),
    // Root's standard folders (uid 0).
    RamEntry::dir("/root/Desktop", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/Documents", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/Downloads", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/Pictures", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/Music", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/Videos", 0, 0, mode::DIR_700),

    RamEntry::dir("/home", 0, 0, mode::DIR_755),
    RamEntry::dir("/root/.config", 0, 0, mode::DIR_700),
    RamEntry::dir("/root/.config/sunlight", 0, 0, mode::DIR_700),
    // Yazi's user theme is supplied by the OS so its directory entries stay
    // legible on the Sunlight terminal without requiring writable config.
    RamEntry::dir("/root/.config/yazi", 0, 0, mode::DIR_700),
    RamEntry::dir("/tests", 0, 0, mode::DIR_755),
    RamEntry::dir("/home/user", 1000, 1000, mode::DIR_755),
    // Default unprivileged user's standard folders (uid/gid 1000).
    RamEntry::dir("/home/user/Desktop", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Documents", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Downloads", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Pictures", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Music", 1000, 1000, mode::DIR_755),
    RamEntry::dir("/home/user/Videos", 1000, 1000, mode::DIR_755),
    // -- End standard home directory layout ---------------------------------

    // Deterministic built-in Melody Mina fixture: 48 kHz, stereo, signed
    // 16-bit PCM, six seconds. It is a normal RAMFS file so applications use
    // the same media API as user-selected sources.
    RamEntry::dir("/usr/share/sunlightos/media", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/usr/share/sunlightos/media/melody-mina-sample.wav",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/melody-mina-sample-48k-stereo.wav"),
    ),
    RamEntry::file(
        "/home/user/Music/Catch the Sunlight.ogg",
        1000,
        1000,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/catch-the-sunlight-48k.ogg"),
    ),
    RamEntry::file(
        "/root/Music/Catch the Sunlight.ogg",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/catch-the-sunlight-48k.ogg"),
    ),
    // MP3 playback examples are regular Music files for both local users.
    RamEntry::file(
        "/home/user/Music/002-Zamfir-EinsamerHirte.mp3",
        1000,
        1000,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/002-Zamfir-EinsamerHirte.mp3"),
    ),
    RamEntry::file(
        "/root/Music/002-Zamfir-EinsamerHirte.mp3",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/002-Zamfir-EinsamerHirte.mp3"),
    ),
    RamEntry::file(
        "/home/user/Music/014-SecretGarden-SongFromASecretGarden.mp3",
        1000,
        1000,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/014-SecretGarden-SongFromASecretGarden.mp3"),
    ),
    RamEntry::file(
        "/root/Music/014-SecretGarden-SongFromASecretGarden.mp3",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/014-SecretGarden-SongFromASecretGarden.mp3"),
    ),
    RamEntry::file(
        "/home/user/Music/081-Mantovani-Greensleeves.mp3",
        1000,
        1000,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/081-Mantovani-Greensleeves.mp3"),
    ),
    RamEntry::file(
        "/root/Music/081-Mantovani-Greensleeves.mp3",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/081-Mantovani-Greensleeves.mp3"),
    ),
    RamEntry::file(
        "/home/user/Music/095-ClaudeCiari-ElBimbo.mp3",
        1000,
        1000,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/095-ClaudeCiari-ElBimbo.mp3"),
    ),
    RamEntry::file(
        "/root/Music/095-ClaudeCiari-ElBimbo.mp3",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/095-ClaudeCiari-ElBimbo.mp3"),
    ),
    RamEntry::file(
        "/home/user/Music/love story (320).mp3",
        1000,
        1000,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/love story (320).mp3"),
    ),
    RamEntry::file(
        "/root/Music/love story (320).mp3",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/love story (320).mp3"),
    ),
    // The Living Sunlight: procedural G-minor chiptune preview. Melody Mina
    // discovers it through the ordinary Music directory scanner.
    RamEntry::file(
        "/home/user/Music/The Living Sunlight.wav",
        1000,
        1000,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/the-living-sunlight-48k.wav"),
    ),
    RamEntry::file(
        "/root/Music/The Living Sunlight.wav",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/sounds/the-living-sunlight-48k.wav"),
    ),

    RamEntry::file("/tests/cat-empty", 0, 0, mode::FILE_644, b""),
    RamEntry::file("/tests/cat-hello", 0, 0, mode::FILE_644, b"hello from cat\n"),
    RamEntry::file("/tests/cat-nonewline", 0, 0, mode::FILE_644, b"nonewline"),
    RamEntry::file("/tests/cat-big", 0, 0, mode::FILE_644, &CAT_BIG_BYTES),
    RamEntry::file("/tests/ho", 0, 0, mode::FILE_644, HEAD_ONELINE),
    RamEntry::file("/tests/hm", 0, 0, mode::FILE_644, HEAD_MULTILINE),
    RamEntry::file("/tests/ia", 0, 0, mode::FILE_644, CMP_IDENTICAL),
    RamEntry::file("/tests/ib", 0, 0, mode::FILE_644, CMP_IDENTICAL),
    RamEntry::file("/tests/da", 0, 0, mode::FILE_644, CMP_DIFF_A),
    RamEntry::file("/tests/db", 0, 0, mode::FILE_644, CMP_DIFF_B),
    // Phase 2B.3: wc, cut, fold, expand
    RamEntry::file("/tests/wc-text", 0, 0, mode::FILE_644, WC_TEXT),
    RamEntry::file("/tests/wc-empty", 0, 0, mode::FILE_644, WC_EMPTY),
    RamEntry::file("/tests/wc-nonl", 0, 0, mode::FILE_644, WC_NO_NL),
    RamEntry::file("/tests/wc-spaces", 0, 0, mode::FILE_644, WC_SPACES),
    RamEntry::file("/tests/wc-utf8", 0, 0, mode::FILE_644, WC_UTF8),
    RamEntry::file("/tests/cut-delim", 0, 0, mode::FILE_644, CUT_DELIM),
    RamEntry::file("/tests/cut-nodelim", 0, 0, mode::FILE_644, CUT_NO_DELIM),
    RamEntry::file("/tests/cut-bytes", 0, 0, mode::FILE_644, CUT_BYTES),
    RamEntry::file("/tests/fold-long", 0, 0, mode::FILE_644, FOLD_LONG),
    RamEntry::file("/tests/fold-exact", 0, 0, mode::FILE_644, FOLD_EXACT),
    RamEntry::file("/tests/fold-over", 0, 0, mode::FILE_644, FOLD_OVER),
    RamEntry::file("/tests/fold-tabs", 0, 0, mode::FILE_644, FOLD_TABS),
    RamEntry::file("/tests/expand-tabs", 0, 0, mode::FILE_644, EXPAND_TABS),
    RamEntry::file("/tests/expand-cons", 0, 0, mode::FILE_644, EXPAND_CONSECUTIVE),
    RamEntry::file("/tests/expand-nonl", 0, 0, mode::FILE_644, EXPAND_NO_NL),
    RamEntry::file("/tests/grep-text", 0, 0, mode::FILE_644, GREP_TEXT),
    RamEntry::file("/tests/sort-data", 0, 0, mode::FILE_644, SORT_DATA),
    RamEntry::file("/tests/uniq-data", 0, 0, mode::FILE_644, UNIQ_DATA),
    RamEntry::file("/tests/comm-a", 0, 0, mode::FILE_644, COMM_A),
    RamEntry::file("/tests/comm-b", 0, 0, mode::FILE_644, COMM_B),
    RamEntry::file("/tests/paste-a", 0, 0, mode::FILE_644, PASTE_A),
    RamEntry::file("/tests/paste-b", 0, 0, mode::FILE_644, PASTE_B),
    RamEntry::file("/tests/paste-serial", 0, 0, mode::FILE_644, PASTE_SERIAL),
    RamEntry::file("/tests/join-a", 0, 0, mode::FILE_644, JOIN_A),
    RamEntry::file("/tests/join-b", 0, 0, mode::FILE_644, JOIN_B),

    RamEntry::file("/bin/voice-recorder", 0, 0, mode::FILE_755, b"#!/sunlight/voice-recorder\n"),
    RamEntry::file("/usr/bin/voice-recorder", 0, 0, mode::FILE_755, b"#!/sunlight/voice-recorder\n"),
    RamEntry::dir("/Applications/VoiceRecorder.sunapp", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/VoiceRecorder.sunapp/Resources", 0, 0, mode::DIR_755),
    RamEntry::file("/Applications/VoiceRecorder.sunapp/Manifest.toml", 0, 0, mode::FILE_644, include_bytes!("../../VoiceRecorder.sunapp/Manifest.toml")),
    RamEntry::file("/Applications/VoiceRecorder.sunapp/Resources/icon.svg", 0, 0, mode::FILE_644, include_bytes!("../../docs/icons/SunlightOS/devices/16/audio-input-microphone.svg")),
    RamEntry::dir("/tmp", 0, 0, mode::DIR_1777),
    RamEntry::dir("/mnt", 0, 0, mode::DIR_755),
    RamEntry::file("/bin/lsblk", 0, 0, mode::FILE_755, b"#!/kernel/lsblk\n"),
    RamEntry::file("/bin/storagectl", 0, 0, mode::FILE_755, b"#!/kernel/storagectl\n"),
    // The kernel only supplies this fixture alias in an explicit runtime test image.
    RamEntry::file("/bin/storage-unprivileged", 0, 0, mode::FILE_755, b"#!/kernel/storage-unprivileged\n"),
    RamEntry::dir("/run", 0, 0, mode::DIR_755),
    RamEntry::dir("/state", 0, 0, mode::DIR_755),
    RamEntry::dir("/state/sunlightd", 0, 0, mode::DIR_700),
    RamEntry::dir("/state/sunlight-kv", 0, 0, mode::DIR_700),
    RamEntry::dir("/state/wiseowl-memoryd", 0, 0, mode::DIR_700),
    RamEntry::dir("/state/wiseowl-memorydb", 0, 0, mode::DIR_700),
    RamEntry::file(
        "/bin/wiseowl-memorydbctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/wiseowl-memorydbctl\n",
    ),
    RamEntry::file(
        "/bin/wiseowl-memoryctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/wiseowl-memoryctl\n",
    ),
    RamEntry::file(
        "/bin/wiseowl-indexctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/wiseowl-indexctl\n",
    ),
    RamEntry::dir("/state/wiseowl-braind", 0, 0, mode::DIR_700),
    RamEntry::file(
        "/bin/wiseowl-brainctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/wiseowl-brainctl\n",
    ),
    RamEntry::file("/bin/wiseowl", 0, 0, mode::FILE_755, b"#!/sunlight/wiseowl\n"),
    RamEntry::dir("/state/sunlight-tls", 0, 0, mode::DIR_700),
    RamEntry::dir("/state/sunlight-uac", 0, 0, mode::DIR_700),
    RamEntry::dir("/state/capability-broker", 0, 0, mode::DIR_700),
    RamEntry::dir("/srv", 0, 0, mode::DIR_755),
    RamEntry::dir("/srv/http", 0, 0, mode::DIR_755),
    RamEntry::dir("/var", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/lib", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/log", 0, 0, mode::DIR_755),
    RamEntry::dir("/system", 0, 0, mode::DIR_755),
    RamEntry::dir("/system/share", 0, 0, mode::DIR_755),
    RamEntry::dir("/system/share/wallpapers", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/wallpapers", 0, 0, mode::DIR_755),
    // SunlightOS icon theme directory structure.
    // Icons are embedded directly in service binaries (same approach as the wallpaper).
    // This directory tree is provided so future VFS-based icon loading works at:
    //   /var/sunlightos/icons/SunlightOS/{category}/{size}/{name}.svg
    RamEntry::dir("/var/sunlightos/icons", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/apps", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/places", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/mimetypes", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/devices", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/actions", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/status", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/preferences", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/apps/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/apps/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/apps/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/places/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/places/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/places/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/mimetypes/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/mimetypes/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/mimetypes/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/devices/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/devices/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/devices/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/actions/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/actions/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/actions/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/status/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/status/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/status/16", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/preferences/48", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/preferences/32", 0, 0, mode::DIR_755),
    RamEntry::dir("/var/sunlightos/icons/SunlightOS/preferences/16", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/var/sunlightos/icons/SunlightOS/apps/48/sunlight-runner.svg",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/icons/SunlightOS/apps/48/system-run.svg"),
    ),
    RamEntry::file(
        "/var/sunlightos/icons/SunlightOS/apps/48/runner.svg",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/icons/SunlightOS/apps/48/system-run.svg"),
    ),
    RamEntry::file(
        "/var/sunlightos/icons/SunlightOS/apps/48/generic-app.svg",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/icons/SunlightOS/apps/48/applications-system.svg"),
    ),
    // Desktop wallpapers as SIMG v2 (sub+lz4). Default desktop.toml points at
    // wallpaper.simg so Vortex loads an image on first boot. See docs/SIMG_V2.md.
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper.simg",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper.simg"),
    ),
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper1.simg",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper1.simg"),
    ),
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper2.simg",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper2.simg"),
    ),
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper3.simg",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper3.simg"),
    ),
    RamEntry::file(
        "/var/sunlightos/wallpapers/wallpaper4.simg",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/wallpaper4.simg"),
    ),
    // System config files (world-readable)
    RamEntry::file(
        "/etc/passwd",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../etc/passwd"),
    ),
    RamEntry::file(
        "/etc/group",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../etc/group"),
    ),
    RamEntry::file(
        "/etc/shadow",
        0,
        0,
        mode::FILE_600,
        include_bytes!("../etc/shadow"),
    ),
    RamEntry::file(
        "/etc/motd",
        0,
        0,
        mode::FILE_644,
        b"Welcome to SunlightOS\n",
    ),
    RamEntry::file(
        "/root/.config/yazi/theme.toml",
        0,
        0,
        mode::FILE_644,
        b"[filetype]\nrules = [\n  { url = \"*/\", fg = \"lightblue\" },\n]\n",
    ),
    RamEntry::file(
        "/root/.config/yazi/yazi.toml",
        0,
        0,
        mode::FILE_644,
        b"# SunlightOS TTY does not implement terminal image protocols. Keep previews\n# and their background preload workers disabled for a clean, responsive UI.\n[plugin]\npreviewers = []\npreloaders = []\n",
    ),
    // Large file for shared-memory IPC read test (>48 bytes triggers shm path)
    RamEntry::file("/etc/large_test", 0, 0, mode::FILE_644, &[b'A'; 2048]),
    RamEntry::file("/etc/hostname", 0, 0, mode::FILE_644, b"sunlight\n"),
    RamEntry::file(
        "/etc/fstab",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../etc/fstab"),
    ),
    // /etc/hosts for DNS resolver (hosts first, then hardcoded fallback)
    RamEntry::file(
        "/etc/hosts",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../etc/hosts"),
    ),
    RamEntry::file(
        "/etc/resolv.conf",
        0,
        0,
        mode::FILE_644,
        b"# Generated by SunlightOS resolved.\n# Do not edit this file directly unless you know what you are doing.\nnameserver 208.67.222.222\nnameserver 208.67.220.220\n",
    ),
    // Locale foundation (Phase locale prep for Calendar)
    RamEntry::file(
        "/etc/locale.conf",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../etc/locale.conf"),
    ),
    RamEntry::file(
        "/etc/locale.gen",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../etc/locale.gen"),
    ),
    RamEntry::file(
        "/root/.config/sunlight/desktop.toml",
        0,
        0,
        mode::FILE_644,
        b"[desktop]\nwallpaper = \"/var/sunlightos/wallpapers/wallpaper.simg\"\nwallpaper_mode = \"cover\"\n",
    ),
    RamEntry::file(
        "/etc/sunlight/session.toml",
        0,
        0,
        mode::FILE_644,
        br#"
[default]
mode = "terminal"

[terminal]
shell = "/bin/sh"
initial_tabs = 1
theme = "sunlight-dark"

[multi_user]
enabled = false
max_ttys = 6
"#,
    ),
    RamEntry::file(
        "/etc/sunlight/sessions/sunlight-desktop.toml",
        0,
        0,
        mode::FILE_644,
        br#"format_version = 1
id = "org.sunlight.session.desktop"
name = "Sunlight Desktop"
kind = "desktop"

[[components]]
id = "org.sunlight.vortex-shell"
role = "shell"
required = true
enabled = true
launch_policy = "session-start"
restart_policy = "on-failure"
restart_limit = 8
restart_window_seconds = 120
readiness_timeout_seconds = 10
order = 0
"#,
    ),
    // Stable onboarding/release generation for FirstLoginAfterSystemUpgrade.
    RamEntry::file(
        "/etc/sunlight/release-generation",
        0,
        0,
        mode::FILE_644,
        b"1\n",
    ),
    RamEntry::dir("/var/lib/sunlight-sessiond", 0, 0, mode::DIR_755),
    // Session Configuration Phase 1 eligible test .sunapp fixtures.
    RamEntry::dir("/Applications/StartupOne.sunapp", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/StartupOne.sunapp/Resources", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/Applications/StartupOne.sunapp/Manifest.toml",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../StartupOne.sunapp/Manifest.toml"),
    ),
    RamEntry::file(
        "/Applications/StartupOne.sunapp/Resources/icon.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../StartupOne.sunapp/Resources/icon.tga"),
    ),
    RamEntry::dir("/Applications/StartupTwo.sunapp", 0, 0, mode::DIR_755),
    RamEntry::dir("/Applications/StartupTwo.sunapp/Resources", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/Applications/StartupTwo.sunapp/Manifest.toml",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../StartupTwo.sunapp/Manifest.toml"),
    ),
    RamEntry::file(
        "/Applications/StartupTwo.sunapp/Resources/icon.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../StartupTwo.sunapp/Resources/icon.tga"),
    ),
    RamEntry::file("/bin/su1", 0, 0, mode::FILE_755, b"#!/sunlight/su1\n"),
    RamEntry::file("/bin/su2", 0, 0, mode::FILE_755, b"#!/sunlight/su2\n"),
    // Welcome Wizard (org.sunlight.wiseowl-welcome) — ordinary startup-eligible .sunapp.
    RamEntry::dir("/Applications/WiseOwlWelcome.sunapp", 0, 0, mode::DIR_755),
    RamEntry::dir(
        "/Applications/WiseOwlWelcome.sunapp/Resources",
        0,
        0,
        mode::DIR_755,
    ),
    RamEntry::file(
        "/Applications/WiseOwlWelcome.sunapp/Manifest.toml",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../WiseOwlWelcome.sunapp/Manifest.toml"),
    ),
    RamEntry::file(
        "/Applications/WiseOwlWelcome.sunapp/Resources/icon.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../WiseOwlWelcome.sunapp/Resources/icon.tga"),
    ),
    RamEntry::dir("/Applications/WiseOwl.sunapp", 0, 0, mode::DIR_755),
    RamEntry::dir(
        "/Applications/WiseOwl.sunapp/Resources",
        0,
        0,
        mode::DIR_755,
    ),
    RamEntry::file(
        "/Applications/WiseOwl.sunapp/Manifest.toml",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../WiseOwl.sunapp/Manifest.toml"),
    ),
    RamEntry::file(
        "/Applications/WiseOwl.sunapp/Resources/icon.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../WiseOwl.sunapp/Resources/icon.tga"),
    ),
    RamEntry::file("/bin/welcome", 0, 0, mode::FILE_755, b"#!/sunlight/welcome\n"),
    RamEntry::file(
        "/usr/bin/welcome",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/welcome\n",
    ),
    RamEntry::file(
        "/etc/sunlight/ssh.toml",
        0,
        0,
        mode::FILE_644,
        br#"listen_address = "0.0.0.0"
port = 22
host_key_file = "/etc/sunlight/ssh_host_ed25519_key"
password_authentication = true
max_auth_attempts = 3
max_connections = 8
max_sessions_per_connection = 1
login_timeout_seconds = 30
"#,
    ),
    RamEntry::file(
        "/srv/http/index.html",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/solar.html"),
    ),
    RamEntry::file("/bin/sshl", 0, 0, mode::FILE_755, b"#!/sunlight/sunshell\n"),
    RamEntry::file("/bin/sh", 0, 0, mode::FILE_755, b"#!/sunlight/builtin-sh\n"),
    // POSIX-style applet stubs in /bin so PATH resolution behaves like a
    // normal Unix layout. Kernel spawn maps these paths to the embedded
    // sunlight-utils / sunlight-net-utils multi-call ELFs.
    RamEntry::file(
        "/bin/ls",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/cat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/cp",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/mv",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/rm",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/mkdir",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/rmdir",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/touch",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/find",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/xargs",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/grep",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/head",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/tail",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/wc",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/sort",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/uniq",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/tr",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/paste",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/join",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/printf",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/tee",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/nl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/od",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/split",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/cut",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/fold",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/expand",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/file",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/stat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/pwd",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/date",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/whoami",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/id",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/uname",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/echo",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/true",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/false",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/basename",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/dirname",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/cmp",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/cksum",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/nice",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/renice",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/free",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/freezram",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/kill",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/killall",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/bin/pkill",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file("/bin/top", 0, 0, mode::FILE_755, b"#!/sunlight/top\n"),
    RamEntry::file(
        "/bin/memoryctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/memoryctl\n",
    ),
    RamEntry::file(
        "/bin/sunlightctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlightctl\n",
    ),
    RamEntry::file(
        "/bin/devicectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/devicectl\n",
    ),
    RamEntry::file(
        "/bin/sunlight-hwinfo",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-hwinfo\n",
    ),
    RamEntry::file(
        "/bin/networkctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/networkctl\n",
    ),
    RamEntry::file(
        "/bin/resolvectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/resolvectl\n",
    ),
    RamEntry::file(
        "/bin/powerctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/powerctl\n",
    ),
    RamEntry::file(
        "/bin/thermalctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/thermalctl\n",
    ),
    RamEntry::file(
        "/bin/audioctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/audioctl\n",
    ),
    RamEntry::file(
        "/bin/nicectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/nicectl\n",
    ),
    RamEntry::file(
        "/bin/capabilityctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/capabilityctl\n",
    ),
    RamEntry::file("/bin/runas", 0, 0, mode::FILE_755, b"#!/sunlight/runas\n"),
    // Session lock recovery CLI (mezzo daemon is init-launched at /sbin/mezzo).
    RamEntry::file(
        "/bin/mezzoctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/mezzoctl\n",
    ),
    RamEntry::file(
        "/usr/bin/mezzoctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/mezzoctl\n",
    ),
    RamEntry::file(
        "/bin/sunlight-sessionctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-sessionctl\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-sessionctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-sessionctl\n",
    ),
    RamEntry::file("/bin/fetch", 0, 0, mode::FILE_755, b"#!/sunlight/fetch\n"),
    RamEntry::file(
        "/bin/sunlight-sunsay",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-sunsay\n",
    ),
    RamEntry::file("/bin/z", 0, 0, mode::FILE_755, b"#!/sunlight/z\n"),
    RamEntry::file("/bin/dict", 0, 0, mode::FILE_755, b"#!/sunlight/dict\n"),
    RamEntry::file(
        "/bin/hangman",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/hangman\n",
    ),
    // GUI Eyes Tracker demo client
    RamEntry::file("/bin/eyes", 0, 0, mode::FILE_755, b"#!/sunlight/eyes\n"),
    RamEntry::file("/bin/sunlight-runner", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-runner\n"),
    RamEntry::file("/bin/sun-exec", 0, 0, mode::FILE_755, b"#!/sunlight/sun-exec\n"),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Manifest.toml",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Manifest.toml"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/SUNSH.EXE",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/SUNSH.EXE"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/AUTOEXEC.BAT",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/AUTOEXEC.BAT"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/CHILD.BAT",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/CHILD.BAT"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/MIDTERM.BAT",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/MIDTERM.BAT"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/TESTS/VGALAB.COM",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/VGALAB.COM"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/TESTS/SUNPAINT.COM",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/SUNPAINT.COM"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Program/TESTS/SUNMINE.EXE",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Program/TESTS/SUNMINE.EXE"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Resources/icon.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Resources/icon.tga"),
    ),
    RamEntry::file(
        "/Applications/ChronosDosShell.sunapp/Licenses/README.txt",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosDosShell.sunapp/Licenses/README.txt"),
    ),

    RamEntry::file(
        "/Applications/SunlightMines.sunapp/Manifest.toml",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/Manifest.toml"),
    ),
    RamEntry::file(
        "/Applications/SunlightMines.sunapp/Program/SUNMINE.EXE",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/Program/SUNMINE.EXE"),
    ),
    RamEntry::file(
        "/Applications/SunlightMines.sunapp/Resources/icon.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/Resources/icon.tga"),
    ),
    RamEntry::file(
        "/Applications/SunlightMines.sunapp/Resources/icon-large.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/Resources/icon-large.tga"),
    ),
    RamEntry::file(
        "/Applications/SunlightMines.sunapp/README.md",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/README.md"),
    ),
    RamEntry::file(
        "/Applications/SunlightMines.sunapp/Licenses/README.txt",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../SunlightMines.sunapp/Licenses/README.txt"),
    ),

    RamEntry::file(
        "/Applications/ChronosFileLab.sunapp/Manifest.toml",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosFileLab.sunapp/Manifest.toml"),
    ),
    RamEntry::file(
        "/Applications/ChronosFileLab.sunapp/Program/FILELAB.COM",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosFileLab.sunapp/Program/FILELAB.COM"),
    ),
    RamEntry::file(
        "/Applications/ChronosFileLab.sunapp/Program/README.TXT",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosFileLab.sunapp/Program/README.TXT"),
    ),
    RamEntry::file(
        "/Applications/ChronosFileLab.sunapp/Program/FILELAB.ASM",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosFileLab.sunapp/Program/FILELAB.ASM"),
    ),
    RamEntry::file(
        "/Applications/ChronosFileLab.sunapp/Resources/icon.tga",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../ChronosFileLab.sunapp/Resources/icon.tga"),
    ),
    RamEntry::file("/bin/sun-open", 0, 0, mode::FILE_755, b"#!/sunlight/sun-open\n"),
    // GUI Terminal emulator
    RamEntry::file("/bin/sunlight-terminal", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-terminal\n"),
    // Chronos: native DOS compatibility application.
    RamEntry::file("/bin/sunlight-chronos", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-chronos\n"),
    // GUI Task Monitor
    RamEntry::file("/bin/sunlight-tasks", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-tasks\n"),
    // SunLight-Bench: CPU/multi-core performance benchmark
    RamEntry::file("/bin/sunbench", 0, 0, mode::FILE_755, b"#!/sunlight/sunbench\n"),
    // GUI calculator client
    RamEntry::file("/bin/calculator", 0, 0, mode::FILE_755, b"#!/sunlight/calculator\n"),
    // Developer widget gallery (not Control Panel / Lock Screen)
    RamEntry::file(
        "/bin/widget-gallery",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/widget-gallery\n",
    ),
    // GUI file manager client
    RamEntry::file(
        "/bin/sunlight-files",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-files\n",
    ),
    RamEntry::file(
        "/bin/light-lens",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/light-lens\n",
    ),
    // Melody Mina: native responsive music-player frontend.
    RamEntry::file(
        "/bin/melody-mina",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/melody-mina\n",
    ),
    RamEntry::file(
        "/bin/sunlight-edit",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-edit\n",
    ),
    RamEntry::file(
        "/bin/sunlight-text",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-text\n",
    ),
    RamEntry::file(
        "/bin/sunlight-writer",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-writer\n",
    ),
    // Silicon Echoes: 1993: native graphical narrative game.
    RamEntry::file(
        "/bin/silicon-echoes",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/silicon-echoes\n",
    ),
    // Sunlight Calendar: graphical calendar client
    RamEntry::file(
        "/bin/sunlight-calendar",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-calendar\n",
    ),
    // Sunlight Reminders: personal tasks and reminders client
    RamEntry::file(
        "/bin/sunlight-reminders",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-reminders\n",
    ),
    RamEntry::file(
        "/bin/sunlight-devices",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-devices\n",
    ),
    RamEntry::file(
        "/bin/rappid-rabbit",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/rappid-rabbit\n",
    ),
    RamEntry::file(
        "/bin/sunlight-api-lab",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-api-lab\n",
    ),
    // System Preferences (Control Panel)
    RamEntry::file("/bin/control-panel", 0, 0, mode::FILE_755, b"#!/sunlight/control-panel\n"),
    RamEntry::file(
        "/bin/cpufeat",
        0,
        0,
        mode::FILE_755,
        b"#!/cpu-utils/cpufeat\n",
    ),
    RamEntry::file(
        "/bin/hello-linux",
        0,
        0,
        mode::FILE_755,
        b"#!/helios/hello-linux\n",
    ),
    RamEntry::file(
        "/bin/note",
        0,
        0,
        mode::FILE_755,
        b"#!/helios/note\n",
    ),
    RamEntry::file(
        "/bin/sunlight-kvctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-kvctl\n",
    ),
    // Embedded account service and its read-only native IPC probe entry point.
    RamEntry::file(
        "/sbin/sunlight-account-broker",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-account-broker\n",
    ),
    RamEntry::file(
        "/bin/certificatectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/certificatectl\n",
    ),
    RamEntry::file(
        "/bin/sunlight-clip",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-clip\n",
    ),
    RamEntry::file(
        "/bin/sunlight-clipman",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-clipman\n",
    ),
    RamEntry::file(
        "/bin/emoji-picker",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/emoji-picker\n",
    ),
    // sunlight-sm binary stub (real ELF embedded in kernel for spawn; entry for FS visibility/stat)
    RamEntry::file(
        "/sbin/sunlight-sm",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-sm\n",
    ),
    RamEntry::file(
        "/bin/ping",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/ifconfig",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/wget",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/curl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/dig",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/nslookup",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/hostname",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/netstat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/ss",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/traceroute",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/arp",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/bin/dhclient",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file("/bin/mktemp", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    RamEntry::file("/sunlight-utils/mktemp", 0, 0, mode::FILE_755, b"#!/sunlight/sunlight-utils\n"),
    // Backward-compat applet paths kept for existing docs/scripts.
    RamEntry::dir("/sunlight-utils", 0, 0, mode::DIR_755),
    RamEntry::dir("/sunlight-net-utils", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/sunlight-utils/ls",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/cat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/cp",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/mv",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/rm",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/mkdir",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/rmdir",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/touch",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/find",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/grep",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/head",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/tail",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/wc",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/sort",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/uniq",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/cut",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/fold",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/expand",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/file",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/stat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/pwd",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/true",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/false",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/basename",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/dirname",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/date",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/whoami",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/id",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/uname",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/nice",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/renice",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/free",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-utils/freezram",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/ping",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/ifconfig",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/wget",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/curl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/dig",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/nslookup",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/hostname",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    RamEntry::file(
        "/sunlight-net-utils/netstat",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-net-utils\n",
    ),
    // timezone refactor support files (Phase TZ)
    RamEntry::dir("/etc/zoneinfo", 0, 0, mode::DIR_755),
    RamEntry::dir("/usr/share/zoneinfo", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc/sunlight/services", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc/sunlight/sessions", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/etc/localtime",
        0,
        0,
        mode::FILE_644,
        b"{\
\"id\":\"UTC\",\
\"display_name\":\"Coordinated Universal Time\",\
\"utc_offset_hours\":0,\
\"utc_offset_minutes\":0,\
\"dst_offset_minutes\":0,\
\"dst_start_month\":0,\
\"dst_end_month\":0\
}",
    ),
    RamEntry::file(
        "/etc/zoneinfo/Asia/Tehran.txt",
        0,
        0,
        mode::FILE_644,
        b"id=Asia/Tehran\ndisplay_name=Iran Standard Time\n\
utc_offset=+03:30\ndst=none (abolished 2022)\n",
    ),
    RamEntry::file(
        "/etc/zoneinfo/UTC.txt",
        0,
        0,
        mode::FILE_644,
        b"id=UTC\ndisplay_name=Coordinated Universal Time\n\
utc_offset=+00:00\ndst=none\n",
    ),
    RamEntry::file(
        "/usr/share/zoneinfo/zones.csv",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/Timezones.csv"),
    ),
    RamEntry::file(
        "/etc/sunlight/services/timezone.service",
        0,
        0,
        mode::FILE_644,
        b"[Unit]\nDescription=SunlightOS Timezone Service\n\
After=vfs.service\nRequires=vfs.service\n\n\
[Service]\nType=simple\nExecStart=/usr/sbin/timezone_service\n\
Restart=on-failure\nRestartSec=3\nUser=root\n\
StandardOutput=journal\nStandardError=journal\n\n\
[Install]\nWantedBy=sunlight.target\n",
    ),
    // tzctl client (standalone binary path + shell builtin alias)
    RamEntry::file(
        "/usr/bin/tzctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/tzctl\n",
    ),
    RamEntry::file(
        "/usr/bin/tzutils",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/tzutils\n",
    ),
    RamEntry::file(
        "/etc/sunlight/ntp.conf",
        0,
        0,
        mode::FILE_644,
        b"# Optional explicit NTP servers (one per line).\n\
# When present, these override regional pool selection.\n\
# server 0.pool.ntp.org\n",
    ),
    RamEntry::file("/usr/bin/top", 0, 0, mode::FILE_755, b"#!/sunlight/top\n"),
    RamEntry::file(
        "/usr/bin/memoryctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/memoryctl\n",
    ),
    RamEntry::file(
        "/usr/bin/devicectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/devicectl\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-hwinfo",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-hwinfo\n",
    ),
    RamEntry::file(
        "/usr/bin/networkctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/networkctl\n",
    ),
    RamEntry::file(
        "/usr/bin/resolvectl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/resolvectl\n",
    ),
    RamEntry::file(
        "/usr/bin/powerctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/powerctl\n",
    ),
    RamEntry::file(
        "/usr/bin/thermalctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/thermalctl\n",
    ),
    RamEntry::file(
        "/usr/bin/audioctl",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/audioctl\n",
    ),
    RamEntry::file(
        "/usr/bin/fetch",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/fetch\n",
    ),
    RamEntry::file(
        "/usr/bin/hangman",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/hangman\n",
    ),
    // GUI Eyes Tracker demo client (usr path)
    RamEntry::file("/usr/bin/eyes", 0, 0, mode::FILE_755, b"#!/sunlight/eyes\n"),
    RamEntry::file(
        "/usr/bin/sunlight-runner",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-runner\n",
    ),
    RamEntry::file(
        "/usr/bin/sun-exec",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sun-exec\n",
    ),
    RamEntry::file(
        "/usr/bin/sun-open",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sun-open\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-terminal",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-terminal\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-chronos",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-chronos\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-tasks",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-tasks\n",
    ),
    RamEntry::file(
        "/usr/bin/sunbench",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunbench\n",
    ),
    RamEntry::file(
        "/usr/bin/calculator",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/calculator\n",
    ),
    RamEntry::file(
        "/usr/bin/widget-gallery",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/widget-gallery\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-files",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-files\n",
    ),
    RamEntry::file(
        "/usr/bin/light-lens",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/light-lens\n",
    ),
    RamEntry::file(
        "/usr/bin/melody-mina",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/melody-mina\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-edit",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-edit\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-text",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-text\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-writer",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-writer\n",
    ),
    // Silicon Echoes: 1993: native graphical narrative game.
    RamEntry::file(
        "/usr/bin/silicon-echoes",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/silicon-echoes\n",
    ),
    // Sunlight Calendar: graphical calendar client
    RamEntry::file(
        "/usr/bin/sunlight-calendar",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-calendar\n",
    ),
    // Sunlight Reminders: personal tasks and reminders client
    RamEntry::file(
        "/usr/bin/sunlight-reminders",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-reminders\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-devices",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-devices\n",
    ),
    RamEntry::file(
        "/usr/bin/rappid-rabbit",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/rappid-rabbit\n",
    ),
    RamEntry::file(
        "/usr/bin/sunlight-api-lab",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-api-lab\n",
    ),
    // System Preferences (Control Panel)
    RamEntry::file(
        "/usr/bin/control-panel",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/control-panel\n",
    ),
    RamEntry::file(
        "/usr/bin/cpufeat",
        0,
        0,
        mode::FILE_755,
        b"#!/cpu-utils/cpufeat\n",
    ),
    RamEntry::file(
        "/usr/bin/hello-linux",
        0,
        0,
        mode::FILE_755,
        b"#!/helios/hello-linux\n",
    ),
    RamEntry::file(
        "/usr/bin/note",
        0,
        0,
        mode::FILE_755,
        b"#!/helios/note\n",
    ),
    // sunlight-thumbd: asynchronous thumbnail daemon.
    // Keep the canonical /bin paths used by the launcher and service units in
    // sync with the /usr/bin compatibility aliases.
    RamEntry::file("/bin/sunlight-docserv", 0, 0, mode::FILE_755, b"#!/helios/sunlight-docserv\n"),
    RamEntry::file("/usr/bin/sunlight-docserv", 0, 0, mode::FILE_755, b"#!/helios/sunlight-docserv\n"),
    RamEntry::file("/sbin/sunlight-docserv", 0, 0, mode::FILE_755, b"#!/helios/sunlight-docserv\n"),
    RamEntry::file("/bin/gs", 0, 0, mode::FILE_755, b"#!/helios/gs\n"),
    RamEntry::file("/usr/bin/gs", 0, 0, mode::FILE_755, b"#!/helios/gs\n"),
    RamEntry::file("/bin/sunlight-docview", 0, 0, mode::FILE_755, b"#!/helios/sunlight-docview\n"),
    RamEntry::file("/usr/bin/sunlight-docview", 0, 0, mode::FILE_755, b"#!/helios/sunlight-docview\n"),
    RamEntry::file(
        "/usr/bin/sunlight-thumbd",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-thumbd\n",
    ),
    RamEntry::file(
        "/sbin/sunlight-thumbd",
        0,
        0,
        mode::FILE_755,
        b"#!/sunlight/sunlight-thumbd\n",
    ),
    // ── Sample pictures ────────────────────────────────────────────────────
    // Installed under /usr/share/sunlightos/sample-pictures/ (system-wide).
    // Also visible to users under ~/Pictures/Sample Pictures/ (real directory,
    // same file data — no extra memory cost since data pointers are shared).
    RamEntry::dir("/usr/share", 0, 0, mode::DIR_755),
    RamEntry::dir("/usr/share/sunlightos", 0, 0, mode::DIR_755),
    // Login background (SIMG v2). tty_server / lock presenter also embed this
    // at compile time; the VFS path is for other consumers.
    RamEntry::dir("/usr/share/sunlightos/backgrounds", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/usr/share/sunlightos/backgrounds/login-background.simg",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../docs/images/sunlight-login-background.simg"),
    ),
    // MiniType fonts (standalone .mtf) for dynamic font loader / future OS image use.
    // Generated via assets/fonts/minitype/generate.sh (and sun-font/build.rs).
    // See docs/MINITYPE_FONTS.md
    RamEntry::dir("/usr/share/sunlightos/fonts", 0, 0, mode::DIR_755),
    RamEntry::dir("/usr/share/sunlightos/fonts/minitype", 0, 0, mode::DIR_755),

    // Material Symbols font (for shell panel icons, future dynamic loaders).
    // Bundled from local system font; see assets/fonts/material-symbols/
    RamEntry::dir("/usr/share/fonts", 0, 0, mode::DIR_755),
    RamEntry::dir("/usr/share/fonts/material-symbols", 0, 0, mode::DIR_755),
    // Font file is present in source (assets/fonts/material-symbols/) and would be
    // installed to runtime image in a full build. For kernel ramfs size we include
    // a marker (real .ttf can be added when dynamic font loading lands).
    RamEntry::file(
        "/usr/share/fonts/material-symbols/MaterialSymbolsOutlined.ttf",
        0,
        0,
        mode::FILE_644,
        b"MaterialSymbolsOutlined (bundled; see assets/fonts/material-symbols/)\0",
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_11.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_11.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_13.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_13.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_16.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_16.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_medium_13.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_medium_13.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_semibold_13.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_semibold_13.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_ui_title_18.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_ui_title_18.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_mono_regular_14.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_mono_regular_14.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_mono_medium_14.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_mono_medium_14.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_serif_regular_16.mtf",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_serif_regular_16.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_emoji_16.mtf",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_emoji_16.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/sunlight_emoji_manifest.txt",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/sunlight_emoji_manifest.txt"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/material_icons_16.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/material_icons_16.mtf"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/fonts/minitype/material_icons_24.mtf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/fonts/minitype/material_icons_24.mtf"),
    ),
    RamEntry::dir("/usr/share/licenses", 0, 0, mode::DIR_755),
    RamEntry::dir("/usr/share/licenses/openmoji", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/usr/share/licenses/openmoji/LICENSE.txt",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../licenses/openmoji/LICENSE.txt"),
    ),
    RamEntry::file(
        "/usr/share/licenses/openmoji/ATTRIBUTION.txt",
        0,
        0,
        mode::FILE_644,
        include_bytes!("../../licenses/openmoji/ATTRIBUTION.txt"),
    ),
    RamEntry::dir("/usr/share/sunlightos/sample-pictures", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/01_solar_blossom.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/01_solar_blossom.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/02_amber_dunes.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/02_amber_dunes.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/03_blue_garden.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/03_blue_garden.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/04_lantern_jellyfish.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/04_lantern_jellyfish.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/05_sleepy_koala.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/05_sleepy_koala.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/06_sunrise_lighthouse.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/06_sunrise_lighthouse.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/07_penguin_walk.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/07_penguin_walk.simg"),
    ),
    RamEntry::file(
        "/usr/share/sunlightos/sample-pictures/08_orange_tulips.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/08_orange_tulips.simg"),
    ),
    // ~/Pictures/Sample Pictures/ for root (same file data, separate dir entry).
    RamEntry::dir("/root/Pictures/Sample Pictures", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/01_solar_blossom.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/01_solar_blossom.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/02_amber_dunes.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/02_amber_dunes.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/03_blue_garden.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/03_blue_garden.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/04_lantern_jellyfish.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/04_lantern_jellyfish.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/05_sleepy_koala.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/05_sleepy_koala.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/06_sunrise_lighthouse.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/06_sunrise_lighthouse.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/07_penguin_walk.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/07_penguin_walk.simg"),
    ),
    RamEntry::file(
        "/root/Pictures/Sample Pictures/08_orange_tulips.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/08_orange_tulips.simg"),
    ),
    // ~/Pictures/Sample Pictures/ for the default unprivileged user.
    RamEntry::dir("/home/user/Pictures/Sample Pictures", 1000, 1000, mode::DIR_755),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/01_solar_blossom.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/01_solar_blossom.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/02_amber_dunes.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/02_amber_dunes.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/03_blue_garden.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/03_blue_garden.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/04_lantern_jellyfish.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/04_lantern_jellyfish.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/05_sleepy_koala.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/05_sleepy_koala.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/06_sunrise_lighthouse.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/06_sunrise_lighthouse.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/07_penguin_walk.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/07_penguin_walk.simg"),
    ),
    RamEntry::file(
        "/home/user/Pictures/Sample Pictures/08_orange_tulips.simg",
        0, 0, mode::FILE_644,
        include_bytes!("../../docs/images/Samples/08_orange_tulips.simg"),
    ),

    // ── Welcome document ──────────────────────────────────────────────────
    // Text asset copied into the Documents folder for the default accounts.
    // /home/user/ serves as the home template; later the installer will copy
    // from equivalent template locations when provisioning real users.
    // A system copy is provided under /usr/share for reference/reuse.
    RamEntry::dir("/usr/share/sunlightos/documents", 0, 0, mode::DIR_755),
    RamEntry::file(
        "/usr/share/sunlightos/documents/Welcome to SunlightOS.txt",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/documents/Welcome to SunlightOS.txt"),
    ),
    // Root's Documents (seeded read-only by the OS).
    RamEntry::file(
        "/root/Documents/Welcome to SunlightOS.txt",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/documents/Welcome to SunlightOS.txt"),
    ),
    // Default user's Documents (template home, uid/gid 1000).
    RamEntry::file(
        "/home/user/Documents/Welcome to SunlightOS.txt",
        1000, 1000, mode::FILE_644,
        include_bytes!("../../assets/documents/Welcome to SunlightOS.txt"),
    ),

    // PDF reader test document, available in both default Documents folders.
    RamEntry::file(
        "/root/Documents/SunlightOS_PDF_Reader_Test.pdf",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/documents/SunlightOS_PDF_Reader_Test.pdf"),
    ),
    RamEntry::file(
        "/home/user/Documents/SunlightOS_PDF_Reader_Test.pdf",
        1000, 1000, mode::FILE_644,
        include_bytes!("../../assets/documents/SunlightOS_PDF_Reader_Test.pdf"),
    ),

    // ── Why SunlightOS Exists document ──────────────────────────────────────
    // Identity and philosophy document seeded alongside Welcome.
    // Same deployment pattern: system reference + both default home accounts.
    RamEntry::file(
        "/usr/share/sunlightos/documents/Why SunlightOS Exists.txt",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/documents/Why SunlightOS Exists.txt"),
    ),
    RamEntry::file(
        "/root/Documents/Why SunlightOS Exists.txt",
        0, 0, mode::FILE_644,
        include_bytes!("../../assets/documents/Why SunlightOS Exists.txt"),
    ),
    RamEntry::file(
        "/home/user/Documents/Why SunlightOS Exists.txt",
        1000, 1000, mode::FILE_644,
        include_bytes!("../../assets/documents/Why SunlightOS Exists.txt"),
    ),
];
