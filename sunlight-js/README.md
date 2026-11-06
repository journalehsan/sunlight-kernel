# SunJS

An independent native-std interpreter for a documented strict JavaScript subset.
No browser, DOM, I/O, runtime, or third-party dependencies; no unsafe code.

See [language matrix, embedding API, ownership, and DOM adapter plan](../docs/javascript.md)
and [validation results](../docs/javascript-validation.md).

```sh
cargo test -p sunlight-js --target x86_64-unknown-linux-gnu
cargo run -p sunlight-js-cli --target x86_64-unknown-linux-gnu -- sunlight-js/examples/counter.js
```
