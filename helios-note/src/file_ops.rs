//! File loading and error classification.

use crate::core::buffer::TextBuffer;
use std::{fs, io, path::Path};

pub struct FileResult {
    pub buffer: TextBuffer,
    pub path: String,
    pub is_new: bool,
}

/// Explain a filesystem failure in editor terms, keeping the errno visible for
/// diagnostics instead of surfacing a raw OS string as the primary message.
pub fn describe_io_error(err: &io::Error) -> String {
    let reason = match err.raw_os_error() {
        Some(libc::EACCES) | Some(libc::EPERM) => "Permission denied",
        Some(libc::ENOENT) => "Directory does not exist",
        Some(libc::EISDIR) => "Path is a directory",
        Some(libc::ENOTDIR) => "A path component is not a directory",
        Some(libc::EINVAL) => "Invalid path or filename",
        Some(libc::ENOSPC) => "No space left on device",
        Some(libc::EROFS) => "Read-only filesystem",
        Some(libc::ENAMETOOLONG) => "Path too long",
        _ => match err.kind() {
            io::ErrorKind::PermissionDenied => "Permission denied",
            io::ErrorKind::NotFound => "Path not found",
            _ => return format!("Save failed: {}", err),
        },
    };

    match err.raw_os_error() {
        Some(code) => format!("{} (errno {})", reason, code),
        None => reason.to_string(),
    }
}

/// Open an existing file or prepare an empty buffer for a new file.
pub fn open_file(path_str: &str) -> io::Result<FileResult> {
    let path = Path::new(path_str);
    if !path.exists() {
        return Ok(FileResult {
            buffer: TextBuffer::new(),
            path: path_str.to_string(),
            is_new: true,
        });
    }

    let bytes = fs::read(path)?;
    // Validate UTF-8 safely
    let content = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).to_string(),
    };

    let buffer = TextBuffer::from_str(&content);

    Ok(FileResult {
        buffer,
        path: path_str.to_string(),
        is_new: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_error_is_explained_with_errno() {
        let msg = describe_io_error(&io::Error::from_raw_os_error(libc::EACCES));
        assert!(msg.starts_with("Permission denied"));
        assert!(msg.contains(&libc::EACCES.to_string()));
    }

    #[test]
    fn missing_directory_is_not_reported_as_permission_denied() {
        let msg = describe_io_error(&io::Error::from_raw_os_error(libc::ENOENT));
        assert!(msg.starts_with("Directory does not exist"));
    }

    #[test]
    fn read_only_filesystem_is_explained() {
        let msg = describe_io_error(&io::Error::from_raw_os_error(libc::EROFS));
        assert!(msg.starts_with("Read-only filesystem"));
    }
}
