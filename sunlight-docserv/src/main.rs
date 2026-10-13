fn main() -> anyhow::Result<()> {
    #[cfg(feature = "hayro-host")]
    anyhow::ensure!(
        std::env::args().nth(1).as_deref() == Some("--check"),
        "Hayro is host-only; use --check document.pdf. Native PDF rendering is not available yet."
    );
    if cfg!(feature = "native-smoke") || std::env::args().nth(1).as_deref() == Some("--self-test") {
        use sunlight_docserv::{
            client::{DocClient, NativeClient},
            ipc::{DocRequest, DocResponse},
        };
        let path = "/tmp/docserv-fixture.pdf";
        std::fs::write(path, include_bytes!("../tests/fixtures/sample.pdf"))?;
        std::thread::Builder::new().spawn(|| {
            if let Err(error) = sunlight_docserv::service::run_native(Default::default()) {
                eprintln!("[DOCSERV] service failed: {error:#}");
            }
        })?;
        let started = std::time::Instant::now();
        let mut client = loop {
            match NativeClient::connect() {
                Ok(client) => break client,
                Err(error) if started.elapsed() > std::time::Duration::from_secs(10) => {
                    return Err(error)
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        };
        let DocResponse::Opened { handle } =
            client.request(DocRequest::Open { path: path.into() })?
        else {
            anyhow::bail!("native Open failed");
        };
        let DocResponse::PageCount { count } = client.request(DocRequest::PageCount { handle })?
        else {
            anyhow::bail!("native PageCount failed");
        };
        anyhow::ensure!(count == 6, "fixture page count differs");
        let DocResponse::RenderedPage {
            width,
            height,
            stride,
            data,
        } = client.request(DocRequest::RenderPage {
            handle,
            page: 0,
            scale: 1.0,
        })?
        else {
            anyhow::bail!("native RenderPage failed");
        };
        anyhow::ensure!(
            width > 0 && height > 0 && data.len() == stride as usize * height as usize,
            "invalid bitmap"
        );
        let DocResponse::Text { content } =
            client.request(DocRequest::ExtractText { handle, page: 0 })?
        else {
            anyhow::bail!("native ExtractText failed");
        };
        anyhow::ensure!(content.contains("SunlightOS"), "fixture text missing");
        anyhow::ensure!(
            client.request(DocRequest::Close { handle })? == DocResponse::Ok,
            "close failed"
        );
        anyhow::ensure!(
            matches!(
                client.request(DocRequest::PageCount { handle })?,
                DocResponse::Err(_)
            ),
            "closed handle survived"
        );
        let thumbnail = sunlight_docserv::thumbnail::generate_with_client(&mut client, path, 128)?;
        anyhow::ensure!(
            thumbnail.len() == 128 * 128 * 4,
            "native thumbnail size differs"
        );
        sunlight_ipc::debug_log("[DOCSERV] native thumbnail OK\n");
        if cfg!(feature = "native-postscript-smoke") {
            let ps = "/tmp/docserv-fixture.ps";
            std::fs::write(ps, include_bytes!("../tests/fixtures/sample.ps"))?;
            let DocResponse::Opened { handle } =
                client.request(DocRequest::Open { path: ps.into() })?
            else {
                anyhow::bail!("native PostScript Open failed");
            };
            let DocResponse::Text { content } =
                client.request(DocRequest::ExtractText { handle, page: 0 })?
            else {
                anyhow::bail!("native PostScript text failed");
            };
            anyhow::ensure!(
                content.contains("PostScript"),
                "native PostScript fixture text missing"
            );
            anyhow::ensure!(
                client.request(DocRequest::Close { handle })? == DocResponse::Ok,
                "PostScript close failed"
            );
            sunlight_ipc::debug_log("[DOCSERV] native PostScript conversion OK\n");
        }
        sunlight_ipc::debug_log("[DOCSERV] native IPC open/render/text/close OK\n");
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("--check") {
        let mut service = sunlight_docserv::service::DocService::default();
        let path = std::env::args()
            .nth(2)
            .ok_or_else(|| anyhow::anyhow!("usage: sunlight-docserv --check document.pdf"))?;
        let response = service.dispatch(
            0,
            sunlight_docserv::ipc::DocRequest::Open { path: path.into() },
        );
        match response {
            sunlight_docserv::ipc::DocResponse::Opened { handle } => {
                println!("[DOCSERV] opened {handle}");
                service.dispatch(0, sunlight_docserv::ipc::DocRequest::Close { handle });
                anyhow::ensure!(service.handle_count() == 0, "handle leaked");
                return Ok(());
            }
            reply => anyhow::bail!("open failed: {reply:?}"),
        }
    }
    sunlight_docserv::service::run_native(Default::default())
}
