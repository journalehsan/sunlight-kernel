use super::*;
use crate::ramfs::RamFs;
use crate::ramfs::*;
use alloc::borrow::ToOwned;

static TEST_ENTRIES: &[RamEntry] = &[
    RamEntry::file(
        "/etc/motd",
        0,
        0,
        mode::FILE_644,
        b"Welcome to SunlightOS\n",
    ),
    RamEntry::file("/bin/sh", 0, 0, mode::FILE_755, b"shell"),
];

#[test]
fn chmod_rejects_immutable_static_entry() {
    let mut fs = RamFs::new(TEST_ENTRIES);
    assert_eq!(
        fs.chmod("/etc/motd", mode::FILE_600),
        Err(FsError::ReadOnlyFilesystem)
    );
    assert_eq!(fs.stat("/etc/motd").unwrap().mode, mode::FILE_644);
}

#[test]
fn open_and_read_whole_file() {
    let mut fs = RamFs::new(TEST_ENTRIES);
    let handle = fs.open("/etc/motd").unwrap();
    let mut buf = [0u8; 32];

    let read = fs.read(handle, 0, &mut buf).unwrap();

    assert_eq!(read, b"Welcome to SunlightOS\n".len());
    assert_eq!(&buf[..read], b"Welcome to SunlightOS\n");
}

#[test]
fn read_respects_offset_and_buffer_size() {
    let mut fs = RamFs::new(TEST_ENTRIES);
    let handle = fs.open("/etc/motd").unwrap();
    let mut buf = [0u8; 8];

    let read = fs.read(handle, 11, &mut buf).unwrap();

    assert_eq!(read, 8);
    assert_eq!(&buf, b"Sunlight");
}

#[test]
fn read_past_end_returns_zero() {
    let mut fs = RamFs::new(TEST_ENTRIES);
    let handle = fs.open("/bin/sh").unwrap();
    let mut buf = [0u8; 4];

    assert_eq!(fs.read(handle, 99, &mut buf), Ok(0));
}

#[test]
fn stat_reports_file_size_and_permissions() {
    let mut fs = RamFs::new(TEST_ENTRIES);

    assert_eq!(
        fs.stat("/bin/sh"),
        Ok(FileStat {
            file_type: FileType::File,
            size: 5,
            uid: 0,
            gid: 0,
            mode: mode::FILE_755,
            nlinks: 1,
        })
    );
}

#[test]
fn missing_file_returns_not_found() {
    let mut fs = RamFs::new(TEST_ENTRIES);

    assert_eq!(fs.open("/missing"), Err(FsError::NotFound));
}

#[test]
fn invalid_path_returns_invalid_path() {
    let mut fs = RamFs::new(TEST_ENTRIES);

    assert_eq!(fs.open("etc/motd"), Err(FsError::InvalidPath));
}

#[test]
fn close_rejects_stale_handle() {
    let mut fs = RamFs::new(TEST_ENTRIES);
    let handle = fs.open("/bin/sh").unwrap();

    assert_eq!(fs.close(handle), Ok(()));
    assert_eq!(fs.close(handle), Err(FsError::BadHandle));
}

#[test]
fn reused_slot_rejects_the_previous_handle_incarnation() {
    let mut fs = RamFs::new(TEST_ENTRIES);
    let first = fs.open("/bin/sh").unwrap();
    fs.close(first).unwrap();
    let second = fs.open("/etc/motd").unwrap();

    assert_ne!(first, second);
    assert_eq!(fs.read(first, 0, &mut [0; 1]), Err(FsError::BadHandle));
    assert_eq!(fs.close(first), Err(FsError::BadHandle));
    assert!(fs.read(second, 0, &mut [0; 1]).is_ok());
}

#[test]
fn truncate_keeps_the_handle_valid_and_discards_old_contents() {
    let mut fs = RamFs::new(TEST_ENTRIES);
    let handle = fs.open("/bin/sh").unwrap();
    fs.truncate(handle).unwrap();

    let mut one = [0u8; 1];
    assert_eq!(fs.read(handle, 0, &mut one), Ok(0));
    assert_eq!(fs.write(handle, 0, b"x"), Ok(1));
    assert_eq!(fs.read(handle, 0, &mut one), Ok(1));
    assert_eq!(one, [b'x']);
}

#[test]
fn too_many_open_files_is_reported() {
    let mut fs = RamFs::new(TEST_ENTRIES);
    for _ in 0..RAMFS_MAX_HANDLES {
        fs.open("/bin/sh").unwrap();
    }

    assert_eq!(fs.open("/bin/sh"), Err(FsError::TooManyOpenFiles));
}

static DIR_ENTRIES: &[RamEntry] = &[
    RamEntry::dir("/", 0, 0, mode::DIR_755),
    RamEntry::dir("/etc", 0, 0, mode::DIR_755),
    RamEntry::file("/etc/motd", 0, 0, mode::FILE_644, b"hello\n"),
];

#[test]
fn open_dir_succeeds_and_read_returns_isdir() {
    let mut fs = RamFs::new(DIR_ENTRIES);
    let handle = fs.open("/etc").unwrap();
    let stat = fs.fstat_handle(handle).unwrap();
    assert_eq!(stat.file_type, FileType::Directory);
    let mut buf = [0u8; 8];
    assert_eq!(fs.read(handle, 0, &mut buf), Err(FsError::IsDir));
    let mut path = [0u8; 16];
    let n = fs.handle_path(handle, &mut path).unwrap();
    assert_eq!(&path[..n], b"/etc");
}

#[test]
fn mkdir_existing_returns_already_exists() {
    let mut fs = RamFs::new(DIR_ENTRIES);
    assert_eq!(fs.mkdir("/etc", 0, 0, 0o755), Err(FsError::AlreadyExists));
}

#[test]
fn stat_dir_returns_directory_type() {
    let mut fs = RamFs::new(DIR_ENTRIES);
    let stat = fs.stat("/etc").unwrap();
    assert_eq!(stat.file_type, FileType::Directory);
    assert_eq!(stat.mode, mode::DIR_755);
    assert_eq!(stat.nlinks, 2);
}

#[test]
fn write_extends_file() {
    let mut fs = RamFs::new(TEST_ENTRIES);
    let handle = fs.open("/bin/sh").unwrap();
    assert_eq!(fs.write(handle, 0, b"newdata"), Ok(7));
    let mut buf = [0u8; 16];
    let n = fs.read(handle, 0, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"newdata");
}

#[test]
fn streamed_large_write_grows_in_place_without_losing_data() {
    let mut fs = RamFs::new(&[]);
    let handle = fs.create_file("/large.simg", 0, 0, mode::FILE_644).unwrap();
    let chunk = [0x5au8; 4096];
    let chunk_count = 512;
    fs.reserve(handle, chunk.len() * chunk_count).unwrap();
    assert_eq!(fs.stat("/large.simg").unwrap().size, 0);

    for index in 0..chunk_count {
        assert_eq!(
            fs.write(handle, index * chunk.len(), &chunk),
            Ok(chunk.len())
        );
    }

    assert_eq!(
        fs.stat("/large.simg").unwrap().size,
        chunk.len() * chunk_count
    );
    let mut data = alloc::vec![0; chunk.len() * chunk_count];
    assert_eq!(fs.read(handle, 0, &mut data), Ok(data.len()));
    assert!(data.iter().all(|byte| *byte == 0x5a));
}

#[test]
fn write_overwrite_preserves_existing_suffix_and_zero_fills_holes() {
    let mut fs = RamFs::new(&[]);
    let handle = fs.create_file("/data", 0, 0, mode::FILE_644).unwrap();
    fs.write(handle, 0, b"abcdefgh").unwrap();
    fs.write(handle, 2, b"XY").unwrap();
    fs.write(handle, 10, b"Z").unwrap();

    let mut data = [0; 11];
    assert_eq!(fs.read(handle, 0, &mut data), Ok(11));
    assert_eq!(&data, b"abXYefgh\0\0Z");
}

#[test]
fn mkdir_creates_directory() {
    let mut fs = RamFs::new(TEST_ENTRIES);
    assert_eq!(fs.mkdir("/newdir", 0, 0, 0o755), Ok(()));
    let stat = fs.stat("/newdir").unwrap();
    assert_eq!(stat.file_type, FileType::Directory);
    assert_eq!(stat.mode, mode::S_IFDIR | 0o755);
}

#[test]
fn create_file_creates_and_opens_file() {
    let mut fs = RamFs::new(DIR_ENTRIES);
    let handle = fs.create_file("/etc/new", 1000, 1000, 0o644).unwrap();
    assert_eq!(fs.write(handle, 0, b"ok"), Ok(2));
    let stat = fs.stat("/etc/new").unwrap();
    assert_eq!(stat.file_type, FileType::File);
    assert_eq!(stat.uid, 1000);
    assert_eq!(stat.mode, mode::S_IFREG | 0o644);
}

#[test]
fn rename_dynamic_file_replaces_static_file_contents() {
    let mut fs = RamFs::new(DIR_ENTRIES);
    let handle = fs
        .create_file("/etc/motd.tmp", 0, 0, mode::FILE_644)
        .unwrap();
    fs.write(handle, 0, b"updated\n").unwrap();
    fs.close(handle).unwrap();

    assert_eq!(fs.rename("/etc/motd.tmp", "/etc/motd"), Ok(()));
    assert_eq!(fs.open("/etc/motd.tmp"), Err(FsError::NotFound));

    let handle = fs.open("/etc/motd").unwrap();
    let mut buf = [0u8; 16];
    let read = fs.read(handle, 0, &mut buf).unwrap();
    assert_eq!(&buf[..read], b"updated\n");
}

#[test]
fn folder_rename_moves_descendants_and_keeps_open_handles() {
    let mut fs = RamFs::new(&[]);
    fs.mkdir("/source", 0, 0, 0o755).unwrap();
    fs.mkdir("/source/sub", 0, 0, 0o755).unwrap();
    fs.mkdir("/destination", 0, 0, 0o755).unwrap();
    let handle = fs.create_file("/source/sub/file", 0, 0, 0o644).unwrap();
    fs.write(handle, 0, b"retained").unwrap();
    fs.rename("/source", "/destination/moved").unwrap();
    assert_eq!(fs.open("/source/sub/file"), Err(FsError::NotFound));
    assert_eq!(fs.stat("/destination/moved/sub/file").unwrap().size, 8);
    let mut bytes = [0; 8];
    assert_eq!(fs.read(handle, 0, &mut bytes).unwrap(), 8);
    assert_eq!(&bytes, b"retained");
    assert_eq!(
        fs.rename("/destination", "/destination/moved/loop"),
        Err(FsError::Unsupported)
    );
    assert!(fs.stat("/destination/moved/sub/file").is_ok());
}

#[test]
fn exclusive_create_never_reopens_or_truncates_existing_file() {
    let mut fs = RamFs::new(DIR_ENTRIES);
    let original = fs
        .create_file_exclusive("/etc/secret", 0, 0, mode::FILE_600)
        .unwrap();
    fs.write(original, 0, b"retained").unwrap();
    fs.close(original).unwrap();

    assert_eq!(
        fs.create_file_exclusive("/etc/secret", 0, 0, mode::FILE_600),
        Err(FsError::AlreadyExists)
    );
    let handle = fs.open("/etc/secret").unwrap();
    let mut bytes = [0u8; 16];
    let len = fs.read(handle, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..len], b"retained");
}

#[test]
fn read_dir_lists_direct_children_only() {
    let mut fs = RamFs::new(DIR_ENTRIES);

    let mut root_names = Vec::new();
    fs.read_dir("/", &mut |entry| {
        root_names.push((entry.name().to_owned(), entry.file_type));
        true
    })
    .unwrap();
    // "/etc/motd" is not a direct child of "/".
    assert_eq!(
        root_names,
        std::vec![("etc".to_owned(), FileType::Directory)]
    );

    let mut etc_names = Vec::new();
    fs.read_dir("/etc", &mut |entry| {
        etc_names.push((entry.name().to_owned(), entry.size));
        true
    })
    .unwrap();
    assert_eq!(etc_names, std::vec![("motd".to_owned(), 6)]);
}

#[test]
fn read_dir_includes_dynamic_entries_and_rejects_files() {
    let mut fs = RamFs::new(DIR_ENTRIES);
    fs.mkdir("/etc/sunlight", 0, 0, 0o755).unwrap();

    let mut names = Vec::new();
    fs.read_dir("/etc", &mut |entry| {
        names.push(entry.name().to_owned());
        true
    })
    .unwrap();
    assert_eq!(names, std::vec!["motd".to_owned(), "sunlight".to_owned()]);

    assert_eq!(
        fs.read_dir("/etc/motd", &mut |_| true),
        Err(FsError::NotDir)
    );
    assert_eq!(
        fs.read_dir("/missing", &mut |_| true),
        Err(FsError::NotFound)
    );
}

#[test]
fn welcome_document_is_present_in_default_homes() {
    // Verify the welcome document asset is wired into root and the
    // default user template under Documents, and that Documents dirs
    // exist (they are declared earlier in INITRAMFS).
    let has_root_welcome = INITRAMFS
        .iter()
        .any(|e| e.path == "/root/Documents/Welcome to SunlightOS.txt" && !e.is_dir);
    let has_user_welcome = INITRAMFS
        .iter()
        .any(|e| e.path == "/home/user/Documents/Welcome to SunlightOS.txt" && !e.is_dir);
    let has_documents_root = INITRAMFS
        .iter()
        .any(|e| e.path == "/root/Documents" && e.is_dir);
    let has_documents_user = INITRAMFS
        .iter()
        .any(|e| e.path == "/home/user/Documents" && e.is_dir);

    assert!(has_root_welcome, "missing /root/Documents welcome file");
    assert!(
        has_user_welcome,
        "missing /home/user/Documents welcome file"
    );
    assert!(has_documents_root, "missing /root/Documents dir entry");
    assert!(has_documents_user, "missing /home/user/Documents dir entry");

    // Also spot-check content by opening via a fresh RamFs.
    let mut fs = RamFs::new(INITRAMFS);
    let h = fs
        .open("/home/user/Documents/Welcome to SunlightOS.txt")
        .unwrap();
    let mut buf = [0u8; 80];
    let n = fs.read(h, 0, &mut buf).unwrap();
    // Should start with the ASCII title box.
    assert!(n > 0);
    assert_eq!(&buf[..10], b"+---------");
    // Verify text from the header/intro is present (full content is the
    // asset bytes; we just sanity-check a few early distinctive words).
    let text = core::str::from_utf8(&buf[..n]).unwrap_or("");
    assert!(text.contains("Welcome to SunlightOS") || text.contains("A bright"));
}

#[test]
fn document_tools_have_launcher_paths_in_initramfs() {
    for path in [
        "/bin/sunlight-docserv",
        "/usr/bin/sunlight-docserv",
        "/sbin/sunlight-docserv",
        "/bin/sunlight-docview",
        "/usr/bin/sunlight-docview",
        "/bin/gs",
        "/usr/bin/gs",
    ] {
        let entry = INITRAMFS
            .iter()
            .find(|entry| entry.path == path)
            .unwrap_or_else(|| panic!("missing document tool path: {path}"));
        assert!(!entry.is_dir, "document tool path is a directory: {path}");
        assert_ne!(
            entry.mode & 0o111,
            0,
            "document tool is not executable: {path}"
        );
    }
}

#[test]
fn melody_mina_wav_fixture_is_present_in_system_media() {
    let entry = INITRAMFS
        .iter()
        .find(|entry| entry.path == "/usr/share/sunlightos/media/melody-mina-sample.wav")
        .expect("missing built-in Melody Mina WAV");
    assert!(!entry.is_dir);
    assert_eq!(&entry.data[..4], b"RIFF");
    assert_eq!(&entry.data[8..12], b"WAVE");
}

#[test]
fn melody_mina_user_music_track_is_present() {
    for path in [
        "/home/user/Music/Catch the Sunlight.ogg",
        "/root/Music/Catch the Sunlight.ogg",
    ] {
        let entry = INITRAMFS
            .iter()
            .find(|entry| entry.path == path)
            .expect("missing seeded user Music track");
        assert!(!entry.is_dir);
        assert_eq!(&entry.data[..4], b"OggS");
    }
    for name in [
        "002-Zamfir-EinsamerHirte.mp3",
        "014-SecretGarden-SongFromASecretGarden.mp3",
        "081-Mantovani-Greensleeves.mp3",
        "095-ClaudeCiari-ElBimbo.mp3",
        "love story (320).mp3",
    ] {
        for home in ["/home/user", "/root"] {
            let path = alloc::format!("{home}/Music/{name}");
            let entry = INITRAMFS
                .iter()
                .find(|entry| entry.path == path)
                .expect("missing seeded Melody Mina MP3");
            assert!(!entry.is_dir);
            assert_eq!(&entry.data[..3], b"ID3");
        }
    }
}

#[test]
fn living_sunlight_is_available_in_default_music_libraries() {
    for path in [
        "/home/user/Music/The Living Sunlight.wav",
        "/root/Music/The Living Sunlight.wav",
    ] {
        let entry = INITRAMFS.iter().find(|entry| entry.path == path).unwrap();
        assert_eq!(&entry.data[..4], b"RIFF");
        assert_eq!(&entry.data[8..12], b"WAVE");
        assert!(entry.data.len() <= 4 * 1024 * 1024);
    }
}

#[test]
fn chronos_command_stubs_are_present_in_initramfs() {
    for path in ["/bin/sunlight-chronos", "/usr/bin/sunlight-chronos"] {
        let entry = INITRAMFS
            .iter()
            .find(|entry| entry.path == path)
            .unwrap_or_else(|| panic!("missing {path}"));
        assert!(!entry.is_dir);
        assert_eq!(entry.mode, mode::FILE_755);
        assert_eq!(entry.data, b"#!/sunlight/sunlight-chronos\n");
    }
}

#[test]
fn melody_mina_command_stubs_are_present_in_initramfs() {
    for path in ["/bin/melody-mina", "/usr/bin/melody-mina"] {
        let entry = INITRAMFS
            .iter()
            .find(|entry| entry.path == path)
            .unwrap_or_else(|| panic!("missing {path}"));
        assert!(!entry.is_dir);
        assert_eq!(entry.mode, mode::FILE_755);
        assert_eq!(entry.data, b"#!/sunlight/melody-mina\n");
    }
}

#[test]
fn chronos_dos_shell_bundle_is_present_in_initramfs() {
    for path in [
        "/Applications/ChronosDosShell.sunapp/Manifest.toml",
        "/Applications/ChronosDosShell.sunapp/Program/SUNSH.EXE",
        "/Applications/ChronosDosShell.sunapp/Program/TESTS/VGALAB.COM",
        "/Applications/ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM",
        "/Applications/ChronosDosShell.sunapp/Program/TESTS/SUNPAINT.COM",
        "/Applications/ChronosDosShell.sunapp/Resources/icon.tga",
    ] {
        assert!(
            INITRAMFS
                .iter()
                .any(|entry| entry.path == path && !entry.is_dir),
            "missing {path}"
        );
    }
}

#[test]
fn why_sunlightos_exists_document_is_present_in_default_homes() {
    // Verify the "Why SunlightOS Exists" identity document is seeded
    // into the same locations as the Welcome document.
    let has_root_copy = INITRAMFS
        .iter()
        .any(|e| e.path == "/root/Documents/Why SunlightOS Exists.txt" && !e.is_dir);
    let has_user_copy = INITRAMFS
        .iter()
        .any(|e| e.path == "/home/user/Documents/Why SunlightOS Exists.txt" && !e.is_dir);
    let has_system_copy = INITRAMFS.iter().any(|e| {
        e.path == "/usr/share/sunlightos/documents/Why SunlightOS Exists.txt" && !e.is_dir
    });

    assert!(
        has_root_copy,
        "missing /root/Documents/Why SunlightOS Exists.txt"
    );
    assert!(
        has_user_copy,
        "missing /home/user/Documents/Why SunlightOS Exists.txt"
    );
    assert!(
        has_system_copy,
        "missing /usr/share/sunlightos/documents/Why SunlightOS Exists.txt"
    );

    // Spot-check content via a fresh RamFs.
    let mut fs = RamFs::new(INITRAMFS);
    let h = fs
        .open("/home/user/Documents/Why SunlightOS Exists.txt")
        .unwrap();
    let mut buf = [0u8; 160];
    let n = fs.read(h, 0, &mut buf).unwrap();
    assert!(n > 0);
    assert_eq!(&buf[..10], b"+---------");
    let text = core::str::from_utf8(&buf[..n]).unwrap_or("");
    assert!(text.contains("SunlightOS exists") || text.contains("future-first"));
}

#[test]
fn locale_foundation_files_present_in_initramfs() {
    // Verify /etc/locale.conf and /etc/locale.gen are registered for boot.
    let has_locale_conf = INITRAMFS
        .iter()
        .any(|e| e.path == "/etc/locale.conf" && !e.is_dir);
    let has_locale_gen = INITRAMFS
        .iter()
        .any(|e| e.path == "/etc/locale.gen" && !e.is_dir);
    assert!(has_locale_conf, "missing /etc/locale.conf in INITRAMFS");
    assert!(has_locale_gen, "missing /etc/locale.gen in INITRAMFS");

    // Spot-check content is the expected default.
    let mut fs = RamFs::new(INITRAMFS);
    let h = fs.open("/etc/locale.conf").unwrap();
    let mut buf = [0u8; 256];
    let n = fs.read(h, 0, &mut buf).unwrap();
    let text = core::str::from_utf8(&buf[..n]).unwrap_or("");
    assert!(text.contains("LANG=en_US.UTF-8"));
    assert!(text.contains("LC_TIME=en_US.UTF-8"));

    let h2 = fs.open("/etc/locale.gen").unwrap();
    let mut buf2 = [0u8; 64];
    let n2 = fs.read(h2, 0, &mut buf2).unwrap();
    let text2 = core::str::from_utf8(&buf2[..n2]).unwrap_or("");
    assert!(text2.contains("C.UTF-8"));
    assert!(text2.contains("en_US.UTF-8"));
}

#[test]
fn sun_emoji_assets_and_license_are_present_in_initramfs() {
    for path in [
        "/usr/share/sunlightos/fonts/minitype/sunlight_emoji_16.mtf",
        "/usr/share/sunlightos/fonts/minitype/sunlight_emoji_manifest.txt",
        "/usr/share/licenses/openmoji/LICENSE.txt",
        "/usr/share/licenses/openmoji/ATTRIBUTION.txt",
    ] {
        assert!(
            INITRAMFS
                .iter()
                .any(|entry| entry.path == path && !entry.is_dir),
            "missing {path} in INITRAMFS"
        );
    }
}

#[test]
fn phase_2b5_utilities_have_path_visible_stubs() {
    for path in [
        "/bin/tr",
        "/bin/paste",
        "/bin/join",
        "/bin/printf",
        "/bin/tee",
        "/bin/nl",
        "/bin/od",
        "/bin/split",
    ] {
        assert!(
            INITRAMFS.iter().any(|entry| {
                entry.path == path && !entry.is_dir && entry.mode == mode::FILE_755
            }),
            "missing executable stub for {path}"
        );
    }
}

#[test]
fn find_xargs_pipeline_utilities_have_path_visible_stubs() {
    for path in [
        "/bin/find",
        "/bin/xargs",
        "/bin/grep",
        "/bin/sort",
        "/bin/uniq",
    ] {
        assert!(
            INITRAMFS.iter().any(|entry| {
                entry.path == path && !entry.is_dir && entry.mode == mode::FILE_755
            }),
            "missing executable stub for {path}"
        );
    }
}
