//! Launcher argument filtering for Sunlight Edit.

/// True when `arg` is launcher/session metadata, not a user file path.
pub fn is_ignored_launch_arg(arg: &str) -> bool {
    arg.is_empty()
        || arg.starts_with("--sunlight-")
        || arg.contains("--sunlight-launch")
        || arg.starts_with('?')
        || arg.starts_with('-')
}

/// Return the first argv entry that looks like a real file path.
pub fn extract_first_real_file_path<'a>(args: &'a [&'a str]) -> Option<&'a str> {
    for arg in args {
        if !is_ignored_launch_arg(arg) {
            return Some(arg);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_launch_metadata() {
        assert!(is_ignored_launch_arg(""));
        assert!(is_ignored_launch_arg("--sunlight-launch=2:shell:47330"));
        assert!(is_ignored_launch_arg("?"));
        assert!(is_ignored_launch_arg("-h"));
        assert!(is_ignored_launch_arg("--sunlight-foo"));
    }

    #[test]
    fn accepts_real_paths() {
        assert!(!is_ignored_launch_arg("/root/test.txt"));
        assert!(!is_ignored_launch_arg("notes.md"));
    }

    #[test]
    fn extract_skips_metadata() {
        let args = [
            "--sunlight-launch=2:shell:47330",
            "?",
            "/root/roadmap.md",
        ];
        assert_eq!(
            extract_first_real_file_path(&args),
            Some("/root/roadmap.md")
        );
    }

    #[test]
    fn extract_none_when_only_metadata() {
        let args = ["--sunlight-launch=1:menu:99", "-v"];
        assert_eq!(extract_first_real_file_path(&args), None);
    }
}