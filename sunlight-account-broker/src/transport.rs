//! Single POST over the existing sunlight-fetch transport. Native feature uses
//! sunlight-tls IPC and its KV trust store; host feature uses verified rustls.
use crate::{Error, Result, TokenResponse, TokenTransport};
use alloc::{string::String, vec};
use sunlight_fetch::{
    http::{HttpRequest, ParsedUrl},
    ipc,
};
use zeroize::Zeroizing;

pub struct VerifiedTls;
impl TokenTransport for VerifiedTls {
    fn exchange(&mut self, endpoint: &str, form: &str) -> Result<TokenResponse> {
        if !ParsedUrl::parse(endpoint)
            .map_err(|_| Error::Configuration)?
            .uses_tls()
        {
            return Err(Error::Configuration);
        }
        #[cfg(feature = "host-runtime")]
        {
            deadline_exchange(endpoint, form, std::time::Duration::from_secs(25))
        }
        #[cfg(not(feature = "host-runtime"))]
        {
            let _deadline = ipc::NativeDeadline::new(25_000).map_err(|_| Error::Capacity)?;
            let result = exchange_once(endpoint, form);
            if ipc::native_deadline_expired() {
                Err(Error::Timeout)
            } else {
                result
            }
        }
    }
}

fn exchange_once(endpoint: &str, form: &str) -> Result<TokenResponse> {
    let url = ParsedUrl::parse(endpoint).map_err(|_| Error::Configuration)?;
    if !url.uses_tls() {
        return Err(Error::Configuration);
    }
    ipc::acquire_capabilities().map_err(|_| Error::Tls)?;
    let addr = ipc::dns_resolve(&url.host).map_err(|_| Error::Tls)?;
    let mut request = HttpRequest {
        method: "POST",
        path: url.path.clone(),
        host: url.host_header(),
        headers: vec![
            (
                String::from("Content-Type"),
                String::from("application/x-www-form-urlencoded"),
            ),
            (String::from("Accept"), String::from("application/json")),
        ],
        body: Some(form.as_bytes().to_vec()),
    };
    // Deliberately bypass the higher-level redirect-following backend:
    // a 307 must never forward a verifier/code to a different endpoint.
    let result = ipc::http_request(&url.host, &addr, url.port, true, &request);
    if let Some(body) = request.body.as_mut() {
        use zeroize::Zeroize;
        body.zeroize();
    }
    let (response, mut handle, initial) = result.map_err(|_| Error::Tls)?;
    let body = ipc::read_body_limited(&mut handle, "POST", &response, &initial, 16384);
    let _ = handle.close();
    let body = Zeroizing::new(body.map_err(|_| Error::Tls)?);
    Ok(TokenResponse {
        status: response.status_code,
        body: Zeroizing::new(String::from_utf8(body.to_vec()).map_err(|_| Error::MalformedToken)?),
    })
}

// A stalled host TLS backend must not strand the coordinator. There is at most
// one outstanding worker, including after timeout; retries fail closed while
// that worker is still blocked. Native cancellation/deadlines remain blocked.
#[cfg(feature = "host-runtime")]
fn deadline_exchange(
    endpoint: &str,
    form: &str,
    timeout: std::time::Duration,
) -> Result<TokenResponse> {
    use core::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{sync_channel, RecvTimeoutError};
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    if ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(Error::Capacity);
    }
    let endpoint = String::from(endpoint);
    let form = Zeroizing::new(String::from(form));
    let (send, recv) = sync_channel(1);
    if std::thread::Builder::new()
        .name(String::from("oauth-tls"))
        .spawn(move || {
            struct Release;
            impl Drop for Release {
                fn drop(&mut self) {
                    ACTIVE.store(false, Ordering::Release);
                }
            }
            let _release = Release;
            let _ = send.send(exchange_once(&endpoint, &form));
        })
        .is_err()
    {
        ACTIVE.store(false, Ordering::Release);
        return Err(Error::Tls);
    }
    recv.recv_timeout(timeout).map_err(|e| match e {
        RecvTimeoutError::Timeout => Error::Timeout,
        RecvTimeoutError::Disconnected => Error::Tls,
    })?
}
