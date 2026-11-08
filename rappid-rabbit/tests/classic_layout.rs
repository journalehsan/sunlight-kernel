use golden_fish::Document;
use rappid_rabbit::{
    css::{collect_embedded_stylesheets, Property, PropertyValue, StyleContext},
    layout::{layout_document, BoxType, FontMeasurer, LayoutBox, LayoutResult, NoImageMetrics},
    render::DocumentRenderState,
};
use sunlight_ui::{widgets::RenderObjectKind, Size};

struct Fonts;
impl FontMeasurer for Fonts {
    fn measure_width(&self, _: &str, size: f32, text: &str) -> f32 {
        text.chars().count() as f32 * size * 0.5
    }
}
impl rappid_rabbit::render::TextMeasurer for Fonts {
    fn measure_width(&self, text: &str) -> u32 {
        text.len() as u32 * 7
    }
    fn line_height(&self) -> u32 {
        16
    }
}
fn inspect(
    html: &str,
    width: u32,
    check: impl FnOnce(&Document, &StyleContext, &LayoutResult<'_>),
) {
    let doc = golden_fish::parse_html(html).unwrap();
    let styles =
        StyleContext::build_with_viewport(&doc, &collect_embedded_stylesheets(&doc), width);
    let result = layout_document(
        &doc,
        &styles,
        Size::new(width, 642),
        &Fonts,
        &NoImageMetrics,
    );
    check(&doc, &styles, &result);
}
fn element<'a, 'b>(doc: &Document, result: &'b LayoutResult<'a>, id: &str) -> &'b LayoutBox<'a> {
    result
        .geometry
        .boxes
        .iter()
        .find(|b| {
            doc.attributes(b.dom_node)
                .is_some_and(|attrs| attrs.iter().any(|a| a.name() == "id" && a.value() == id))
        })
        .unwrap_or_else(|| panic!("missing {id}"))
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.05, "{a} != {b}");
}

#[test]
fn package_columns_align_wrap_stripe_and_follow_the_search_filters() {
    for width in [1056, 754] {
        inspect(
            include_str!("fixtures/classic-package-list.html"),
            width,
            |d, _, r| {
                for ids in [
                    ["h-name", "name", "next-name"],
                    ["h-version", "version", "next-version"],
                    ["h-desc", "description", "next-description"],
                ] {
                    let first = element(d, r, ids[0]);
                    for id in &ids[1..] {
                        near(element(d, r, id).border_box.x, first.border_box.x);
                        near(element(d, r, id).border_box.width, first.border_box.width);
                    }
                }
                let table = element(d, r, "packages");
                near(table.border_box.width, width as f32);
                assert!(table.border_box.y >= element(d, r, "filters").border_box.bottom());
                assert!(element(d, r, "filters").border_box.height > 20.0);
                let desc = element(d, r, "description");
                assert!(desc.border_box.height > element(d, r, "header").border_box.height);
                assert!(r
                    .render_boxes
                    .iter()
                    .filter(|b| matches!(b.node_type, BoxType::TextRun(_))
                        && b.bounds.y >= desc.content_box.y
                        && b.bounds.y < desc.content_box.bottom())
                    .all(|b| b.bounds.right() <= table.border_box.right() + 0.05));
                assert_eq!(
                    element(d, r, "second").style.background_color,
                    sunlight_ui::Color::rgb(228, 238, 255)
                );
                assert!(element(d, r, "after").border_box.y >= table.border_box.bottom());
            },
        );
    }
}

#[test]
fn distro_nested_tables_and_omitted_ends_keep_three_columns() {
    for width in [1056, 754] {
        inspect(
            include_str!("fixtures/classic-distro-tables.html"),
            width,
            |d, _, r| {
                let left = element(d, r, "left");
                let center = element(d, r, "center");
                let right = element(d, r, "right");
                near(left.border_box.y, center.border_box.y);
                near(center.border_box.y, right.border_box.y);
                assert!(center.border_box.x >= left.border_box.right());
                assert!(right.border_box.x >= center.border_box.right());
                assert!(center.border_box.width > left.border_box.width * 2.0);
                assert!(right.border_box.right() <= width as f32);
                let updates = element(d, r, "updates");
                assert!(
                    element(d, r, "updates-title").border_box.width
                        > element(d, r, "date").border_box.width
                );
                assert!(updates.border_box.right() <= left.content_box.right() + 0.05);
                assert_eq!(
                    d.parent(element(d, r, "distribution").dom_node),
                    d.parent(element(d, r, "date").dom_node)
                );
                assert!(
                    element(d, r, "following").border_box.y
                        >= element(d, r, "outer").border_box.bottom()
                );
            },
        );
    }
}

#[test]
fn rowspans_reserve_slots_and_expand_rows_without_overlap() {
    inspect("<style>body{margin:0}table{width:300px;border-spacing:2px}td{padding:3px}</style><table><tr><td id=a rowspan=2><div style='height:80px'>Tall</div></td><td id=b>One</td></tr><tr><td id=c>Two</td></tr><tr><td id=d>Three</td><td id=e>Four</td></tr></table>", 400, |d,_,r| {
        let a=element(d,r,"a"); let b=element(d,r,"b"); let c=element(d,r,"c");
        near(b.border_box.x,c.border_box.x); near(a.border_box.bottom(),c.border_box.bottom());
        assert!(b.border_box.right() <= 300.0); assert!(c.border_box.y >= b.border_box.bottom());
        assert!(element(d,r,"d").border_box.y >= a.border_box.bottom());
    });
}

#[test]
fn rowspan_zero_stops_at_its_row_group() {
    inspect("<style>body{margin:0}table{width:300px}</style><table><tbody><tr><td rowspan=0 id=a>A</td><td>B</td></tr><tr><td id=b>B</td></tr></tbody><tbody><tr><td id=c>C</td><td>D</td></tr></tbody></table>", 400, |d,_,r| {
        near(element(d,r,"a").border_box.x,element(d,r,"c").border_box.x);
        assert!(element(d,r,"b").border_box.x > element(d,r,"c").border_box.x);
        assert!(element(d,r,"c").border_box.y >= element(d,r,"a").border_box.bottom());
    });
}

#[test]
fn table_cell_vertical_alignment_moves_contents_and_keeps_equal_cell_heights() {
    inspect("<style>body{margin:0}table{width:300px;border-spacing:0}td{padding:0}</style><table><tr><td><div style='height:80px'>Tall</div></td><td valign=middle id=middle><div id=mid style='height:20px'>Middle</div></td><td valign=bottom id=bottom><div id=low style='height:20px'>Bottom</div></td></tr></table>",400,|d,_,r|{
        near(element(d,r,"middle").border_box.height,80.0); near(element(d,r,"bottom").border_box.height,80.0);
        near(element(d,r,"mid").border_box.y,30.0); near(element(d,r,"low").border_box.y,60.0);
    });
}

#[test]
fn html_table_hints_lose_to_author_css_and_explicit_inherit_works() {
    inspect("<style>body{margin:0}#table{width:240px;border-spacing:0}td{padding:4px;background:white}tr{vertical-align:top}</style><table width=500 cellspacing=8 cellpadding=12 id=table><tr><td width=100 bgcolor=red id=cell>Cell</td><td>Other</td></tr></table>",600,|d,s,r|{
        near(element(d,r,"table").border_box.width,240.0); near(element(d,r,"cell").used_padding.left,4.0);
        assert_eq!(element(d,r,"cell").style.background_color,sunlight_ui::Color::rgb(255,255,255));
        assert_eq!(s.style_for(element(d,r,"cell").dom_node).unwrap().value(&Property::VerticalAlign),Some(&PropertyValue::Keyword("top".into())));
    });
}

#[test]
fn wiki_invisible_positioned_dropdowns_do_not_push_down_the_article() {
    for width in [1056, 754] {
        inspect(
            include_str!("fixtures/wiki-static-layout.html"),
            width,
            |d, _, r| {
                near(element(d, r, "header").border_box.height, 48.0);
                near(element(d, r, "article").border_box.y, 48.0);
                assert!(!r
                    .render_boxes
                    .iter()
                    .any(|b| b.node_id == element(d, r, "checkbox").dom_node));
                assert!(!r
                    .render_boxes
                    .iter()
                    .any(|b| matches!(b.node_type, BoxType::TextRun("Hidden menu"))));
                near(element(d, r, "title").border_box.x, 0.0);
                near(element(d, r, "languages").border_box.right(), width as f32);
                near(
                    element(d, r, "a").border_box.y,
                    element(d, r, "c").border_box.y,
                );
                near(
                    element(d, r, "b").border_box.y,
                    element(d, r, "d").border_box.y,
                );
                near(element(d, r, "c").border_box.x, (width as f32 + 20.0) / 2.0);
            },
        );
    }
}

#[test]
fn positioned_offsets_use_nearest_positioned_ancestor_and_leave_normal_flow() {
    inspect("<style>body{margin:0}#parent{position:relative;margin:20px;padding:10px;width:200px;height:100px}#abs{position:absolute;right:5px;bottom:7px;width:30px;height:12px}</style><div id=parent><div id=abs></div><div id=normal style='height:10px'></div></div>",400,|d,_,r|{
        let parent=element(d,r,"parent");let abs=element(d,r,"abs");
        near(abs.border_box.right(),parent.padding_box.right()-5.0);near(abs.border_box.bottom(),parent.padding_box.bottom()-7.0);
        near(element(d,r,"normal").border_box.y,parent.content_box.y);
    });
}

#[test]
fn fixed_update_table_clips_long_names_and_keeps_dates_visible() {
    let html="<style>body{margin:0}table{width:280px;table-layout:fixed;border-spacing:0}td{padding:0;white-space:nowrap}.name{overflow:hidden}</style><table><tr><td class=name><a href='/pkg'>a-very-long-package-name-that-must-not-cover-the-date</a></td><td>2026-11-07</td></tr></table>";
    let dom = golden_fish::parse_html(html).unwrap();
    let styles = StyleContext::build(&dom, &collect_embedded_stylesheets(&dom));
    let state = DocumentRenderState::new(
        1,
        "https://example.org/".into(),
        dom,
        styles,
        Size::new(400, 642),
        &Fonts,
    );
    let text=state.current_scene.objects.iter().find(|o| matches!(&o.kind,RenderObjectKind::Text{text,..} if text.starts_with("a-very-long"))).unwrap();
    assert_eq!(text.clip_bounds.unwrap().w, 140);
    let date = state
        .current_scene
        .objects
        .iter()
        .find(|o| matches!(&o.kind,RenderObjectKind::Text{text,..} if text=="2026-11-07"))
        .unwrap();
    assert_eq!(date.bounds.x, 140);
    assert!(date.clip_bounds.is_none());
    assert!(state
        .current_scene
        .hit_test(sunlight_ui::Point::new(275, 7))
        .unwrap()
        .interaction
        .is_none());
}

#[test]
fn google_legacy_center_and_percentage_side_cells_keep_the_search_box_centered() {
    inspect("<style>body{margin:0}td{padding:0}</style><center><form><table id=table cellspacing=0 cellpadding=0><tr valign=top><td width='25%'>&nbsp;</td><td align=center nowrap><input id=search style='width:400px;box-sizing:border-box'><div><button>Search</button></div></td><td width='25%' nowrap>Advanced</td></tr></table></form></center>",1056,|d,_,r|{
        let table=element(d,r,"table");let search=element(d,r,"search");
        near(table.border_box.x+table.border_box.width*0.5,528.0);
        near(search.border_box.x+search.border_box.width*0.5,528.0);
        near(table.border_box.width,800.0);
    });
}

#[test]
fn failed_header_icon_does_not_turn_into_a_large_unresolved_image_placeholder() {
    let dom=golden_fish::parse_html("<style>body{margin:0}</style><img src='/missing.png'><img src='/missing.png' width=40 height=20>").unwrap();
    let styles = StyleContext::build(&dom, &collect_embedded_stylesheets(&dom));
    let mut cache = rappid_rabbit::images::ImageCache::default();
    cache.insert_failed("https://example.org/missing.png".into(), "HTTP 404");
    let state = DocumentRenderState::new_with_images(
        1,
        "https://example.org/".into(),
        dom,
        styles,
        Size::new(400, 642),
        &Fonts,
        cache,
    );
    let images: Vec<_> = state
        .current_scene
        .objects
        .iter()
        .filter(|o| matches!(&o.kind, RenderObjectKind::ImagePlaceholder { .. }))
        .collect();
    assert_eq!((images[0].bounds.w, images[0].bounds.h), (16, 16));
    assert_eq!((images[1].bounds.w, images[1].bounds.h), (40, 20));
}
