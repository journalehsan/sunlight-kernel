//! Deterministic, offline first-scene and request benchmarks. No live-site claims.
use rappid_rabbit::{
    content_shield::{bundled_manager, CanonicalUrl, CosmeticPolicy, ResourceType},
    css::{collect_embedded_stylesheets, StyleContext},
    render::{DocumentRenderState, TextMeasurer},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};
use sunlight_ui::Size;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
struct Counted;
unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let n = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(n, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = System.realloc(p, l, n);
        if !q.is_null() {
            if n >= l.size() {
                let v = LIVE.fetch_add(n - l.size(), Ordering::Relaxed) + n - l.size();
                PEAK.fetch_max(v, Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(l.size() - n, Ordering::Relaxed);
            }
        }
        q
    }
}
#[global_allocator]
static ALLOCATOR: Counted = Counted;
struct Fonts;
impl TextMeasurer for Fonts {
    fn measure_width(&self, s: &str) -> u32 {
        s.len() as u32 * 8
    }
    fn line_height(&self) -> u32 {
        16
    }
}
fn main() {
    let live = LIVE.load(Ordering::Relaxed);
    let start = Instant::now();
    let mut m = bundled_manager();
    let parse_us = start.elapsed().as_micros();
    let retained = LIVE.load(Ordering::Relaxed) - live;
    let peak = PEAK.load(Ordering::Relaxed) - live;
    println!("parse_us={parse_us} counted_retained_bytes={retained} counted_peak_bytes={peak} supported={} unsupported={} estimate_compiled_bytes={}",m.lists[0].compiled.diagnostics.supported,m.lists[0].compiled.diagnostics.unsupported,m.lists[0].compiled.compiled_bytes());
    println!(
        "reject_categories={:?}",
        m.lists[0].compiled.diagnostics.reasons
    );
    let doc = CanonicalUrl::parse("https://fixture.test/").unwrap();
    let requests = [
        "https://fixture.test/images/flower.png",
        "https://fixture.test/ads/banners/sale.png",
        "https://cdn.example.test/style.css",
        "https://doubleclick.net/ad.js",
    ]
    .map(|s| CanonicalUrl::parse(s).unwrap());
    for enabled in [false, true] {
        m.settings.enabled = enabled;
        m.changed();
        let before = LIVE.load(Ordering::Relaxed);
        let mut samples = Vec::new();
        let mut blocked = 0;
        for i in 0..10_000 {
            let start = Instant::now();
            let d = m.evaluate(&requests[i % requests.len()], &doc, ResourceType::Image);
            samples.push(start.elapsed().as_nanos());
            blocked += usize::from(matches!(black_box(d), golden_fish::shield::Decision::Block));
        }
        samples.sort_unstable();
        println!("enabled={enabled} requests=10000 blocked={blocked} match_p50_ns={} match_p95_ns={} match_max_ns={}",samples[5000],samples[9500],samples[9999]);
        drop(samples);
        assert_eq!(before, LIVE.load(Ordering::Relaxed));
        let mut html=String::from("<style>body{margin:0}div{padding:2px}</style><div id='ad-banner-top'>Advertisement</div>");
        for i in 0..250 {
            html.push_str(&format!(
                "<div class='article'><p>Article {i}</p><img src='/images/flower.png'></div>"
            ));
        }
        let mut style_us = 0;
        let mut cosmetic_us = 0;
        let mut cosmetic_match_us = 0;
        let mut layout_us = 0;
        let mut ready_us = 0;
        let mut hidden = 0;
        let mut nodes = 0;
        let pipeline_before = LIVE.load(Ordering::Relaxed);
        let mut cold_pipeline_us = 0;
        for run in 0..20 {
            let ready = Instant::now();
            let dom = golden_fish::parse_html(&html).unwrap();
            nodes = dom.node_count();
            let start = Instant::now();
            let styles = StyleContext::build(&dom, &collect_embedded_stylesheets(&dom));
            style_us += start.elapsed().as_micros();
            let start = Instant::now();
            let policy = CosmeticPolicy::for_document(&m, &doc.url);
            let matching = Instant::now();
            let styles = styles.with_shield(&dom, Some(policy));
            cosmetic_match_us += matching.elapsed().as_micros();
            cosmetic_us += start.elapsed().as_micros();
            hidden = styles.shield_hidden_count();
            let start = Instant::now();
            let scene = black_box(DocumentRenderState::new(
                1,
                doc.url.clone(),
                dom,
                styles,
                Size::new(900, 650),
                &Fonts,
            ));
            layout_us += start.elapsed().as_micros();
            ready_us += ready.elapsed().as_micros();
            if run == 0 {
                cold_pipeline_us = ready.elapsed().as_micros();
            }
            drop(scene);
        }
        println!("enabled={enabled} fixture_runs=20 nodes={nodes} style_mean_us={} cosmetic_stage_mean_us={} cosmetic_match_mean_us={} layout_first_scene_mean_us={} first_scene_pipeline_mean_us={} hidden={hidden}",style_us/20,cosmetic_us/20,cosmetic_match_us/20,layout_us/20,ready_us/20);
        let index_retained = LIVE.load(Ordering::Relaxed).saturating_sub(pipeline_before);
        println!("enabled={enabled} cold_first_scene_pipeline_us={cold_pipeline_us} cosmetic_index_retained_bytes={index_retained}");
    }
}
