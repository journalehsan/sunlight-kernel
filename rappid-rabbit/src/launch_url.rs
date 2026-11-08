//! Bounded launch-argument helpers for initial browser navigation.

/// Return the first explicit browser URL from an argv-like byte list.
///
/// Unknown flags, launch-trace metadata, file paths, and malformed UTF-8 are
/// intentionally ignored. Internal URLs are passed to Rabbit's own resolver.
pub fn initial_url_from_values<'a>(values: &[&'a [u8]]) -> Option<&'a str> {
    values.iter().find_map(|value| {
        let text = core::str::from_utf8(value).ok()?;
        if text.starts_with("http://")
            || text.starts_with("https://")
            || text
                .get(..7)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("rappid:"))
        {
            Some(text)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::initial_url_from_values;

    #[test]
    fn accepts_bounded_http_url_and_ignores_trace_metadata() {
        assert_eq!(
            initial_url_from_values(&[
                b"--sunlight-launch=1:2:3",
                b"https://github.com/journalehsan/sunlight-kernel",
            ]),
            Some("https://github.com/journalehsan/sunlight-kernel")
        );
    }

    #[test]
    fn ignores_non_url_arguments() {
        assert_eq!(
            initial_url_from_values(&[b"--safe-mode", b"file:///tmp/a"]),
            None
        );
    }

    #[test]
    fn accepts_rappid_internal_urls() {
        assert_eq!(
            initial_url_from_values(&[b"rappid:startpage"]),
            Some("rappid:startpage")
        );
        assert_eq!(
            initial_url_from_values(&[b"rappid://startpage"]),
            Some("rappid://startpage")
        );
    }
}
