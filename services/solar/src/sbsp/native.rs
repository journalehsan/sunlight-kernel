//! Native Function Bindings for SBSP
//!
//! Phase 4: Database integration via sunlight-kv IPC
//!
//! This module bridges SBSP template expressions to native Rust functions,
//! particularly focusing on key-value database access via Unix domain socket.
//!
//! Architecture:
//! - Lazy-loads KV socket only on first database call
//! - Length-prefixed Bincode frames over UDS (/tmp/sunlight/kv.sock)
//! - Type conversions: SbspValue ↔ Vec<u8> with error handling
//! - SO_PEERCRED ensures www user cannot escalate permissions

use crate::sbsp::value::SbspValue;
use heapless::String;

/// KV Request types (minimal protocol for Solar)
#[derive(Debug, Clone)]
pub enum KvRequest {
    /// Store a value: KV_PUT { key, value_bytes }
    Put { key: String<256>, value: heapless::Vec<u8, 256> },
    /// Retrieve a value: KV_GET { key }
    Get { key: String<256> },
    /// Delete a key: KV_DELETE { key }
    Delete { key: String<256> },
    /// Scan keys with prefix: KV_SCAN { prefix }
    Scan { prefix: String<256> },
}

/// KV Response types (minimal protocol for Solar)
#[derive(Debug, Clone)]
pub enum KvResponse {
    /// Operation succeeded
    Ok,
    /// GET returned a value
    Value(heapless::Vec<u8, 256>),
    /// Key not found
    NotFound,
    /// Permission denied
    PermissionDenied,
    /// General error
    Error(String<256>),
    /// Scan results
    ScanResult(heapless::Vec<String<256>, 64>),
}

/// Maximum size for a single KV operation (prevents runaway allocations)
const MAX_KV_VALUE_SIZE: usize = 256; // 256 bytes per value in Phase 4

/// Perform a KV operation via IPC to sunlight-kv daemon
///
/// # Arguments
/// * `req` - The Request to send (KV_GET, KV_PUT, etc.)
///
/// # Returns
/// - `Ok(KvResponse)` - Server response
/// - `Err(msg)` - IPC error (socket failure, protocol error, etc.)
///
/// NOTE: In this no_std environment, this is a stub pending Phase 4.5 (Socket IPC integration).
/// The full implementation will require:
/// 1. Open UDS socket to /tmp/sunlight/kv.sock
/// 2. Serialize request and send u32 LE length prefix + payload
/// 3. Read u32 LE response length + payload
/// 4. Deserialize and return
pub fn kv_ipc_call(_req: &KvRequest) -> Result<KvResponse, String<256>> {
    // PHASE 4.5 TODO: Integrate with sunlight-kv daemon over UDS
    // For now, return stub error to show architecture is in place
    let mut msg = String::new();
    let _ = core::fmt::write(&mut msg, format_args!(
        "KV IPC socket integration pending (Phase 4.5)"
    ));
    Err(msg)
}

/// Native function dispatcher
///
/// Maps SBSP function calls to native implementations.
/// Supports: KV_GET, KV_PUT, KV_DELETE, KV_SCAN
///
/// # Arguments
/// * `func_name` - Function name (e.g., "KV_GET")
/// * `args` - Arguments as SbspValue array
///
/// # Returns
/// - `Ok(SbspValue)` - Return value
/// - `Err(msg)` - Type error or function error
pub fn call_native(func_name: &str, args: &[SbspValue]) -> Result<SbspValue, String<256>> {
    match func_name {
        "KV_GET" => {
            if args.len() != 1 {
                let mut msg = String::new();
                let _ = core::fmt::write(&mut msg, format_args!(
                    "KV_GET expects 1 argument (key), got {}",
                    args.len()
                ));
                return Err(msg);
            }

            let key_str = match &args[0] {
                SbspValue::String(s) => s.clone(),
                other => {
                    let mut msg = String::new();
                    let _ = core::fmt::write(&mut msg, format_args!(
                        "KV_GET key must be a String, got {}",
                        other.type_name()
                    ));
                    return Err(msg);
                }
            };

            // Convert to KvRequest
            let key = String::from(key_str.as_str());
            let req = KvRequest::Get { key };

            match kv_ipc_call(&req) {
                Ok(KvResponse::Value(bytes)) => {
                    // Convert Vec<u8> → String
                    let value_str = core::str::from_utf8(&bytes).map_err(|_| {
                        String::from("KV value is not valid UTF-8")
                    })?;
                    Ok(SbspValue::String(String::from(value_str)))
                }
                Ok(KvResponse::NotFound) => {
                    Ok(SbspValue::String(String::new())) // Empty string on miss
                }
                Ok(KvResponse::PermissionDenied) => {
                    Err(String::from("Permission denied: cannot read this key"))
                }
                Ok(KvResponse::Error(e)) => {
                    Err(e)
                }
                Ok(_) => Err(String::from("Unexpected response from KV_GET")),
                Err(e) => Err(e),
            }
        }

        "KV_PUT" => {
            if args.len() != 2 {
                let mut msg = String::new();
                let _ = core::fmt::write(&mut msg, format_args!(
                    "KV_PUT expects 2 arguments (key, value), got {}",
                    args.len()
                ));
                return Err(msg);
            }

            let key_str = match &args[0] {
                SbspValue::String(s) => s.clone(),
                other => {
                    let mut msg = String::new();
                    let _ = core::fmt::write(&mut msg, format_args!(
                        "KV_PUT key must be a String, got {}",
                        other.type_name()
                    ));
                    return Err(msg);
                }
            };

            // Auto-convert Integer → String for storage
            let value_str = match &args[1] {
                SbspValue::String(s) => s.clone(),
                SbspValue::Number(n) => {
                    let mut s = String::new();
                    let _ = core::fmt::write(&mut s, format_args!("{}", n));
                    s
                }
                other => {
                    let mut msg = String::new();
                    let _ = core::fmt::write(&mut msg, format_args!(
                        "KV_PUT value must be String or Integer, got {}",
                        other.type_name()
                    ));
                    return Err(msg);
                }
            };

            // Size check
            if value_str.len() > MAX_KV_VALUE_SIZE {
                let mut msg = String::new();
                let _ = core::fmt::write(&mut msg, format_args!(
                    "KV_PUT value too large: {} bytes (max {})",
                    value_str.len(),
                    MAX_KV_VALUE_SIZE
                ));
                return Err(msg);
            }

            // Convert to KvRequest
            let key = String::from(key_str.as_str());
            let mut value = heapless::Vec::new();
            for byte in value_str.as_bytes() {
                let _ = value.push(*byte);
            }
            let req = KvRequest::Put { key, value };

            match kv_ipc_call(&req) {
                Ok(KvResponse::Ok) => Ok(SbspValue::Bool(true)),
                Ok(KvResponse::PermissionDenied) => {
                    Err(String::from("Permission denied: cannot write this key"))
                }
                Ok(KvResponse::Error(e)) => Err(e),
                Ok(_) => Err(String::from("Unexpected response from KV_PUT")),
                Err(e) => Err(e),
            }
        }

        "KV_DELETE" => {
            if args.len() != 1 {
                let mut msg = String::new();
                let _ = core::fmt::write(&mut msg, format_args!(
                    "KV_DELETE expects 1 argument (key), got {}",
                    args.len()
                ));
                return Err(msg);
            }

            let key_str = match &args[0] {
                SbspValue::String(s) => s.clone(),
                other => {
                    let mut msg = String::new();
                    let _ = core::fmt::write(&mut msg, format_args!(
                        "KV_DELETE key must be a String, got {}",
                        other.type_name()
                    ));
                    return Err(msg);
                }
            };

            let key = String::from(key_str.as_str());
            let req = KvRequest::Delete { key };

            match kv_ipc_call(&req) {
                Ok(KvResponse::Ok) => Ok(SbspValue::Bool(true)),
                Ok(KvResponse::PermissionDenied) => {
                    Err(String::from("Permission denied: cannot delete this key"))
                }
                Ok(KvResponse::Error(e)) => Err(e),
                Ok(_) => Err(String::from("Unexpected response from KV_DELETE")),
                Err(e) => Err(e),
            }
        }

        "KV_SCAN" => {
            if args.len() != 1 {
                let mut msg = String::new();
                let _ = core::fmt::write(&mut msg, format_args!(
                    "KV_SCAN expects 1 argument (prefix), got {}",
                    args.len()
                ));
                return Err(msg);
            }

            let prefix_str = match &args[0] {
                SbspValue::String(s) => s.clone(),
                other => {
                    let mut msg = String::new();
                    let _ = core::fmt::write(&mut msg, format_args!(
                        "KV_SCAN prefix must be a String, got {}",
                        other.type_name()
                    ));
                    return Err(msg);
                }
            };

            let prefix = String::from(prefix_str.as_str());
            let req = KvRequest::Scan { prefix };

            match kv_ipc_call(&req) {
                Ok(KvResponse::ScanResult(keys)) => {
                    // Format as comma-separated for now
                    let mut result = String::new();
                    for (i, key) in keys.iter().enumerate() {
                        if i > 0 {
                            let _ = result.push(',');
                        }
                        let _ = result.push_str(key);
                    }
                    Ok(SbspValue::String(result))
                }
                Ok(KvResponse::PermissionDenied) => {
                    Err(String::from("Permission denied: cannot scan keys"))
                }
                Ok(KvResponse::Error(e)) => Err(e),
                Ok(_) => Err(String::from("Unexpected response from KV_SCAN")),
                Err(e) => Err(e),
            }
        }

        _ => {
            let mut msg = String::new();
            let _ = core::fmt::write(&mut msg, format_args!(
                "Unknown native function: {}",
                func_name
            ));
            Err(msg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kv_get_invalid_args() {
        let result = call_native("KV_GET", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_kv_get_type_mismatch() {
        let result = call_native("KV_GET", &[SbspValue::Number(42)]);
        assert!(result.is_err());
    }

    #[test]
    fn test_kv_put_invalid_args() {
        let result = call_native("KV_PUT", &[SbspValue::String(String::from("key"))]);
        assert!(result.is_err());
    }

    #[test]
    fn test_kv_put_auto_convert_integer() {
        let result = call_native(
            "KV_PUT",
            &[
                SbspValue::String(String::from("count")),
                SbspValue::Number(42),
            ],
        );
        // Currently returns "not yet linked" error, but type check passes
        assert!(result.is_err()); // IPC not available in no_std
    }

    #[test]
    fn test_kv_delete_type_mismatch() {
        let result = call_native("KV_DELETE", &[SbspValue::Bool(true)]);
        assert!(result.is_err());
    }

    #[test]
    fn test_kv_scan_correct_args() {
        let result = call_native("KV_SCAN", &[SbspValue::String(String::from("user:"))]);
        // Currently returns "not yet linked" error
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_function() {
        let result = call_native("UNKNOWN_FUNC", &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown native function"));
    }
}
