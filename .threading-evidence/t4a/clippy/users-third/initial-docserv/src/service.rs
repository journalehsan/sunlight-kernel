//! Document registry and native IPC accept loop.
use crate::{
    backend::{DocumentBackend, DocumentHandle},
    client::{Mapping, ENDPOINT, ERROR, FETCH, MAX_REQUEST, MAX_RESPONSE, READY, REQUEST},
    ipc::{DocRequest, DocResponse},
};
use anyhow::{ensure, Context, Result};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};
use sunlight_ipc::{endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, IpcMsg};

/// Kernel endpoint queue depth for the docserv endpoint
/// (`kernel/src/ipc/mod.rs::ENDPOINT_QUEUE_CAPACITY`). Reported so
/// `MemoryReport` can expose the kernel-side message ring's worst-case size
/// without a kernel query.
const IPC_ENDPOINT_QUEUE_CAPACITY: u64 = 64;

/// Backend registry. Handles are private to the authenticated IPC sender badge.
pub struct DocService {
    handles: HashMap<u64, Box<dyn DocumentHandle>>,
    owners: HashMap<u64, u64>,
    next: AtomicU64,
    backends: Vec<Box<dyn DocumentBackend>>,
    /// Bytes held by the transport in pending (unfetched) reply buffers.
    ///
    /// The transport owns the pending map, so it mirrors the total here before
    /// each dispatch; `MemoryReport` reads it. Bounded by the transport's
    /// 128 MiB pending budget.
    pending_reply_bytes: u64,
}
impl Default for DocService {
    fn default() -> Self {
        let backends: Vec<Box<dyn DocumentBackend>> = vec![
            #[cfg(any(feature = "hayro-host", feature = "hayro-native"))]
            Box::new(crate::backends::hayro_backend::HayroBackend),
            #[cfg(feature = "mupdf-legacy")]
            Box::new(crate::backends::mupdf_backend::MuPdfBackend),
            #[cfg(feature = "mupdf-legacy")]
            Box::new(crate::backends::ghostscript_backend::GhostscriptBackend),
        ];
        Self::new(backends)
    }
}
impl DocService {
    /// Construct a registry with injectable format engines.
    pub fn new(backends: Vec<Box<dyn DocumentBackend>>) -> Self {
        Self {
            handles: HashMap::new(),
            owners: HashMap::new(),
            next: AtomicU64::new(1),
            backends,
            pending_reply_bytes: 0,
        }
    }
    /// Number of open handles, for lifecycle verification.
    pub fn handle_count(&self) -> usize {
        self.handles.len()
    }

    /// Record the transport's current pending-reply bytes for `MemoryReport`.
    pub fn note_pending_reply_bytes(&mut self, bytes: u64) {
        self.pending_reply_bytes = bytes;
    }

    /// Release documents whose authenticated IPC owners have terminated.
    pub fn reap_closed_clients(&mut self, is_alive: impl Fn(u64) -> bool) {
        let orphaned: Vec<u64> = self
            .owners
            .iter()
            .filter_map(|(&handle, &owner)| (!is_alive(owner)).then_some(handle))
            .collect();
        for handle in orphaned {
            self.owners.remove(&handle);
            self.handles.remove(&handle);
        }
    }
    /// Process one request without terminating the service on errors.
    pub fn dispatch(&mut self, owner: u64, request: DocRequest) -> DocResponse {
        match self.try_dispatch(owner, request) {
            Ok(reply) => reply,
            Err(error) => {
                eprintln!("[DOCSERV] {error:#}");
                DocResponse::Err(format!("{error:#}"))
            }
        }
    }
    fn lookup(&self, owner: u64, handle: u64) -> Result<&dyn DocumentHandle> {
        ensure!(
            self.owners.get(&handle) == Some(&owner),
            "unknown document handle {handle}"
        );
        self.handles
            .get(&handle)
            .map(|h| h.as_ref())
            .context("document handle was closed")
    }
    fn try_dispatch(&mut self, owner: u64, request: DocRequest) -> Result<DocResponse> {
        Ok(match request {
            DocRequest::Open { path } => {
                ensure!(self.handles.len() < 64, "document registry is full");
                let extension = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .context("document has no extension")?
                    .to_ascii_lowercase();
                let backend = self
                    .backends
                    .iter()
                    .find(|b| b.supported_extensions().contains(&extension.as_str()))
                    .context("unsupported document format")?;
                let document = backend.open(&path)?;
                let handle = self
                    .next
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                    .map_err(|_| anyhow::anyhow!("document identifiers exhausted"))?;
                self.handles.insert(handle, document);
                self.owners.insert(handle, owner);
                DocResponse::Opened { handle }
            }
            DocRequest::PageCount { handle } => DocResponse::PageCount {
                count: self.lookup(owner, handle)?.page_count(),
            },
            DocRequest::PageInfo { handle, page } => {
                let info = self.lookup(owner, handle)?.page_info(page)?;
                DocResponse::PageDimensions {
                    width_pt: info.width_pt,
                    height_pt: info.height_pt,
                }
            }
            DocRequest::RenderPage {
                handle,
                page,
                scale,
            } => {
                let bitmap = self.lookup(owner, handle)?.render_page(page, scale)?;
                ensure!(
                    bitmap.data.len() < MAX_RESPONSE - 64,
                    "rendered page exceeds IPC budget"
                );
                DocResponse::RenderedPage {
                    width: bitmap.width,
                    height: bitmap.height,
                    stride: bitmap.stride,
                    data: bitmap.data,
                }
            }
            DocRequest::ExtractText { handle, page } => DocResponse::Text {
                content: self.lookup(owner, handle)?.extract_text(page)?,
            },
            DocRequest::MemoryReport => DocResponse::MemoryReport {
                // Hayro 0.8.0 exposes no cache/store size query; see the report
                // docs. Left at 0 = "not exposed" rather than guessed.
                hayro_store_bytes: 0,
                handle_count: self.handles.len() as u32,
                source_bytes: self.handles.values().map(|h| h.source_bytes()).sum(),
                ipc_pending_bytes: self.pending_reply_bytes as u64,
                // One docserv endpoint queue of `IPC_ENDPOINT_QUEUE_CAPACITY`
                // inline `IpcMsg` records. Inline messages carry no payload
                // here (page data rides caller-owned shared mappings), so this
                // is the kernel-side ring cost for the service channel.
                ipc_ring_bytes: IPC_ENDPOINT_QUEUE_CAPACITY * core::mem::size_of::<IpcMsg>() as u64,
                heap_estimate_bytes: heap_estimate_bytes(),
            },
            DocRequest::Close { handle } => {
                self.lookup(owner, handle)?;
                self.handles.remove(&handle);
                self.owners.remove(&handle);
                DocResponse::Ok
            }
        })
    }
}

/// Register and serve native document RPCs. Response storage is bounded and
/// scoped to each sender; a new request replaces that sender's pending reply.
pub fn run_native(mut service: DocService) -> Result<()> {
    ensure!(
        !cfg!(feature = "hayro-host"),
        "Hayro is host-only with hayro-host; use tools/build_docserv.sh and hayro-native for the native service"
    );
    let endpoint = endpoint_create();
    ensure!(
        nameserver_register(ENDPOINT, endpoint),
        "register docserv endpoint"
    );
    eprintln!("[DOCSERV] ready");
    let mut pending: HashMap<u64, Vec<u8>> = HashMap::new();
    let mut message = ipc_recv(endpoint);
    loop {
        service.reap_closed_clients(sunlight_ipc::process_is_alive);
        pending.retain(|&owner, _| sunlight_ipc::process_is_alive(owner));
        service.note_pending_reply_bytes(pending.values().map(Vec::len).sum::<usize>() as u64);
        let reply = match transport(&mut service, &mut pending, message) {
            Ok(reply) => reply,
            Err(error) => {
                eprintln!("[DOCSERV] transport: {error:#}");
                IpcMsg::with_label(ERROR)
            }
        };
        message = ipc_reply_and_wait(endpoint, reply);
    }
}

fn transport(
    service: &mut DocService,
    pending: &mut HashMap<u64, Vec<u8>>,
    message: IpcMsg,
) -> Result<IpcMsg> {
    ensure!(
        message.cap_count == 1,
        "expected one shared-memory capability"
    );
    let owner = message.badge;
    match message.label {
        REQUEST => {
            pending.remove(&owner);
            ensure!(pending.len() < 16, "too many pending document replies");
            let len = usize::try_from(message.words[0])?;
            ensure!(len > 0 && len <= MAX_REQUEST, "invalid request length");
            let mapping = Mapping::receive(message.caps[0])?;
            let bytes = mapping.read(len)?;
            let response = match postcard::from_bytes(&bytes) {
                Ok(request) => service.dispatch(owner, request),
                Err(error) => DocResponse::Err(format!("malformed document request: {error}")),
            };
            let bytes = postcard::to_allocvec(&response)?;
            ensure!(bytes.len() <= MAX_RESPONSE, "response is too large");
            ensure!(
                pending.values().map(Vec::len).sum::<usize>() + bytes.len() <= 128 * 1024 * 1024,
                "pending document replies exceed the memory budget"
            );
            let reply = IpcMsg::with_label(READY).word(0, bytes.len() as u64);
            pending.insert(owner, bytes);
            Ok(reply)
        }
        FETCH => {
            let bytes = pending.remove(&owner).context("no pending response")?;
            let mut mapping = Mapping::receive(message.caps[0])?;
            mapping.write(&bytes)?;
            Ok(IpcMsg::with_label(READY).word(0, bytes.len() as u64))
        }
        _ => anyhow::bail!("unknown document IPC operation"),
    }
}

/// Live bytes held by the process allocator, when the target exposes that.
///
/// The native SunlightOS std build uses `sunlight-libc`'s `System` allocator,
/// whose `heap_stats()` reports `requested_user_bytes` (live bytes handed to
/// callers). Other targets (Linux GNU host, `alloc`-only clients) have no such
/// API, so this returns 0 = "not accessible" rather than a guess. `mallinfo2`
/// is deliberately not used: the host glibc arena also serves this process's
/// own standard I/O and would over-report the document work.
fn heap_estimate_bytes() -> u64 {
    #[cfg(all(feature = "hayro-native", target_os = "sunlightos"))]
    {
        sunlight_libc::alloc::heap_stats().requested_user_bytes as u64
    }
    #[cfg(not(all(feature = "hayro-native", target_os = "sunlightos")))]
    {
        0
    }
}
