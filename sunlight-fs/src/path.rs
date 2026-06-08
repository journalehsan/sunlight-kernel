use crate::FsError;

pub fn validate_absolute(path: &str) -> Result<(), FsError> {
    if path.is_empty() || !path.starts_with('/') || path.as_bytes().contains(&0) {
        return Err(FsError::InvalidPath);
    }
    if path.len() > 1 && path.ends_with('/') {
        return Err(FsError::InvalidPath);
    }
    if path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(FsError::InvalidPath);
    }
    Ok(())
}

pub fn strip_mount<'a>(path: &'a str, mount: &str) -> Option<&'a str> {
    if mount == "/" {
        return Some(path);
    }
    if path == mount {
        return Some("/");
    }
    path.strip_prefix(mount)
        .filter(|suffix| suffix.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_absolute_paths() {
        assert_eq!(validate_absolute("/etc/motd"), Ok(()));
        assert_eq!(validate_absolute("etc/motd"), Err(FsError::InvalidPath));
        assert_eq!(validate_absolute("/etc/../motd"), Err(FsError::InvalidPath));
        assert_eq!(validate_absolute("/etc/motd/"), Err(FsError::InvalidPath));
    }
}
