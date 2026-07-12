//! HTML character-reference decoding shared by both parser backends.

use alloc::{borrow::Cow, string::String};

use crate::named_entities::NAMED_ENTITIES;

pub const MAX_ENTITY_NAME_BYTES: usize = 64;
pub const MAX_NUMERIC_REFERENCE_DIGITS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterReferenceContext {
    Data,
    Attribute,
}

/// Decodes HTML references in one pass. Replacement text is never scanned
/// again, so `&amp;nbsp;` becomes the literal spelling `&nbsp;`.
pub fn decode_character_references(
    input: &str,
    context: CharacterReferenceContext,
) -> Cow<'_, str> {
    if !input.as_bytes().contains(&b'&') {
        return Cow::Borrowed(input);
    }

    let mut output = String::with_capacity(input.len());
    let mut copied_through = 0usize;
    let mut cursor = 0usize;
    while let Some(relative) = input[cursor..].find('&') {
        let amp = cursor + relative;
        output.push_str(&input[copied_through..amp]);
        if let Some(reference) = consume_reference(input, amp, context) {
            output.push(reference.first);
            if let Some(second) = reference.second {
                output.push(second);
            }
            cursor = reference.end;
            copied_through = cursor;
        } else {
            output.push('&');
            cursor = amp + 1;
            copied_through = cursor;
        }
    }
    output.push_str(&input[copied_through..]);
    Cow::Owned(output)
}

struct ConsumedReference {
    first: char,
    second: Option<char>,
    end: usize,
}

fn consume_reference(
    input: &str,
    amp: usize,
    context: CharacterReferenceContext,
) -> Option<ConsumedReference> {
    let bytes = input.as_bytes();
    if bytes.get(amp + 1) == Some(&b'#') {
        return consume_numeric_reference(input, amp);
    }

    let available_end = (amp + MAX_ENTITY_NAME_BYTES + 2).min(input.len());
    let available = &input[amp..available_end];
    let mut best: Option<(&str, u32, u32)> = None;
    for &(name, first, second) in &NAMED_ENTITIES {
        if name.len() <= available.len()
            && available.starts_with(name)
            && best.is_none_or(|(old, _, _)| name.len() > old.len())
        {
            best = Some((name, first, second));
        }
    }
    let (name, first, second) = best?;
    let end = amp + name.len();
    if context == CharacterReferenceContext::Attribute
        && !name.ends_with(';')
        && bytes
            .get(end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'=')
    {
        return None;
    }
    Some(ConsumedReference {
        first: char::from_u32(first).unwrap_or('\u{fffd}'),
        second: (second != 0).then(|| char::from_u32(second).unwrap_or('\u{fffd}')),
        end,
    })
}

fn consume_numeric_reference(input: &str, amp: usize) -> Option<ConsumedReference> {
    let bytes = input.as_bytes();
    let mut cursor = amp + 2;
    let radix = if matches!(bytes.get(cursor), Some(b'x' | b'X')) {
        cursor += 1;
        16u32
    } else {
        10u32
    };
    let digits_start = cursor;
    let mut value = 0u32;
    let mut overflowed = false;
    let mut digits = 0usize;
    while let Some(&byte) = bytes.get(cursor) {
        let digit = match radix {
            16 => (byte as char).to_digit(16),
            _ => (byte as char).to_digit(10),
        };
        let Some(digit) = digit else { break };
        digits += 1;
        if digits <= MAX_NUMERIC_REFERENCE_DIGITS {
            value = value
                .checked_mul(radix)
                .and_then(|current| current.checked_add(digit))
                .unwrap_or_else(|| {
                    overflowed = true;
                    0
                });
        } else {
            overflowed = true;
        }
        cursor += 1;
    }
    if cursor == digits_start {
        return None;
    }
    if bytes.get(cursor) == Some(&b';') {
        cursor += 1;
    }
    let scalar = if overflowed {
        '\u{fffd}'
    } else {
        normalize_numeric_reference(value)
    };
    Some(ConsumedReference {
        first: scalar,
        second: None,
        end: cursor,
    })
}

fn normalize_numeric_reference(value: u32) -> char {
    if value == 0 || value > 0x10ffff || (0xd800..=0xdfff).contains(&value) {
        return '\u{fffd}';
    }
    let remapped = match value {
        0x80 => 0x20ac,
        0x82 => 0x201a,
        0x83 => 0x0192,
        0x84 => 0x201e,
        0x85 => 0x2026,
        0x86 => 0x2020,
        0x87 => 0x2021,
        0x88 => 0x02c6,
        0x89 => 0x2030,
        0x8a => 0x0160,
        0x8b => 0x2039,
        0x8c => 0x0152,
        0x8e => 0x017d,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201c,
        0x94 => 0x201d,
        0x95 => 0x2022,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x98 => 0x02dc,
        0x99 => 0x2122,
        0x9a => 0x0161,
        0x9b => 0x203a,
        0x9c => 0x0153,
        0x9e => 0x017e,
        0x9f => 0x0178,
        _ => value,
    };
    char::from_u32(remapped).unwrap_or('\u{fffd}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(input: &str) -> String {
        decode_character_references(input, CharacterReferenceContext::Data).into_owned()
    }

    #[test]
    fn decodes_named_numeric_and_multi_scalar_references_once() {
        assert_eq!(
            data("&amp; &lt; &gt; &quot; &apos; &nbsp;"),
            "& < > \" ' \u{a0}"
        );
        assert_eq!(data("&copy; &#169; &#xA9; &#X1F600;"), "© © © 😀");
        assert_eq!(data("&NotEqualTilde;"), "\u{2242}\u{338}");
        assert_eq!(data("&amp;nbsp;"), "&nbsp;");
    }

    #[test]
    fn malformed_numeric_references_are_bounded_and_safe() {
        assert_eq!(data("&#0; &#xD800; &#1114112;"), "� � �");
        assert_eq!(data("&#999999999999999999999999999999;"), "�");
        assert_eq!(data("&#; &#x; &doesnotexist;"), "&#; &#x; &doesnotexist;");
    }

    #[test]
    fn omitted_semicolon_obeys_attribute_ambiguity_rule() {
        assert_eq!(data("&amp &nbsp"), "& \u{a0}");
        assert_eq!(
            decode_character_references("&amp=next", CharacterReferenceContext::Attribute),
            "&amp=next"
        );
        assert_eq!(
            decode_character_references("&amp!", CharacterReferenceContext::Attribute),
            "&!"
        );
    }
}
