//! Real document engines through a background service and serialized RPCs.
//! The native capability/shared-memory transport is exercised by test_docserv.sh.
#![cfg(feature = "renderer")]
use anyhow::{Context, Result};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};
use sunlight_docserv::{
    client::DocClient,
    ipc::{DocRequest, DocResponse},
    service::DocService,
};

enum Command {
    Request(u64, Vec<u8>, Sender<(Vec<u8>, usize)>),
    Stop,
}
struct Server {
    sender: Sender<Command>,
    thread: Option<JoinHandle<Result<()>>>,
    handles: usize,
}
impl Server {
    fn start() -> Self {
        let (sender, incoming) = mpsc::channel();
        let thread = thread::spawn(move || -> Result<()> {
            let mut service = DocService::default();
            while let Ok(Command::Request(owner, bytes, reply)) = incoming.recv() {
                let request = postcard::from_bytes(&bytes).context("decode request")?;
                let response = service.dispatch(owner, request);
                let _ = reply.send((postcard::to_allocvec(&response)?, service.handle_count()));
            }
            Ok(())
        });
        Self {
            sender,
            thread: Some(thread),
            handles: 0,
        }
    }
    fn call(&mut self, owner: u64, request: DocRequest) -> Result<DocResponse> {
        let (sender, reply) = mpsc::channel();
        self.sender.send(Command::Request(
            owner,
            postcard::to_allocvec(&request)?,
            sender,
        ))?;
        let (bytes, handles) = reply.recv_timeout(Duration::from_secs(35))?;
        self.handles = handles;
        postcard::from_bytes(&bytes).context("decode response")
    }
    fn finish(&mut self) -> Result<()> {
        self.sender.send(Command::Stop)?;
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("service thread panicked"))??;
        }
        Ok(())
    }
}
impl DocClient for Server {
    fn request(&mut self, request: DocRequest) -> Result<DocResponse> {
        self.call(7, request)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn open(server: &mut Server, name: &str) -> Result<u64> {
    match server.request(DocRequest::Open {
        path: fixture(name),
    })? {
        DocResponse::Opened { handle } => Ok(handle),
        response => anyhow::bail!("Open failed: {response:?}"),
    }
}

#[test]
fn pdf_open_render_extract_close_registry_empty() -> Result<()> {
    let mut server = Server::start();
    let handle = open(&mut server, "sample.pdf")?;
    let DocResponse::PageCount { count } = server.request(DocRequest::PageCount { handle })? else {
        anyhow::bail!("unexpected page count response");
    };
    assert!(count > 0);
    let DocResponse::RenderedPage {
        width,
        height,
        stride,
        data,
    } = server.request(DocRequest::RenderPage {
        handle,
        page: 0,
        scale: 1.0,
    })?
    else {
        anyhow::bail!("unexpected bitmap response");
    };
    assert!(width > 0 && height > 0);
    assert_eq!(data.len(), stride as usize * height as usize);
    #[cfg(all(feature = "mupdf-legacy", not(feature = "hayro-host")))]
    {
        let DocResponse::Text { content } =
            server.request(DocRequest::ExtractText { handle, page: 0 })?
        else {
            anyhow::bail!("unexpected text response");
        };
        assert!(!content.trim().is_empty());
    }
    #[cfg(feature = "hayro-host")]
    assert!(matches!(
        server.request(DocRequest::ExtractText { handle, page: 0 })?,
        DocResponse::Err(message) if message.contains("text extraction")
    ));
    assert_eq!(
        server.request(DocRequest::Close { handle })?,
        DocResponse::Ok
    );
    assert_eq!(server.handles, 0);
    server.finish()
}

#[test]
fn bad_requests_and_foreign_handles_do_not_kill_service() -> Result<()> {
    let mut server = Server::start();
    let handle = open(&mut server, "sample.pdf")?;
    assert!(matches!(
        server.call(99, DocRequest::PageCount { handle })?,
        DocResponse::Err(_)
    ));
    assert!(matches!(
        server.request(DocRequest::RenderPage {
            handle,
            page: u32::MAX,
            scale: 1.0
        })?,
        DocResponse::Err(_)
    ));
    for scale in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
        assert!(matches!(
            server.request(DocRequest::RenderPage {
                handle,
                page: 0,
                scale
            })?,
            DocResponse::Err(_)
        ));
    }
    assert!(matches!(
        server.request(DocRequest::PageCount { handle })?,
        DocResponse::PageCount { count: 6 }
    ));
    assert_eq!(
        server.request(DocRequest::Close { handle })?,
        DocResponse::Ok
    );
    assert!(matches!(
        server.request(DocRequest::Close { handle })?,
        DocResponse::Err(_)
    ));
    assert_eq!(server.handles, 0);
    server.finish()
}

#[test]
fn real_pdf_thumbnail_is_square_and_does_not_leak_handles() -> Result<()> {
    let mut server = Server::start();
    let data =
        sunlight_docserv::thumbnail::generate_with_client(&mut server, fixture("sample.pdf"), 128)?;
    assert_eq!(data.len(), 128 * 128 * 4);
    assert!(data.chunks_exact(4).any(|p| p[0] == 204 && p[2] < 30));
    assert_eq!(server.handles, 0);
    server.finish()
}

#[cfg(all(feature = "mupdf-legacy", not(feature = "hayro-host")))]
#[test]
fn ghostscript_postscript_conversion_render_and_text() -> Result<()> {
    let mut server = Server::start();
    let handle = open(&mut server, "sample.ps")?;
    let DocResponse::Text { content } =
        server.request(DocRequest::ExtractText { handle, page: 0 })?
    else {
        anyhow::bail!("unexpected PostScript text response");
    };
    assert!(content.contains("PostScript"));
    assert!(matches!(
        server.request(DocRequest::RenderPage {
            handle,
            page: 0,
            scale: 0.5
        })?,
        DocResponse::RenderedPage { .. }
    ));
    assert_eq!(
        server.request(DocRequest::Close { handle })?,
        DocResponse::Ok
    );
    assert_eq!(server.handles, 0);
    server.finish()
}

#[test]
fn terminated_client_documents_are_reclaimed() -> Result<()> {
    let mut service = DocService::default();
    assert!(matches!(
        service.dispatch(
            7,
            DocRequest::Open {
                path: fixture("sample.pdf")
            }
        ),
        DocResponse::Opened { .. }
    ));
    assert_eq!(service.handle_count(), 1);
    service.reap_closed_clients(|owner| owner == 7);
    assert_eq!(service.handle_count(), 1);
    service.reap_closed_clients(|_| false);
    assert_eq!(service.handle_count(), 0);
    Ok(())
}

#[cfg(all(feature = "hayro-host", not(feature = "mupdf-legacy")))]
#[test]
fn hayro_does_not_advertise_legacy_formats() {
    let mut service = DocService::default();
    for name in [
        "sample.ps",
        "sample.eps",
        "sample.xps",
        "sample.cbz",
        "sample.epub",
    ] {
        assert!(matches!(
            service.dispatch(7, DocRequest::Open { path: fixture(name) }),
            DocResponse::Err(message) if message.contains("unsupported document format")
        ));
    }
    assert_eq!(service.handle_count(), 0);
}

#[cfg(feature = "hayro-host")]
#[test]
fn hayro_rejects_native_service_before_using_guest_syscalls() {
    let error = sunlight_docserv::service::run_native(DocService::default()).unwrap_err();
    assert!(error.to_string().contains("Hayro is host-only"));
}
