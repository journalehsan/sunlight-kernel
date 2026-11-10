//! Named temporary-file applet shared by standalone and multicall entry points.
use sunlight_libc as libc;

pub fn run(args: &[&str]) -> i32 {
    if args == ["--help"] {
        return if libc::write_all(libc::STDOUT, b"usage: mktemp [TEMPLATE]\nTEMPLATE must end in XXXXXX. Default: TMPDIR/TEMP or /tmp, plus tmp.XXXXXX.\n").is_ok() { 0 } else { 1 };
    }
    let mut path = [0; libc::MAX_PATH];
    let len = match prepare(
        args,
        libc::env::getenv_bytes(b"TMPDIR"),
        libc::env::getenv_bytes(b"TEMP"),
        &mut path,
    ) {
        Ok(len) => len,
        Err(()) => {
            let _ = libc::write_all(
                libc::STDERR,
                b"mktemp: expected one template ending in XXXXXX\n",
            );
            return 2;
        }
    };
    match libc::temp::mkstemp(&mut path[..len]) {
        Ok(fd) => {
            if libc::close(fd).is_err() {
                return 1;
            }
            if libc::write_all(libc::STDOUT, &path[..len])
                .and_then(|_| libc::write_all(libc::STDOUT, b"\n"))
                .is_ok()
            {
                0
            } else {
                1
            }
        }
        Err(_) => {
            let _ = libc::write_all(libc::STDERR, b"mktemp: cannot create temporary file\n");
            1
        }
    }
}

fn prepare(
    args: &[&str],
    tmpdir: Option<&[u8]>,
    temp: Option<&[u8]>,
    out: &mut [u8],
) -> Result<usize, ()> {
    let len = match args {
        [] => {
            let dir = libc::temp::directory(tmpdir, temp);
            let suffix = if dir.ends_with(b"/") {
                b"tmp.XXXXXX".as_slice()
            } else {
                b"/tmp.XXXXXX".as_slice()
            };
            let len = dir.len().checked_add(suffix.len()).ok_or(())?;
            if len >= out.len() {
                return Err(());
            }
            out[..dir.len()].copy_from_slice(dir);
            out[dir.len()..len].copy_from_slice(suffix);
            len
        }
        [name] if !name.starts_with('-') && name.len() < out.len() => {
            out[..name.len()].copy_from_slice(name.as_bytes());
            name.len()
        }
        _ => return Err(()),
    };
    if !out[..len].ends_with(b"XXXXXX") || out[..len].contains(&0) {
        return Err(());
    }
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_and_environment() {
        let mut out = [0; 256];
        for (args, dir, expected) in [
            (&[][..], None, "/tmp/tmp.XXXXXX"),
            (&[][..], Some(b"/custom/".as_slice()), "/custom/tmp.XXXXXX"),
            (
                &["local.XXXXXX"][..],
                Some(b"/ignored".as_slice()),
                "local.XXXXXX",
            ),
        ] {
            let len = prepare(args, dir, None, &mut out).unwrap();
            assert_eq!(&out[..len], expected.as_bytes());
        }
        assert!(prepare(&["bad"], None, None, &mut out).is_err());
        assert!(prepare(&["-u"], None, None, &mut out).is_err());
        assert!(prepare(&[], None, None, &mut [0; 4]).is_err());
    }
}
