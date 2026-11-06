use golden_fish::{parse_html, Node};

#[test]
fn script_comparisons_leave_body_outside_hidden_head() {
    let doc = parse_html(include_str!("fixtures/script-head.html")).unwrap();
    let html = doc.find_first_element("html").unwrap();
    let head = doc.find_first_element("head").unwrap();
    let body = doc.find_first_element("body").unwrap();
    assert_eq!(doc.parent(head), Some(html));
    assert_eq!(doc.parent(body), Some(html));
    assert!(doc.find_first_element("div").is_none());
    let script = doc.find_first_element("script").unwrap();
    assert_eq!(doc.children(script).len(), 1);
    assert_eq!(
        doc.text_content(doc.children(script)[0]),
        Some("for(var i=0;i<10;i++){if(i<5){window.label=\"<div>Search</div>\";}}")
    );
}

#[test]
fn raw_text_closes_only_on_matching_name_and_delimiter() {
    for tag in ["script", "style", "SCRIPT", "STYLE"] {
        for closing in [
            tag.to_owned(),
            tag.to_ascii_uppercase(),
            format!("{tag}/"),
            format!("{tag} \t\r\n"),
        ] {
            let text = format!("a < b; </{tag}ure> <p> &amp;");
            let doc = parse_html(&format!("<{tag}>{text}</{closing}><p>after</p>")).unwrap();
            let raw = doc.find_first_element(tag).unwrap();
            assert_eq!(doc.children(raw).len(), 1);
            assert_eq!(doc.text_content(doc.children(raw)[0]), Some(text.as_str()));
            let p = doc.find_first_element("p").unwrap();
            assert_eq!(doc.parent(p), Some(doc.root()));
            assert_eq!(doc.text_content(doc.children(p)[0]), Some("after"));
        }
    }
}

#[test]
fn unterminated_raw_text_consumes_remaining_input() {
    for tag in ["script", "style"] {
        let text = "a < b; <p>not markup</p> &amp; </other>";
        let doc = parse_html(&format!("<{tag}>{text}")).unwrap();
        let raw = doc.find_first_element(tag).unwrap();
        assert_eq!(doc.children(raw).len(), 1);
        assert_eq!(doc.text_content(doc.children(raw)[0]), Some(text));
        assert!(!(0..doc.node_count()).any(|id| matches!(
            doc.get(id), Some(Node::Element { tag_name, .. }) if tag_name == "p"
        )));
    }
}
