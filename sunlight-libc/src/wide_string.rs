//! Locale-independent operations on the native signed 32-bit wchar_t ABI.
#[no_mangle]
pub unsafe extern "C" fn wcslen(s: *const i32) -> usize {
    let mut n = 0;
    while *s.add(n) != 0 {
        n += 1;
    }
    n
}
#[no_mangle]
pub unsafe extern "C" fn wcscpy(d: *mut i32, s: *const i32) -> *mut i32 {
    let mut n = 0;
    loop {
        let c = *s.add(n);
        d.add(n).write(c);
        if c == 0 {
            return d;
        }
        n += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn wcsncpy(d: *mut i32, s: *const i32, n: usize) -> *mut i32 {
    let mut i = 0;
    while i < n {
        let c = *s.add(i);
        d.add(i).write(c);
        i += 1;
        if c == 0 {
            break;
        }
    }
    while i < n {
        d.add(i).write(0);
        i += 1;
    }
    d
}
#[no_mangle]
pub unsafe extern "C" fn wcsncmp(a: *const i32, b: *const i32, n: usize) -> i32 {
    for i in 0..n {
        let x = *a.add(i);
        let y = *b.add(i);
        if x != y {
            return if x < y { -1 } else { 1 };
        }
        if x == 0 {
            break;
        }
    }
    0
}
#[no_mangle]
pub unsafe extern "C" fn wcscmp(a: *const i32, b: *const i32) -> i32 {
    let mut i = 0;
    loop {
        let x = *a.add(i);
        let y = *b.add(i);
        if x != y {
            return if x < y { -1 } else { 1 };
        }
        if x == 0 {
            return 0;
        }
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn wcschr(s: *const i32, c: i32) -> *mut i32 {
    let mut p = s;
    loop {
        if *p == c {
            return p as *mut i32;
        }
        if *p == 0 {
            return core::ptr::null_mut();
        }
        p = p.add(1);
    }
}
#[no_mangle]
pub unsafe extern "C" fn wcsrchr(s: *const i32, c: i32) -> *mut i32 {
    let mut p = s;
    let mut found = core::ptr::null_mut();
    loop {
        if *p == c {
            found = p as *mut i32;
        }
        if *p == 0 {
            return found;
        }
        p = p.add(1);
    }
}
#[no_mangle]
pub unsafe extern "C" fn wmemchr(s: *const i32, c: i32, n: usize) -> *mut i32 {
    for i in 0..n {
        if *s.add(i) == c {
            return s.add(i) as *mut i32;
        }
    }
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn wcstok(s: *mut i32, delim: *const i32, save: *mut *mut i32) -> *mut i32 {
    let mut p = if s.is_null() { *save } else { s };
    if p.is_null() {
        return p;
    }
    while *p != 0 && !wcschr(delim, *p).is_null() {
        p = p.add(1);
    }
    if *p == 0 {
        save.write(core::ptr::null_mut());
        return core::ptr::null_mut();
    }
    let first = p;
    while *p != 0 {
        if !wcschr(delim, *p).is_null() {
            p.write(0);
            save.write(p.add(1));
            return first;
        }
        p = p.add(1);
    }
    save.write(core::ptr::null_mut());
    first
}
