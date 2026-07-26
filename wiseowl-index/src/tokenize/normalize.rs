//! Text normalization for retrieval indexing only.
//!
//! Never mutates stored document payloads. Locale-independent.

use alloc::string::String;

/// Normalize input for WiseOwlLexicalV1 retrieval.
///
/// Rules (version 1):
/// - valid UTF-8 only (caller validates)
/// - CRLF → space separator normalization via whitespace collapse
/// - common whitespace → single space
/// - Latin case fold (ASCII A-Z → a-z)
/// - Persian/Arabic letter canonicalization:
///   - Arabic ي (U+064A) → Persian ی (U+06CC)
///   - Arabic ى (U+0649) → Persian ی (U+06CC)
///   - Arabic ك (U+0643) → Persian ک (U+06A9)
///   - tatweel ـ (U+0640) removed
/// - Arabic digits ٠-٩ → Latin 0-9
/// - Persian digits ۰-۹ → Latin 0-9
/// - strip Arabic combining diacritics (tashkeel) for retrieval
/// - ZWNJ (U+200C) treated as internal word boundary → space
/// - zero-width space / BOM-like stripped
pub fn normalize_for_retrieval(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_space = true;
    for ch in input.chars() {
        let mapped = map_char(ch);
        for mc in mapped {
            if mc.is_none() {
                continue;
            }
            let c = mc.unwrap();
            if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
                if !last_space && !out.is_empty() {
                    out.push(' ');
                    last_space = true;
                }
                continue;
            }
            out.push(c);
            last_space = false;
        }
    }
    // trim trailing space
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

fn map_char(ch: char) -> [Option<char>; 2] {
    // Returns up to 2 chars; None slots ignored.
    match ch {
        // whitespace
        ' ' | '\t' | '\n' | '\r' => [Some(' '), None],
        // ZWNJ / ZWSP → word boundary
        '\u{200C}' | '\u{200B}' | '\u{FEFF}' => [Some(' '), None],
        // tatweel
        '\u{0640}' => [None, None],
        // Arabic / Persian letters
        '\u{064A}' | '\u{0649}' => [Some('\u{06CC}'), None], // ي ى → ی
        '\u{0643}' => [Some('\u{06A9}'), None],              // ك → ک
        // Arabic diacritics (tashkeel) strip
        '\u{064B}'..='\u{065F}' | '\u{0670}' => [None, None],
        // Arabic-Indic digits
        '\u{0660}'..='\u{0669}' => {
            let d = (ch as u32 - 0x0660) as u8;
            [Some((b'0' + d) as char), None]
        }
        // Extended Arabic-Indic (Persian) digits
        '\u{06F0}'..='\u{06F9}' => {
            let d = (ch as u32 - 0x06F0) as u8;
            [Some((b'0' + d) as char), None]
        }
        // ASCII case fold
        'A'..='Z' => [Some((ch as u8 + 32) as char), None],
        // Arabic comma / semicolon → space separator for retrieval
        '\u{060C}' | '\u{061B}' | '\u{061F}' => [Some(' '), None],
        // keep letters, digits, common connector punctuation for identifiers
        c if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/' || c == ':' => {
            [Some(c), None]
        }
        // other punctuation → space
        c if c.is_ascii_punctuation() => [Some(' '), None],
        // other unicode punctuation → space
        c if is_unicode_punct(c) => [Some(' '), None],
        c => [Some(c), None],
    }
}

fn is_unicode_punct(c: char) -> bool {
    matches!(
        c,
        '«' | '»'
            | '،'
            | '؛'
            | '؟'
            | '…'
            | '—'
            | '–'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
    ) || (c as u32 >= 0x2000 && c as u32 <= 0x206F)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_casefold() {
        assert_eq!(normalize_for_retrieval("Hello World"), "hello world");
    }

    #[test]
    fn persian_yeh_kaf() {
        // Arabic yeh and kaf should match Persian forms
        let arabic = "يك";
        let persian = "یک";
        assert_eq!(
            normalize_for_retrieval(arabic),
            normalize_for_retrieval(persian)
        );
    }

    #[test]
    fn zwnj_boundary() {
        let s = "می‌روم"; // mi + ZWNJ + ravam style
        let n = normalize_for_retrieval(s);
        assert!(n.contains(' '));
    }

    #[test]
    fn digit_families() {
        assert_eq!(normalize_for_retrieval("١٢٣"), "123"); // Arabic-Indic
        assert_eq!(normalize_for_retrieval("۱۲۳"), "123"); // Persian
        assert_eq!(normalize_for_retrieval("123"), "123");
    }

    #[test]
    fn whitespace_collapse() {
        assert_eq!(normalize_for_retrieval("a \n\t  b"), "a b");
    }
}
