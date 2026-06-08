#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    NotDir,
    IsDir,
    InvalidPath,
    BadHandle,
    TooManyOpenFiles,
    PermissionDenied,
    Io,
    Unsupported,
}
