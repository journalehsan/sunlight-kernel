//! ASCII byte classes for all currently supported native locales.
#[no_mangle]
pub extern "C" fn isalnum(c: i32) -> i32 {
    (isalpha(c) != 0 || isdigit(c) != 0) as i32
}
#[no_mangle]
pub extern "C" fn isalpha(c: i32) -> i32 {
    (isupper(c) != 0 || islower(c) != 0) as i32
}
#[no_mangle]
pub extern "C" fn isblank(c: i32) -> i32 {
    (c == 32 || c == 9) as i32
}
#[no_mangle]
pub extern "C" fn iscntrl(c: i32) -> i32 {
    ((0..32).contains(&c) || c == 127) as i32
}
#[no_mangle]
pub extern "C" fn isdigit(c: i32) -> i32 {
    ((48..=57).contains(&c)) as i32
}
#[no_mangle]
pub extern "C" fn isgraph(c: i32) -> i32 {
    ((33..=126).contains(&c)) as i32
}
#[no_mangle]
pub extern "C" fn islower(c: i32) -> i32 {
    ((97..=122).contains(&c)) as i32
}
#[no_mangle]
pub extern "C" fn isprint(c: i32) -> i32 {
    ((32..=126).contains(&c)) as i32
}
#[no_mangle]
pub extern "C" fn ispunct(c: i32) -> i32 {
    (isgraph(c) != 0 && isalnum(c) == 0) as i32
}
#[no_mangle]
pub extern "C" fn isspace(c: i32) -> i32 {
    (c == 32 || (9..=13).contains(&c)) as i32
}
#[no_mangle]
pub extern "C" fn isupper(c: i32) -> i32 {
    ((65..=90).contains(&c)) as i32
}
#[no_mangle]
pub extern "C" fn isxdigit(c: i32) -> i32 {
    (isdigit(c) != 0 || (65..=70).contains(&c) || (97..=102).contains(&c)) as i32
}
#[no_mangle]
pub extern "C" fn tolower(c: i32) -> i32 {
    if isupper(c) != 0 {
        c + 32
    } else {
        c
    }
}
#[no_mangle]
pub extern "C" fn toupper(c: i32) -> i32 {
    if islower(c) != 0 {
        c - 32
    } else {
        c
    }
}
