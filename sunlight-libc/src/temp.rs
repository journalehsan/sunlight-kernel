//! Exclusive named temporary files. Callers own closing and unlinking them.
use crate::{Errno, Fd, MAX_PATH, O_CREAT, O_EXCL, O_RDWR};

/// Replace exactly six trailing X bytes and create a private read/write file.
/// The template contains path bytes without a terminating NUL.
pub fn mkstemp(template: &mut [u8]) -> Result<Fd, Errno> {
    create_with(
        template,
        &mut |bytes| {
            if crate::getrandom(bytes, 0) == bytes.len() as isize {
                Ok(())
            } else {
                Err(Errno::Again)
            }
        },
        &mut |path| crate::open_with_flags_mode(path, O_RDWR | O_CREAT | O_EXCL, 0o600),
    )
}

fn create_with(
    template: &mut [u8],
    random: &mut impl FnMut(&mut [u8]) -> Result<(), Errno>,
    open: &mut impl FnMut(&[u8]) -> Result<Fd, Errno>,
) -> Result<Fd, Errno> {
    if template.len() >= MAX_PATH {
        return Err(Errno::TooBig);
    }
    if template.len() < 6 || !template.ends_with(b"XXXXXX") || template.contains(&0) {
        return Err(Errno::Inval);
    }
    let start = template.len() - 6;
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
    for _ in 0..128 {
        let mut bytes = [0; 6];
        random(&mut bytes)?;
        for (out, byte) in template[start..].iter_mut().zip(bytes) {
            *out = LETTERS[(byte & 63) as usize];
        }
        match open(template) {
            Ok(fd) => return Ok(fd),
            // The native ABI currently collapses AlreadyExists into Failed.
            // Retry this bounded generic failure; never probe then create.
            Err(Errno::Failed | Errno::IsDir) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(Errno::Failed)
}

/// TEMP is a compatibility alias; empty values are ignored.
pub fn directory<'a>(tmpdir: Option<&'a [u8]>, temp: Option<&'a [u8]>) -> &'a [u8] {
    tmpdir
        .filter(|v| !v.is_empty())
        .or_else(|| temp.filter(|v| !v.is_empty()))
        .unwrap_or(b"/tmp")
}

/// C ABI: template must point to a writable NUL-terminated character array.
#[cfg_attr(
    all(not(test), any(target_os = "none", target_os = "sunlightos")),
    export_name = "mkstemp"
)]
pub unsafe extern "C" fn c_mkstemp(template: *mut u8) -> core::ffi::c_int {
    if template.is_null() {
        crate::errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    let mut len = 0;
    while len < MAX_PATH && *template.add(len) != 0 {
        len += 1;
    }
    match mkstemp(core::slice::from_raw_parts_mut(template, len)) {
        Ok(fd) => fd.0 as core::ffi::c_int,
        Err(error) => {
            crate::errno::set_from_errno(error);
            -1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_before_io() {
        for bytes in [b"bad".as_slice(), b"/tmp/aXXXXX", b"/tmp/\0XXXXXX"] {
            let mut buf = [0; 64];
            buf[..bytes.len()].copy_from_slice(bytes);
            assert_eq!(
                create_with(
                    &mut buf[..bytes.len()],
                    &mut |_| panic!(),
                    &mut |_| panic!()
                ),
                Err(Errno::Inval)
            );
        }
    }
    #[test]
    fn retries_without_truncating_and_preserves_prefix() {
        let mut name = *b"/tmp/test.XXXXXX";
        let mut calls = 0;
        let fd = create_with(
            &mut name,
            &mut |b| {
                b.fill(7);
                Ok(())
            },
            &mut |path| {
                assert_eq!(path, b"/tmp/test.hhhhhh");
                calls += 1;
                if calls == 1 {
                    Err(Errno::Failed)
                } else {
                    Ok(Fd(5))
                }
            },
        )
        .unwrap();
        assert_eq!(fd, Fd(5));
        assert_eq!(calls, 2);
    }
    #[test]
    fn entropy_failure_creates_nothing() {
        let mut template = *b"XXXXXX";
        assert_eq!(
            create_with(&mut template, &mut |_| Err(Errno::Again), &mut |_| panic!()),
            Err(Errno::Again)
        );
        assert_eq!(directory(Some(b""), Some(b"/custom")), b"/custom");
        assert_eq!(directory(Some(b"/first"), Some(b"/second")), b"/first");
        assert_eq!(directory(None, None), b"/tmp");
    }
}
