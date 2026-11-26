use anyhow::{bail, ensure, Result};
use std::{
    alloc::GlobalAlloc,
    time::{Duration, Instant},
};
use sunlight_docserv::{
    client::{DocClient, NativeClient},
    ipc::{DocRequest, DocResponse},
    service::heap_probe_sample,
};

pub fn run() -> Result<()> {
    std::thread::sleep(Duration::from_secs(10));
    let mut client = NativeClient::connect()?;
    heap_probe_sample("idle");
    let path = "/tmp/docserv-heap-probe.pdf";
    std::fs::write(path, include_bytes!("../tests/fixtures/sample.pdf"))?;

    let mut value = Vec::new();
    value.try_reserve_exact(32)?;
    value.push(0xa5u8);
    ensure!(value.try_reserve_exact(128 * 1024 * 1024).is_err());
    ensure!(value == [0xa5]);
    drop(value);

    let layout = std::alloc::Layout::from_size_align(17 * 1024 * 1024, 65536)?;
    let pointer = unsafe { std::alloc::System.alloc(layout) };
    ensure!(!pointer.is_null() && pointer as usize % layout.align() == 0);
    unsafe { std::alloc::System.dealloc(pointer, layout) };
    sunlight_ipc::debug_log("[DOC-HEAP] growth/alignment/bounded failure OK\n");

    for (page, scale) in [(0, 1.0), (5, 3.0), (2, 1.0)] {
        let started = Instant::now();
        let DocResponse::Opened { handle } =
            client.request(DocRequest::Open { path: path.into() })?
        else {
            bail!("open failed")
        };
        ensure!(
            client.request(DocRequest::PageCount { handle })?
                == (DocResponse::PageCount { count: 6 })
        );
        let DocResponse::RenderedPage {
            width,
            height,
            stride,
            data,
        } = client.request(DocRequest::RenderPage {
            handle,
            page,
            scale,
        })?
        else {
            bail!("render failed")
        };
        let bytes = data.len();
        ensure!(width > 0 && height > 0 && bytes == stride as usize * height as usize);
        drop(data);
        ensure!(client.request(DocRequest::Close { handle })? == DocResponse::Ok);
        ensure!(matches!(
            client.request(DocRequest::PageCount { handle })?,
            DocResponse::Err(_)
        ));
        eprintln!(
            "[DOC-HEAP] render page={page} scale={scale} bytes={bytes} ms={}",
            started.elapsed().as_millis()
        );
    }
    let thumbnail = sunlight_docserv::thumbnail::generate_with_client(&mut client, path, 128)?;
    ensure!(thumbnail.len() == 128 * 128 * 4);
    drop(thumbnail);
    sunlight_libc::unlink(path.as_bytes()).map_err(|error| anyhow::anyhow!("{error:?}"))?;
    heap_probe_sample("recovered");
    sunlight_ipc::debug_log("[DOC-HEAP] workload PASS\n");
    Ok(())
}
