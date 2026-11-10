#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    AlreadyExists,
    Busy,
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
    /// ENOSPC: mount byte or inode quota exhausted.
    NoSpace,
    /// ENOMEM: metadata allocator or PMM exhausted.
    NoMemory,
    Unsupported,
}
