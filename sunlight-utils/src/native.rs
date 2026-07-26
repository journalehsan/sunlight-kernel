//! Shared fixed-buffer startup plumbing for native utility binaries.

use sunlight_libc::{crt0, MAX_ARGS};

/// Maximum payload bytes accepted for one argv string by the native utility
/// path. The extra scan byte distinguishes a missing terminator from a full
/// but valid payload without reading beyond the maintained bound.
pub const MAX_ARG_LENGTH: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgError {
    TooMany,
    TooLong,
}

/// Collect argv as bounded, byte-preserving slices owned by the exec-time
/// stack arena. Sunlight's current ABI guarantees UTF-8 argv, but retaining
/// bytes here avoids silently changing pathname text on the utility side.
///
/// # Safety
/// `argc` and `argv` must be the values supplied by the native `_start` ABI.
pub unsafe fn collect_bytes<'a>(
    argc: u64,
    argv: *const *const u8,
    out: &mut [&'a [u8]],
) -> Result<usize, ArgError> {
    if argc as usize > out.len() || argc as usize > MAX_ARGS + 1 {
        return Err(ArgError::TooMany);
    }

    let mut pointers = [core::ptr::null::<u8>(); MAX_ARGS + 1];
    let count = crt0::collect_raw_args(argc, argv, &mut pointers);
    for index in 0..count {
        let pointer = pointers[index];
        let length = crt0::cstr_len(pointer, MAX_ARG_LENGTH + 1);
        if length > MAX_ARG_LENGTH {
            return Err(ArgError::TooLong);
        }
        out[index] = core::slice::from_raw_parts(pointer, length);
    }
    Ok(count)
}

/// Remove argv[0] from a native argument slice.
pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}
