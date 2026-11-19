# DN-17 audit

The host probe uses the 80386 real-mode profile and imports the original
DOS Navigator files into a private in-memory drive. The input hashes in
`inputs.sha256` matched before and after execution.
Both probes return 2 because the direct run trapped and the launcher run
recorded a child trap, even though the launcher parent was restored.

From the repository root:

```sh
cargo build -p chronos-core --example dos_probe --target x86_64-unknown-linux-gnu
probe=target/x86_64-unknown-linux-gnu/debug/examples/dos_probe
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=3749458 "$probe" target/dos-navigator/dn151/DN.PRG
CHRONOS_TRACE=1 CHRONOS_DETAIL_START=3755856 "$probe" target/dos-navigator/dn151/DN.COM
sha256sum -c docs/audit/chronos-dn/dn-17/inputs.sha256

cargo test -p chronos-core -p sunlight-chronos -p sunlight-ui \
  --target x86_64-unknown-linux-gnu --lib --bins
cargo check -p chronos-core --target x86_64-unknown-none
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-chronos --release --target x86_64-unknown-none
```

`regressions-first-build.log` preserves the initial test compilation failure
(reference comparisons in the newly added tests, since corrected).
`unit-tests.log` contains the passing scoped command above.
`regressions.log` also exercises doctests and records a failure in the
unchanged `sunlight-ui/horizon.rs` module diagram, which rustdoc parses as
Rust. Neither platform boot evidence nor the native release build runs DN.

Neither embedded platform artifact existed at the start of DN-17. The
separate prerequisite build followed the commands in `tools/build.sh`:

```sh
RUSTFLAGS='-C relocation-model=static -C target-feature=+crt-static -C link-arg=-no-pie' \
  cargo build -p helios-note --release --target x86_64-unknown-linux-musl
bash tools/stamp_helios_elf.sh target/x86_64-unknown-linux-musl/release/helios-note
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p wiseowl-console --bin wiseowl --release --target x86_64-unknown-none
bash -x tools/test.sh
```

The boot gate ran with shell tracing so its temporary build log could be
located if it failed before its cleanup trap was installed.
