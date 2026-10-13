pub fn errno() -> i32 {
    unsafe { crate::sys::sl_std_errno() }
}
pub fn is_interrupted(code: i32) -> bool {
    code == 4
}
pub fn decode_error_kind(code: i32) -> crate::io::ErrorKind {
    use crate::io::ErrorKind::*;
    match code {
        2 => NotFound,
        4 => Interrupted,
        9 => Uncategorized,
        11 => WouldBlock,
        13 => PermissionDenied,
        17 => AlreadyExists,
        20 => NotADirectory,
        21 => IsADirectory,
        22 => InvalidInput,
        12 => OutOfMemory,
        38 => Unsupported,
        _ => Uncategorized,
    }
}
pub fn error_string(code: i32) -> String {
    format!("native OS error {code}: {:?}", decode_error_kind(code))
}
