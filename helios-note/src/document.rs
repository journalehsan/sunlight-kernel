//! Document identity: display name plus optional writable backing path.
//!
//! A document opened from a path has a backing path. A new document has none;
//! `untitled.txt` is a title only and is never treated as a filesystem path.

use std::path::{Path, PathBuf};

pub const UNTITLED_NAME: &str = "untitled.txt";

/// Base used when the session environment supplies no usable home directory.
const FALLBACK_BASE: &str = "/tmp";

pub struct Document {
    display_name: String,
    backing_path: Option<PathBuf>,
}

impl Document {
    pub fn untitled() -> Self {
        Self {
            display_name: String::from(UNTITLED_NAME),
            backing_path: None,
        }
    }

    pub fn from_path(path: &str) -> Self {
        Self {
            display_name: display_name_for(path),
            backing_path: Some(PathBuf::from(path)),
        }
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn backing_path(&self) -> Option<&Path> {
        self.backing_path.as_deref()
    }

    pub fn is_untitled(&self) -> bool {
        self.backing_path.is_none()
    }

    /// Establish the backing path. Call only after a write has succeeded.
    pub fn set_backing_path(&mut self, path: PathBuf) {
        self.display_name = display_name_for(&path.to_string_lossy());
        self.backing_path = Some(path);
    }

    /// Base directory used to resolve relative Save As input.
    pub fn save_as_base(&self) -> PathBuf {
        match self.backing_path.as_deref().and_then(Path::parent) {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => default_base(),
        }
    }

    /// Text prefilled into the Save As prompt.
    pub fn save_as_prefill(&self) -> String {
        match self.backing_path.as_deref() {
            Some(path) => path.to_string_lossy().into_owned(),
            None => default_base()
                .join(UNTITLED_NAME)
                .to_string_lossy()
                .into_owned(),
        }
    }
}

/// Writable default directory taken from the session environment, never `/`.
pub fn default_base() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) if home.starts_with('/') && !home.trim_end_matches('/').is_empty() => {
            PathBuf::from(home)
        }
        _ => PathBuf::from(FALLBACK_BASE),
    }
}

/// Resolve Save As input into a target path. Absolute input is used as typed;
/// relative input resolves against `base` so a bare filename never lands in
/// the process working directory, which can be `/`.
pub fn resolve_target(input: &str, base: &Path) -> Result<PathBuf, &'static str> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Enter a file path");
    }
    if trimmed.ends_with('/') {
        return Err("Invalid path or filename");
    }

    let candidate = Path::new(trimmed);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base.join(candidate)
    };

    if resolved.file_name().is_none() {
        return Err("Invalid path or filename");
    }
    Ok(resolved)
}

fn display_name_for(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untitled_document_has_no_backing_path() {
        let doc = Document::untitled();
        assert_eq!(doc.display_name(), UNTITLED_NAME);
        assert!(doc.backing_path().is_none());
        assert!(doc.is_untitled());
    }

    #[test]
    fn opened_document_has_backing_path() {
        let doc = Document::from_path("/tmp/existing.txt");
        assert_eq!(doc.backing_path(), Some(Path::new("/tmp/existing.txt")));
        assert_eq!(doc.display_name(), "existing.txt");
        assert!(!doc.is_untitled());
    }

    #[test]
    fn successful_save_as_updates_title_and_path() {
        let mut doc = Document::untitled();
        doc.set_backing_path(PathBuf::from("/tmp/notes.txt"));
        assert_eq!(doc.display_name(), "notes.txt");
        assert_eq!(doc.backing_path(), Some(Path::new("/tmp/notes.txt")));
    }

    #[test]
    fn bare_filename_resolves_against_base() {
        let target = resolve_target("notes.txt", Path::new("/home/tester")).unwrap();
        assert_eq!(target, PathBuf::from("/home/tester/notes.txt"));
    }

    #[test]
    fn absolute_input_is_used_as_typed() {
        let target = resolve_target("/tmp/notes.txt", Path::new("/home/tester")).unwrap();
        assert_eq!(target, PathBuf::from("/tmp/notes.txt"));
    }

    #[test]
    fn nested_relative_input_resolves_against_base() {
        let target = resolve_target("sub/dir/notes.txt", Path::new("/tmp")).unwrap();
        assert_eq!(target, PathBuf::from("/tmp/sub/dir/notes.txt"));
    }

    #[test]
    fn empty_and_directory_input_is_rejected() {
        assert!(resolve_target("   ", Path::new("/tmp")).is_err());
        assert!(resolve_target("/tmp/", Path::new("/tmp")).is_err());
    }

    #[test]
    fn save_as_base_prefers_existing_directory() {
        let doc = Document::from_path("/tmp/a.txt");
        assert_eq!(doc.save_as_base(), PathBuf::from("/tmp"));
        assert_eq!(doc.save_as_prefill(), "/tmp/a.txt");
    }

    #[test]
    fn default_base_is_absolute_and_not_root() {
        let base = default_base();
        assert!(base.is_absolute());
        assert_ne!(base, PathBuf::from("/"));
        let prefill = Document::untitled().save_as_prefill();
        assert!(prefill.ends_with(UNTITLED_NAME));
        assert_ne!(prefill, format!("/{}", UNTITLED_NAME));
    }
}
