/// Split a line into tokens on ASCII whitespace. No quoting or escaping.
/// Returns an empty Vec for blank/comment lines.
pub fn tokenize(line: &str) -> Vec<String> {
    line.split_ascii_whitespace()
        .map(|s| s.to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("   ").is_empty());
    }

    #[test]
    fn basic() {
        assert_eq!(tokenize("echo hello world"), vec!["echo", "hello", "world"]);
    }

    #[test]
    fn extra_spaces() {
        assert_eq!(tokenize("  cd  /tmp  "), vec!["cd", "/tmp"]);
    }
}
