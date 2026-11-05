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
    native_thread_mapping_rollback_checks()?;
    native_thread_admission_checks()?;
    native_futex_handoff_checks()?;
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
    // Render actual pages concurrently through four independent native std
    // workers. Each owns its PDF/cache state and returns only a small receipt.
    let mut render_workers = Vec::new();
    for page in 0..4 {
        render_workers.push(std::thread::Builder::new().spawn(
            move || -> anyhow::Result<(usize, u64)> {
                let backend = sunlight_docserv::backends::hayro_backend::HayroBackend;
                let document = backend.open(std::path::Path::new(path))?;
                let bitmap = document.render_page(page, 0.5)?;
                anyhow::ensure!(bitmap.width > 0 && bitmap.height > 0);
                let checksum = bitmap.data.iter().map(|byte| u64::from(*byte)).sum();
                Ok((bitmap.data.len(), checksum))
            },
        )?);
    }
    for worker in render_workers {
        let (bytes, checksum) = worker.join().unwrap()?;
        anyhow::ensure!(bytes > 0 && checksum > 0);
    }
    sunlight_ipc::debug_log("[NATIVE-STD] T2 Hayro workers=4 concurrent render/join OK\n");
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

/// Exercise the real std Builder error contract, TLS destructors, joins and
/// detach cleanup against kernel admission. This runs only in the smoke image.
#[cfg(feature = "native-hayro-smoke")]
fn native_thread_admission_checks() -> anyhow::Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct Witness;
    impl Drop for Witness {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    std::thread_local! { static TLS: Witness = const { Witness }; }
    for round in 0..3 {
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let mut handles = Vec::new();
        let mut rejection = None;
        for _ in 0..33 {
            let release = release.clone();
            match std::thread::Builder::new().spawn(move || {
                TLS.with(|_| ());
                let mut ready = release.0.lock().unwrap();
                for _ in 0..5000 {
                    if *ready {
                        return;
                    }
                    ready = release
                        .1
                        .wait_timeout(ready, Duration::from_millis(10))
                        .unwrap()
                        .0;
                }
                panic!("bounded admission worker timed out");
            }) {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    rejection = error.raw_os_error();
                    break;
                }
            }
        }
        anyhow::ensure!(handles.len() == 32, "std capacity was {}", handles.len());
        anyhow::ensure!(rejection == Some(11), "std quota error was {rejection:?}");
        *release.0.lock().unwrap() = true;
        release.1.notify_all();
        for (index, handle) in handles.into_iter().enumerate() {
            if index % 2 == 0 {
                handle.join().unwrap();
            } else {
                drop(handle);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        while DROPS.load(Ordering::SeqCst) != (round + 1) * 32 {
            anyhow::ensure!(Instant::now() < deadline, "detached TLS cleanup timed out");
            std::thread::yield_now();
        }
        // Spawn/join drives the runtime's quiescent detached-handle sweep.
        // Retry only EAGAIN while the final physical handoff is being reaped.
        for attempt in 0..4096 {
            match std::thread::Builder::new().spawn(|| 42) {
                Ok(handle) => {
                    anyhow::ensure!(handle.join().unwrap() == 42);
                    break;
                }
                Err(error) => {
                    anyhow::ensure!(error.raw_os_error() == Some(11) && attempt < 4095);
                    std::thread::yield_now();
                }
            }
        }
    }
    sunlight_ipc::debug_log(
        "[NATIVE-STD] T2 capacity=32 EAGAIN cycles=3 join/detach/TLS recovery OK\n",
    );
    Ok(())
}

/// Fill the actual native region ledger, leaving exactly one record for a
/// worker stack. TLS mapping then fails before publication; the complete stack
/// mapping must be releasable even while the ledger is full.
#[cfg(feature = "native-hayro-smoke")]
fn native_thread_mapping_rollback_checks() -> anyhow::Result<()> {
    use sunlight_libc::mman::*;
    let flags = MAP_PRIVATE | MAP_ANONYMOUS | MAP_NATIVE_THREAD;
    let mut pages = Vec::new();
    let mut full = false;
    for _ in 0..257 {
        match mmap(
            core::ptr::null_mut(),
            4096,
            PROT_READ | PROT_WRITE,
            flags,
            -1,
            0,
        ) {
            Ok(page) => pages.push(page),
            Err(_) => {
                full = true;
                break;
            }
        }
    }
    anyhow::ensure!(
        full && !pages.is_empty(),
        "bounded ledger pressure was not reached"
    );
    munmap(pages.pop().unwrap(), 4096).map_err(|_| anyhow::anyhow!("ledger release failed"))?;
    for _ in 0..3 {
        let result = std::thread::Builder::new().spawn(|| panic!("rejected child published"));
        anyhow::ensure!(result.is_err(), "TLS mapping unexpectedly admitted");
        anyhow::ensure!(result.unwrap_err().raw_os_error() == Some(12));
        let proof = mmap(
            core::ptr::null_mut(),
            4096,
            PROT_READ | PROT_WRITE,
            flags,
            -1,
            0,
        )
        .map_err(|_| anyhow::anyhow!("failed spawn leaked its stack ledger slot"))?;
        munmap(proof, 4096).map_err(|_| anyhow::anyhow!("proof mapping release failed"))?;
    }
    for page in pages {
        munmap(page, 4096).map_err(|_| anyhow::anyhow!("ledger pressure cleanup failed"))?;
    }
    sunlight_ipc::debug_log(
        "[NATIVE-STD] T2 full-ledger TLS failure rollback cycles=3 cleanup=allocation-free OK\n",
    );
    Ok(())
}

/// Exercise real std Mutex/Condvar paths on four independent addresses while
/// another thread keeps making progress. Warm once, then reconcile allocations.
#[cfg(feature = "native-hayro-smoke")]
fn native_futex_handoff_checks() -> anyhow::Result<()> {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    };
    std::thread_local! { static T4A_TLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
    let mut baseline = None;
    for round in 0..3 {
        let running = Arc::new(AtomicBool::new(true));
        let progress = Arc::new(AtomicUsize::new(0));
        let observer_running = running.clone();
        let observer_progress = progress.clone();
        let observer = std::thread::spawn(move || {
            while observer_running.load(Ordering::Acquire) {
                observer_progress.fetch_add(1, Ordering::Relaxed);
                std::thread::yield_now();
            }
        });
        let mut threads = Vec::new();
        for pair in 0..4 {
            let channel = Arc::new((Mutex::new((false, 0usize)), Condvar::new(), Condvar::new()));
            for producer in [true, false] {
                let channel = channel.clone();
                threads.push(std::thread::spawn(move || {
                    let seed = pair * 2 + usize::from(producer) + 1;
                    T4A_TLS.with(|tls| {
                        assert_eq!(tls.get(), 0);
                        tls.set(seed);
                    });
                    let canary = [seed as u64; 32];
                    for sequence in 0..128 {
                        let (lock, data_ready, empty) = &*channel;
                        if producer {
                            let mut state = empty
                                .wait_while(lock.lock().unwrap(), |state| state.0)
                                .unwrap();
                            state.1 = sequence;
                            state.0 = true;
                            data_ready.notify_one();
                        } else {
                            let mut state = data_ready
                                .wait_while(lock.lock().unwrap(), |state| !state.0)
                                .unwrap();
                            assert_eq!(state.1, sequence);
                            state.0 = false;
                            empty.notify_one();
                        }
                        std::thread::yield_now();
                    }
                    assert_eq!(std::hint::black_box(canary), [seed as u64; 32]);
                    T4A_TLS.with(|tls| assert_eq!(tls.get(), seed));
                }));
            }
        }
        for thread in threads {
            thread.join().unwrap();
        }
        running.store(false, Ordering::Release);
        observer.join().unwrap();
        anyhow::ensure!(progress.load(Ordering::Acquire) > 0);
        drop(running);
        drop(progress);
        let stats = sunlight_libc::alloc::heap_stats();
        let live = (stats.allocated_backing_bytes, stats.live_allocation_count);
        if let Some(expected) = baseline {
            anyhow::ensure!(live == expected, "T4A user allocation drift");
        } else {
            baseline = Some(live);
        }
        sunlight_ipc::debug_log(&format!("[NATIVE-STD] T4A handoff round={round} transfers=512 addresses=4 observer=ok TLS=ok heap={} live={}\n", live.0, live.1));
    }
    sunlight_ipc::debug_log(
        "[NATIVE-STD] T4A mutex/condvar handoffs=1536 independent=4 progress/TLS/resources OK\n",
    );
    Ok(())
}
