use golden_fish::{parse_html, Document};

fn tagged_children(doc: &Document, id: usize) -> Vec<&str> {
    doc.children(id)
        .iter()
        .filter_map(|&id| doc.tag_name(id))
        .collect()
}

#[test]
fn omitted_table_cell_and_row_ends_produce_siblings() {
    let doc = parse_html("<table><tr><th>Heading<th>Other<tr><td>One<td>Two<tr><td>Three<td>Four</table><p>After</p>").unwrap();
    let table = doc.find_first_element("table").unwrap();
    assert_eq!(tagged_children(&doc, table), ["tr", "tr", "tr"]);
    for &row in doc.children(table) {
        if doc.tag_name(row).is_some() {
            assert_eq!(
                doc.children(row)
                    .iter()
                    .filter(|&&id| doc.tag_name(id).is_some())
                    .count(),
                2
            );
        }
    }
    assert_eq!(
        doc.parent(doc.find_first_element("p").unwrap()),
        Some(doc.root())
    );
}

#[test]
fn nested_table_does_not_close_its_outer_cell() {
    let doc = parse_html("<table id=outer><tr><td><table id=inner><tr><td>Inner<td>Other</table><td>Outer right</table>").unwrap();
    let tables: Vec<_> = (0..doc.node_count())
        .filter(|&id| doc.tag_name(id) == Some("table"))
        .collect();
    let outer_row = doc.children(tables[0])[0];
    assert_eq!(tagged_children(&doc, outer_row), ["td", "td"]);
    assert_eq!(doc.parent(tables[1]), Some(doc.children(outer_row)[0]));
    let inner_row = doc.children(tables[1])[0];
    assert_eq!(tagged_children(&doc, inner_row), ["td", "td"]);
}

#[test]
fn ancestor_end_tag_closes_unclosed_inline_children_and_keeps_the_sidebar() {
    let doc = parse_html("<div><p><strong>Main</div><aside>Sidebar</aside>").unwrap();
    assert_eq!(tagged_children(&doc, doc.root()), ["div", "aside"]);
}

#[test]
fn uppercase_void_tags_and_optional_lists_do_not_capture_the_following_text() {
    let doc = parse_html("<UL><LI>First<LI>Second</UL><BR><P>After</P>").unwrap();
    let list = (0..doc.node_count())
        .find(|&id| {
            doc.tag_name(id)
                .is_some_and(|t| t.eq_ignore_ascii_case("ul"))
        })
        .unwrap();
    assert_eq!(tagged_children(&doc, list), ["LI", "LI"]);
    assert_eq!(tagged_children(&doc, doc.root()), ["UL", "BR", "P"]);
}
