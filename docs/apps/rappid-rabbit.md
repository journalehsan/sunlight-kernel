# Rappid Rabbit internal start page

Rappid Rabbit includes a start page in its application binary. Open it with
`rappid:startpage`; `rappid://startpage` is an alias. Both spellings resolve to
the canonical address `rappid:startpage` in the address bar.

The HTML and stylesheet are compiled from `rappid-rabbit/assets/`. Rabbit
recognizes the internal scheme before HTTP URL parsing, so internal documents
do not enter `sunlight-fetch`. Unknown `rappid:` addresses show an internal
page error.

Golden Fish parses the compiled start-page DOM once and retains it in a
`OnceLock`. Each navigation clones that immutable DOM and builds styles and a
render scene for the active viewport. The retained scene is not shared across
viewports because its layout geometry depends on the viewport.

When Rabbit starts without an explicit URL, it opens `rappid:startpage`.
