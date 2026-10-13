use std::{
    alloc::{GlobalAlloc, Layout, System},
    fs::File,
    io::{Read, Write},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};
use sunlight_hayro_prototype::{into_raster, Document, HayroDocument, MAX_SOURCE_BYTES};
use sunlight_ui::{
    geom::{Rect, Size},
    paint::Canvas,
    widgets::{
        document_canvas::{
            DocumentCanvas, DocumentCanvasMode, DocumentScene, PaintOrder, RenderObject,
            RenderObjectKind,
        },
        DocumentNodeId, RenderObjectId,
    },
    Theme,
};

// Requested live heap bytes, not allocator overhead or RSS. Single-process probe.
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
struct TrackingAllocator;
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(pointer, layout) };
    }
}
#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

fn write_ppm(
    path: &std::path::Path,
    width: u32,
    height: u32,
    pixels: &[u32],
) -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(File::create(path)?);
    write!(output, "P6\n{width} {height}\n255\n")?;
    for pixel in pixels {
        output.write_all(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8])?;
    }
    output.flush()
}

fn read_snapshot(path: &std::ffi::OsStr) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    if length > MAX_SOURCE_BYTES as u64 {
        return Err("document exceeds source budget".into());
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length as usize + 1)?;
    file.take((MAX_SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err("document exceeds source budget".into());
    }
    Ok(bytes)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 {
        return Err("usage: sunlight-hayro-prototype INPUT.pdf OUTPUT_DIRECTORY".into());
    }
    let output = std::path::PathBuf::from(&args[2]);
    std::fs::create_dir_all(&output)?;
    // Bounded snapshot, including files which grow during the read.
    let bytes = read_snapshot(&args[1])?;
    let started = Instant::now();
    let document = HayroDocument::load(bytes)?;
    println!(
        "open_ms={:.3} pages={}",
        started.elapsed().as_secs_f64() * 1000.0,
        document.page_count()
    );
    for page in 0..document.page_count() {
        let info = document.page_info(page)?;
        for scale in [0.75, 1.0, 1.5, 2.0] {
            let started = Instant::now();
            let bitmap = document.render_page(page, scale)?;
            println!("page={} points={:.3}x{:.3} scale={} pixels={}x{} bytes={} render_ms={:.3} live={} peak={}",
                page+1, info.width_pt, info.height_pt, scale, bitmap.width(), bitmap.height(),
                bitmap.data_as_u8_slice().len(), started.elapsed().as_secs_f64()*1000.0,
                LIVE.load(Ordering::Relaxed), PEAK.load(Ordering::Relaxed));
            if scale != 1.0 {
                continue;
            }
            let image = Arc::new(into_raster(bitmap)?);
            write_ppm(
                &output.join(format!("page-{}.ppm", page + 1)),
                image.width,
                image.height,
                &image.pixels,
            )?;
            let size = Size::new(image.width, image.height);
            let mut scene = DocumentScene::new(1, size, size);
            scene.push(RenderObject {
                id: RenderObjectId(1),
                owner_node_id: DocumentNodeId(1),
                bounds: Rect::new(0, 0, size.w, size.h),
                clip_bounds: None,
                paint_order: PaintOrder::default(),
                interaction: None,
                kind: RenderObjectKind::Image {
                    image,
                    source_url: String::new(),
                    intrinsic_width: size.w,
                    intrinsic_height: size.h,
                    alt: "PDF page".into(),
                },
            });
            scene.finalize();
            let mut pixels = Vec::new();
            pixels.try_reserve_exact(size.w as usize * size.h as usize)?;
            pixels.resize(size.w as usize * size.h as usize, 0);
            DocumentCanvas::new(Rect::new(0, 0, size.w, size.h), &[])
                .with_mode(DocumentCanvasMode::ReadOnly)
                .with_scene(&scene)
                .draw(
                    &mut Canvas::new(&mut pixels, size.w, size.w, size.h),
                    &Theme::sunlight_dark(),
                );
            write_ppm(
                &output.join(format!("canvas-{}.ppm", page + 1)),
                size.w,
                size.h,
                &pixels,
            )?;
        }
    }
    drop(document);
    // Warm up library globals before recording the lifecycle baseline.
    let mut baseline = None;
    for cycle in 0..21 {
        let document = HayroDocument::load(read_snapshot(&args[1])?)?;
        for page in 0..document.page_count() {
            drop(document.render_page(page, 1.0)?);
        }
        drop(document);
        println!(
            "closed_cycle={} live={} peak={}",
            cycle,
            LIVE.load(Ordering::Relaxed),
            PEAK.load(Ordering::Relaxed)
        );
        let live = LIVE.load(Ordering::Relaxed);
        if let Some(baseline) = baseline {
            if live != baseline {
                return Err("live allocations changed after close".into());
            }
        } else {
            baseline = Some(live);
        }
    }
    Ok(())
}
