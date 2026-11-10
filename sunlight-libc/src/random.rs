//! C getrandom ABI over the authoritative capability-IPC random service.
use crate::{errno, sys};
use core::ffi::{c_uint, c_void};

/// # Safety
/// For nonzero len, a non-NULL pointer must identify len writable bytes with
/// exclusive access for this call. IPC replies arrive in registers, so there
/// is no kernel copy-to-user boundary that could recover arbitrary bad memory.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn getrandom(ptr: *mut c_void, len: usize, flags: c_uint) -> isize {
    getrandom_with(ptr, len, flags, |bytes| {
        // Never forward C flags: Rust's private bit 1 selects a local PRNG,
        // whereas C callers may interpret that value as GRND_NONBLOCK.
        let count = crate::rand::getrandom(bytes, 0);
        if count < 0 {
            // The existing secure Rust API returns only -1 on any service,
            // readiness, lookup, timeout or protocol failure, not -POSIX errno.
            Err(sys::Errno::Failed)
        } else {
            Ok(count as usize)
        }
    })
}

unsafe fn getrandom_with(
    ptr: *mut c_void,
    len: usize,
    flags: c_uint,
    fill: impl FnOnce(&mut [u8]) -> Result<usize, sys::Errno>,
) -> isize {
    let invalid = if flags != 0 || len > sys::MAX_IO_COUNT {
        Some(errno::EINVAL)
    } else if len != 0 && ptr.is_null() {
        Some(errno::EFAULT)
    } else {
        None
    };
    if let Some(error) = invalid {
        errno::set_errno(error);
        return -1;
    }
    if len == 0 {
        // Do not construct a Rust slice from NULL or any other empty pointer.
        return 0;
    }
    let bytes = core::slice::from_raw_parts_mut(ptr.cast::<u8>(), len);
    match fill(bytes) {
        Ok(count) if count <= len => count as isize,
        Ok(_) => {
            errno::set_errno(errno::EIO);
            -1
        }
        Err(error) => {
            errno::set_from_errno(error);
            -1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonempty_fill_preserves_errno_and_destination_guards() {
        for len in [1, 16, 31, 32, 33, 65, 257] {
            let mut storage = [0xabu8; 273];
            errno::set_errno(77);
            let result = unsafe {
                getrandom_with(storage.as_mut_ptr().add(7).cast(), len, 0, |bytes| {
                    assert_eq!(bytes.len(), len);
                    bytes.fill(0x42);
                    Ok(len)
                })
            };
            assert_eq!(result, len as isize);
            assert_eq!(errno::get_errno(), 77);
            assert!(storage[..7].iter().all(|&x| x == 0xab));
            assert!(storage[7..7 + len].iter().all(|&x| x == 0x42));
            assert!(storage[7 + len..].iter().all(|&x| x == 0xab));
        }
    }

    #[test]
    fn zero_length_never_touches_memory_or_service() {
        for ptr in [core::ptr::null_mut(), 1usize as *mut c_void] {
            errno::set_errno(77);
            assert_eq!(
                unsafe { getrandom_with(ptr, 0, 0, |_| panic!("empty IPC")) },
                0
            );
            assert_eq!(errno::get_errno(), 77);
        }
        // Exercise the actual export, without issuing native syscalls on Linux.
        assert_eq!(unsafe { getrandom(core::ptr::null_mut(), 0, 0) }, 0);
    }

    #[test]
    fn null_nonempty_buffer_is_efault_without_service_access() {
        assert_eq!(
            unsafe { getrandom_with(core::ptr::null_mut(), 1, 0, |_| panic!("NULL IPC")) },
            -1
        );
        assert_eq!(errno::get_errno(), errno::EFAULT);
    }

    #[test]
    fn every_nonzero_flag_including_private_prng_bit_is_rejected() {
        for flags in [crate::rand::GRND_NONCRYPTO, 2, 3, 4, 0x8000_0000, u32::MAX] {
            for len in [0, 1] {
                assert_eq!(
                    unsafe {
                        getrandom_with(1usize as *mut c_void, len, flags, |_| panic!("flag IPC"))
                    },
                    -1
                );
                assert_eq!(errno::get_errno(), errno::EINVAL);
            }
        }
    }

    #[test]
    fn oversized_request_is_einval_before_memory_access() {
        assert_eq!(
            unsafe {
                getrandom_with(core::ptr::null_mut(), usize::MAX, 0, |_| {
                    panic!("large IPC")
                })
            },
            -1
        );
        assert_eq!(errno::get_errno(), errno::EINVAL);
    }

    #[test]
    fn runtime_failures_and_impossible_counts_do_not_become_success() {
        let mut byte = 0u8;
        for (error, expected) in [
            (sys::Errno::Failed, errno::EIO),
            (sys::Errno::Again, errno::EAGAIN),
        ] {
            assert_eq!(
                unsafe { getrandom_with((&mut byte as *mut u8).cast(), 1, 0, |_| Err(error)) },
                -1
            );
            assert_eq!(errno::get_errno(), expected);
        }
        assert_eq!(
            unsafe { getrandom_with((&mut byte as *mut u8).cast(), 1, 0, |_| Ok(2)) },
            -1
        );
        assert_eq!(errno::get_errno(), errno::EIO);
    }
}
