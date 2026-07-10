extern crate alloc;

use alloc::string::String;
use serde_json::Value;

pub fn pretty_json(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Some(String::new());
    }
    let value: Value = serde_json::from_str(trimmed).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

#[cfg(test)]
mod tests {
    use super::pretty_json;

    #[test]
    fn pretty_prints_minified_object() {
        let pretty = pretty_json("{\"ok\":true,\"items\":[1,2]}").unwrap();
        assert!(pretty.contains("\n  \"ok\": true,"));
        assert!(pretty.contains("\n  \"items\": ["));
    }

    #[test]
    fn rejects_unbalanced_json() {
        assert!(pretty_json("{\"ok\": true").is_none());
    }
}
