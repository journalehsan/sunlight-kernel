//! Shared line-comparison primitives for `sort`, `uniq`, and `comm`.
//!
//! Portable/C locale byte comparison (memcmp ordering).
//! Supports field skipping, character skipping, numeric comparison,
//! case folding (ASCII only), and dictionary-order blank suppression.
//!
//! Does NOT implement locale-sensitive collation weights, equivalence
//! classes, or multi-character collation elements — those are conformance
//! gaps recorded for non-C locales.

/// Compare two byte slices in portable/C locale byte order.
/// Returns `Ordering::Less` if `a < b`, `Greater` if `a > b`, `Equal` if equal.
pub fn byte_cmp(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    let len = a.len().min(b.len());
    for i in 0..len {
        match a[i].cmp(&b[i]) {
            core::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    a.len().cmp(&b.len())
}

/// Skip `nfields` fields separated by one or more of the given delimiter bytes.
/// Returns the byte offset past the skipped fields, or `line.len()` if all
/// fields are consumed. Leading delimiters count as field boundaries.
pub fn skip_fields(line: &[u8], nfields: usize, delims: &[u8]) -> usize {
    if nfields == 0 {
        return 0;
    }
    let mut pos = 0;
    let mut fields = 0;
    let mut in_field = false;
    while pos < line.len() {
        let is_delim = delims.contains(&line[pos]);
        if is_delim {
            if in_field {
                fields += 1;
                in_field = false;
                if fields >= nfields {
                    return pos + 1;
                }
            }
        } else {
            in_field = true;
        }
        pos += 1;
    }
    if in_field {
        fields += 1;
    }
    if fields >= nfields {
        pos
    } else {
        line.len()
    }
}

/// Skip `nchars` characters (bytes in C locale) from the start of `line`.
/// Returns the byte offset past the skipped characters, bounded by `line.len()`.
pub fn skip_chars(line: &[u8], nchars: usize) -> usize {
    let mut count = 0;
    let mut pos = 0;
    while pos < line.len() && count < nchars {
        let b = line[pos];
        if b < 0x80 || b >= 0xC0 {
            count += 1;
        }
        pos += 1;
    }
    pos
}

/// Numeric comparison: interpret `a` and `b` as decimal integers (possibly with
/// leading sign and spaces). Returns the ordering, or `Equal` if parse fails on
/// both sides.
pub fn numeric_cmp(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    let (a_neg, a_digits) = parse_int_prefix(a);
    let (b_neg, b_digits) = parse_int_prefix(b);

    // skip leading zeros
    let a_trim = trim_leading_zeros(a_digits);
    let b_trim = trim_leading_zeros(b_digits);

    if a_trim.is_empty() && b_trim.is_empty() {
        return core::cmp::Ordering::Equal;
    }
    if a_neg && !b_neg {
        return core::cmp::Ordering::Less;
    }
    if !a_neg && b_neg {
        return core::cmp::Ordering::Greater;
    }

    let mult = if a_neg { -1 } else { 1 };

    if a_trim.len() != b_trim.len() {
        return cmp_i32(
            mult * (a_trim.len() as i32),
            mult * (b_trim.len() as i32),
        );
    }
    for i in 0..a_trim.len() {
        match a_trim[i].cmp(&b_trim[i]) {
            core::cmp::Ordering::Equal => continue,
            o => return cmp_i32(mult as i32, 0).then(o),
        }
    }
    core::cmp::Ordering::Equal
}

fn cmp_i32(a: i32, b: i32) -> core::cmp::Ordering {
    if a < b {
        core::cmp::Ordering::Less
    } else if a > b {
        core::cmp::Ordering::Greater
    } else {
        core::cmp::Ordering::Equal
    }
}

/// Parse leading integer from bytes. Returns (is_negative, digit_slice).
fn parse_int_prefix(s: &[u8]) -> (bool, &[u8]) {
    let s = skip_blanks(s);
    if s.is_empty() {
        return (false, s);
    }
    let neg = s[0] == b'-';
    let start = if neg || s[0] == b'+' { 1 } else { 0 };
    let end = s[start..]
        .iter()
        .position(|&b| !b.is_ascii_digit())
        .map_or(s.len(), |p| start + p);
    (neg, &s[start..end])
}

fn skip_blanks(b: &[u8]) -> &[u8] {
    let pos = b.iter().position(|&x| x != b' ' && x != b'\t');
    match pos {
        Some(p) => &b[p..],
        None => &b[b.len()..],
    }
}

fn trim_leading_zeros(b: &[u8]) -> &[u8] {
    let pos = b.iter().position(|&x| x != b'0');
    match pos {
        Some(p) => &b[p..],
        None => &b[b.len()..],
    }
}

/// Fold ASCII upper-case letters to lower-case in-place.
/// Returns the number of bytes folded.
pub fn fold_case_ascii(buf: &mut [u8]) -> usize {
    let mut count = 0;
    for b in buf.iter_mut() {
        if b.is_ascii_uppercase() {
            *b = b.to_ascii_lowercase();
            count += 1;
        }
    }
    count
}

/// Dictionary-order comparison: skip leading non-alphanumeric characters,
/// then compare byte-by-byte. Only letters, digits, and blanks are
/// "printing" in this simplified model.
pub fn dict_cmp(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    let a_trim = skip_non_print(a);
    let b_trim = skip_non_print(b);
    byte_cmp(a_trim, b_trim)
}

fn skip_non_print(b: &[u8]) -> &[u8] {
    let pos = b.iter().position(|&x| x.is_ascii_alphanumeric() || x == b' ');
    match pos {
        Some(p) => &b[p..],
        None => &b[b.len()..],
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use core::cmp::Ordering;

    #[test]
    fn byte_cmp_basic() {
        assert_eq!(byte_cmp(b"a", b"a"), Ordering::Equal);
        assert_eq!(byte_cmp(b"a", b"b"), Ordering::Less);
        assert_eq!(byte_cmp(b"b", b"a"), Ordering::Greater);
        assert_eq!(byte_cmp(b"ab", b"a"), Ordering::Greater);
        assert_eq!(byte_cmp(b"a", b"ab"), Ordering::Less);
    }

    #[test]
    fn byte_cmp_empty() {
        assert_eq!(byte_cmp(b"", b""), Ordering::Equal);
        assert_eq!(byte_cmp(b"", b"a"), Ordering::Less);
        assert_eq!(byte_cmp(b"a", b""), Ordering::Greater);
    }

    #[test]
    fn skip_fields_basic() {
        assert_eq!(skip_fields(b"a b c", 0, b" "), 0);
        assert_eq!(skip_fields(b"a b c", 1, b" "), 2);
        assert_eq!(skip_fields(b"a b c", 2, b" "), 4);
        assert_eq!(skip_fields(b"a b c", 3, b" "), 5);
        assert_eq!(skip_fields(b"a b c", 4, b" "), 5);
    }

    #[test]
    fn skip_fields_tab() {
        assert_eq!(skip_fields(b"a\tb\tc", 1, b"\t "), 2);
        assert_eq!(skip_fields(b"a\tb c", 2, b"\t "), 4);
    }

    #[test]
    fn skip_fields_leading_delim() {
        assert_eq!(skip_fields(b"  a b", 1, b" "), 3);
        assert_eq!(skip_fields(b"  a b", 2, b" "), 5);
    }

    #[test]
    fn skip_chars_basic() {
        assert_eq!(skip_chars(b"abc", 0), 0);
        assert_eq!(skip_chars(b"abc", 1), 1);
        assert_eq!(skip_chars(b"abc", 3), 3);
        assert_eq!(skip_chars(b"abc", 5), 3);
    }

    #[test]
    fn numeric_cmp_basic() {
        assert_eq!(numeric_cmp(b"0", b"0"), Ordering::Equal);
        assert_eq!(numeric_cmp(b"1", b"2"), Ordering::Less);
        assert_eq!(numeric_cmp(b"10", b"2"), Ordering::Greater);
        assert_eq!(numeric_cmp(b"  -5", b"-5"), Ordering::Equal);
        assert_eq!(numeric_cmp(b"-5", b"3"), Ordering::Less);
        assert_eq!(numeric_cmp(b"3", b"-5"), Ordering::Greater);
        assert_eq!(numeric_cmp(b"001", b"1"), Ordering::Equal);
    }

    #[test]
    fn fold_case_ascii_test() {
        let mut buf = *b"Hello WORLD";
        fold_case_ascii(&mut buf);
        assert_eq!(&buf, b"hello world");
    }

    #[test]
    fn dict_cmp_test() {
        assert_eq!(dict_cmp(b".hidden", b"hidden"), Ordering::Equal);
        assert_eq!(dict_cmp(b"  foo", b"foo"), Ordering::Equal);
    }
}
