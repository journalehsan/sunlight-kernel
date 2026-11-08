Based on tl 0.7.8 (MIT). Local patch: scan unquoted attribute values until HTML whitespace or >; / is part of the value. This fixes root-relative URL tokenization. Regression coverage lives in golden-fish tests. SIMD remains disabled.

Script and style contents are consumed as raw text until an appropriate end
tag (case insensitive, followed by whitespace, `/`, or `>`). This prevents
JavaScript comparisons and HTML strings from corrupting the parent stack and
trapping visible body content inside the hidden head. End tag matching also
accepts case differences and trailing whitespace. Regression coverage lives in
`golden-fish/tests/raw_text.rs` and `rappid-rabbit/tests/script_head.rs`.

Classic HTML recovery closes intervening open elements when an ancestor's end
tag is encountered. Starting a cell, row, row group, list item, or definition
item closes its optional predecessor within the nearest table/list scope.
Nested tables retain their enclosing cells. Void tag matching is case
insensitive. Coverage is in `golden-fish/tests/classic_tables.rs` and the Rabbit
classic-layout fixtures. This remains a bounded subset of HTML tree building;
it does not implement foster parenting or the adoption agency algorithm.
