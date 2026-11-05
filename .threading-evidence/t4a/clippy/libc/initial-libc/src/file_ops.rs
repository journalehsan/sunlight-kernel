//! Bounded file operations shared by Files and the desktop.
//! Copies use exclusive creation. Moves use the atomic no-replace syscall and
//! never fall back to deleting source data after a failed rename.
extern crate alloc as rust_alloc;
#[cfg(not(test))]
use crate as libc;
use crate::{DirEntry, FT_DIR, FT_FILE};
use rust_alloc::{format, string::String, vec, vec::Vec};
#[cfg(test)]
use tests::backend as libc;

const MAX_NODES: usize = 8192;
const MAX_DEPTH: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub struct TransferError {
    pub completed: usize,
    pub message: &'static str,
}

pub fn normalize(path: &str) -> Result<String, &'static str> {
    if !path.starts_with('/') || path.as_bytes().contains(&0) {
        return Err("Expected an absolute file path");
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    let result = format!("/{}", parts.join("/"));
    if result.len() >= libc::MAX_PATH {
        return Err("Path is too long");
    }
    Ok(result)
}

fn child(parent: &str, name: &str) -> Result<String, &'static str> {
    if name.len() > 64
        || name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.as_bytes().contains(&0)
    {
        return Err("Invalid file name");
    }
    let path = format!("{}/{}", parent.trim_end_matches('/'), name);
    if path.len() >= libc::MAX_PATH {
        return Err("Path is too long");
    }
    Ok(path)
}

fn within(path: &str, ancestor: &str) -> bool {
    path == ancestor
        || path
            .strip_prefix(ancestor)
            .is_some_and(|tail| tail.starts_with('/'))
}

/// Preserve extensions when making a duplicate next to its original.
fn copy_name(name: &str, directory: bool, number: usize) -> String {
    let (stem, extension) = if directory {
        (name, "")
    } else {
        name.rfind('.')
            .filter(|index| *index > 0)
            .map(|index| (&name[..index], &name[index..]))
            .unwrap_or((name, ""))
    };
    if number == 1 {
        format!("{stem} (copy){extension}")
    } else {
        format!("{stem} (copy {number}){extension}")
    }
}

#[derive(Clone)]
struct Node {
    source: String,
    destination: String,
    directory: bool,
    mode: u16,
    size: usize,
}

/// Enumerate every page without silently truncating at the syscall's 64-entry limit.
pub fn read_directory(path: &str, limit: usize) -> Result<Vec<DirEntry>, &'static str> {
    let mut entries = Vec::new();
    let mut page = vec![DirEntry::zeroed(); 64];
    loop {
        let count = libc::read_dir_from(path.as_bytes(), &mut page, entries.len())
            .map_err(|_| "Cannot read folder")?;
        if count == 0 {
            return Ok(entries);
        }
        if count > page.len() || entries.len() + count > limit {
            return Err("Folder contains too many items");
        }
        entries.extend_from_slice(&page[..count]);
    }
}

fn plan_tree(
    source: &str,
    destination: &str,
    depth: usize,
    limit: usize,
    nodes: &mut Vec<Node>,
) -> Result<(), &'static str> {
    if depth > MAX_DEPTH || nodes.len() >= limit {
        return Err("Folder is too large or too deeply nested");
    }
    let stat =
        libc::stat(source.as_bytes()).map_err(|_| "Source is unavailable or cannot be read")?;
    if stat.file_type != FT_FILE && stat.file_type != FT_DIR {
        return Err("Unsupported file type");
    }
    let directory = stat.file_type == FT_DIR;
    nodes.push(Node {
        source: String::from(source),
        destination: String::from(destination),
        directory,
        mode: stat.mode & 0o777,
        size: usize::try_from(stat.size).map_err(|_| "Source file is too large")?,
    });
    if directory {
        let entries = read_directory(source, limit)?;
        for entry in &entries {
            let name = core::str::from_utf8(entry.name_bytes()).map_err(|_| "Invalid file name")?;
            if name == "." || name == ".." {
                continue;
            }
            plan_tree(
                &child(source, name)?,
                &child(destination, name)?,
                depth + 1,
                limit,
                nodes,
            )?;
        }
    }
    Ok(())
}

fn copy_file(node: &Node) -> Result<(), &'static str> {
    let source = libc::open(node.source.as_bytes()).map_err(|_| "Cannot open source file")?;
    let destination = match libc::open_with_flags_mode(
        node.destination.as_bytes(),
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        node.mode,
    ) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = libc::close(source);
            return Err("Cannot create destination; it may already exist or be read-only");
        }
    };
    if libc::file_reserve(destination, node.size).is_err() {
        let _ = libc::close(source);
        let _ = libc::close(destination);
        let _ = libc::unlink(node.destination.as_bytes());
        return Err("Cannot reserve destination storage; disk or memory may be full");
    }
    let result = (|| {
        let mut buffer = [0u8; 4096];
        loop {
            let count = libc::read(source, &mut buffer).map_err(|_| "Cannot read source file")?;
            if count == 0 {
                break;
            }
            libc::write_all(destination, &buffer[..count])
                .map_err(|_| "Cannot write destination; disk may be full")?;
        }
        Ok(())
    })();
    let _ = libc::close(source);
    let closed = libc::close(destination).map_err(|_| "Cannot close destination file");
    let result = result.and(closed);
    if result.is_err() {
        let _ = libc::unlink(node.destination.as_bytes());
    }
    result
}

/// Returns the number of completed top-level entries. On failure the caller can
/// consume exactly that prefix from a cut clipboard; remaining sources stay put.
/// Existing destinations are never overwritten. Copying into the same folder
/// generates a new name. Copying onto an existing name elsewhere reports a conflict.
pub fn transfer(paths: &[String], destination: &str, cut: bool) -> Result<usize, TransferError> {
    let fail = |message| TransferError {
        completed: 0,
        message,
    };
    let destination = normalize(destination).map_err(fail)?;
    if libc::stat(destination.as_bytes())
        .map_err(|_| fail("Destination folder is unavailable"))?
        .file_type
        != FT_DIR
    {
        return Err(fail("Destination is not a folder"));
    }
    if paths.is_empty() {
        return Err(fail("No files selected"));
    }
    let mut sources = Vec::new();
    for path in paths {
        let path = normalize(path).map_err(fail)?;
        if path == "/" {
            return Err(fail("Cannot transfer the filesystem root"));
        }
        if sources
            .iter()
            .any(|other: &String| within(&path, other) || within(other, &path))
        {
            return Err(fail("Selection contains duplicate or overlapping paths"));
        }
        sources.push(path);
    }
    // Validate all top-level targets before making any change.
    let mut plans = Vec::new();
    let mut total_nodes = 0;
    for source in &sources {
        let stat = libc::stat(source.as_bytes())
            .map_err(|_| fail("Clipboard source is no longer available"))?;
        let name = source
            .rsplit('/')
            .next()
            .ok_or_else(|| fail("Invalid source path"))?;
        let mut target = child(&destination, name).map_err(fail)?;
        if stat.file_type == FT_DIR && within(&destination, source) {
            return Err(fail("Cannot paste a folder into itself or its children"));
        }
        if target == *source {
            if cut {
                return Err(fail("Item is already in this folder"));
            }
            let mut available = None;
            for number in 1..=999 {
                let candidate = child(
                    &destination,
                    &copy_name(name, stat.file_type == FT_DIR, number),
                )
                .map_err(fail)?;
                if libc::stat(candidate.as_bytes()).is_err()
                    && !plans.iter().any(|(_, target, _)| target == &candidate)
                {
                    available = Some(candidate);
                    break;
                }
            }
            target = available.ok_or_else(|| fail("Cannot find an unused copy name"))?;
        } else if libc::stat(target.as_bytes()).is_ok() {
            return Err(fail(
                "Destination name already exists; rename it or choose another folder",
            ));
        }
        if plans.iter().any(|(_, previous, _)| previous == &target) {
            return Err(fail("Selected items have the same destination name"));
        }
        let mut nodes = Vec::new();
        plan_tree(source, &target, 0, MAX_NODES - total_nodes, &mut nodes).map_err(fail)?;
        total_nodes += nodes.len();
        plans.push((source.clone(), target, nodes));
    }
    for (completed, (source, target, nodes)) in plans.iter().enumerate() {
        let result = if cut {
            libc::rename_no_replace(source.as_bytes(), target.as_bytes()).map_err(|_| {
                "Move failed; source kept. Check permissions, conflicts, or use Copy across drives"
            })
        } else {
            let mut result = Ok(());
            for node in nodes {
                result = if node.directory {
                    libc::mkdir(node.destination.as_bytes(), node.mode)
                        .map_err(|_| "Cannot create folder; a partial copy may remain")
                } else {
                    copy_file(node)
                };
                if result.is_err() {
                    break;
                }
            }
            result
        };
        if let Err(message) = result {
            return Err(TransferError { completed, message });
        }
    }
    Ok(plans.len())
}

pub fn create_item(directory: &str, folder: bool) -> Result<String, &'static str> {
    let directory = normalize(directory)?;
    for number in 1..=999 {
        let stem = if folder {
            "New Folder"
        } else {
            "New Text File"
        };
        let extension = if folder { "" } else { ".txt" };
        let name = if number == 1 {
            format!("{stem}{extension}")
        } else {
            format!("{stem} {number}{extension}")
        };
        let path = child(&directory, &name)?;
        if libc::stat(path.as_bytes()).is_ok() {
            continue;
        }
        if folder {
            libc::mkdir(path.as_bytes(), 0o755).map_err(|_| "Cannot create folder")?;
        } else {
            let fd = libc::open_with_flags_mode(
                path.as_bytes(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o644,
            )
            .map_err(|_| "Cannot create text file")?;
            libc::close(fd).map_err(|_| "Cannot close new file")?;
        }
        return Ok(path);
    }
    Err("Cannot find an unused name")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_paths_cannot_hide_descendants() {
        assert_eq!(normalize("/home/a/../a//child/.").unwrap(), "/home/a/child");
        assert!(within(&normalize("/a/b/../c").unwrap(), "/a"));
        assert!(!within("/ab/c", "/a"));
        assert!(normalize("relative").is_err());
        assert!(normalize("/a\0b").is_err());
    }
    #[test]
    fn duplicate_names_keep_extensions_and_dotfiles() {
        assert_eq!(copy_name("report.txt", false, 1), "report (copy).txt");
        assert_eq!(copy_name(".config", false, 2), ".config (copy 2)");
        assert_eq!(copy_name("folder.name", true, 1), "folder.name (copy)");
        assert!(child("/home", "../bad").is_err());
    }

    extern crate std;
    use self::std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(String);
    impl Fixture {
        fn new() -> Self {
            let path = format!(
                "/tmp/sunlight-file-ops-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self, suffix: &str) -> String {
            format!("{}/{}", self.0, suffix)
        }
        fn folder(&self, suffix: &str) -> String {
            let path = self.path(suffix);
            fs::create_dir_all(&path).unwrap();
            path
        }
        fn file(&self, suffix: &str, bytes: &[u8]) -> String {
            let path = self.path(suffix);
            fs::write(&path, bytes).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn copies_large_nested_tree_and_empty_files_across_directory_pages() {
        let fixture = Fixture::new();
        let source = fixture.folder("source");
        fixture.folder("source/sub/empty-folder");
        fixture.file("source/sub/empty", b"");
        let data = vec![0xA5; 19000];
        for i in 0..130 {
            fixture.file(&format!("source/file-{i}"), &data);
        }
        let destination = fixture.folder("destination");
        assert_eq!(transfer(&[source], &destination, false), Ok(1));
        for i in 0..130 {
            assert_eq!(
                fs::read(fixture.path(&format!("destination/source/file-{i}"))).unwrap(),
                data
            );
        }
        assert!(
            fs::metadata(fixture.path("destination/source/sub/empty-folder"))
                .unwrap()
                .is_dir()
        );
        assert_eq!(
            fs::read(fixture.path("destination/source/sub/empty")).unwrap(),
            b""
        );
        assert!(fs::metadata(fixture.path("source/file-0")).is_ok());
    }

    #[test]
    fn conflicts_and_descendant_targets_do_not_modify_sources() {
        let fixture = Fixture::new();
        let source = fixture.folder("source");
        fixture.file("source/file", b"original");
        let child = fixture.folder("source/child");
        assert_eq!(
            transfer(&[source.clone()], &child, false)
                .unwrap_err()
                .completed,
            0
        );
        let destination = fixture.folder("destination");
        fixture.file("destination/source", b"keep me");
        assert!(transfer(&[source], &destination, true).is_err());
        assert_eq!(
            fs::read(fixture.path("destination/source")).unwrap(),
            b"keep me"
        );
        assert_eq!(fs::read(fixture.path("source/file")).unwrap(), b"original");
    }

    #[test]
    fn same_folder_copy_gets_unique_name_but_cut_is_a_noop() {
        let fixture = Fixture::new();
        let source = fixture.file("report.txt", b"content");
        assert_eq!(transfer(&[source.clone()], &fixture.0, false), Ok(1));
        assert_eq!(transfer(&[source.clone()], &fixture.0, false), Ok(1));
        assert_eq!(
            fs::read(fixture.path("report (copy).txt")).unwrap(),
            b"content"
        );
        assert_eq!(
            fs::read(fixture.path("report (copy 2).txt")).unwrap(),
            b"content"
        );
        assert!(transfer(&[source], &fixture.0, true).is_err());
        assert_eq!(fs::read(fixture.path("report.txt")).unwrap(), b"content");
    }

    #[test]
    fn partial_move_reports_only_successful_prefix() {
        let fixture = Fixture::new();
        let first = fixture.file("first", b"first");
        let second = fixture.file("second", b"second");
        let destination = fixture.folder("destination");
        backend::FAIL_MOVE.with(|path| *path.borrow_mut() = second.clone());
        let error = transfer(&[first.clone(), second.clone()], &destination, true).unwrap_err();
        backend::FAIL_MOVE.with(|path| path.borrow_mut().clear());
        assert_eq!(error.completed, 1);
        assert!(fs::metadata(first).is_err());
        assert_eq!(
            fs::read(fixture.path("destination/first")).unwrap(),
            b"first"
        );
        assert_eq!(fs::read(second).unwrap(), b"second");
        assert!(fs::metadata(fixture.path("destination/second")).is_err());
    }

    #[test]
    fn failed_write_removes_incomplete_file_and_keeps_original() {
        let fixture = Fixture::new();
        let source = fixture.file("file", b"original");
        let destination = fixture.folder("destination");
        backend::FAIL_WRITE.with(|flag| flag.set(true));
        let error = transfer(&[source.clone()], &destination, false).unwrap_err();
        backend::FAIL_WRITE.with(|flag| flag.set(false));
        assert_eq!(error.completed, 0);
        assert!(fs::metadata(fixture.path("destination/file")).is_err());
        assert_eq!(fs::read(source).unwrap(), b"original");
    }

    #[test]
    fn failed_reservation_removes_empty_destination_and_keeps_original() {
        let fixture = Fixture::new();
        let source = fixture.file("large.simg", &[0x5a; 8192]);
        let destination = fixture.folder("destination");
        backend::FAIL_RESERVE.with(|flag| flag.set(true));
        let error = transfer(&[source.clone()], &destination, false).unwrap_err();
        backend::FAIL_RESERVE.with(|flag| flag.set(false));
        assert_eq!(error.completed, 0);
        assert!(fs::metadata(fixture.path("destination/large.simg")).is_err());
        assert_eq!(fs::read(source).unwrap(), vec![0x5a; 8192]);
    }

    #[test]
    fn overlapping_and_missing_sources_fail_before_copying() {
        let fixture = Fixture::new();
        let source = fixture.folder("source");
        let file = fixture.file("source/file", b"original");
        let destination = fixture.folder("destination");
        assert!(transfer(&[source, file.clone()], &destination, false).is_err());
        assert!(transfer(&[file, fixture.path("missing")], &destination, false).is_err());
        assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    }

    // Host filesystem adapter: exercises the actual transfer planner and streaming
    // loop without issuing Sunlight syscalls on Linux. Native atomic rename and
    // RAMFS subtree behavior are tested separately in sunlight-fs.
    pub(super) mod backend {
        use super::std;
        use super::{String, Vec};
        use crate::{DirEntry, Errno, Fd, Stat, FT_DIR, FT_FILE};
        pub use crate::{MAX_PATH, O_CREAT, O_EXCL, O_WRONLY};
        use std::{
            cell::{Cell, RefCell},
            fs::{self, File, OpenOptions},
            io::{Read, Write},
            mem::ManuallyDrop,
            os::{
                fd::{FromRawFd, IntoRawFd},
                unix::fs::PermissionsExt,
            },
        };
        std::thread_local! {
            pub static FAIL_MOVE: RefCell<String> = RefCell::new(String::new());
            pub static FAIL_WRITE: Cell<bool> = const { Cell::new(false) };
            pub static FAIL_RESERVE: Cell<bool> = const { Cell::new(false) };
        }
        fn path(bytes: &[u8]) -> &str {
            core::str::from_utf8(bytes).unwrap()
        }
        pub fn stat(bytes: &[u8]) -> Result<Stat, Errno> {
            let metadata = fs::symlink_metadata(path(bytes)).map_err(|_| Errno::Failed)?;
            let mut stat = Stat::zeroed();
            stat.file_type = if metadata.is_dir() {
                FT_DIR
            } else if metadata.is_file() {
                FT_FILE
            } else {
                0
            };
            stat.mode = metadata.permissions().mode() as u16;
            stat.size = metadata.len();
            Ok(stat)
        }
        pub fn read_dir_from(
            bytes: &[u8],
            entries: &mut [DirEntry],
            offset: usize,
        ) -> Result<usize, Errno> {
            let mut children: Vec<_> = fs::read_dir(path(bytes))
                .map_err(|_| Errno::Failed)?
                .map(|entry| entry.unwrap())
                .collect();
            children.sort_by_key(|entry| entry.file_name());
            let mut count = 0;
            for (entry, output) in children
                .iter()
                .skip(offset)
                .zip(entries.iter_mut().take(64))
            {
                let name = entry.file_name();
                let name = name.to_str().unwrap().as_bytes();
                *output = DirEntry::zeroed();
                output.name[..name.len()].copy_from_slice(name);
                output.name_len = name.len() as u8;
                count += 1;
            }
            Ok(count)
        }
        pub fn open(bytes: &[u8]) -> Result<Fd, Errno> {
            File::open(path(bytes))
                .map(|file| Fd(file.into_raw_fd() as u32))
                .map_err(|_| Errno::Failed)
        }
        pub fn open_with_flags_mode(bytes: &[u8], flags: u64, _mode: u16) -> Result<Fd, Errno> {
            assert_eq!(flags, O_WRONLY | O_CREAT | O_EXCL);
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path(bytes))
                .map(|file| Fd(file.into_raw_fd() as u32))
                .map_err(|_| Errno::Failed)
        }
        pub fn file_reserve(_fd: Fd, _size: usize) -> Result<(), Errno> {
            if FAIL_RESERVE.with(|flag| flag.get()) {
                Err(Errno::Failed)
            } else {
                Ok(())
            }
        }
        pub fn read(fd: Fd, bytes: &mut [u8]) -> Result<usize, Errno> {
            let mut file = ManuallyDrop::new(unsafe { File::from_raw_fd(fd.0 as i32) });
            // Deliberately short reads to exercise the streaming loop.
            let size = bytes.len().min(257);
            file.read(&mut bytes[..size]).map_err(|_| Errno::Failed)
        }
        pub fn write_all(fd: Fd, bytes: &[u8]) -> Result<(), Errno> {
            if FAIL_WRITE.with(|flag| flag.get()) {
                return Err(Errno::Failed);
            }
            let mut file = ManuallyDrop::new(unsafe { File::from_raw_fd(fd.0 as i32) });
            file.write_all(bytes).map_err(|_| Errno::Failed)
        }
        pub fn close(fd: Fd) -> Result<(), Errno> {
            drop(unsafe { File::from_raw_fd(fd.0 as i32) });
            Ok(())
        }
        pub fn unlink(bytes: &[u8]) -> Result<(), Errno> {
            fs::remove_file(path(bytes)).map_err(|_| Errno::Failed)
        }
        pub fn mkdir(bytes: &[u8], _mode: u16) -> Result<(), Errno> {
            fs::create_dir(path(bytes)).map_err(|_| Errno::Failed)
        }
        pub fn rename_no_replace(old: &[u8], new: &[u8]) -> Result<(), Errno> {
            if stat(new).is_ok() || FAIL_MOVE.with(|item| item.borrow().as_str() == path(old)) {
                return Err(Errno::Failed);
            }
            fs::rename(path(old), path(new)).map_err(|_| Errno::Failed)
        }
    }
}
