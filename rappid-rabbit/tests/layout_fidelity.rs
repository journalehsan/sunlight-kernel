use golden_fish::Document;
use rappid_rabbit::{
    css::{collect_embedded_stylesheets, StyleContext},
    layout::{layout_document, FontMeasurer, LayoutBox, LayoutResult, NoImageMetrics},
};
use sunlight_ui::Size;
struct Fonts;
impl FontMeasurer for Fonts {
    fn measure_width(&self, _: &str, size: f32, text: &str) -> f32 {
        text.chars().count() as f32 * size * 0.5
    }
}
fn inspect(html: &str, w: u32, h: u32, check: impl FnOnce(&Document, &LayoutResult<'_>)) {
    let doc = golden_fish::parse_html(html).unwrap();
    let styles = StyleContext::build_with_viewport(&doc, &collect_embedded_stylesheets(&doc), w);
    let result = layout_document(&doc, &styles, Size::new(w, h), &Fonts, &NoImageMetrics);
    check(&doc, &result);
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
        .unwrap()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.05, "{a} != {b}");
}
#[test]
fn captured_google_row_centers_against_48px_at_multiple_viewport_heights() {
    for width in [1056, 756] {
        for height in [400, 642, 900] {
            inspect(
                include_str!("fixtures/google-navigation-layout.html"),
                width,
                height,
                |doc, r| {
                    let row = element(doc, r, "row");
                    let links = element(doc, r, "links");
                    let actions = element(doc, r, "actions");
                    let sign = element(doc, r, "signin");
                    near(row.border_box.y, 6.0);
                    near(row.border_box.right(), width as f32 - 6.0);
                    near(links.border_box.y + links.border_box.height / 2.0, 30.0);
                    near(actions.border_box.y + actions.border_box.height / 2.0, 30.0);
                    near(sign.border_box.y + sign.border_box.height / 2.0, 30.0);
                    near(sign.border_box.width, 85.0);
                    near(sign.used_padding.left, 12.0);
                    assert!(sign.border_box.x >= links.border_box.right());
                    near(element(doc, r, "following").border_box.y, 76.0);
                },
            );
        }
    }
}
#[test]
fn captured_arch_negative_margin_columns_header_and_rss() {
    for width in [1056, 756] {
        inspect(
            include_str!("fixtures/arch-home-layout.html"),
            width,
            642,
            |doc, r| {
                let content = element(doc, r, "content");
                let left = element(doc, r, "content-left");
                let side = element(doc, r, "content-right");
                let heading = element(doc, r, "news-title");
                let label = element(doc, r, "news-label");
                let rss = element(doc, r, "rss");
                near(side.border_box.y, content.content_box.y);
                near(side.border_box.right(), content.content_box.right());
                near(side.used_margin.left, -300.0);
                near(left.border_box.right() + 40.0, side.border_box.x);
                near(label.border_box.width, 102.5); // 11 * 7.5px + 20px padding
                assert_eq!(
                    label.style.background_color,
                    sunlight_ui::Color::rgb(23, 148, 209)
                );
                near(rss.border_box.right(), left.content_box.right());
                assert!(rss.border_box.y < heading.border_box.bottom());
                assert!(
                    element(doc, r, "article-title").border_box.y >= heading.margin_box.bottom()
                );
                assert!(element(doc, r, "footer").border_box.y >= side.margin_box.bottom());
            },
        );
    }
}
#[test]
fn row_uses_padding_border_margins_and_gaps_once() {
    inspect("<style>body{margin:0}.row{display:flex;width:300px;height:40px;align-items:center;gap:10px}.item{width:50px;height:10px;padding:5px;border:2px solid black;margin:3px}</style><div class=row><span class=item id=a></span><span class=item id=b></span></div>",400,900,|d,r|{
        let a=element(d,r,"a");let b=element(d,r,"b");
        near(a.border_box.width,64.0);near(a.border_box.y,8.0);
        near(b.border_box.x-a.border_box.right(),16.0);
    });
}
#[test]
fn auto_height_flex_uses_its_line_height_at_a_nonzero_origin() {
    inspect("<style>body{margin:0}.row{display:flex;width:300px;align-items:center;justify-content:space-between}.a{width:30px;height:40px}.b{width:20px;height:10px}</style><div style='height:100px'></div><div class=row id=row><div class=a id=a></div><div class=b id=b></div></div>",500,900,|d,r|{
        near(element(d,r,"row").border_box.height,40.0);
        near(element(d,r,"b").border_box.y,115.0);
        near(element(d,r,"b").border_box.right(),300.0);
    });
}
#[test]
fn floats_wrap_at_narrow_width_and_clear_only_the_requested_side() {
    for (width, y) in [(300, 0.0), (150, 30.0)] {
        inspect("<style>body{margin:0}.a{float:left;width:100px;height:30px}.b{float:right;width:100px;height:50px}.c{clear:left;height:10px}</style><div class=a></div><div class=b id=b></div><div class=c id=c></div>",width,400,|d,r|{
            near(element(d,r,"b").border_box.y,y);
            near(element(d,r,"b").border_box.right(),width as f32);
            near(element(d,r,"c").border_box.y,30.0);
        });
    }
}
#[test]
fn inline_text_returns_to_full_width_below_a_float() {
    inspect("<style>body{margin:0;font-size:10px;line-height:12px}.f{float:left;width:60px;height:24px}</style><div class=f></div>one two three four five six seven eight nine ten eleven twelve",120,400,|_,r|{
        let words:Vec<_>=r.render_boxes.iter().filter(|b|matches!(b.node_type,rappid_rabbit::layout::BoxType::TextRun(_))).collect();
        assert!(words.iter().any(|b|b.bounds.y<24.0&&b.bounds.x>=60.0));
        assert!(words.iter().any(|b|b.bounds.y>=24.0&&b.bounds.x==0.0));
    });
}

#[test]
fn floated_text_keeps_fragments_and_negative_vertical_margin() {
    inspect("<style>body{margin:0}#f{float:right;margin-top:-12px}</style><div style='height:30px'></div><p id=f>timestamp</p>",300,400,|d,r|{
        let f=element(d,r,"f");
        near(f.used_margin.top,-12.0);
        assert!(r.render_boxes.iter().any(|b|matches!(b.node_type,rappid_rabbit::layout::BoxType::TextRun("timestamp"))));
    });
}

#[test]
fn flex_form_controls_do_not_count_their_border_and_padding_twice() {
    inspect("<style>body{margin:0}.row{display:flex;gap:10px}</style><div class=row><button id=a>Go</button><button id=b>More</button></div>",300,400,|d,r|{
        let a=element(d,r,"a");let b=element(d,r,"b");
        near(b.border_box.x-a.border_box.right(),10.0);
        assert!(a.border_box.width>0.0&&b.border_box.width>a.border_box.width);
    });
}

#[test]
fn descendant_line_boxes_share_float_exclusions_without_shrinking_the_block() {
    inspect("<style>body{margin:0;font-size:10px;line-height:12px}#logo{float:left;width:60px;height:36px}#menu{padding:4px;border:1px solid black}#menu span{display:inline-block;width:40px;height:12px}</style><div id=logo></div><div id=menu><div><span id=a></span><span id=b></span><span id=c></span><span id=d></span></div></div><div style='clear:both' id=clear></div>",160,400,|d,r|{
        let logo=element(d,r,"logo");let menu=element(d,r,"menu");
        near(menu.border_box.width,160.0);
        for id in ["a","b","c","d"] {
            let b=element(d,r,id);
            assert!(b.border_box.y>=logo.margin_box.bottom() || b.border_box.x>=logo.margin_box.right(),"{id}: {:?}",b.border_box);
        }
        assert!(element(d,r,"clear").border_box.y>=logo.margin_box.bottom());
    });
}
#[test]
fn descendant_floats_affect_following_siblings_and_stop_at_inline_blocks() {
    inspect("<style>body{margin:0;font-size:10px;line-height:12px}.f{float:left;width:60px;height:48px}.island{display:inline-block;width:100px}</style><div><div class=f id=f></div></div><div id=after>one two three</div><div style='clear:both'></div><div class=island id=island><div class=f></div></div><div id=following>following text</div>",160,400,|d,r|{
        let f=element(d,r,"f");let after=element(d,r,"after");
        near(after.border_box.width,160.0);
        assert!(r.render_boxes.iter().any(|b|matches!(b.node_type,rappid_rabbit::layout::BoxType::TextRun("one"))&&b.bounds.x>=f.margin_box.right()));
        let island=element(d,r,"island");
        assert!(island.border_box.height>=48.0);
        assert!(element(d,r,"following").border_box.y>=island.border_box.bottom());
    });
}
