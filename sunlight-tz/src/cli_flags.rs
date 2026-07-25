//! Shared flag parsing for `tzutils` (host-testable).
//!
//! Combined short options: `-sf` is exactly equivalent to `--sync --force`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TzutilsArgs {
    pub sync: bool,
    pub force: bool,
    pub status: bool,
    pub help: bool,
}

/// Parse `tzutils` argv tokens (including optional program name at index 0).
pub fn parse_tzutils_args(argv: &[&str]) -> TzutilsArgs {
    let mut args = TzutilsArgs {
        sync: false,
        force: false,
        status: false,
        help: false,
    };
    let skip0 = argv
        .first()
        .map(|s| s.contains("tzutils") || s.ends_with("tzutils"))
        .unwrap_or(false);
    let iter = if skip0 { &argv[1..] } else { argv };
    for a in iter {
        if *a == "--sync" || *a == "-s" {
            args.sync = true;
        } else if *a == "--force" || *a == "-f" {
            args.force = true;
        } else if *a == "--status" || *a == "-S" {
            args.status = true;
        } else if *a == "--help" || *a == "-h" {
            args.help = true;
        } else if a.starts_with('-') && !a.starts_with("--") {
            for ch in a.bytes().skip(1) {
                match ch {
                    b's' => args.sync = true,
                    b'f' => args.force = true,
                    b'S' => args.status = true,
                    b'h' => args.help = true,
                    _ => {}
                }
            }
        }
    }
    if args.force {
        args.sync = true;
    }
    if !args.sync && !args.status && !args.help {
        args.status = true;
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sf_equals_sync_force() {
        let a = parse_tzutils_args(&["tzutils", "-sf"]);
        let b = parse_tzutils_args(&["tzutils", "--sync", "--force"]);
        assert_eq!(a, b);
        assert!(a.sync && a.force);
    }

    #[test]
    fn short_s_only() {
        let a = parse_tzutils_args(&["-s"]);
        assert!(a.sync && !a.force);
    }

    #[test]
    fn default_is_status() {
        let a = parse_tzutils_args(&["tzutils"]);
        assert!(a.status && !a.sync);
    }
}
