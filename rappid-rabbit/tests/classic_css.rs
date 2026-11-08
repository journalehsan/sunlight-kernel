use rappid_rabbit::css::{self, Property, PropertyValue, StyleContext, StylesheetSource};

fn styles(html: &str) -> (golden_fish::Document, StyleContext) {
    let dom = golden_fish::parse_html(html).unwrap();
    let styles = StyleContext::build(&dom, &css::collect_embedded_stylesheets(&dom));
    (dom, styles)
}

#[test]
fn attribute_selectors_preserve_quoted_spaces_and_attribute_specificity() {
    let (dom,s) = styles("<style>input{color:red}.form input[type = 'text'][data-title='two words']{color:blue}</style><div class=form><input type=text data-title='two words'></div>");
    let id = dom.find_first_element("input").unwrap();
    assert_eq!(
        s.style_for(id).unwrap().value(&Property::Color),
        Some(&PropertyValue::Color(css::Color::Rgb(0, 0, 255)))
    );
    let sheet = css::parse_stylesheet(".form [type='text']{color:red}", StylesheetSource::Embedded);
    assert_eq!(sheet.rules[0].selectors[0].parts.len(), 2);
    assert_eq!(
        css::selector_specificity(&sheet.rules[0].selectors[0]).classes,
        2
    );
}

#[test]
fn is_not_and_where_keep_argument_lists_and_css_specificity() {
    let (dom,s)=styles("<style>:where(#a){color:red}div{color:blue}:is(#b,.other):not([hidden],.disabled){color:green}</style><div id=a>A</div><div id=b>B</div><div class='other disabled'>Disabled</div>");
    let ids: Vec<_> = (0..dom.node_count())
        .filter(|&id| dom.tag_name(id) == Some("div"))
        .collect();
    assert_eq!(
        s.style_for(ids[0]).unwrap().value(&Property::Color),
        Some(&PropertyValue::Color(css::Color::Rgb(0, 0, 255)))
    );
    assert_eq!(
        s.style_for(ids[1]).unwrap().value(&Property::Color),
        Some(&PropertyValue::Color(css::Color::Rgb(0, 128, 0)))
    );
    assert_eq!(
        s.style_for(ids[2]).unwrap().value(&Property::Color),
        Some(&PropertyValue::Color(css::Color::Rgb(0, 0, 255)))
    );
    let sheet = css::parse_stylesheet(":not(.a):not(#b){color:red}", StylesheetSource::Embedded);
    let specificity = css::selector_specificity(&sheet.rules[0].selectors[0]);
    assert_eq!((specificity.ids, specificity.classes), (1, 1));
}

#[test]
fn static_unfocused_screen_reader_link_keeps_its_small_positioned_box() {
    let (dom,s)=styles("<style>.jump:not(:focus){position:absolute;width:1px;height:1px;overflow:hidden}</style><a class=jump>Jump to content</a>");
    let id = dom.find_first_element("a").unwrap();
    assert_eq!(
        s.style_for(id).unwrap().value(&Property::Position),
        Some(&PropertyValue::Keyword("absolute".into()))
    );
    assert_eq!(
        s.style_for(id).unwrap().value(&Property::Width),
        Some(&PropertyValue::LengthPx(1))
    );
}

#[test]
fn media_queries_evaluate_all_bounds_print_dark_and_comma_alternatives() {
    for (query, width, expected) in [
        (
            "screen and (min-width:700px) and (max-width:1100px)",
            1056,
            true,
        ),
        (
            "screen and (min-width:700px) and (max-width:1100px)",
            1200,
            false,
        ),
        ("screen and (max-width:calc(1120px - 1px))", 1056, true),
        ("screen and (max-width:calc(1120px - 1px))", 1120, false),
        ("print and (min-width:1px)", 1056, false),
        ("screen and (prefers-color-scheme:dark)", 1056, false),
        ("screen and (prefers-color-scheme:light)", 1056, true),
        ("not screen", 1056, false),
        ("(max-width:600px), (min-width:1200px)", 1056, false),
        ("(max-width:600px), (min-width:1200px)", 1400, true),
        ("(min-width:45em)", 754, true),
    ] {
        assert_eq!(
            css::import_media_active(query, width),
            (expected, None),
            "{query} at {width}"
        );
    }
}

#[test]
fn media_reflow_switches_nested_query_rules_without_enabling_print() {
    let (dom,s)=styles("<style>@media (max-width:600px), (min-width:1000px){@media screen and (max-width:1100px){div{color:red}}}@media print and (min-width:1px){div{display:none}}</style><div>Article</div>");
    let id = dom.find_first_element("div").unwrap();
    assert_eq!(
        s.style_for(id).unwrap().value(&Property::Color),
        Some(&PropertyValue::Color(css::Color::Rgb(255, 0, 0)))
    );
    let narrow = s.for_viewport(&dom, 754);
    assert_ne!(
        narrow.style_for(id).unwrap().value(&Property::Color),
        Some(&PropertyValue::Color(css::Color::Rgb(255, 0, 0)))
    );
    assert_ne!(
        narrow.style_for(id).unwrap().value(&Property::Display),
        Some(&PropertyValue::Keyword("none".into()))
    );
}

#[test]
fn vector_sized_stylesheet_keeps_rules_after_128_kib_with_a_bounded_limit() {
    let text = format!("/*{}*/ #late{{color:blue}}", " ".repeat(172_000));
    let sheet = css::parse_stylesheet(&text, StylesheetSource::Embedded);
    assert_eq!(sheet.rules.len(), 1);
    let oversized = format!(
        "/*{}*/ #discarded{{color:red}}",
        " ".repeat(css::MAX_STYLESHEET_BYTES)
    );
    assert!(
        css::parse_stylesheet(&oversized, StylesheetSource::Embedded)
            .rules
            .is_empty()
    );
    assert!(css::MAX_STYLESHEET_BYTES <= css::MAX_TOTAL_STYLESHEET_BYTES);
}

#[test]
fn font_shorthand_resets_longhands_and_keeps_unitless_line_height_for_children() {
    let (dom,s)=styles("<style>div{font-weight:bold;font-style:italic;line-height:40px;font:normal 13px/1.4 sans-serif}span{font-size:20px}</style><div><span>Text</span></div>");
    let id = dom.find_first_element("div").unwrap();
    let style = s.style_for(id).unwrap();
    assert_eq!(
        style.value(&Property::FontSize),
        Some(&PropertyValue::LengthPx(13))
    );
    assert_eq!(
        style.value(&Property::FontWeight),
        Some(&PropertyValue::Normal)
    );
    assert_eq!(
        style.value(&Property::FontStyle),
        Some(&PropertyValue::Normal)
    );
    let child = dom.find_first_element("span").unwrap();
    assert_eq!(
        s.style_for(child).unwrap().value(&Property::LineHeight),
        Some(&PropertyValue::Raw("1.4".into()))
    );
}

#[test]
fn light_dark_selects_light_color_even_with_function_commas() {
    assert_eq!(
        css::parse_color("light-dark(rgb(10,20,30),#ffffff)"),
        Some(css::Color::Rgb(10, 20, 30))
    );
}

#[test]
fn repeated_table_styles_share_storage_without_losing_cascade_or_media_state() {
    let html=format!("<style>td{{color:blue;padding:2px}}td:nth-child(2){{color:red}}@media(max-width:800px){{td{{padding:4px}}}}</style><table>{}</table>","<tr><td>First</td><td>Second</td></tr>".repeat(1000));
    let (dom, s) = styles(&html);
    let cells: Vec<_> = (0..dom.node_count())
        .filter(|&id| dom.tag_name(id) == Some("td"))
        .collect();
    let first = s.style_for(cells[0]).unwrap();
    let repeated = s.style_for(cells[2]).unwrap();
    assert!(
        core::ptr::eq(first, repeated),
        "identical repeated cells must not retain duplicate styles"
    );
    assert!(!core::ptr::eq(first, s.style_for(cells[1]).unwrap()));
    assert_eq!(
        s.style_for(cells[1]).unwrap().value(&Property::Color),
        Some(&PropertyValue::Color(css::Color::Rgb(255, 0, 0)))
    );
    assert!(first
        .matched_declarations
        .iter()
        .any(|d| d.selector == "td"));
    let narrow = s.for_viewport(&dom, 754);
    assert_eq!(
        narrow
            .style_for(cells[0])
            .unwrap()
            .value(&Property::PaddingLeft),
        Some(&PropertyValue::LengthPx(4))
    );
    assert_eq!(
        first.value(&Property::PaddingLeft),
        Some(&PropertyValue::LengthPx(2))
    );
}
