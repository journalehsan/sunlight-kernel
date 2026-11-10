use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use stdio::*;
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Stream {
    ptr: *mut SunlightFile,
    path: std::path::PathBuf,
}
impl Stream {
    fn new() -> Self {
        FAULTS.with(|f| *f.borrow_mut() = Faults::default());
        let path = std::env::temp_dir().join(format!(
            "sunlight-stdio-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let cpath = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        let ptr = unsafe { fopen(cpath.as_ptr().cast(), c"w+".as_ptr().cast()) };
        assert!(!ptr.is_null());
        Self { ptr, path }
    }
    fn bytes(&self) -> Vec<u8> {
        std::fs::read(&self.path).unwrap()
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        FAULTS.with(|f| *f.borrow_mut() = Faults::default());
        if !self.ptr.is_null() {
            assert_eq!(unsafe { fclose(self.ptr) }, 0);
        }
        std::fs::remove_file(&self.path).unwrap();
    }
}
#[test]
fn standard_descriptor_mapping() {
    unsafe {
        assert_eq!(fileno(stdin), 0);
        assert_eq!(fileno(stdout), 1);
        assert_eq!(fileno(stderr), 2);
        assert_eq!(fflush(core::ptr::null_mut()), 0);
    }
}

#[test]
fn tmpfile_anonymous_roundtrip_independent_streams_and_indicators() {
    FAULTS.with(|f| *f.borrow_mut() = Faults::default());
    unsafe {
        let a = tmpfile();
        let b = tmpfile();
        assert!(!a.is_null() && !b.is_null());
        assert_ne!(fileno(a), fileno(b));
        let names = FAULTS.with(|f| f.borrow().unlinks.clone());
        assert_eq!(names.len(), 2);
        for name in names {
            assert_eq!(
                open_with_flags_mode(&name, O_RDONLY, 0),
                Err(sys::Errno::NoEntry)
            );
        }
        assert_eq!(fputs(c"alpha".as_ptr().cast(), a), 0);
        assert_eq!(fwrite(b"beta".as_ptr().cast(), 2, 2, b), 2);
        assert_eq!(ftell(a), 5);
        assert_eq!(ftell(b), 4);
        assert_eq!(fseek(a, -2, 2), 0);
        assert_eq!(fgetc(a), b'h' as i32);
        rewind(a);
        rewind(b);
        let mut bytes = [0; 8];
        assert_eq!(fread(bytes.as_mut_ptr().cast(), 1, 8, a), 5);
        assert_eq!(&bytes[..5], b"alpha");
        assert_eq!(feof(a), 1);
        assert_eq!(ferror(a), 0);
        assert_eq!(feof(b), 0);
        assert_eq!(fread(bytes.as_mut_ptr().cast(), 1, 4, b), 4);
        assert_eq!(&bytes[..4], b"beta");
        FAULTS.with(|f| f.borrow_mut().writes.push_back(Err(sys::Errno::NoSpace)));
        assert_eq!(fputc(65, a), EOF);
        assert_eq!(ferror(a), 1);
        assert_eq!(errno::get_errno(), 28);
        rewind(a);
        assert_eq!(feof(a), 0);
        assert_eq!(ferror(a), 0);
        assert_eq!(fgetc(a), b'a' as i32);
        assert_eq!(fclose(a), 0);
        assert_eq!(fgetc(b), EOF);
        assert_eq!(feof(b), 1);
        assert_eq!(fclose(b), 0);
        FAULTS.with(|f| {
            let f = f.borrow();
            assert_eq!(f.created, f.closed);
            assert_eq!(f.allocations, f.frees);
        });
    }
}

#[test]
fn tmpfile_allocation_entropy_and_open_failures_create_no_file() {
    for (allocation, entropy, open_error, expected) in [
        (true, false, None, errno::ENOMEM),
        (false, true, None, errno::EAGAIN),
        (false, false, Some(sys::Errno::Access), 13),
        (false, false, Some(sys::Errno::NoEntry), errno::ENOENT),
        (false, false, Some(sys::Errno::NoSpace), 28),
    ] {
        FAULTS.with(|f| {
            let mut f = f.borrow_mut();
            *f = Faults::default();
            f.allocation_failure = allocation;
            f.free_errno = Some(errno::E2BIG);
            f.open_failure = open_error;
            if entropy {
                f.random.push_back(Err(()));
            }
        });
        assert!(unsafe { tmpfile() }.is_null());
        assert_eq!(errno::get_errno(), expected);
        FAULTS.with(|f| {
            let f = f.borrow();
            assert!(f.created.is_empty() && f.closed.is_empty() && f.unlinks.is_empty());
            assert_eq!(f.opens.len(), usize::from(open_error.is_some()));
            assert_eq!(f.allocations, f.frees);
        });
    }
}

#[test]
fn tmpfile_unlink_failure_closes_fd_and_preserves_errno_over_cleanup() {
    FAULTS.with(|f| {
        *f.borrow_mut() = Faults {
            unlink_failure: Some(sys::Errno::Access),
            close_failure: true,
            free_errno: Some(errno::E2BIG),
            ..Faults::default()
        };
    });
    assert!(unsafe { tmpfile() }.is_null());
    assert_eq!(errno::get_errno(), 13);
    let name = FAULTS.with(|f| {
        let f = f.borrow();
        assert_eq!(f.created.len(), 1);
        assert_eq!(f.created, f.closed);
        assert_eq!(f.allocations, f.frees);
        f.unlinks[0].clone()
    });
    // A failed unlink can leave the name; the API reports failure, never a
    // successful named stream. Remove the injected failure's residue here.
    std::fs::remove_file(std::path::Path::new(
        <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&name),
    ))
    .unwrap();
}

#[test]
fn tmpfile_collisions_are_exclusive_bounded_and_do_not_truncate() {
    FAULTS.with(|f| *f.borrow_mut() = Faults::default());
    let first = unsafe { tmpfile() };
    assert!(!first.is_null());
    let name = FAULTS.with(|f| f.borrow().unlinks[0].clone());
    assert_eq!(unsafe { fclose(first) }, 0);
    let path =
        std::path::Path::new(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&name));
    std::fs::write(path, b"untouched").unwrap();
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
    let suffix: [u8; 6] = name[name.len() - 6..].try_into().unwrap();
    let entropy = suffix.map(|b| LETTERS.iter().position(|&letter| letter == b).unwrap() as u8);
    for error in [None, Some(sys::Errno::IsDir), Some(sys::Errno::Failed)] {
        FAULTS.with(|f| {
            let mut f = f.borrow_mut();
            *f = Faults::default();
            f.open_failure = error;
            f.random.extend(std::iter::repeat_n(Ok(entropy), 128));
        });
        assert!(unsafe { tmpfile() }.is_null());
        assert_eq!(errno::get_errno(), errno::EIO);
        assert_eq!(std::fs::read(path).unwrap(), b"untouched");
        FAULTS.with(|f| {
            let f = f.borrow();
            assert_eq!(f.opens.len(), 128);
            assert!(f.created.is_empty() && f.closed.is_empty() && f.unlinks.is_empty());
            assert_eq!(f.allocations, f.frees);
            for (_, flags, mode) in &f.opens {
                assert_eq!(*flags, O_RDWR | O_CREAT | O_EXCL);
                assert_eq!(*mode, 0o600);
            }
        });
    }
    // One real collision, then a fresh candidate succeeds.
    FAULTS.with(|f| {
        *f.borrow_mut() = Faults::default();
        f.borrow_mut().random.push_back(Ok(entropy));
    });
    let second = unsafe { tmpfile() };
    assert!(!second.is_null());
    assert_eq!(std::fs::read(path).unwrap(), b"untouched");
    assert_eq!(FAULTS.with(|f| f.borrow().opens.len()), 2);
    assert_eq!(unsafe { fclose(second) }, 0);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn tmpfile_simultaneous_calls_use_independent_streams() {
    std::thread::scope(|scope| {
        for i in 0..12 {
            scope.spawn(move || unsafe {
                let file = tmpfile();
                assert!(!file.is_null());
                assert_eq!(fputc(i, file), i);
                rewind(file);
                assert_eq!(fgetc(file), i);
                assert_eq!(fclose(file), 0);
            });
        }
    });
}
#[test]
fn short_writes_and_partial_items_preserve_errors() {
    let s = Stream::new();
    FAULTS.with(|f| {
        f.borrow_mut()
            .writes
            .extend([Ok(1), Ok(2), Err(sys::Errno::BrokenPipe)])
    });
    unsafe {
        assert_eq!(fwrite(b"abcdef".as_ptr().cast(), 2, 3, s.ptr), 1);
        assert_eq!(s.bytes(), b"abc");
        assert_eq!(ferror(s.ptr), 1);
        assert_eq!(feof(s.ptr), 0);
        assert_eq!(errno::get_errno(), 32);
        // An unbuffered flush never retries/replays failed output.
        assert_eq!(fflush(s.ptr), 0);
        assert_eq!(ferror(s.ptr), 1);
        clearerr(s.ptr);
        assert_eq!(ferror(s.ptr), 0);
        FAULTS.with(|f| f.borrow_mut().writes.extend([Ok(1), Ok(1)]));
        assert_eq!(fwrite(b"de".as_ptr().cast(), 1, 2, s.ptr), 2);
        assert_eq!(s.bytes(), b"abcde");
    }
}
#[test]
fn zero_progress_and_again_are_errors_without_retry_loop() {
    let s = Stream::new();
    unsafe {
        for error in [Ok(0), Err(sys::Errno::Again)] {
            FAULTS.with(|f| f.borrow_mut().writes.push_back(error));
            assert_eq!(fputc(65, s.ptr), EOF);
            assert_eq!(ferror(s.ptr), 1);
            assert_eq!(
                errno::get_errno(),
                if error.is_ok() {
                    errno::EIO
                } else {
                    errno::EAGAIN
                }
            );
            clearerr(s.ptr);
        }
        assert!(s.bytes().is_empty());
    }
}
#[test]
fn short_reads_eof_sticky_and_seek_clear() {
    let s = Stream::new();
    unsafe {
        assert_eq!(fwrite(b"abcde".as_ptr().cast(), 1, 5, s.ptr), 5);
        rewind(s.ptr);
        FAULTS.with(|f| f.borrow_mut().reads.extend([Ok(1), Ok(2)]));
        let mut bytes = [0u8; 8];
        assert_eq!(fread(bytes.as_mut_ptr().cast(), 2, 4, s.ptr), 2);
        assert_eq!(&bytes[..5], b"abcde");
        assert_eq!(feof(s.ptr), 1);
        assert_eq!(ferror(s.ptr), 0);
        // EOF is sticky, even after an external reposition.
        assert_eq!(lseek(Fd(fileno(s.ptr) as u32), 0, 0).unwrap(), 0);
        assert_eq!(fgetc(s.ptr), EOF);
        clearerr(s.ptr);
        assert_eq!(fgetc(s.ptr), b'a' as i32);
        assert_eq!(fseek(s.ptr, 0, 0), 0);
        assert_eq!(feof(s.ptr), 0);
        assert_eq!(ftell(s.ptr), 0);
    }
}
#[test]
fn read_failure_is_distinct_from_eof_and_preserves_partial_items() {
    let s = Stream::new();
    unsafe {
        fputs(c"abcd".as_ptr().cast(), s.ptr);
        rewind(s.ptr);
        FAULTS.with(|f| f.borrow_mut().reads.extend([Ok(3), Err(sys::Errno::Again)]));
        let mut bytes = [0; 4];
        assert_eq!(fread(bytes.as_mut_ptr().cast(), 2, 2, s.ptr), 1);
        assert_eq!(&bytes[..3], b"abc");
        assert_eq!(ferror(s.ptr), 1);
        assert_eq!(feof(s.ptr), 0);
        assert_eq!(errno::get_errno(), errno::EAGAIN);
    }
}
#[test]
fn zero_sizes_overflow_and_wrong_direction() {
    let s = Stream::new();
    unsafe {
        assert_eq!(fwrite(core::ptr::null(), 0, usize::MAX, s.ptr), 0);
        assert_eq!(fread(core::ptr::null_mut(), 1, 0, s.ptr), 0);
        assert_eq!(ferror(s.ptr), 0);
        assert_eq!(fwrite(b"x".as_ptr().cast(), usize::MAX, 2, s.ptr), 0);
        assert_eq!(errno::get_errno(), errno::EINVAL);
        assert_eq!(ferror(s.ptr), 1);
        let path = std::ffi::CString::new(s.path.as_os_str().as_encoded_bytes()).unwrap();
        let read_only = fopen(path.as_ptr().cast(), c"r".as_ptr().cast());
        assert_eq!(fputs(c"".as_ptr().cast(), read_only), EOF);
        assert_eq!(errno::get_errno(), errno::EBADF);
        assert_eq!(ferror(read_only), 1);
        assert_eq!(fclose(read_only), 0);
        let write_only = fopen(path.as_ptr().cast(), c"a".as_ptr().cast());
        assert_eq!(fgetc(write_only), EOF);
        assert_eq!(ferror(write_only), 1);
        assert_eq!(fclose(write_only), 0);
    }
}
#[test]
fn modes_append_truncate_and_allocation_before_open() {
    let s = Stream::new();
    let path = std::ffi::CString::new(s.path.as_os_str().as_encoded_bytes()).unwrap();
    unsafe {
        fputs(c"abc".as_ptr().cast(), s.ptr);
        let a = fopen(path.as_ptr().cast(), c"ab+".as_ptr().cast());
        assert!(!a.is_null());
        assert_eq!(fseek(a, 0, 0), 0);
        assert_eq!(fputc(100, a), 100);
        assert_eq!(fclose(a), 0);
        assert_eq!(s.bytes(), b"abcd");
        for bad in [c"", c"q", c"r++", c"rbb", c"rx", c"wz"] {
            assert!(fopen(path.as_ptr().cast(), bad.as_ptr().cast()).is_null());
            assert_eq!(errno::get_errno(), errno::EINVAL);
        }
        assert!(fopen(path.as_ptr().cast(), c"wx".as_ptr().cast()).is_null());
        assert_eq!(s.bytes(), b"abcd");
        FAULTS.with(|f| f.borrow_mut().allocation_failure = true);
        assert!(fopen(path.as_ptr().cast(), c"w".as_ptr().cast()).is_null());
        assert_eq!(errno::get_errno(), errno::ENOMEM);
        assert_eq!(s.bytes(), b"abcd");
        FAULTS.with(|f| f.borrow_mut().allocation_failure = false);
        let w = fopen(path.as_ptr().cast(), c"wb".as_ptr().cast());
        assert!(!w.is_null());
        assert_eq!(fclose(w), 0);
        assert!(s.bytes().is_empty());
    }
}
#[test]
fn seek_and_close_failures_are_reported_and_close_consumes_fd() {
    let mut s = Stream::new();
    unsafe {
        FAULTS.with(|f| f.borrow_mut().seek_failure = true);
        assert_eq!(fseek(s.ptr, 0, 0), -1);
        assert_eq!(ferror(s.ptr), 1);
        rewind(s.ptr);
        assert_eq!(ferror(s.ptr), 0);
        assert_eq!(errno::get_errno(), errno::EINVAL);
        FAULTS.with(|f| f.borrow_mut().close_failure = true);
        let fd = fileno(s.ptr);
        assert_eq!(fclose(s.ptr), EOF);
        s.ptr = core::ptr::null_mut();
        assert_eq!(errno::get_errno(), errno::EIO);
        assert!(close(Fd(fd as u32)).is_err());
    }
}
#[test]
fn formatting_promotions_precision_limits_and_explicit_failures() {
    let s = Stream::new();
    unsafe {
        let n = fprintf(
            s.ptr,
            c"%+06d|%#x|%#o|%.0u|%hhd|%hu|%lld|%zu|%td|%-5.3s|%.*s|%%"
                .as_ptr()
                .cast(),
            -42i32,
            42u32,
            8u32,
            0u32,
            255i32,
            65537u32,
            i64::MIN,
            9usize,
            -3isize,
            c"hello".as_ptr(),
            3i32,
            b"abc".as_ptr(),
        );
        let expected = b"-00042|0x2a|010||-1|1|-9223372036854775808|9|-3|hel  |abc|%";
        assert_eq!(n, expected.len() as i32);
        assert_eq!(s.bytes(), expected);
        assert_eq!(fprintf(s.ptr, c"%f".as_ptr().cast(), 1.0f64), -1);
        assert_eq!(errno::get_errno(), errno::ENOSYS);
        assert_eq!(ferror(s.ptr), 1);
        clearerr(s.ptr);
        assert_eq!(
            fprintf(s.ptr, c"%n".as_ptr().cast(), core::ptr::null_mut::<i32>()),
            -1
        );
        assert_eq!(errno::get_errno(), errno::ENOSYS);
        assert_eq!(fprintf(s.ptr, c"%2147483648d".as_ptr().cast(), 1i32), -1);
        assert_eq!(errno::get_errno(), errno::E2BIG);
        assert_eq!(fprintf(s.ptr, c"%".as_ptr().cast()), -1);
        assert_eq!(errno::get_errno(), errno::EINVAL);
    }
}
#[test]
fn fgets_unsigned_character_and_short_format_write_failure() {
    let s = Stream::new();
    unsafe {
        fwrite(b"\xff\nxy".as_ptr().cast(), 1, 4, s.ptr);
        rewind(s.ptr);
        assert_eq!(fgetc(s.ptr), 255);
        // A pre-existing error indicator must not turn later partial EOF into
        // a new read failure. fseek rejects whence without changing position.
        assert_eq!(fseek(s.ptr, 0, 99), -1);
        assert_eq!(ferror(s.ptr), 1);
        let mut line = [42u8; 8];
        assert!(!fgets(line.as_mut_ptr(), 8, s.ptr).is_null());
        assert_eq!(&line[..2], b"\n\0");
        assert!(!fgets(line.as_mut_ptr(), 8, s.ptr).is_null());
        assert_eq!(&line[..3], b"xy\0");
        assert!(fgets(line.as_mut_ptr(), 8, s.ptr).is_null());
        FAULTS.with(|f| {
            f.borrow_mut()
                .writes
                .extend([Ok(2), Err(sys::Errno::NoSpace)])
        });
        assert_eq!(fprintf(s.ptr, c"hello %d".as_ptr().cast(), 42i32), -1);
        assert_eq!(errno::get_errno(), 28);
        assert_eq!(ferror(s.ptr), 1);
    }
}

#[test]
fn concurrent_formatted_calls_hold_the_stream_lock() {
    let s = Stream::new();
    let address = s.ptr as usize;
    let workers: Vec<_> = (0..4i32)
        .map(|worker| {
            std::thread::spawn(move || {
                for seq in 0..32i32 {
                    let result = unsafe {
                        fprintf(
                            address as *mut SunlightFile,
                            c"worker=%d seq=%02d\n".as_ptr().cast(),
                            worker,
                            seq,
                        )
                    };
                    assert_eq!(result, 16);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let bytes = s.bytes();
    let text = std::str::from_utf8(&bytes).unwrap();
    let lines: std::collections::HashSet<_> = text.lines().collect();
    assert_eq!(text.lines().count(), 128);
    assert_eq!(lines.len(), 128);
    for worker in 0..4 {
        for seq in 0..32 {
            assert!(lines.contains(format!("worker={worker} seq={seq:02}").as_str()));
        }
    }
}
