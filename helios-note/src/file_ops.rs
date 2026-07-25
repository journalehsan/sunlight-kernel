//! File loading and error classification.

use crate::core::buffer::TextBuffer;
use std::{fs, io, path::Path};

pub struct FileResult {
    pub buffer: TextBuffer,
    pub path: String,
    pub is_new: bool,
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
