//! Hand-rolled argument parser — zero dependencies, no heap for fixed args.
//! Follows SunlightOS convention: explicit, no magic.

use crate::prelude::{String, Vec};

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

/// A DNS override scoped to one hostname and port; TLS still verifies the hostname.
#[derive(Debug)]
pub struct ResolveOverride {
    pub host: String,
    pub port: u16,
    pub ip: [u8; 4],
}

impl core::str::FromStr for ResolveOverride {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, String> {
        let error = || String::from("--resolve requires HOST:PORT:IPv4 with a nonzero port");
        let mut parts = value.split(':');
        let host = parts.next().ok_or_else(error)?;
        let port = parts
            .next()
            .ok_or_else(error)?
            .parse::<u16>()
            .map_err(|_| error())?;
        let address = parts.next().ok_or_else(error)?;
        if host.is_empty()
            || host.len() > 253
            || host
                .bytes()
                .any(|b| !b.is_ascii_alphanumeric() && b != b'.' && b != b'-')
            || port == 0
            || parts.next().is_some()
        {
            return Err(error());
        }
        let mut octets = address.split('.');
        let mut ip = [0; 4];
        for octet in &mut ip {
            let part = octets.next().ok_or_else(error)?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(error());
            }
            *octet = part.parse().map_err(|_| error())?;
        }
        if octets.next().is_some() {
            return Err(error());
        }
        Ok(Self {
            host: String::from(host),
            port,
            ip,
        })
    }
}

pub(crate) fn resolve_address(
    url: &crate::http::ParsedUrl,
    proxy: Option<sunlight_net::socks5::Proxy>,
    overrides: &[ResolveOverride],
) -> crate::error::FetchResult<crate::ipc::ResolvedAddr> {
    if proxy.is_none() {
        if let Some(entry) = overrides
            .iter()
            .rev()
            .find(|entry| entry.port == url.port && entry.host.eq_ignore_ascii_case(&url.host))
        {
            return Ok(crate::ipc::ResolvedAddr { octets: entry.ip });
        }
    }
    crate::ipc::resolve_for_request(&url.host, proxy)
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

    /// Explicit SOCKS5 proxy; the proxy resolves destination hostnames.
    pub socks5_proxy: Option<sunlight_net::socks5::Proxy>,

    pub resolve: Vec<ResolveOverride>,

    /// Show help and exit
    pub help: bool,
}

pub const SAMPLE_FILE_URL: &str =
    "https://edge10.82.ir.cdn.ir/soft/c/Codec.Tweak.Tool.6.7.6.rar?1781646885";

pub const SAMPLE_GET_REQUEST_URL: &str = "https://api.sampleapis.com/coffee/hot";

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: HttpMethod::Get,
            post_data: None,
            chunks: 16,
            output: None,
            socks5_proxy: None,
            resolve: Vec::new(),
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

            "--sample-file" => {
                config.url = String::from(SAMPLE_FILE_URL);
                url_found = true;
            }

            "--sample-get-request" => {
                config.url = String::from(SAMPLE_GET_REQUEST_URL);
                url_found = true;
            }

            "-T" | "--method" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| String::from("-T/--method requires a value"))?;
                config.method = HttpMethod::from_str(val)
                    .ok_or_else(|| format!("unsupported HTTP method: '{val}' (use GET or POST)"))?;
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

            "--resolve" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| String::from("--resolve requires HOST:PORT:IPv4"))?;
                config.resolve.push(value.parse()?);
            }

            "--socks5-hostname" => {
                i += 1;
                let value = args.get(i).ok_or_else(|| {
                    String::from("--socks5-hostname requires an IPv4 address and port")
                })?;
                config.socks5_proxy = Some(value.parse().map_err(String::from)?);
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

    if config.socks5_proxy.is_some() && !config.resolve.is_empty() {
        return Err(String::from(
            "--resolve cannot be combined with --socks5-hostname",
        ));
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
    fetch --sample-file

OPTIONS:
    -T, --method <METHOD>   HTTP method: GET (default) or POST
    -d, --data <DATA>       POST body data (use '-' for stdin)
    -c, --chunks <NUM>      Parallel download chunks (default: 16)
    -o, --output <FILE>     Output filename (default: infer from URL)
        --sample-file           Download a 1MB test file (plain HTTP)
        --sample-get-request    GET https://api.sampleapis.com/coffee/hot (JSON)
        --socks5-hostname <IP:PORT>  Use SOCKS5 with proxy-side DNS
        --resolve <HOST:PORT:IPv4>   Override DNS; repeat for multiple hosts
    -h, --help              Show this help

EXAMPLES:
    fetch http://example.com
    fetch -o page.html http://example.com
    fetch --socks5-hostname 10.0.2.2:9050 https://archlinux.org
    fetch --sample-file
    fetch -T POST -d 'key=value' http://httpbin.org/post

NOTES:
    Requires 'net' and 'vfs_write' capabilities.
    DNS uses --resolve overrides, then the system resolver.
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
    fn explicit_socks_proxy_and_direct_default() {
        let direct = parse_args(&[s("https://archlinux.org")]).unwrap();
        assert_eq!(direct.socks5_proxy, None);
        let proxied = parse_args(&[
            s("--socks5-hostname"),
            s("10.0.2.2:9050"),
            s("https://archlinux.org"),
        ])
        .unwrap();
        assert_eq!(proxied.socks5_proxy.unwrap().ip, [10, 0, 2, 2]);
        assert!(parse_args(&[s("--socks5-hostname")]).is_err());
        assert!(parse_args(&[
            s("--socks5-hostname"),
            s("10.0.2.2:0"),
            s("https://archlinux.org")
        ])
        .is_err());
    }

    #[test]
    fn resolve_override_validation_and_scope() {
        for invalid in [
            "",
            "host:0:1.2.3.4",
            "host:443:256.2.3.4",
            "host:443:1.2.3",
            "host:443:1.2.3.4:5",
            ":443:1.2.3.4",
        ] {
            assert!(invalid.parse::<ResolveOverride>().is_err(), "{invalid}");
        }
        let config = parse_args(&[
            s("--resolve"),
            s("127.0.0.1:443:10.0.2.2"),
            s("https://127.0.0.1"),
        ])
        .unwrap();
        let https = crate::http::ParsedUrl::parse("https://127.0.0.1").unwrap();
        assert_eq!(
            resolve_address(&https, None, &config.resolve)
                .unwrap()
                .octets,
            [10, 0, 2, 2]
        );
        let http = crate::http::ParsedUrl::parse("http://127.0.0.1").unwrap();
        assert_eq!(
            resolve_address(&http, None, &config.resolve)
                .unwrap()
                .octets,
            [127, 0, 0, 1]
        );
        assert!(parse_args(&[
            s("--resolve"),
            s("a:443:1.2.3.4"),
            s("--socks5-hostname"),
            s("10.0.2.2:9050"),
            s("https://a")
        ])
        .is_err());
    }

    #[test]
    fn test_basic_url() {
        let args = vec![s("http://example.com")];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.url, "http://example.com");
        assert_eq!(config.method, HttpMethod::Get);
        assert_eq!(config.chunks, 16);
        assert!(config.output.is_none());
    }

    #[test]
    fn test_post_with_data() {
        let args = vec![s("-T"), s("POST"), s("-d"), s("hello"), s("http://x.com")];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.method, HttpMethod::Post);
        assert_eq!(config.post_data.as_deref(), Some("hello"));
    }

    #[test]
    fn test_implicit_post() {
        let args = vec![s("-d"), s("body"), s("http://x.com")];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.method, HttpMethod::Post);
    }

    #[test]
    fn test_chunks_and_output() {
        let args = vec![
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
    fn test_sample_get_request() {
        let args = vec![s("--sample-get-request")];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.url, SAMPLE_GET_REQUEST_URL);
        assert_eq!(config.method, HttpMethod::Get);
    }

    #[test]
    fn test_no_url_error() {
        let args: Vec<String> = vec![];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn test_zero_chunks_error() {
        let args = vec![s("-c"), s("0"), s("http://x.com")];
        assert!(parse_args(&args).is_err());
    }
}
