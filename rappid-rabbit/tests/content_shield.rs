#![cfg(feature = "dom")]
use golden_fish::shield::{CanonicalUrl, Decision, FilterListManager, ResourceType};
use rappid_rabbit::{
    content_shield::{self, CosmeticPolicy},
    css::{collect_embedded_stylesheets, StyleContext},
    render::{DocumentRenderState, TextMeasurer},
};
use std::sync::Arc;
use sunlight_ui::{
    widgets::{DocumentCanvas, DocumentCanvasPresentation, RenderObjectKind},
    Point, Rect, Size,
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
fn policy(text: &str) -> FilterListManager {
    let mut m = FilterListManager::new();
    m.add(
        "test",
        "Test",
        "bundled:test",
        &format!("[Adblock Plus 2.0]\n{text}"),
        content_shield::selector_valid,
    )
    .unwrap();
    m
}
fn render(m: &FilterListManager) -> DocumentRenderState {
    let dom=golden_fish::parse_html("<style>body{margin:0}.advert{height:90px}</style><div id='ad' class='advert'><a href='/click'>Advertisement</a><input id='hidden-control'></div><p id='good'>Normal content</p><form><input id='value' name='q' value='ok'><button>Submit</button></form>").unwrap();
    let styles = StyleContext::build(&dom, &collect_embedded_stylesheets(&dom)).with_shield(
        &dom,
        Some(CosmeticPolicy::for_document(m, "https://page.test/")),
    );
    DocumentRenderState::new(
        1,
        "https://page.test/".into(),
        dom,
        styles,
        Size::new(640, 480),
        &Fonts,
    )
}
fn text(state: &DocumentRenderState) -> String {
    state
        .current_scene
        .objects
        .iter()
        .filter_map(|o| match &o.kind {
            RenderObjectKind::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}
#[test]
fn reversible_cosmetics_keep_dom_styles_identity_and_remove_layout_and_hits() {
    let mut m = policy("##.advert");
    let mut s = render(&m);
    let dom = s.dom.clone();
    let count = s.dom.node_count();
    let ad = s.dom.find_first_element("div").unwrap();
    assert!(s.styles.is_shield_hidden(ad));
    assert!(!text(&s).contains("Advertisement"));
    assert!(text(&s).contains("Normal"));
    assert!(!s.layout_geometry.iter().any(|g| g.node_id == ad));
    let canvas = DocumentCanvas::new(Rect::new(0, 0, 640, 480), &[])
        .with_presentation(DocumentCanvasPresentation::Browser)
        .with_scene(&s.current_scene);
    for p in [Point::new(5, 5), Point::new(30, 20), Point::new(40, 60)] {
        assert!(canvas
            .hit_test(p)
            .is_none_or(|o| o.owner_node_id.0 as usize != ad));
    }
    for _ in 0..20 {
        m.settings.enabled = false;
        m.changed();
        s.set_shield_policy(CosmeticPolicy::for_document(&m, &s.final_url), &Fonts);
        assert!(text(&s).contains("Advertisement"));
        m.settings.enabled = true;
        m.changed();
        s.set_shield_policy(CosmeticPolicy::for_document(&m, &s.final_url), &Fonts);
        assert!(!text(&s).contains("Advertisement"));
        assert_eq!(s.dom.node_count(), count);
        assert!(Arc::ptr_eq(&dom, &s.dom));
        assert_eq!(s.dom.get_attribute(ad, "class"), Some("advert"));
    }
}
#[test]
fn strict_selector_subset_never_drops_unsupported_components() {
    for good in [
        "#ad",
        ".ad",
        "div.ad > a[href^=\"https:\"]",
        "[data-ad]",
        "div .ad",
        "[class~=advert]",
    ] {
        assert!(content_shield::selector_valid(good), "{good}");
    }
    for bad in [
        "div:has(.ad)",
        "div:hover",
        "div:nth-child(2)",
        "div + span",
        "> .ad",
        ".ad >",
        ".ad >> p",
        ".ad, div:has(a)",
        ".ad{display:none}",
        "+js(foo)",
        "[a=]",
        "[a=x y]",
        "[a=\"x\" \"y\"]",
        "[a=x.y]",
        "[a='unterminated]",
        ".ad\\31",
    ] {
        assert!(!content_shield::selector_valid(bad), "{bad}");
    }
    let c = golden_fish::shield::compile(
        "[Adblock Plus 2.0]\n##.safe\n##div:has(.ad)\n##.ad, div:hover",
        content_shield::selector_valid,
    )
    .unwrap();
    assert_eq!(c.diagnostics.supported, 1);
    assert_eq!(c.diagnostics.unsupported, 2);
}
#[test]
fn list_and_site_toggles_remove_cosmetic_and_network_effects() {
    let mut m = policy("/ads/*$image\n##.advert");
    let req = CanonicalUrl::parse("https://cdn.test/ads/banner.png").unwrap();
    let page = CanonicalUrl::parse("https://page.test/").unwrap();
    assert_eq!(
        m.evaluate(&req, &page, ResourceType::Image),
        Decision::Block
    );
    m.settings.disabled_lists.insert("test".into());
    assert!(text(&render(&m)).contains("Advertisement"));
    assert_eq!(
        m.evaluate(&req, &page, ResourceType::Image),
        Decision::Allow
    );
    m.settings.disabled_lists.clear();
    m.settings.set_site_exception("page.test", true);
    assert!(text(&render(&m)).contains("Advertisement"));
    assert_eq!(
        m.evaluate(&req, &page, ResourceType::Image),
        Decision::Allow
    );
    m.settings.set_site_exception("page.test", false);
    assert!(!text(&render(&m)).contains("Advertisement"));
}

#[test]
fn adversarial_dom_features_do_not_allocate_unbounded_selector_keys() {
    let m = policy("##.advert");
    let html = format!(
        "<div class='advert'>ad</div><p class='{}'>normal</p>",
        "x ".repeat(1000)
    );
    let dom = golden_fish::parse_html(&html).unwrap();
    let count = dom.node_count();
    assert!(CosmeticPolicy::for_document(&m, "https://page.test/")
        .hidden_nodes(&dom)
        .is_empty());
    assert_eq!(count, dom.node_count());
}
#[test]
fn pinned_easylist_is_offline_and_filters_real_snapshot_rules() {
    let m = content_shield::bundled_manager();
    let c = &m.lists[0].compiled;
    assert_eq!(c.version.as_deref(), Some("202611081212"));
    assert!(c.diagnostics.supported > 1000);
    assert!(c.diagnostics.unsupported > 0);
    assert!(c.compiled_bytes() < 48 * 1024 * 1024);
    assert_eq!(
        content_shield::request_decision(
            &m,
            "https://page.test/ads/banners/sale.png",
            "https://page.test/",
            ResourceType::Image
        ),
        Decision::Block
    );
    assert_eq!(
        content_shield::request_decision(
            &m,
            "https://page.test/images/flower.png",
            "https://page.test/",
            ResourceType::Image
        ),
        Decision::Allow
    );
    let dom = golden_fish::parse_html("<div id='ad-banner-top'>ad</div><p>good</p>").unwrap();
    assert!(CosmeticPolicy::for_document(&m, "https://page.test/")
        .hidden_nodes(&dom)
        .contains(&dom.find_first_element("div").unwrap()));
}
#[cfg(feature = "js-dom-fixture")]
#[test]
fn sunjs_mutations_reapply_policy_and_keep_source_nodes_and_forms() {
    let m = policy("##.advert");
    let mut s = render(&m);
    let mut session = rappid_rabbit::dom_fixture::FixtureSession::new(&s).unwrap();
    session
        .evaluate(
            &mut s,
            "if(window.ContentShield !== undefined) throw 'unexpected privileged API';",
            &Fonts,
        )
        .unwrap();
    assert!(m.settings.enabled);
    for i in 0..20 {
        session.evaluate(&mut s,&format!("document.getElementById('ad').setAttribute('class','{}'); document.getElementById('value').value='ok{i}';",if i%2==0{"ordinary"}else{"advert"}),&Fonts).unwrap();
        assert_eq!(text(&s).contains("Advertisement"), i % 2 == 0);
        session.evaluate(&mut s,"if(!document.getElementById('ad') || !document.getElementById('hidden-control')) throw 'removed';",&Fonts).unwrap();
    }
    session
        .evaluate(
            &mut s,
            "document.getElementById('ad').innerHTML='<a>changed</a>';",
            &Fonts,
        )
        .unwrap();
    assert!(s.styles.shield_hidden_count() > 0);
    assert!(!text(&s).contains("changed"));
}
