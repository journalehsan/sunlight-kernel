//! Serializable document-service requests and replies. Page indices are zero-based.
use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

/// Filesystem path: native alloc-only clients use UTF-8 strings. Both forms
/// serialize as the same string on the wire.
#[cfg(feature = "std")]
pub type DocumentPath = std::path::PathBuf;
/// UTF-8 filesystem path for alloc-only native clients.
#[cfg(not(feature = "std"))]
pub type DocumentPath = String;
use DocumentPath as PathBuf;

/// A request to the named `docserv` endpoint.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum DocRequest {
    /// Open a document by filesystem path.
    Open { path: PathBuf },
    /// Query the number of pages in an open document.
    PageCount { handle: u64 },
    /// Query a page's unscaled point dimensions, for fit-mode zoom.
    PageInfo { handle: u64, page: u32 },
    /// Render one page at the specified pixels-per-point scale.
    RenderPage { handle: u64, page: u32, scale: f32 },
    /// Extract text from one page.
    ExtractText { handle: u64, page: u32 },
    /// Query a point-in-time memory breakdown for the service process.
    MemoryReport,
    /// Release the document and its source snapshot.
    Close { handle: u64 },
}

/// Result of one document operation.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum DocResponse {
    /// A newly allocated, nonzero document identifier.
    Opened { handle: u64 },
    /// Total page count.
    PageCount { count: u32 },
    /// Unscaled page dimensions in typographic points.
    PageDimensions { width_pt: f32, height_pt: f32 },
    /// Top-down BGRA8 pixels, with `data.len() == stride * height`.
    RenderedPage {
        width: u32,
        height: u32,
        stride: u32,
        data: Vec<u8>,
    },
    /// Extracted UTF-8 text.
    Text { content: String },
    /// Point-in-time service memory accounting (see the field comments).
    MemoryReport {
        /// Hayro's internal interpreter/cache size if the API exposes it;
        /// 0 means "not exposed" (Hayro 0.8.0 has no size query).
        hayro_store_bytes: u64,
        /// Live document handles in the registry.
        handle_count: u32,
        /// Sum of source snapshot bytes (the `Pdf` ingest buffer) across handles.
        source_bytes: u64,
        /// Service-side pending reply buffers awaiting a FETCH (transport).
        ipc_pending_bytes: u64,
        /// Statically configured per-endpoint IPC queue capacity, in bytes.
        ipc_ring_bytes: u64,
        /// Live bytes in the process allocator if accessible, else 0.
        heap_estimate_bytes: u64,
    },
    /// Successful operation without a payload.
    Ok,
    /// A recoverable request, format, or transport error.
    Err(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_request_round_trips() -> anyhow::Result<()> {
        let requests = [
            DocRequest::Open {
                path: "sample.pdf".into(),
            },
            DocRequest::PageCount { handle: 1 },
            DocRequest::PageInfo { handle: 1, page: 0 },
            DocRequest::RenderPage {
                handle: 2,
                page: 3,
                scale: 1.5,
            },
            DocRequest::ExtractText { handle: 4, page: 5 },
            DocRequest::MemoryReport,
            DocRequest::Close { handle: 6 },
        ];
        for request in requests {
            assert_eq!(
                serde_json::from_slice::<DocRequest>(&serde_json::to_vec(&request)?)?,
                request
            );
        }
        Ok(())
    }

    #[test]
    fn every_response_round_trips() -> anyhow::Result<()> {
        let responses = [
            DocResponse::Opened { handle: 1 },
            DocResponse::PageCount { count: 2 },
            DocResponse::PageDimensions {
                width_pt: 595.276,
                height_pt: 841.89,
            },
            DocResponse::RenderedPage {
                width: 1,
                height: 1,
                stride: 4,
                data: alloc::vec![1, 2, 3, 255],
            },
            DocResponse::Text {
                content: "Sunlight ☀".into(),
            },
            DocResponse::MemoryReport {
                hayro_store_bytes: 0,
                handle_count: 1,
                source_bytes: 1024,
                ipc_pending_bytes: 0,
                ipc_ring_bytes: 4096,
                heap_estimate_bytes: 2048,
            },
            DocResponse::Ok,
            DocResponse::Err("unknown handle".into()),
        ];
        for response in responses {
            assert_eq!(
                serde_json::from_slice::<DocResponse>(&serde_json::to_vec(&response)?)?,
                response
            );
        }
        Ok(())
    }
}
