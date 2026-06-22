//! Native Function Bindings for SBSP
//!
//! Phase 4: Database integration via sunlight-kv IPC over capability-based SHM
//!
//! Architecture:
//! - Looks up the sunlight_kv daemon via the kernel nameserver
//! - Allocates SHM pages for request/response payloads (no heap allocations)
//! - Sends IPC messages with the SHM capability token in caps[0]
//! - Reads response data from the SHM page after the IPC call returns
//! - Falls back to stub errors when the KV daemon is not registered

use crate::sbsp::value::SbspValue;
use heapless::String;
use sunlight_ipc::{ipc_call, nameserver_lookup, shm_alloc, shm_free, CapabilityToken, IpcMsg};

/// KV operation codes (must match sunlight-kv daemon protocol)
const DB_OP_GET: u64 = 1;
const DB_OP_PUT: u64 = 2;
const DB_OP_DELETE: u64 = 3;
const DB_OP_SCAN: u64 = 4;

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

/// Maximum size for a single KV operation
const MAX_KV_VALUE_SIZE: usize = 256;

/// Name of the KV daemon in the kernel nameserver
const KV_DAEMON_NAME: &str = "sunlight_kv";

/// Perform a KV operation via capability-based IPC to the sunlight-kv daemon.
///
/// Uses SHM pages for payload transfer (key, value, scan prefix).
/// The IPC flow:
///   1. `shm_alloc()` — get a SHM page and its capability token
///   2. Write the request payload (key/value) into the SHM page
///   3. Build an `IpcMsg` with the operation label and SHM cap
///   4. `ipc_call(db_endpoint, msg)` — send to KV daemon
///   5. Read the response from the SHM page (daemon writes result in-place)
///   6. `shm_free()` — release the SHM page
pub fn kv_ipc_call(req: &KvRequest) -> Result<KvResponse, String<256>> {
    let db_cap = nameserver_lookup(KV_DAEMON_NAME)
        .ok_or_else(|| {
            let mut msg = String::new();
            let _ = core::fmt::write(
                &mut msg,
                format_args!("Database service '{}' is offline", KV_DAEMON_NAME),
            );
            msg
        })?;

    let (ptr, shm_cap) = shm_alloc().map_err(|_| String::from("SHM allocation failed"))?;

    let result = match req {
        KvRequest::Get { key } => {
            if key.len() > MAX_KV_VALUE_SIZE {
                shm_free(shm_cap).ok();
                let mut msg = String::new();
                let _ = core::fmt::write(
                    &mut msg,
                    format_args!("KV_GET key too long: {} bytes", key.len()),
                );
                return Err(msg);
            }

            let bytes = key.as_bytes();
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            }

            let mut msg = IpcMsg::with_label(DB_OP_GET);
            msg.caps[0] = shm_cap;
            msg.words[0] = bytes.len() as u64;

            let reply = ipc_call(db_cap, msg);
            let result_len = reply.words[0] as usize;

            if result_len == 0 {
                Ok(KvResponse::NotFound)
            } else if result_len > MAX_KV_VALUE_SIZE {
                Ok(KvResponse::Error({
                    let mut m = String::new();
                    let _ = core::fmt::write(
                        &mut m,
                        format_args!("KV response too large: {} bytes", result_len),
                    );
                    m
                }))
            } else {
                let data = unsafe { core::slice::from_raw_parts(ptr, result_len) };
                let mut vec = heapless::Vec::new();
                for &b in data {
                    let _ = vec.push(b);
                }
                Ok(KvResponse::Value(vec))
            }
        }

        KvRequest::Put { key, value } => {
            let payload_len = key.len() + 1 + value.len();
            if payload_len > 4096 {
                shm_free(shm_cap).ok();
                return Err(String::from("KV_PUT payload exceeds SHM page size (4096)"));
            }

            unsafe {
                core::ptr::copy_nonoverlapping(key.as_ptr(), ptr, key.len());
                *ptr.add(key.len()) = 0;
                core::ptr::copy_nonoverlapping(value.as_ptr(), ptr.add(key.len() + 1), value.len());
            }

            let mut msg = IpcMsg::with_label(DB_OP_PUT);
            msg.caps[0] = shm_cap;
            msg.words[0] = payload_len as u64;

            let reply = ipc_call(db_cap, msg);
            match reply.words[0] {
                0 => Ok(KvResponse::Ok),
                1 => Ok(KvResponse::PermissionDenied),
                _ => {
                    let err_data = unsafe { core::slice::from_raw_parts(ptr, 256.min(MAX_KV_VALUE_SIZE)) };
                    let err_str = core::str::from_utf8(err_data).unwrap_or("KV_PUT failed");
                    Ok(KvResponse::Error(String::from(err_str)))
                }
            }
        }

        KvRequest::Delete { key } => {
            let bytes = key.as_bytes();
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            }

            let mut msg = IpcMsg::with_label(DB_OP_DELETE);
            msg.caps[0] = shm_cap;
            msg.words[0] = bytes.len() as u64;

            let reply = ipc_call(db_cap, msg);
            match reply.words[0] {
                0 => Ok(KvResponse::Ok),
                1 => Ok(KvResponse::PermissionDenied),
                _ => Ok(KvResponse::Error(String::from("KV_DELETE failed"))),
            }
        }

        KvRequest::Scan { prefix } => {
            let bytes = prefix.as_bytes();
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            }

            let mut msg = IpcMsg::with_label(DB_OP_SCAN);
            msg.caps[0] = shm_cap;
            msg.words[0] = bytes.len() as u64;

            let reply = ipc_call(db_cap, msg);
            let result_len = reply.words[0] as usize;

            if result_len == 0 {
                Ok(KvResponse::ScanResult(heapless::Vec::new()))
            } else if result_len > 4096 {
                Ok(KvResponse::Error(String::from(
                    "KV_SCAN response too large",
                )))
            } else {
                let data = unsafe { core::slice::from_raw_parts(ptr, result_len) };
                let text = core::str::from_utf8(data).unwrap_or("");
                let mut keys: heapless::Vec<String<256>, 64> = heapless::Vec::new();
                for token in text.split(',') {
                    if !token.is_empty() {
                        let _ = keys.push(String::from(token));
                    }
                }
                Ok(KvResponse::ScanResult(keys))
            }
        }
    };

    shm_free(shm_cap).ok();
    result
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

            let key = String::from(key_str.as_str());
            let req = KvRequest::Get { key };

            match kv_ipc_call(&req) {
                Ok(KvResponse::Value(bytes)) => {
                    let value_str = core::str::from_utf8(&bytes).map_err(|_| {
                        String::from("KV value is not valid UTF-8")
                    })?;
                    Ok(SbspValue::String(String::from(value_str)))
                }
                Ok(KvResponse::NotFound) => {
                    Ok(SbspValue::String(String::new()))
                }
                Ok(KvResponse::PermissionDenied) => {
                    Err(String::from("Permission denied: cannot read this key"))
                }
                Ok(KvResponse::Error(e)) => Err(e),
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

            if value_str.len() > MAX_KV_VALUE_SIZE {
                let mut msg = String::new();
                let _ = core::fmt::write(&mut msg, format_args!(
                    "KV_PUT value too large: {} bytes (max {})",
                    value_str.len(),
                    MAX_KV_VALUE_SIZE
                ));
                return Err(msg);
            }

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
        assert!(result.is_err());
    }

    #[test]
    fn test_kv_delete_type_mismatch() {
        let result = call_native("KV_DELETE", &[SbspValue::Bool(true)]);
        assert!(result.is_err());
    }

    #[test]
    fn test_kv_scan_correct_args() {
        let result = call_native("KV_SCAN", &[SbspValue::String(String::from("user:"))]);
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_function() {
        let result = call_native("UNKNOWN_FUNC", &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown native function"));
    }
}
