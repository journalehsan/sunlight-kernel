//! Offline screenshot through Rabbit's retained scene and shared DocumentCanvas.
//! Usage: page_snapshot HTML URL WIDTH HEIGHT OUTPUT.ppm [RESOURCES.tsv]
//! Resource rows: absolute URL, MIME type, local path (tab separated).
use std::{collections::BTreeMap, fs, io::Write};

use rappid_rabbit::{
    css::{self, StyleContext, Stylesheet, StylesheetSource},
    images::{decode_image, ImageCache},
    render::{DocumentRenderState, TextMeasurer},
    resources::discovery::resolve_url,
};
use sun_font::{FontRole, VecFont};
use sunlight_http::ParsedUrl;
use sunlight_ui::{
    widgets::{DocumentCanvas, DocumentCanvasPresentation, DocumentFontFamily},
    Canvas, Rect, Size, Theme, VecText,
};

static UI: VecFont = VecFont(FontRole::UiRegular);
static SMALL: VecFont = VecFont(FontRole::UiSmall);
static LARGE: VecFont = VecFont(FontRole::UiLarge);
static SERIF: VecFont = VecFont(FontRole::SerifRegular);
static MONO: VecFont = VecFont(FontRole::MonoRegular);
struct Fonts;
impl TextMeasurer for Fonts {
    fn measure_width(&self, text: &str) -> u32 {
        UI.measure_w(text)
    }
    fn line_height(&self) -> u32 {
        VecText::line_height(&UI)
    }
    fn measure_width_for_size(&self, family: DocumentFontFamily, size: u32, text: &str) -> u32 {
        match family {
            DocumentFontFamily::Serif => SERIF.measure_w(text),
            DocumentFontFamily::Monospace => MONO.measure_w(text),
            DocumentFontFamily::SansSerif if size >= 24 => LARGE.measure_w(text),
            _ => UI.measure_w(text),
        }
    }
    fn line_height_for(&self, family: DocumentFontFamily) -> u32 {
        match family {
            DocumentFontFamily::Serif => VecText::line_height(&SERIF),
            DocumentFontFamily::Monospace => VecText::line_height(&MONO),
            _ => VecText::line_height(&UI),
        }
    }
}

fn linked_sheets(
    url: &str,
    resources: &BTreeMap<String, (String, String)>,
    width: u32,
    depth: usize,
) -> Vec<Stylesheet> {
    let mut sheets = Vec::new();
    if depth > 8 {
        return sheets;
    }
    let Some((_, path)) = resources.get(url) else {
        eprintln!("missing stylesheet: {url}");
        return sheets;
    };
    let text = fs::read_to_string(path).expect("stylesheet");
    let base = ParsedUrl::parse(url).expect("stylesheet URL");
    for import in css::parse_leading_imports(&text) {
        if css::import_media_active(&import.media, width).0 {
            if let Ok(url) = resolve_url(&base, &import.raw_url) {
                sheets.extend(linked_sheets(&url, resources, width, depth + 1));
            }
        }
    }
    sheets.push(css::parse_stylesheet(
        &text,
        StylesheetSource::External(url.into()),
    ));
    sheets
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert!(
        args.len() >= 6,
        "HTML URL WIDTH HEIGHT OUTPUT.ppm [RESOURCES.tsv]"
    );
    let html = fs::read_to_string(&args[1]).expect("HTML");
    let base = ParsedUrl::parse(&args[2]).expect("URL");
    let width: u32 = args[3].parse().expect("width");
    let height: u32 = args[4].parse().expect("height");
    let mut resources = BTreeMap::new();
    if let Some(path) = args.get(6) {
        for line in fs::read_to_string(path).expect("resources").lines() {
            let parts: Vec<_> = line.splitn(3, '\t').collect();
            assert_eq!(parts.len(), 3, "resource row");
            resources.insert(parts[0].into(), (parts[1].into(), parts[2].into()));
        }
    }
    let dom = golden_fish::parse_html(&html).expect("DOM");
    let mut linked = Vec::new();
    for id in 0..dom.node_count() {
        if let Some(golden_fish::Node::Element {
            tag_name,
            attributes,
            ..
        }) = dom.get(id)
        {
            let attr = |name: &str| {
                attributes
                    .iter()
                    .find(|a| a.name() == name)
                    .map(|a| a.value())
            };
            if tag_name == "link"
                && attr("rel").is_some_and(|r| r.split_whitespace().any(|s| s == "stylesheet"))
            {
                linked.push(
                    attr("href")
                        .and_then(|href| resolve_url(&base, href).ok())
                        .map(|url| linked_sheets(&url, &resources, width, 0)),
                );
            }
        }
    }
    let sheets =
        css::order_document_stylesheets(&dom, css::collect_embedded_stylesheets(&dom), linked);
    let styles = StyleContext::build_with_viewport(&dom, &sheets, width);
    let mut images = ImageCache::default();
    let mut image_urls: Vec<_> =
        rappid_rabbit::resources::discovery::discover_resources(&dom, &base)
            .into_iter()
            .filter(|c| c.resource_type == rappid_rabbit::resources::request::ResourceType::Image)
            .map(|c| c.resolved_url)
            .collect();
    for (url, (mime, _)) in &resources {
        if mime.starts_with("image/") && !image_urls.contains(url) {
            image_urls.push(url.clone());
        }
    }
    for url in image_urls
        .iter()
        .take(rappid_rabbit::images::MAX_IMAGE_CACHE_ENTRIES)
    {
        let Some((mime, path)) = resources.get(url) else {
            continue;
        };
        match decode_image(&fs::read(path).expect("image"), Some(mime), url) {
            Ok(image) => images.insert_decoded(url.clone(), image),
            Err(error) => images.insert_failed(url.clone(), error),
        }
    }
    let state = DocumentRenderState::new_with_images(
        1,
        args[2].clone(),
        dom,
        styles,
        Size::new(width, height),
        &Fonts,
        images,
    );
    let mut pixels = vec![0xffffffff; width as usize * height as usize];
    DocumentCanvas::new(Rect::new(0, 0, width, height), &[])
        .with_presentation(DocumentCanvasPresentation::Browser)
        .with_fonts(Some(&LARGE), Some(&SMALL), Some(&UI), Some(&SMALL))
        .with_scene_font_families(Some(&SERIF), Some(&MONO))
        .with_scene(&state.current_scene)
        .draw(
            &mut Canvas::new(&mut pixels, width, width, height),
            &Theme::sunlight_light(),
        );
    let mut out = std::io::BufWriter::new(fs::File::create(&args[5]).expect("output"));
    write!(out, "P6\n{width} {height}\n255\n").unwrap();
    for pixel in pixels {
        out.write_all(&[(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8])
            .unwrap();
    }
    eprintln!(
        "{} content={:?}",
        state.diagnostics.summary(),
        state.current_scene.content_size
    );
}
