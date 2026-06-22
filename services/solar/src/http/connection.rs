//! HTTP Connection Handler
//!
//! Per-connection thread execution model:
//! - Read TCP stream into fixed 8 KB stack buffer
//! - Parse into zero-copy Request struct (all slices)
//! - Route to SBSP engine or static file handler
//! - Send response
//! - Return to pool or exit
//!
//! Every thread is isolated: on error, it simply dies and its stack memory
//! is instantly reclaimed. No cleanup needed, no resource leaks.

use super::parser::{parse_request, HttpParseError};
use super::response::quick;
use crate::solar_log;

/// Parse and handle a single HTTP connection
///
/// # Arguments
/// * `buffer` - Pre-allocated 8 KB stack buffer with raw bytes from socket
/// * `bytes_read` - Number of bytes actually read from socket
/// * `writer` - Callable that writes response bytes (e.g., TcpStream::write_all)
///
/// # Returns
/// Whether the connection should stay alive (keep-alive support)
pub fn handle_http_request(
    buffer: &[u8],
    bytes_read: usize,
    mut writer: impl FnMut(&[u8]) -> Result<(), ()>,
) -> bool {
    if bytes_read == 0 {
        // Client closed connection cleanly
        return false;
    }

    // Parse the request from the buffer
    match parse_request(&buffer[..bytes_read]) {
        Ok(request) => {
            solar_log!("[SOLAR] {} {} (body: {} bytes)", request.method, request.path, request.body.len());

            // ==========================================
            // THE ROUTER
            // ==========================================
            let keep_alive = request.is_keep_alive();

            if request.is_sbsp() {
                // Phase 3: SBSP Script Execution
                // TODO: Pass to SBSP runtime
                // For now, send 501 Not Implemented
                let _ = writer(b"HTTP/1.1 501 Not Implemented\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                return false;
            } else {
                // Phase 3: Static File Serving
                // TODO: serve_static_file(&mut stream, request.path, &ctx.www_read_cap)
                // For now, send 501 Not Implemented
                let _ = writer(b"HTTP/1.1 501 Not Implemented\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                return false;
            }
        }
        Err(e) => {
            solar_log!("[SOLAR] Parse error: {:?}", e);
            let response = match e {
                HttpParseError::InvalidUtf8 => b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad Request" as &[u8],
                HttpParseError::MalformedRequestLine => b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad Request",
                HttpParseError::NoHeaderBodySeparator => b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad Request",
                HttpParseError::InvalidMethod => b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad Request",
                HttpParseError::InvalidHttpVersion => b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad Request",
                HttpParseError::MalformedHeader => b"HTTP/1.1 400 Bad Request\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad Request",
                HttpParseError::BufferOverflow => b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                HttpParseError::EmptyRequest => return false, // Silent close
            };
            let _ = writer(response);
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_get_routing() {
        let buffer = b"GET /index.html HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut response = heapless::Vec::<u8, 256>::new();

        let result = handle_http_request(buffer, buffer.len(), |data| {
            let _ = response.extend_from_slice(data);
            Ok(())
        });

        // Static file routing should return 501 (not implemented yet)
        assert!(response.windows(3).any(|w| w == b"501"));
    }

    #[test]
    fn test_sbsp_routing() {
        let buffer = b"GET /form.sbsp HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut response = heapless::Vec::<u8, 256>::new();

        let result = handle_http_request(buffer, buffer.len(), |data| {
            let _ = response.extend_from_slice(data);
            Ok(())
        });

        // SBSP routing should return 501 (not implemented yet)
        assert!(response.windows(3).any(|w| w == b"501"));
    }

    #[test]
    fn test_malformed_request() {
        let buffer = b"BROKEN HTTP/1.1\r\n\r\n"; // Invalid request line
        let mut response = heapless::Vec::<u8, 256>::new();

        let result = handle_http_request(buffer, buffer.len(), |data| {
            let _ = response.extend_from_slice(data);
            Ok(())
        });

        // Should return 400 Bad Request
        assert!(response.windows(3).any(|w| w == b"400"));
    }
}
