//! Hand-rolled argument parser — zero dependencies, no heap for fixed args.
//! Follows SunlightOS convention: explicit, no magic.

use std::string::String;

/// HTTP method supported by fetch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

impl HttpMethod {
    /// Parse from CLI string, case-insensitive
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "GET" | "get" | "Get" => Some(Self::Get),
            "POST" | "post" | "Post" => Some(Self::Post),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// Parsed CLI configuration — all fields determined at parse time
#[derive(Debug)]
pub struct FetchConfig {
    /// Target URL (required)
    pub url: String,

    /// HTTP method
    pub method: HttpMethod,

    /// POST body data (None for GET)
    /// String "-" means read from stdin
    pub post_data: Option<String>,

    /// Number of parallel download chunks
    pub chunks: usize,

    /// Explicit output filename (None = infer from URL)
    pub output: Option<String>,

    /// Show help and exit
    pub help: bool,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: HttpMethod::Get,
            post_data: None,
            chunks: 16,
            output: None,
            help: false,
        }
    }
}

/// Parse arguments from raw args slice (typically from env).
/// Returns Err(message) on invalid input.
pub fn parse_args(args: &[String]) -> Result<FetchConfig, String> {
    let mut config = FetchConfig::default();
    let mut i = 0;
    let mut url_found = false;

    while i < args.len() {
        let arg = args[i].as_str();

        match arg {
            "--help" | "-h" => {
                config.help = true;
                return Ok(config);
            }

            "-T" | "--method" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| String::from("-T/--method requires a value"))?;
                config.method = HttpMethod::from_str(val).ok_or_else(|| {
                    format!("unsupported HTTP method: '{val}' (use GET or POST)")
                })?;
            }

            "-d" | "--data" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| String::from("-d/--data requires a value"))?;
                config.post_data = Some(val.clone());
                // Implicitly set POST if user provides data but didn't set method
                if config.method == HttpMethod::Get {
                    config.method = HttpMethod::Post;
                }
            }

            "-c" | "--chunks" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| String::from("-c/--chunks requires a value"))?;
                config.chunks = val.parse::<usize>().map_err(|_| {
                    format!("invalid chunk count: '{val}' (must be positive integer)")
                })?;
                if config.chunks == 0 {
                    return Err(String::from("chunk count must be at least 1"));
                }
            }

            "-o" | "--output" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| String::from("-o/--output requires a value"))?;
                config.output = Some(val.clone());
            }

            _ if arg.starts_with('-') => {
                return Err(format!("unknown option: '{arg}'"));
            }

            _ => {
                if url_found {
                    return Err(format!(
                        "unexpected argument: '{arg}' (only one URL allowed)"
                    ));
                }
                config.url = args[i].clone();
                url_found = true;
            }
        }

        i += 1;
    }

    if !config.help && !url_found {
        return Err(String::from("no URL provided (see --help)"));
    }

    Ok(config)
}

/// Print usage information to the provided writer
pub fn print_help(writer: &mut dyn core::fmt::Write) {
    let _ = writer.write_str(
        "\
fetch — SunlightOS lightweight HTTP downloader

USAGE:
    fetch [OPTIONS] <URL>

OPTIONS:
    -T, --method <METHOD>   HTTP method: GET (default) or POST
    -d, --data <DATA>       POST body data (use '-' for stdin)
    -c, --chunks <NUM>      Parallel download chunks (default: 16)
    -o, --output <FILE>     Output filename (default: infer from URL)
    -h, --help              Show this help

EXAMPLES:
    fetch http://example.com
    fetch -o page.html http://example.com
    fetch -T POST -d 'key=value' http://httpbin.org/post
    fetch -c 8 -o large.bin http://mirror.example.com/file.iso

NOTES:
    Requires 'net' and 'vfs_write' capabilities.
    DNS resolves via /etc/hosts first, then hardcoded resolver.
    Chunked downloads use HTTP Range headers with automatic fallback.
",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(val: &str) -> String {
        String::from(val)
    }

    #[test]
    fn test_basic_url() {
        let args = alloc::vec![s("http://example.com")];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.url, "http://example.com");
        assert_eq!(config.method, HttpMethod::Get);
        assert_eq!(config.chunks, 16);
        assert!(config.output.is_none());
    }

    #[test]
    fn test_post_with_data() {
        let args = alloc::vec![s("-T"), s("POST"), s("-d"), s("hello"), s("http://x.com")];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.method, HttpMethod::Post);
        assert_eq!(config.post_data.as_deref(), Some("hello"));
    }

    #[test]
    fn test_implicit_post() {
        let args = alloc::vec![s("-d"), s("body"), s("http://x.com")];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.method, HttpMethod::Post);
    }

    #[test]
    fn test_chunks_and_output() {
        let args = alloc::vec![
            s("-c"),
            s("8"),
            s("-o"),
            s("out.bin"),
            s("http://x.com/file"),
        ];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.chunks, 8);
        assert_eq!(config.output.as_deref(), Some("out.bin"));
    }

    #[test]
    fn test_no_url_error() {
        let args: Vec<String> = alloc::vec![];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn test_zero_chunks_error() {
        let args = alloc::vec![s("-c"), s("0"), s("http://x.com")];
        assert!(parse_args(&args).is_err());
    }
}
