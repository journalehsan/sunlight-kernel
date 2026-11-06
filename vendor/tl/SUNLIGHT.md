Based on tl 0.7.8 (MIT). Local patch: scan unquoted attribute values until HTML whitespace or >; / is part of the value. This fixes root-relative URL tokenization. Regression coverage lives in golden-fish tests. SIMD remains disabled.

Script and style contents are consumed as raw text until an appropriate end
tag (case insensitive, followed by whitespace, `/`, or `>`). This prevents
JavaScript comparisons and HTML strings from corrupting the parent stack and
trapping visible body content inside the hidden head. End tag matching also
accepts case differences and trailing whitespace. Regression coverage lives in
`golden-fish/tests/raw_text.rs` and `rappid-rabbit/tests/script_head.rs`.
