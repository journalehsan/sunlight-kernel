use crate::ffi::{CStr, OsStr, OsString};
use crate::sync::{Mutex, OnceLock};
use crate::{fmt, io, vec};
static ENV: OnceLock<Mutex<Vec<(OsString, OsString)>>> = OnceLock::new();
fn store() -> &'static Mutex<Vec<(OsString, OsString)>> {
    ENV.get_or_init(|| {
        let mut result = Vec::new();
        for index in 0..128 {
            let ptr = unsafe { crate::sys::sl_std_env(index) };
            if ptr.is_null() {
                break;
            }
            let entry = unsafe { CStr::from_ptr(ptr.cast()) }.to_bytes();
            if let Some(split) = entry.iter().position(|&b| b == b'=') {
                let key =
                    unsafe { OsStr::from_encoded_bytes_unchecked(&entry[..split]) }.to_owned();
                let value =
                    unsafe { OsStr::from_encoded_bytes_unchecked(&entry[split + 1..]) }.to_owned();
                result.push((key, value));
            }
        }
        Mutex::new(result)
    })
}
pub struct Env(vec::IntoIter<(OsString, OsString)>);
impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.as_slice().fmt(f)
    }
}
impl Iterator for Env {
    type Item = (OsString, OsString);
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}
pub fn env() -> Env {
    Env(store().lock().unwrap().clone().into_iter())
}
pub fn getenv(key: &OsStr) -> Option<OsString> {
    store()
        .lock()
        .unwrap()
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}
fn validate(key: &OsStr) -> io::Result<()> {
    let bytes = key.as_encoded_bytes();
    if bytes.is_empty() || bytes.contains(&0) || bytes.contains(&b'=') {
        Err(io::Error::from_raw_os_error(22))
    } else {
        Ok(())
    }
}
pub unsafe fn setenv(key: &OsStr, value: &OsStr) -> io::Result<()> {
    validate(key)?;
    if value.as_encoded_bytes().contains(&0) {
        return Err(io::Error::from_raw_os_error(22));
    }
    let mut env = store().lock().unwrap();
    if let Some((_, old)) = env.iter_mut().find(|(k, _)| k == key) {
        *old = value.to_owned();
    } else {
        env.push((key.to_owned(), value.to_owned()));
    }
    Ok(())
}
pub unsafe fn unsetenv(key: &OsStr) -> io::Result<()> {
    validate(key)?;
    store().lock().unwrap().retain(|(k, _)| k != key);
    Ok(())
}
