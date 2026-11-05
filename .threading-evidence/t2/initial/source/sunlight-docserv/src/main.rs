fn main() -> anyhow::Result<()> {
    #[cfg(feature = "native-hayro-smoke")]
    match std::env::args().nth(1).as_deref() {
        Some("--native-oom-test") => {
            use std::alloc::GlobalAlloc;
            let layout =
                std::alloc::Layout::from_size_align(std::hint::black_box(129 * 1024 * 1024), 16)?;
            let pointer = std::hint::black_box(unsafe { std::alloc::System.alloc(layout) });
            anyhow::ensure!(pointer.is_null(), "native heap exceeded its bound");
            anyhow::ensure!(
                sunlight_libc::alloc::heap_stats().failed_allocation_count > 0,
                "OOM probe did not reach the native allocator"
            );
            std::alloc::handle_alloc_error(layout);
        }
        Some("--native-exit-test") => {
            std::thread::spawn(|| loop {
                std::thread::yield_now();
            });
            std::process::exit(43);
        }
        Some("--native-panic-test") => {
            std::thread::spawn(|| panic!("native worker panic containment probe"));
            loop {
                std::thread::yield_now();
            }
        }
        _ => {}
    }
    #[cfg(feature = "hayro-host")]
    anyhow::ensure!(
        std::env::args().nth(1).as_deref() == Some("--check"),
        "hayro-host requires --check document.pdf; build hayro-native with tools/build_docserv.sh for the native service."
    );
    if cfg!(feature = "native-smoke")
        || cfg!(feature = "native-hayro-smoke")
        || std::env::args().nth(1).as_deref() == Some("--self-test")
    {
        #[cfg(feature = "native-hayro-smoke")]
        native_std_checks()?;
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
        let text = client.request(DocRequest::ExtractText { handle, page: 0 })?;
        #[cfg(not(feature = "hayro-native"))]
        {
            let DocResponse::Text { content } = text else {
                anyhow::bail!("native ExtractText failed");
            };
            anyhow::ensure!(content.contains("SunlightOS"), "fixture text missing");
        }
        #[cfg(feature = "hayro-native")]
        anyhow::ensure!(
            matches!(text, DocResponse::Err(_)),
            "Hayro must report unsupported text extraction"
        );
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
        #[cfg(feature = "hayro-native")]
        sunlight_ipc::debug_log("[DOCSERV] native Hayro IPC open/render/close OK\n");
        #[cfg(not(feature = "hayro-native"))]
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

#[cfg(feature = "native-hayro-smoke")]
fn native_std_checks() -> anyhow::Result<()> {
    use std::{
        io::{Read, Seek, SeekFrom},
        sync::{Arc, Mutex, OnceLock},
        time::{Duration, Instant},
    };
    use sunlight_docserv::backend::DocumentBackend;
    // Rust's real collections, memory I/O, clock and allocator/System paths.
    let mut map = std::collections::HashMap::new();
    map.insert("native", vec![1, 2, 3]);
    anyhow::ensure!(map["native"] == [1, 2, 3]);
    let mut cursor = std::io::Cursor::new(b"native std");
    let mut bytes = [0; 6];
    cursor.read_exact(&mut bytes)?;
    anyhow::ensure!(&bytes == b"native");
    let started = Instant::now();
    std::thread::sleep(Duration::from_millis(20));
    anyhow::ensure!(started.elapsed() >= Duration::from_millis(20));
    anyhow::ensure!(std::time::SystemTime::now() > std::time::UNIX_EPOCH);
    let counter = Arc::new(Mutex::new(0usize));
    let once = Arc::new(OnceLock::new());
    static DROPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    struct DropWitness;
    impl Drop for DropWitness {
        fn drop(&mut self) {
            DROPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    std::thread_local! { static TLS: (std::cell::Cell<usize>, DropWitness) = (std::cell::Cell::new(0), DropWitness); }
    for round in 0..8 {
        let mut threads = Vec::new();
        for index in 0..4 {
            let counter = counter.clone();
            let once = once.clone();
            threads.push(std::thread::spawn(move || {
                TLS.with(|local| {
                    assert_eq!(local.0.get(), 0);
                    local.0.set(index + 1);
                });
                assert_eq!(*once.get_or_init(|| 42), 42);
                for _ in 0..64 {
                    *counter.lock().unwrap() += 1;
                    std::thread::yield_now();
                }
                TLS.with(|local| assert_eq!(local.0.get(), index + 1));
                index + round
            }));
        }
        for (index, thread) in threads.into_iter().enumerate() {
            anyhow::ensure!(thread.join().unwrap() == index + round);
        }
    }
    anyhow::ensure!(*counter.lock().unwrap() == 8 * 4 * 64);
    anyhow::ensure!(DROPS.load(std::sync::atomic::Ordering::SeqCst) == 32);
    let condition = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let child_condition = condition.clone();
    let child = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        *child_condition.0.lock().unwrap() = true;
        child_condition.1.notify_all();
    });
    let ready = condition
        .1
        .wait_while(condition.0.lock().unwrap(), |ready| !*ready)
        .unwrap();
    anyhow::ensure!(*ready);
    drop(ready);
    child.join().unwrap();
    let (guard, timeout) = condition
        .1
        .wait_timeout(condition.0.lock().unwrap(), Duration::from_millis(20))
        .unwrap();
    anyhow::ensure!(timeout.timed_out());
    drop(guard);
    let shared = Arc::new(std::sync::RwLock::new(0));
    let mut readers_and_writers = Vec::new();
    for _ in 0..4 {
        let shared = shared.clone();
        readers_and_writers.push(std::thread::spawn(move || {
            for _ in 0..32 {
                *shared.write().unwrap() += 1;
                assert!(*shared.read().unwrap() <= 128);
                std::thread::yield_now();
            }
        }));
    }
    for thread in readers_and_writers {
        thread.join().unwrap();
    }
    anyhow::ensure!(*shared.read().unwrap() == 128);
    // The supervisor permits argc=0. Child probes below exercise explicit argv.
    anyhow::ensure!(std::env::var_os("HOME").is_some());
    unsafe { std::env::set_var("NATIVE_STD_PROBE", "owned environment") };
    anyhow::ensure!(std::env::var("NATIVE_STD_PROBE")? == "owned environment");
    unsafe { std::env::remove_var("NATIVE_STD_PROBE") };
    anyhow::ensure!(std::env::var_os("NATIVE_STD_PROBE").is_none());
    anyhow::ensure!(std::env::current_dir()?.is_absolute());
    anyhow::ensure!(
        std::env::current_dir()?.join("/tmp/native.pdf") == std::path::Path::new("/tmp/native.pdf")
    );
    anyhow::ensure!(std::path::absolute("native-relative")?.is_absolute());
    anyhow::ensure!(!std::path::Path::new("native-relative").is_absolute());
    anyhow::ensure!(
        std::fs::File::open("/tmp/native\0invalid")
            .unwrap_err()
            .kind()
            == std::io::ErrorKind::InvalidInput
    );
    sunlight_ipc::debug_log("[NATIVE-STD] collections/clocks/locks/TLS/join OK\n");
    let shared_path = "/tmp/native-std-shared-fd";
    std::fs::write(shared_path, b"shared descriptors")?;
    let file = Arc::new(std::fs::File::open(shared_path)?);
    let child_file = file.clone();
    let child = std::thread::spawn(move || {
        let mut bytes = [0u8; 6];
        (&*child_file).read_exact(&mut bytes).unwrap();
        bytes
    });
    anyhow::ensure!(&child.join().unwrap() == b"shared");
    let mut bytes = [0; 12];
    (&*file).read_exact(&mut bytes)?;
    anyhow::ensure!(&bytes == b" descriptors");
    drop(file);
    for (argument, expected) in [
        (b"--native-oom-test".as_slice(), 101),
        (b"--native-panic-test".as_slice(), 101),
        (b"--native-exit-test".as_slice(), 43),
    ] {
        let pid = sunlight_libc::spawn(
            b"/bin/sunlight-docserv",
            &[b"sunlight-docserv", argument],
            None,
        )
        .map_err(|e| anyhow::anyhow!("spawn runtime containment probe: {e:?}"))?;
        let code = sunlight_libc::waitpid(pid)
            .map_err(|e| anyhow::anyhow!("wait runtime containment probe: {e:?}"))?;
        anyhow::ensure!(
            code == expected,
            "runtime containment probe exit code {code}"
        );
    }
    sunlight_ipc::debug_log("[NATIVE-STD] shared descriptors/OOM/group exit OK\n");
    sunlight_ipc::debug_log("[NATIVE-STD] worker panic containment OK\n");
    let path = "/tmp/native-hayro-fixture.pdf";
    std::fs::write(path, include_bytes!("../tests/fixtures/sample.pdf"))?;
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut bytes[..5])?;
    anyhow::ensure!(&bytes[..5] == b"%PDF-");
    drop(file);
    let backend = sunlight_docserv::backends::hayro_backend::HayroBackend;
    // Warm lazy font/CMap/TLS state, then verify repeated document lifetimes.
    for cycle in 0..5 {
        let before = sunlight_libc::alloc::heap_stats();
        let document = backend.open(std::path::Path::new(path))?;
        anyhow::ensure!(document.page_count() == 6);
        for page in 0..6 {
            let bitmap = document.render_page(page, 0.5)?;
            anyhow::ensure!(bitmap.width > 0 && bitmap.height > 0);
            anyhow::ensure!(bitmap.data.len() == bitmap.stride as usize * bitmap.height as usize);
            anyhow::ensure!(bitmap
                .data
                .chunks_exact(4)
                .any(|p| p[..3] != [255, 255, 255]));
        }
        drop(document);
        let after = sunlight_libc::alloc::heap_stats();
        if cycle > 0 {
            anyhow::ensure!(
                after.requested_user_bytes == before.requested_user_bytes,
                "native renderer heap grew"
            );
        }
    }
    sunlight_ipc::debug_log("[DOCSERV] native Hayro PDF raster/recovery OK\n");
    let path = "/tmp/native-hayro-rich-fixture.pdf";
    std::fs::write(
        path,
        include_bytes!("../../assets/documents/SunlightOS_PDF_Reader_Test.pdf"),
    )?;
    let document = backend.open(std::path::Path::new(path))?;
    anyhow::ensure!(document.page_count() == 4);
    for page in 0..4 {
        let bitmap = document.render_page(page, 1.0)?;
        anyhow::ensure!((bitmap.width, bitmap.height) == (595, 841));
        anyhow::ensure!(
            bitmap
                .data
                .chunks_exact(4)
                .filter(|p| p[..3] != [255, 255, 255])
                .count()
                > 1000
        );
        if page == 2 {
            let colors = |predicate: fn(u8, u8, u8) -> bool| {
                bitmap
                    .data
                    .chunks_exact(4)
                    .filter(|p| predicate(p[2], p[1], p[0]))
                    .count()
            };
            anyhow::ensure!(colors(|r, g, b| r > 180 && g < 150 && b < 150) > 500);
            anyhow::ensure!(colors(|r, g, b| r > 220 && g > 140 && g < 210 && b < 130) > 5000);
            anyhow::ensure!(colors(|r, g, b| r < 50 && g > 130 && b > 160) > 15000);
        }
    }
    sunlight_ipc::debug_log("[DOCSERV] native Hayro text/vector/image pixels OK\n");
    Ok(())
}
