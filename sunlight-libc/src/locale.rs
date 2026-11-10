//! Bounded native locale selection. Names describe the installed C/UTF-8
//! metadata subset, not a promise of collation, catalogs or conversion APIs.
//! Like POSIX setlocale, callers must serialize use of the returned LC_ALL
//! composite string against subsequent setlocale calls.
use core::cell::UnsafeCell;
use core::ffi::{c_char, c_int, CStr};
use core::sync::atomic::{AtomicBool, Ordering};

const KEYS: [&str; 6] = [
    "LC_CTYPE",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_COLLATE",
    "LC_MONETARY",
    "LC_MESSAGES",
];
const NAMES: [&[u8]; 3] = [b"C\0", b"C.UTF-8\0", b"en_US.UTF-8\0"];
struct State {
    selected: [u8; 6],
    composite: [u8; 256],
}
struct Registry {
    lock: AtomicBool,
    state: UnsafeCell<State>,
}
// All state access is protected by lock. The C caller must not modify strings.
unsafe impl Sync for Registry {}
static LOCALE: Registry = Registry {
    lock: AtomicBool::new(false),
    state: UnsafeCell::new(State {
        selected: [0; 6],
        composite: [0; 256],
    }),
};

fn identify(name: &[u8]) -> Option<u8> {
    match name {
        b"C" | b"POSIX" => Some(0),
        b"C.UTF-8" => Some(1),
        b"en_US.UTF-8" => Some(2),
        _ => None,
    }
}

fn config_value<'a>(config: &'a str, key: &str) -> Option<&'a str> {
    let mut result = None;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                let v = v.trim();
                let v = if v.len() >= 2
                    && ((v.starts_with('"') && v.ends_with('"'))
                        || (v.starts_with('\'') && v.ends_with('\'')))
                {
                    &v[1..v.len() - 1]
                } else {
                    v
                };
                result = Some(v);
            }
        }
    }
    result.filter(|v| !v.is_empty())
}

fn resolve(
    category: usize,
    env: impl Fn(&str) -> Option<&'static str>,
    config: &str,
) -> Option<u8> {
    // Environment has priority over system defaults. Empty values are ignored.
    let keys = ["LC_ALL", KEYS[category], "LANG"];
    for key in keys {
        if let Some(value) = env(key).filter(|v| !v.is_empty()) {
            return identify(value.as_bytes());
        }
    }
    for key in keys {
        if let Some(value) = config_value(config, key) {
            return identify(value.as_bytes());
        }
    }
    Some(0)
}

fn load_config(buffer: &mut [u8; 4096]) -> Option<usize> {
    #[cfg(target_os = "sunlightos")]
    {
        let fd = match crate::open(b"/etc/locale.conf") {
            Ok(fd) => fd,
            Err(crate::Errno::NoEntry) => return Some(0),
            Err(_) => return None,
        };
        let result = (|| {
            let mut used = 0;
            loop {
                if used == buffer.len() {
                    return None;
                }
                let n = crate::read(fd, &mut buffer[used..]).ok()?;
                if n == 0 {
                    return Some(used);
                }
                used += n;
            }
        })();
        let _ = crate::close(fd);
        result
    }
    #[cfg(not(target_os = "sunlightos"))]
    {
        let _ = buffer;
        Some(0)
    }
}

fn query(state: &mut State, category: usize) -> *mut c_char {
    if category < 6 {
        return NAMES[state.selected[category] as usize]
            .as_ptr()
            .cast_mut()
            .cast();
    }
    if state.selected.iter().all(|v| *v == state.selected[0]) {
        return NAMES[state.selected[0] as usize].as_ptr().cast_mut().cast();
    }
    let mut used = 0;
    for (i, key) in KEYS.iter().enumerate() {
        if i > 0 {
            state.composite[used] = b';';
            used += 1;
        }
        for bytes in [
            key.as_bytes(),
            b"=",
            &NAMES[state.selected[i] as usize][..NAMES[state.selected[i] as usize].len() - 1],
        ] {
            state.composite[used..used + bytes.len()].copy_from_slice(bytes);
            used += bytes.len();
        }
    }
    state.composite[used] = 0;
    state.composite.as_mut_ptr().cast()
}

/// Query with NULL; select defaults with ""; select an installed name explicitly.
/// Failed selections leave all categories unchanged. Initial locale is C.
/// # Safety
/// A non-NULL locale must point to a readable NUL-terminated string.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char {
    if !(0..=6).contains(&category) {
        return core::ptr::null_mut();
    }
    while LOCALE
        .lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    // Copy first: locale may be the previous LC_ALL composite buffer.
    let mut input = [0u8; 256];
    let name = if locale.is_null() {
        None
    } else {
        let bytes = CStr::from_ptr(locale).to_bytes();
        if bytes.len() >= input.len() {
            LOCALE.lock.store(false, Ordering::Release);
            return core::ptr::null_mut();
        }
        input[..bytes.len()].copy_from_slice(bytes);
        Some(&input[..bytes.len()])
    };
    let state = &mut *LOCALE.state.get();
    let result = (|| {
        if let Some(name) = name {
            let mut selected = state.selected;
            let mut buffer = [0; 4096];
            let needs_config = name.is_empty()
                && (0..6).any(|i| {
                    (category == 6 || category as usize == i)
                        && ["LC_ALL", KEYS[i], "LANG"].iter().all(|k| {
                            crate::env::getenv(k.as_bytes())
                                .filter(|v| !v.is_empty())
                                .is_none()
                        })
                });
            let config = if needs_config {
                let n = load_config(&mut buffer)?;
                core::str::from_utf8(&buffer[..n]).ok()?
            } else {
                ""
            };
            for (i, value) in selected.iter_mut().enumerate() {
                if category == 6 || category as usize == i {
                    *value = if name.is_empty() {
                        resolve(i, |key| crate::env::getenv(key.as_bytes()), config)?
                    } else {
                        if category == 6 && name.starts_with(b"LC_CTYPE=") {
                            let text = core::str::from_utf8(name).ok()?;
                            let fields: [&str; 6] = {
                                let mut fields = [""; 6];
                                let mut parts = text.split(';');
                                for field in &mut fields {
                                    *field = parts.next()?;
                                }
                                if parts.next().is_some() {
                                    return None;
                                }
                                fields
                            };
                            let (key, val) = fields[i].split_once('=')?;
                            if key != KEYS[i] {
                                return None;
                            }
                            identify(val.as_bytes())?
                        } else {
                            identify(name)?
                        }
                    };
                }
            }
            state.selected = selected;
        }
        Some(query(state, category as usize))
    })()
    .unwrap_or(core::ptr::null_mut());
    LOCALE.lock.store(false, Ordering::Release);
    result
}

pub fn ctype_is_utf8() -> bool {
    while LOCALE
        .lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let utf8 = unsafe { (*LOCALE.state.get()).selected[0] != 0 };
    LOCALE.lock.store(false, Ordering::Release);
    utf8
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_precedence() {
        assert_eq!(resolve(0, |_| None, "LANG=\"en_US.UTF-8\"\n"), Some(2));
        assert_eq!(
            resolve(
                0,
                |k| if k == "LANG" { Some("C.UTF-8") } else { None },
                "LC_ALL=C"
            ),
            Some(1)
        );
        assert_eq!(
            resolve(
                0,
                |k| if k == "LC_ALL" {
                    Some("bad")
                } else {
                    Some("C")
                },
                ""
            ),
            None
        );
        assert_eq!(
            resolve(0, |_| Some(""), "#LANG=bad\nLANG=C\nLANG='C.UTF-8'\n"),
            Some(1)
        );
        assert_eq!(resolve(0, |_| None, ""), Some(0));
    }
    #[test]
    fn category_and_config_precedence() {
        assert_eq!(
            resolve(
                0,
                |k| match k {
                    "LC_ALL" => Some("C"),
                    "LC_CTYPE" => Some("C.UTF-8"),
                    _ => Some("en_US.UTF-8"),
                },
                ""
            ),
            Some(0)
        );
        assert_eq!(
            resolve(
                0,
                |k| match k {
                    "LC_CTYPE" => Some("C.UTF-8"),
                    "LANG" => Some("en_US.UTF-8"),
                    _ => None,
                },
                ""
            ),
            Some(1)
        );
        assert_eq!(
            resolve(
                1,
                |_| None,
                "LANG=C\nLC_CTYPE=C.UTF-8\nLC_NUMERIC=en_US.UTF-8"
            ),
            Some(2)
        );
        assert_eq!(resolve(0, |_| None, "LANG=C\nLC_ALL=en_US.UTF-8"), Some(2));
        assert_eq!(resolve(0, |_| None, "LANG=unsupported"), None);
        assert_eq!(config_value("#LANG=C\nOTHER=x\nLANG=\n", "LANG"), None);
    }
    #[test]
    fn supported_names_are_bounded() {
        assert_eq!(identify(b"POSIX"), Some(0));
        assert_eq!(identify(b"de_DE.UTF-8"), None);
        assert_eq!(identify(b"en_US.UTF-8"), Some(2));
    }
}
