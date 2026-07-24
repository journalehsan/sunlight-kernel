//! Minimal IEEE CRC-32 (ISO-HDLC / PNG / ZIP polynomial).
//!
//! Used optionally by SIMG v2 over uncompressed canonical pixel bytes.
//! Not a cryptographic authentication primitive.

/// IEEE CRC-32 (poly 0xEDB88320, init 0xFFFFFFFF, xor-out 0xFFFFFFFF).
pub fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320u32 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_empty_and_known_vector() {
        assert_eq!(crc32_ieee(b""), 0);
        // "123456789" -> 0xCBF43926 (standard IEEE check vector)
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
    }
}
