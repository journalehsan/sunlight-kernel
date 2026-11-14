# Rappid Rabbit loading worker TLS regression (2026-11-14)

The browser's first blocking worker died before it could parse the embedded
start page. The UI remained at Loading; external navigation also relies on
that worker. This failure occurs before a start-page network request (the
start page is embedded).

## Cause and repair

Commit `3dd03e9d8` (2026-11-11) added static C TLS and moved the thread control
block (TCB) toward the end of its allocation. Its size only accounted for the
24-byte libc header. The native Rust std key backend, in
`tools/native-std/overlay/thread_local/key/sunlightos.rs`, accesses 256 pointer
slots starting at `FS_BASE + 32`. Those slots were outside the mapping.

For the reported worker, TLS was mapped at `0x1008401000` for `0x1000` bytes.
The old TCB address was `0x1008401fc0`; key 4 starts at `0x1008402000`, matching
the write fault in the user's log and the independently reproduced QEMU run.
The kernel reaped the worker, leaving the browser window alive.

`sunlight-libc/src/tls.rs` now includes the std key table in the TCB's size and
initialization. Both initial-thread and spawned-thread allocation paths use
this size. The self pointer, errno and pthread pointer retain offsets 0, 8,
and 16; the std keys retain offset 32. Static C TLS remains at negative thread
pointer offsets. Stack sizes, TLS key capacity and kernel mapping policy are
unchanged.

## Verification

- Before the fix, the new bounds test failed with `std TLS keys exceed mapping`.
- A 12-vCPU KVM desktop run of the existing ISO reproduced the exact worker
  fault at `0x1008402000`. Evidence: `target/rabbit-tls-baseline/serial.log`.
- Three focused TLS tests passed: ABI offsets; complete, zero-initialized std
  key storage; initialized data and zeroed static TLS with alignment up to
  8192 bytes, preserving mapping bounds.
- Host libc regression: 124 passed with `--test-threads=1`, excluding
  `secret_store::tests::default_options_are_private_and_bounded` and
  `secret_store::tests::path_policy_rejects_traversal_and_staging_names`.
  The initial unfiltered run hung in those two tests and was terminated.
  A subsequent parallel run excluding them had two allocator failures;
  the serial run passed. These are separate limitations of the broad host
  check, not evidence of native TLS or browser behavior.
- `cargo build -p sunlight-kernel --features native_document_viewer` passed,
  including the kernel build script's native browser and SVG application builds.

Host evidence is under `target/rabbit-tls-*.log`; it does not establish guest
page presentation by itself.

## Native browser result

The patched image passed the same 12-vCPU KVM desktop launch using hardware
keyboard input through QMP and the native Run dialog. In one browser process:

1. Embedded start page: page-ready in 620 ms.
2. Host HTTP fixture `/one`: page-ready in 600 ms; screenshot shows the response
   heading and body.
3. Host HTTP fixture `/two`: page-ready in 610 ms.
4. Return to `rappid:startpage`: page-ready in 610 ms.
5. `https://distrowatch.com`: TLS succeeded; 230679-byte HTML response, document
   parse/layout completed, and page-ready in 54040 ms (initial document 32450 ms).

No `[FAULT]` appeared in the patched run. This establishes worker execution,
repeated navigation and visible document output; it does not establish full
rendering fidelity or fast loading for large sites. The start-page screenshot
has a white paint strip, and DistroWatch still shows image placeholders and
layout artifacts. These were left outside the TLS crash repair.

Evidence: `target/rabbit-tls-fixed/serial.log`,
`target/rabbit-tls-fixed-result.log`, and PNG screenshots in
`target/rabbit-tls-fixed/`. The QMP driver is retained at
`target/rabbit_loading_check.py`; its baseline run and patched run have separate
output directories. The patched ISO was copied to `target/sunlightos.iso`.
Both it and `target/rabbit-tls-fixed.iso` have SHA-256:

```
caa6a3f83778dbd737f408f249fa2da0da8932938c50badb8ce3681d02451bac
```
