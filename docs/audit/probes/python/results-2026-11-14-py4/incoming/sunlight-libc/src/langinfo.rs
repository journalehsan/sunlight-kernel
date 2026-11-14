//! Codeset metadata follows the active LC_CTYPE selection.
use core::ffi::{c_char, c_int};

/// Native nl_item value, shared with include/langinfo.h.
pub const CODESET: c_int = 1;
static C_CODESET: &[u8] = b"US-ASCII\0";
static UTF8_CODESET: &[u8] = b"UTF-8\0";
static EMPTY: &[u8] = b"\0";

/// Return process-lifetime, read-only C locale metadata. As with the C API,
/// callers must not modify the returned string despite its mutable ABI type.
/// Unsupported items return an empty string and leave errno unchanged.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn nl_langinfo(item: c_int) -> *mut c_char {
    let value = if item == CODESET {
        if crate::locale::ctype_is_utf8() {
            UTF8_CODESET
        } else {
            C_CODESET
        }
    } else {
        EMPTY
    };
    value.as_ptr().cast_mut().cast()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ffi::CStr;

    #[test]
    fn initial_c_codeset_has_static_lifetime() {
        let pointer = nl_langinfo(CODESET);
        assert_eq!(unsafe { CStr::from_ptr(pointer) }.to_bytes(), b"US-ASCII");
        assert_eq!(pointer, nl_langinfo(CODESET));
    }

    #[test]
    fn unsupported_items_do_not_invalidate_codeset() {
        let pointer = nl_langinfo(CODESET);
        for item in [c_int::MIN, -1, 0, CODESET + 1, c_int::MAX] {
            assert_eq!(unsafe { CStr::from_ptr(nl_langinfo(item)) }.to_bytes(), b"");
        }
        assert_eq!(unsafe { CStr::from_ptr(pointer) }.to_bytes(), b"US-ASCII");
    }
}
