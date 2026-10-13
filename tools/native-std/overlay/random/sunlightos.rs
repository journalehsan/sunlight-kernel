pub fn fill_bytes(bytes: &mut [u8]) {
    assert_eq!(
        unsafe { crate::sys::sl_std_entropy(bytes.as_mut_ptr(), bytes.len()) },
        0,
        "native secure entropy unavailable"
    );
}
