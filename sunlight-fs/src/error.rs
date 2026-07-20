#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    AlreadyExists,
    NotDir,
    IsDir,
    UnexpectedType,
    InsecureMetadata,
    InvalidPath,
    BadHandle,
    TooManyOpenFiles,
    PermissionDenied,
    OperationNotPermitted,
    ReadOnlyFilesystem,
    Io,
    Unsupported,
}
