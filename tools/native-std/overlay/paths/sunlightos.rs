use crate::ffi::{OsStr, OsString};
use crate::path::{Path, PathBuf};
use crate::{fmt, io, vec};
pub fn getcwd() -> io::Result<PathBuf> {
    let mut bytes = [0; 256];
    let count = crate::sys::cvt(unsafe { crate::sys::sl_std_getcwd(bytes.as_mut_ptr()) })?;
    Ok(PathBuf::from(unsafe {
        OsStr::from_encoded_bytes_unchecked(&bytes[..count])
    }))
}
pub fn chdir(path: &Path) -> io::Result<()> {
    let bytes = path.as_os_str().as_encoded_bytes();
    crate::sys::cvt(unsafe { crate::sys::sl_std_chdir(bytes.as_ptr(), bytes.len()) } as isize)
        .map(|_| ())
}
pub struct SplitPaths<'a>(vec::IntoIter<PathBuf>, core::marker::PhantomData<&'a OsStr>);
pub fn split_paths(paths: &OsStr) -> SplitPaths<'_> {
    SplitPaths(
        paths
            .as_encoded_bytes()
            .split(|&b| b == b':')
            .map(|bytes| PathBuf::from(unsafe { OsStr::from_encoded_bytes_unchecked(bytes) }))
            .collect::<Vec<_>>()
            .into_iter(),
        core::marker::PhantomData,
    )
}
impl Iterator for SplitPaths<'_> {
    type Item = PathBuf;
    fn next(&mut self) -> Option<PathBuf> {
        self.0.next()
    }
}
#[derive(Debug)]
pub struct JoinPathsError;
impl fmt::Display for JoinPathsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("path contains ':'")
    }
}
impl crate::error::Error for JoinPathsError {}
pub fn join_paths<I: Iterator<Item = T>, T: AsRef<OsStr>>(
    paths: I,
) -> Result<OsString, JoinPathsError> {
    let mut result = Vec::new();
    for (index, path) in paths.enumerate() {
        let bytes = path.as_ref().as_encoded_bytes();
        if bytes.contains(&b':') {
            return Err(JoinPathsError);
        }
        if index > 0 {
            result.push(b':');
        }
        result.extend_from_slice(bytes);
    }
    Ok(unsafe { OsString::from_encoded_bytes_unchecked(result) })
}
pub fn temp_dir() -> PathBuf {
    crate::env::var_os("TMPDIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}
pub fn home_dir() -> Option<PathBuf> {
    crate::env::var_os("HOME").map(PathBuf::from)
}
pub fn current_exe() -> io::Result<PathBuf> {
    crate::sys::unsupported()
}
