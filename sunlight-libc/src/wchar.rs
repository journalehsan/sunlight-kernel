//! Restartable C ASCII / UTF-8 decoding. Wide values use native signed i32.
use core::cell::UnsafeCell;
use core::ffi::c_char;
use core::sync::atomic::{AtomicBool, Ordering};

#[repr(C)]
#[derive(Default)]
pub struct MbState {
    value: u32,
    // Low byte: remaining bytes; next byte: total sequence length.
    progress: u32,
}
struct Shared {
    lock: AtomicBool,
    state: UnsafeCell<MbState>,
}
unsafe impl Sync for Shared {}
impl Shared {
    const fn new() -> Self {
        Self {
            lock: AtomicBool::new(false),
            state: UnsafeCell::new(MbState {
                value: 0,
                progress: 0,
            }),
        }
    }
}
static TOWC: Shared = Shared::new();
static LEN: Shared = Shared::new();

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn mbsinit(ps: *const MbState) -> i32 {
    (ps.is_null() || (*ps).progress == 0) as i32
}

unsafe fn decode(out: *mut i32, s: *const c_char, n: usize, st: &mut MbState) -> usize {
    if s.is_null() {
        return decode(core::ptr::null_mut(), b"\0".as_ptr().cast(), 1, st);
    }
    let utf8 = crate::locale::ctype_is_utf8();
    for i in 0..n {
        let b = *s.add(i) as u8;
        let bad = if st.progress == 0 {
            if b < 128 {
                if !out.is_null() {
                    *out = b as i32;
                }
                return if b == 0 { 0 } else { i + 1 };
            }
            let len = match b {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => 0,
            };
            if !utf8 || len == 0 {
                true
            } else {
                st.value = (b & (0x7f >> len)) as u32;
                st.progress = ((len as u32) << 8) | (len - 1) as u32;
                false
            }
        } else {
            let left = st.progress & 255;
            let len = st.progress >> 8;
            if !utf8
                || !(0x80..=0xbf).contains(&b)
                || (left == len - 1
                    && match len {
                        3 => (st.value == 0 && b < 0xa0) || (st.value == 13 && b >= 0xa0),
                        4 => (st.value == 0 && b < 0x90) || (st.value == 4 && b >= 0x90),
                        _ => false,
                    })
            {
                true
            } else {
                st.value = (st.value << 6) | (b & 63) as u32;
                st.progress -= 1;
                if left == 1 {
                    if !out.is_null() {
                        *out = st.value as i32;
                    }
                    *st = MbState::default();
                    return i + 1;
                }
                false
            }
        };
        if bad {
            // State is unspecified after error; caller must reinitialize.
            crate::errno::set_errno(crate::errno::EILSEQ);
            return usize::MAX;
        }
    }
    usize::MAX - 1
}
unsafe fn convert(
    out: *mut i32,
    s: *const c_char,
    n: usize,
    ps: *mut MbState,
    shared: &Shared,
) -> usize {
    if !ps.is_null() {
        return decode(out, s, n, &mut *ps);
    }
    while shared
        .lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let result = decode(out, s, n, &mut *shared.state.get());
    shared.lock.store(false, Ordering::Release);
    result
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn mbrtowc(
    out: *mut i32,
    s: *const c_char,
    n: usize,
    ps: *mut MbState,
) -> usize {
    convert(out, s, n, ps, &TOWC)
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn mbrlen(s: *const c_char, n: usize, ps: *mut MbState) -> usize {
    convert(core::ptr::null_mut(), s, n, ps, &LEN)
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn __sunlight_mb_cur_max() -> usize {
    if crate::locale::ctype_is_utf8() {
        4
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_scalars_and_all_split_boundaries() {
        unsafe {
            assert!(!crate::locale::setlocale(0, c"C.UTF-8".as_ptr()).is_null());
            for value in 0..=0x10ffff {
                let Some(ch) = char::from_u32(value) else {
                    continue;
                };
                let mut buf = [0; 4];
                let bytes = ch.encode_utf8(&mut buf).as_bytes();
                for split in 0..bytes.len() {
                    let mut st = MbState::default();
                    let mut out = -1;
                    assert_eq!(
                        decode(&mut out, bytes.as_ptr().cast(), split, &mut st),
                        usize::MAX - 1
                    );
                    assert_eq!(
                        decode(
                            &mut out,
                            bytes[split..].as_ptr().cast(),
                            bytes.len() - split,
                            &mut st
                        ),
                        if value == 0 { 0 } else { bytes.len() - split }
                    );
                    assert_eq!(out, value as i32);
                    assert_eq!(mbsinit(&st), 1);
                }
            }
            crate::locale::setlocale(0, c"C".as_ptr());
        }
    }
}
