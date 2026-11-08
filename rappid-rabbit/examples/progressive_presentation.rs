//! Repeatable offline renderer benchmark. --legacy reproduces fetch-all before
//! publication; default uses staged resource updates and the UI paint policy.
//! The 30ms/resource delay is controlled, not a claim about network throughput.
use golden_fish::parse_html;
use rappid_rabbit::{
    css::{collect_embedded_stylesheets, StyleContext},
    images::{decode_image, ImageCache},
    presentation::ImagePresentation,
    render::{DocumentRenderState, TextMeasurer},
};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};
use sunlight_ui::{
    widgets::{DocumentCanvas, DocumentCanvasPresentation, RenderObjectKind},
    Canvas, Rect, Size, Theme,
};
struct Fonts;
impl TextMeasurer for Fonts {
    fn measure_width(&self, s: &str) -> u32 {
        s.len() as u32 * 8
    }
    fn line_height(&self) -> u32 {
        16
    }
}
fn paint(
    state: &DocumentRenderState,
    policy: Option<&ImagePresentation>,
    pixels: &mut [u32],
) -> usize {
    let written = Cell::new(0);
    let widget = DocumentCanvas::new(Rect::new(0, 0, 640, 480), &[])
        .with_scene(&state.current_scene)
        .with_presentation(DocumentCanvasPresentation::Browser)
        .with_image_paint_counter(&written);
    let widget = if let Some(policy) = policy {
        widget.with_image_presentation(policy.admitted())
    } else {
        widget
    };
    widget.draw(
        &mut Canvas::new(pixels, 640, 640, 480),
        &Theme::sunlight_dark(),
    );
    written.get()
}
fn snapshot(name: &str, legacy: bool, stage: &str, pixels: &[u32]) {
    if let Ok(dir) = std::env::var("RABBIT_EVIDENCE_DIR") {
        use std::io::Write;
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = std::fs::File::create(format!(
            "{dir}/{name}-{}-{stage}.ppm",
            if legacy { "legacy" } else { "progressive" }
        ))
        .unwrap();
        write!(file, "P6\n640 480\n255\n").unwrap();
        let rgb: Vec<u8> = pixels
            .iter()
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
            .collect();
        file.write_all(&rgb).unwrap();
    }
}

fn main() {
    let legacy = std::env::args().any(|s| s == "--legacy");
    let viewport = Rect::new(0, 0, 640, 480);
    for (name, html, urls, bytes) in [
        (
            "text",
            include_str!("../tests/fixtures/progressive/text.html"),
            vec![],
            include_bytes!("../../sun-img/tests/fixtures/gray.jpg").as_slice(),
        ),
        (
            "large",
            include_str!("../tests/fixtures/progressive/large.html"),
            vec!["portrait.jpg"],
            include_bytes!("../tests/fixtures/portrait-size.jpg").as_slice(),
        ),
        (
            "archwiki",
            include_str!("../tests/fixtures/progressive/archwiki.html"),
            vec!["logo.jpg", "screen.jpg", "footer.jpg"],
            include_bytes!("../../sun-img/tests/fixtures/gray.jpg").as_slice(),
        ),
    ] {
        for iteration in 0..5 {
            let start = Instant::now();
            let dom = parse_html(html).unwrap();
            let styles = StyleContext::build(&dom, &collect_embedded_stylesheets(&dom));
            let mut cache = ImageCache::default();
            if legacy {
                for url in &urls {
                    std::thread::sleep(Duration::from_millis(30));
                    cache.insert_decoded(
                        format!("http://fixture/{url}"),
                        decode_image(bytes, None, url).unwrap(),
                    );
                }
            }
            let mut state = DocumentRenderState::new_with_images(
                1,
                "http://fixture/".into(),
                dom,
                styles,
                Size::new(640, 480),
                &Fonts,
                cache.clone(),
            );
            let mut policy = ImagePresentation::default();
            let mut pixels = vec![0; 640 * 480];
            let initial_writes = paint(
                &state,
                if legacy { None } else { Some(&policy) },
                &mut pixels,
            );
            let initial = start.elapsed().as_micros();
            // Export outside measured intervals: keep IO out of timing samples.
            let initial_pixels = (iteration == 0
                && std::env::var_os("RABBIT_EVIDENCE_DIR").is_some())
            .then(|| pixels.clone());
            let mut first = (initial_writes > 0).then_some(initial);
            let mut visible_complete = if urls.is_empty() || legacy {
                Some(initial)
            } else {
                None
            };
            let mut repaints = 1;
            if !legacy {
                for url in &urls {
                    std::thread::sleep(Duration::from_millis(30));
                    cache.insert_decoded(
                        format!("http://fixture/{url}"),
                        decode_image(bytes, None, url).unwrap(),
                    );
                    state.complete_images(1, &Fonts, cache.clone());
                    while policy.advance(&state.current_scene, state.scene_generation, viewport) {
                        if paint(&state, Some(&policy), &mut pixels) > 0 && first.is_none() {
                            first = Some(start.elapsed().as_micros());
                        }
                        repaints += 1;
                    }
                    if visible_complete.is_none()
                        && policy.visible_complete(&state.current_scene, viewport)
                        && !state.current_scene.objects.iter().any(|o| {
                            matches!(o.kind, RenderObjectKind::ImagePlaceholder { .. })
                                && o.bounds.intersect(viewport).is_some()
                        })
                    {
                        visible_complete = Some(start.elapsed().as_micros());
                    }
                }
            }
            let ready = start.elapsed().as_micros();
            println!("mode={} fixture={name} initial_us={initial} first_image_us={} visible_complete_us={} page_ready_us={ready} layout_runs={} layout_passes={} repaints={repaints} cache_misses={} cache_hits=0",if legacy{"legacy"}else{"progressive"},first.unwrap_or(0),visible_complete.unwrap_or(ready),state.layout_runs,2*state.layout_runs,urls.len());
            if let Some(initial_pixels) = initial_pixels {
                snapshot(name, legacy, "initial", &initial_pixels);
                snapshot(name, legacy, "complete", &pixels);
            }
            std::hint::black_box(pixels);
        }
    }
}
