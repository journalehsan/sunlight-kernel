//! Native LP64 intmax conversions; ASCII C-locale subject sequence.
use core::ffi::{c_char, c_void};

#[repr(C)]
pub struct Imaxdiv {
    quot: i64,
    rem: i64,
}
#[no_mangle]
pub extern "C" fn imaxabs(x: i64) -> i64 {
    // INTMAX_MIN has no representable absolute value (C undefined behavior).
    x.wrapping_abs()
}
#[no_mangle]
pub extern "C" fn imaxdiv(x: i64, y: i64) -> Imaxdiv {
    Imaxdiv {
        quot: x / y,
        rem: x % y,
    }
}

unsafe fn parse<T: Copy + Into<i64>>(
    s: *const T,
    end: *mut *mut T,
    base: i32,
    signed: bool,
) -> u64 {
    let mut p = s;
    if !end.is_null() {
        end.write(s as *mut T);
    }
    if base != 0 && !(2..=36).contains(&base) {
        crate::errno::set_errno(crate::errno::EINVAL);
        return 0;
    }
    let get = |p: *const T| -> i64 { (*p).into() };
    while matches!(get(p), 9..=13 | 32) {
        p = p.add(1);
    }
    let negative = get(p) == 45;
    if negative || get(p) == 43 {
        p = p.add(1);
    }
    let digit = |c: i64| -> u32 {
        match c {
            48..=57 => (c - 48) as u32,
            65..=90 => (c - 65 + 10) as u32,
            97..=122 => (c - 97 + 10) as u32,
            _ => 36,
        }
    };
    let mut radix = base as u32;
    if (radix == 0 || radix == 16)
        && get(p) == 48
        && matches!(get(p.add(1)), 88 | 120)
        && digit(get(p.add(2))) < 16
    {
        radix = 16;
        p = p.add(2);
    } else if radix == 0 {
        radix = if get(p) == 48 { 8 } else { 10 };
    }
    let start = p;
    let limit = if signed {
        if negative {
            1u64 << 63
        } else {
            i64::MAX as u64
        }
    } else {
        u64::MAX
    };
    let mut value = 0u64;
    let mut overflow = false;
    loop {
        let d = digit(get(p));
        if d >= radix {
            break;
        }
        if value > limit / radix as u64
            || (value == limit / radix as u64 && d as u64 > limit % radix as u64)
        {
            overflow = true;
        } else if !overflow {
            value = value * radix as u64 + d as u64;
        }
        p = p.add(1);
    }
    if p == start {
        return 0;
    }
    if !end.is_null() {
        end.write(p as *mut T);
    }
    if overflow {
        crate::errno::set_errno(34);
        return if signed && negative {
            1u64 << 63
        } else {
            limit
        };
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}
#[no_mangle]
pub unsafe extern "C" fn strtoimax(s: *const c_char, e: *mut *mut c_char, base: i32) -> i64 {
    parse(s.cast::<u8>(), e.cast(), base, true) as i64
}
#[no_mangle]
pub unsafe extern "C" fn strtoumax(s: *const c_char, e: *mut *mut c_char, base: i32) -> u64 {
    parse(s.cast::<u8>(), e.cast(), base, false)
}
#[no_mangle]
pub unsafe extern "C" fn wcstoimax(s: *const i32, e: *mut *mut i32, base: i32) -> i64 {
    parse(s, e, base, true) as i64
}
#[no_mangle]
pub unsafe extern "C" fn wcstoumax(s: *const i32, e: *mut *mut i32, base: i32) -> u64 {
    parse(s, e, base, false)
}

#[no_mangle]
pub unsafe extern "C" fn __sunlight_assert_fail(
    expr: *const c_char,
    file: *const c_char,
    line: u32,
    function: *const c_char,
) -> ! {
    extern "C" {
        static mut stderr: *mut c_void;
        fn fprintf(stream: *mut c_void, format: *const c_char, ...) -> i32;
    }
    fprintf(
        stderr,
        c"%s:%u: %s: Assertion `%s' failed.\n".as_ptr(),
        file,
        line,
        function,
        expr,
    );
    // Abnormal, callback-free process termination. Native SIGABRT delivery is
    // not supported; assert has no successful return or atexit path.
    crate::std_runtime::sl_std_exit(134)
}

// The native ABI has 64-bit long, long long and intmax_t.
#[no_mangle]
pub unsafe extern "C" fn strtol(s: *const c_char, end: *mut *mut c_char, base: i32) -> i64 {
    parse(s, end, base, true) as i64
}

#[no_mangle]
pub unsafe extern "C" fn strtoul(s: *const c_char, end: *mut *mut c_char, base: i32) -> u64 {
    parse(s, end, base, false)
}
#[no_mangle]
pub unsafe extern "C" fn wcstol(s: *const i32, end: *mut *mut i32, base: i32) -> i64 {
    parse(s, end, base, true) as i64
}
