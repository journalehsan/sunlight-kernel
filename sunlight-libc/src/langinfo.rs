//! Codeset follows LC_CTYPE; other metadata uses the installed C baseline.
use core::ffi::{c_char, c_int};

/// Native nl_item value, shared with include/langinfo.h.
pub const CODESET: c_int = 1;
static C_CODESET: &[u8] = b"US-ASCII\0";
static UTF8_CODESET: &[u8] = b"UTF-8\0";
static EMPTY: &[u8] = b"\0";

// Native ABI range shared with langinfo.h. Immutable process-lifetime data;
// returning it acquires no lock and never adopts caller storage.
const C_FIRST: c_int = 256;
static C_ITEMS: [&[u8]; 47] = [
    b"Sunday\0",               // DAY_1
    b"Monday\0",               // DAY_2
    b"Tuesday\0",              // DAY_3
    b"Wednesday\0",            // DAY_4
    b"Thursday\0",             // DAY_5
    b"Friday\0",               // DAY_6
    b"Saturday\0",             // DAY_7
    b"Sun\0",                  // ABDAY_1
    b"Mon\0",                  // ABDAY_2
    b"Tue\0",                  // ABDAY_3
    b"Wed\0",                  // ABDAY_4
    b"Thu\0",                  // ABDAY_5
    b"Fri\0",                  // ABDAY_6
    b"Sat\0",                  // ABDAY_7
    b"January\0",              // MON_1
    b"February\0",             // MON_2
    b"March\0",                // MON_3
    b"April\0",                // MON_4
    b"May\0",                  // MON_5
    b"June\0",                 // MON_6
    b"July\0",                 // MON_7
    b"August\0",               // MON_8
    b"September\0",            // MON_9
    b"October\0",              // MON_10
    b"November\0",             // MON_11
    b"December\0",             // MON_12
    b"Jan\0",                  // ABMON_1
    b"Feb\0",                  // ABMON_2
    b"Mar\0",                  // ABMON_3
    b"Apr\0",                  // ABMON_4
    b"May\0",                  // ABMON_5
    b"Jun\0",                  // ABMON_6
    b"Jul\0",                  // ABMON_7
    b"Aug\0",                  // ABMON_8
    b"Sep\0",                  // ABMON_9
    b"Oct\0",                  // ABMON_10
    b"Nov\0",                  // ABMON_11
    b"Dec\0",                  // ABMON_12
    b"%a %b %e %H:%M:%S %Y\0", // D_T_FMT
    b"%m/%d/%y\0",             // D_FMT
    b"%H:%M:%S\0",             // T_FMT
    b"AM\0",                   // AM_STR
    b"PM\0",                   // PM_STR
    b"%I:%M:%S %p\0",          // T_FMT_AMPM
    b".\0",                    // RADIXCHAR
    b"\0",                     // THOUSEP
    b"\0",                     // CRNCYSTR
];

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
    } else if let Some(index) = item
        .checked_sub(C_FIRST)
        .and_then(|n| usize::try_from(n).ok())
    {
        C_ITEMS.get(index).copied().unwrap_or(EMPTY)
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
