//! Native capability IPC transport. Large replies use a caller-owned mapping.
use crate::ipc::{DocRequest, DocResponse};
use alloc::vec::Vec;
use anyhow::{ensure, Context, Result};
use sunlight_ipc::{
    ipc_call_timeout, nameserver_lookup_timeout, shm_create, shm_free, shm_map_sized,
    CapabilityToken, IpcMsg,
};

/// Endpoint name registered with init's nameserver.
pub const ENDPOINT: &str = "docserv";
/// Submit a serialized request through a shared-memory capability.
pub const REQUEST: u64 = 0xD501;
/// Copy the outstanding response into a caller-owned region.
pub const FETCH: u64 = 0xD502;
/// Reply advertising the serialized response size.
pub const READY: u64 = 0xD5FF;
/// Invalid transport operation.
pub const ERROR: u64 = 0xD5FE;
/// Maximum serialized page/text payload (64 MiB, including protocol overhead).
pub const MAX_RESPONSE: usize = 64 * 1024 * 1024 + 4096;
/// Maximum serialized path/request payload.
pub const MAX_REQUEST: usize = 4096;

/// An injectable synchronous RPC client; GUI callers run it on a worker.
pub trait DocClient {
    /// Submit one operation and await its reply.
    fn request(&mut self, request: DocRequest) -> Result<DocResponse>;
}

/// Connection to the native `docserv` endpoint.
pub struct NativeClient {
    endpoint: CapabilityToken,
}
impl NativeClient {
    /// Resolve the native service with a bounded lookup.
    pub fn connect() -> Result<Self> {
        let endpoint =
            nameserver_lookup_timeout(ENDPOINT, 500).context("docserv is unavailable")?;
        Ok(Self { endpoint })
    }
}

/// RAII mapping with a kernel-verified capacity.
pub struct Mapping {
    ptr: *mut u8,
    capacity: usize,
    cap: CapabilityToken,
}
impl Mapping {
    /// Allocate a new caller-owned region.
    pub fn create(size: usize) -> Result<Self> {
        let (ptr, cap) = shm_create(size, 0)
            .map_err(|e| anyhow::anyhow!("allocate document IPC mapping: {e:?}"))?;
        Ok(Self {
            ptr,
            cap,
            capacity: size,
        })
    }
    /// Map a received capability without trusting its advertised length.
    pub fn receive(cap: CapabilityToken) -> Result<Self> {
        let (ptr, capacity) =
            shm_map_sized(cap).map_err(|e| anyhow::anyhow!("map document IPC payload: {e:?}"))?;
        Ok(Self { ptr, capacity, cap })
    }
    /// Copy bytes into the mapping after checking its capacity.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(
            bytes.len() <= self.capacity,
            "document IPC mapping is too small"
        );
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr, bytes.len());
        }
        Ok(())
    }
    /// Copy bytes out so another process cannot mutate data while decoding it.
    pub fn read(&self, len: usize) -> Result<Vec<u8>> {
        ensure!(len <= self.capacity, "document IPC length exceeds mapping");
        Ok(unsafe { core::slice::from_raw_parts(self.ptr, len) }.to_vec())
    }
    /// Capability to transfer with an IPC message.
    pub fn capability(&self) -> CapabilityToken {
        self.cap
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        let _ = shm_free(self.cap);
    }
}

impl DocClient for NativeClient {
    fn request(&mut self, request: DocRequest) -> Result<DocResponse> {
        let bytes = postcard::to_allocvec(&request).context("encode document request")?;
        ensure!(bytes.len() <= MAX_REQUEST, "document request is too large");
        let mut request_map = Mapping::create(bytes.len())?;
        request_map.write(&bytes)?;
        let ready = ipc_call_timeout(
            self.endpoint,
            IpcMsg::with_label(REQUEST)
                .word(0, bytes.len() as u64)
                .with_cap(0, request_map.capability()),
            35_000,
        )
        .map_err(|e| anyhow::anyhow!("document request IPC: {e:?}"))?;
        ensure!(ready.label == READY, "docserv rejected the request");
        let length = usize::try_from(ready.words[0])?;
        ensure!(
            length > 0 && length <= MAX_RESPONSE,
            "invalid document response size"
        );
        let response_map = Mapping::create(length)?;
        let reply = ipc_call_timeout(
            self.endpoint,
            IpcMsg::with_label(FETCH).with_cap(0, response_map.capability()),
            5_000,
        )
        .map_err(|e| anyhow::anyhow!("document response IPC: {e:?}"))?;
        ensure!(
            reply.label == READY && reply.words[0] == length as u64,
            "docserv response transfer failed"
        );
        postcard::from_bytes(&response_map.read(length)?).context("decode document response")
    }
}
