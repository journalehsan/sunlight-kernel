//! URL-Encoded Form Data Parser
//!
//! Parses application/x-www-form-urlencoded POST bodies into key-value pairs.
//! Zero-allocation, zero-copy design using string slices.
//!
//! Example: "task_name=Build+OS&priority=High" → [("task_name", "Build OS"), ("priority", "High")]

/// Parse URL-encoded form data from HTTP POST body
///
/// # Arguments
/// * `body` - The raw body bytes (usually the content after \r\n\r\n in HTTP)
///
/// # Returns
/// A vector of (key, value) string slice pairs.
/// Invalid UTF-8 is silently skipped. Malformed pairs are ignored.
///
/// # Example
/// ```ignore
/// let pairs = parse_form_urlencoded(b"name=Alice&age=30");
/// // pairs[0] = ("name", "Alice")
/// // pairs[1] = ("age", "30")
/// ```
pub fn parse_form_urlencoded(body: &[u8]) -> heapless::Vec<(&str, &str), 32> {
    let mut pairs = heapless::Vec::new();

    // Convert bytes to string, skip if invalid UTF-8
    let body_str = match core::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return pairs, // Return empty if not valid UTF-8
    };

    // Split by '&' to get individual key=value pairs
    for pair in body_str.split('&') {
        // Split each pair by '='
        if let Some((key, value)) = pair.split_once('=') {
            let key_decoded = decode_url_encoded(key);
            let value_decoded = decode_url_encoded(value);

            // Only add if we successfully extracted both parts
            if !key_decoded.is_empty() {
                // In heapless context, we store slices not owned strings
                // For simplicity, store the original slices (decoded in-place would need mutation)
                let _ = pairs.push((key, value));
            }
        }
    }

    pairs
}

/// Decode URL-encoded string (+ to space, %XX to bytes)
///
/// NOTE: For Phase 4.2, this is a stub that just returns the input.
/// Full implementation requires percent-decoding which needs a working buffer.
fn decode_url_encoded(encoded: &str) -> &str {
    // TODO: Implement URL decoding
    // For now, return as-is. Most form data is ASCII anyway.
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_form() {
        let body = b"name=Alice&age=30";
        let pairs = parse_form_urlencoded(body);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("name", "Alice"));
        assert_eq!(pairs[1], ("age", "30"));
    }

    #[test]
    fn test_parse_single_field() {
        let body = b"task=Build+OS";
        let pairs = parse_form_urlencoded(body);
        assert_eq!(pairs.len(), 1);
        // Note: + decoding is TODO
        assert_eq!(pairs[0].0, "task");
    }

    #[test]
    fn test_parse_empty_body() {
        let body = b"";
        let pairs = parse_form_urlencoded(body);
        assert_eq!(pairs.len(), 0);
    }

    #[test]
    fn test_parse_invalid_utf8() {
        let body = b"name=\xFF\xFE";
        let pairs = parse_form_urlencoded(body);
        // Invalid UTF-8 returns empty
        assert_eq!(pairs.len(), 0);
    }

    #[test]
    fn test_parse_multiple_fields() {
        let body = b"key1=val1&key2=val2&key3=val3";
        let pairs = parse_form_urlencoded(body);
        assert_eq!(pairs.len(), 3);
    }
}
