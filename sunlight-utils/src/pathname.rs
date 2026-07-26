//! Small lexical pathname transformations shared by `basename` and `dirname`.
//!
//! These functions operate only on bounded pathname bytes. They never consult
//! cwd or the filesystem. For the implementation-defined `//` case, both
//! utilities process the repeated slashes, so an all-slash result is `/`.

use crate::native::MAX_ARG_LENGTH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
    TooLong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathResult<'a> {
    pub bytes: &'a [u8],
}

fn bounded<'a>(bytes: &'a [u8]) -> Result<&'a [u8], PathError> {
    if bytes.len() > MAX_ARG_LENGTH {
        Err(PathError::TooLong)
    } else {
        Ok(bytes)
    }
}

fn trim_trailing_slashes(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b'/' {
        end -= 1;
    }
    &bytes[..end]
}

fn all_slashes(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(|&byte| byte == b'/')
}

pub fn basename<'a>(path: &'a [u8], suffix: Option<&[u8]>) -> Result<PathResult<'a>, PathError> {
    let path = bounded(path)?;
    if let Some(suffix) = suffix {
        bounded(suffix)?;
    }

    if path.is_empty() {
        return Ok(PathResult { bytes: path });
    }
    if all_slashes(path) {
        return Ok(PathResult { bytes: &path[..1] });
    }

    let trimmed = trim_trailing_slashes(path);
    let start = trimmed
        .iter()
        .rposition(|&byte| byte == b'/')
        .map(|index| index + 1)
        .unwrap_or(0);
    let mut result = &trimmed[start..];

    if let Some(suffix) = suffix {
        if suffix != result && result.ends_with(suffix) {
            result = &result[..result.len() - suffix.len()];
        }
    }

    Ok(PathResult { bytes: result })
}

pub fn dirname<'a>(path: &'a [u8]) -> Result<PathResult<'a>, PathError> {
    let path = bounded(path)?;
    if all_slashes(path) {
        return Ok(PathResult { bytes: &path[..1] });
    }

    let trimmed = trim_trailing_slashes(path);
    let Some(last_slash) = trimmed.iter().rposition(|&byte| byte == b'/') else {
        return Ok(PathResult { bytes: b"." });
    };

    let prefix = trim_trailing_slashes(&trimmed[..last_slash]);
    if prefix.is_empty() {
        Ok(PathResult { bytes: b"/" })
    } else {
        Ok(PathResult { bytes: prefix })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn repeated_slashes_use_the_documented_single_root_choice() {
        assert_eq!(basename(b"//", None).unwrap().bytes, b"/");
        assert_eq!(basename(b"////", None).unwrap().bytes, b"/");
        assert_eq!(dirname(b"//").unwrap().bytes, b"/");
        assert_eq!(dirname(b"////").unwrap().bytes, b"/");
    }

    #[test]
    fn helper_rejects_unbounded_input() {
        let long = [b'x'; MAX_ARG_LENGTH + 1];
        assert_eq!(basename(&long, None), Err(PathError::TooLong));
        assert_eq!(dirname(&long), Err(PathError::TooLong));
    }
}
