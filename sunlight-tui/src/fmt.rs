//! Number formatting without heap allocation

#![allow(dead_code)]

/// Format u32 as decimal into a fixed buffer
/// Returns the slice of `buf` that was written
pub fn fmt_u32<'a>(buf: &'a mut [u8; 20], n: u32) -> &'a str {
    if n == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[0..1]) };
    }
    
    let mut num = n;
    let mut pos = 19;
    
    while num > 0 {
        buf[pos] = b'0' + (num % 10) as u8;
        num /= 10;
        if pos == 0 { break; }
        pos -= 1;
    }
    
    unsafe { core::str::from_utf8_unchecked(&buf[pos + 1..20]) }
}

/// Format u64 as hex (e.g., "0xFFFF800000000000") into fixed buffer
pub fn fmt_hex<'a>(buf: &'a mut [u8; 20], n: u64) -> &'a str {
    buf[0] = b'0';
    buf[1] = b'x';
    
    for i in 0..16 {
        let nibble = ((n >> (60 - i * 4)) & 0xF) as u8;
        buf[2 + i] = if nibble < 10 {
            b'0' + nibble
        } else {
            b'A' + (nibble - 10)
        };
    }
    
    unsafe { core::str::from_utf8_unchecked(&buf[0..18]) }
}

/// Format "X/Y MiB" into fixed buffer
pub fn fmt_mib<'a>(buf: &'a mut [u8; 32], used: u32, total: u32) -> &'a str {
    let mut pos = 0;
    
    // Format used
    let mut temp = [0u8; 20];
    let used_str = fmt_u32(&mut temp, used);
    for &b in used_str.as_bytes() {
        buf[pos] = b;
        pos += 1;
    }
    
    buf[pos] = b'/';
    pos += 1;
    
    // Format total
    let total_str = fmt_u32(&mut temp, total);
    for &b in total_str.as_bytes() {
        buf[pos] = b;
        pos += 1;
    }
    
    // Add " MiB"
    buf[pos] = b' ';
    buf[pos + 1] = b'M';
    buf[pos + 2] = b'i';
    buf[pos + 3] = b'B';
    pos += 4;
    
    unsafe { core::str::from_utf8_unchecked(&buf[0..pos]) }
}
