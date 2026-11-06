use golden_fish::{parse_html, Node};
#[test]
fn captured_example_keeps_real_text_without_tag_fragments() {
    let doc = parse_html(include_str!("fixtures/example-live.html")).unwrap();
    let text: String = (0..doc.node_count())
        .filter_map(|id| match doc.get(id) {
            Some(Node::Text { content }) => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert!(!text.contains("s.js>"), "{text}");
    assert!(text.contains("This domain is for use in documentation examples"));
    assert!(text.contains("monitoring purposes."));
    assert!(doc.find_first_element("h1").is_none());
    let script = doc.find_first_element("script").unwrap();
    assert_eq!(
        doc.attributes(script)
            .unwrap()
            .iter()
            .find(|a| a.name() == "src")
            .unwrap()
            .value(),
        "/s.js"
    );
}
#[test]
fn slash_values_quotes_whitespace_and_malformed_neighbors_preserve_text() {
    for value in [
        "/s.js",
        "scripts/a.js",
        "https://example.com/a.js",
        "a.js/",
        "'/s.js'",
        "\"/s.js\"",
    ] {
        for space in [" ", "\t", "\r", "\n", "\u{c}"] {
            let doc = parse_html(&format!(
                "<body>before<script src={value}{space}></script><p>after /s.js&gt;</p></body>"
            ))
            .unwrap();
            let body = doc.find_first_element("body").unwrap();
            let direct: String = doc
                .children(body)
                .iter()
                .filter_map(|id| doc.text_content(*id))
                .collect();
            assert_eq!(direct, "before", "{value:?} {space:?}");
            let paragraph = doc.find_first_element("p").unwrap();
            let text: String = doc
                .children(paragraph)
                .iter()
                .filter_map(|id| doc.text_content(*id))
                .collect();
            assert_eq!(text, "after /s.js>");
        }
    }
    let doc = parse_html("before<script src=/unterminated").unwrap();
    assert!(doc.text_content(1).unwrap_or("").contains("before"));
}
