# Wise Owl Foundation Memory v1

## Overview

Foundation Memory is immutable Wise Owl identity data generated at build time.
It is not user memory, not conversation history, and not live runtime context.
It exists as a small validated binary blob compiled into `wiseowl-braind`.

In v1, Foundation Memory carries only permanent identity and policy records:

- assistant name
- internal codename
- SunlightOS identity
- general role
- high-level capabilities
- safety principles
- capability-based security model
- runtime information guidance
- Foundation vs Runtime Context vs Learned Memory boundaries

## Build-Time Tokenization

The build happens in [`wiseowl-brain/build.rs`](/home/ehsantor/Projects/sunlightos-kernel/wiseowl-brain/build.rs).

1. Read [`wiseowl-brain/foundation/foundation_v1.txt`](/home/ehsantor/Projects/sunlightos-kernel/wiseowl-brain/foundation/foundation_v1.txt).
2. Parse the required `key = value` identity records.
3. Tokenize each record with the existing `WiseOwlLexicalV1` tokenizer from `wiseowl-index`.
4. Compute a tokenizer fingerprint from the tokenizer source files.
5. Emit:
   - `OUT_DIR/wiseowl-foundation.bin`
   - `OUT_DIR/foundation_build.rs`

The runtime never tokenizes the source text again.

## Blob Format

The blob is decoded by [`wiseowl-brain/src/foundation.rs`](/home/ehsantor/Projects/sunlightos-kernel/wiseowl-brain/src/foundation.rs).

Header fields:

- magic
- format version
- schema version
- tokenizer id
- tokenizer version
- tokenizer fingerprint
- record count
- token count
- records offset and length
- tokens offset and length
- integrity hash

Record section:

- foundation key tag
- UTF-8 value length
- token slice offset
- token slice length
- UTF-8 value bytes

Token section:

- token id
- frequency
- flags
- position count
- token positions

## Validation

At `wiseowl-braind` startup, the embedded blob is loaded and validated for:

- magic and format version
- schema version
- tokenizer id and version
- tokenizer fingerprint
- integrity hash
- internal layout bounds

If any validation step fails, Wise Owl records the failure, logs a degraded
Foundation state, and continues without Foundation Memory.

## Boot Behavior

No new daemon, background service, or IPC protocol is introduced.

Foundation Memory is consumed inside the existing `wiseowl-braind` service as a
read-only context source. `sunlightd` already starts `wiseowl-braind` as an
optional user-space service, so desktop, installer, and login availability do
not depend on Foundation loading succeeding.

Failure behavior:

- the operating system still boots
- installer behavior is unchanged
- Wise Owl continues in degraded mode without Foundation data

## Runtime Context Boundary

Runtime Context is intentionally not implemented in this milestone.

The current code only prepares the layer boundary:

- [`wiseowl-brain/src/memory_layers.rs`](/home/ehsantor/Projects/sunlightos-kernel/wiseowl-brain/src/memory_layers.rs) now includes `FoundationMemoryLayer`
- the same file includes a placeholder `RuntimeContextLayer`
- Foundation records are exposed through a read-only grounded source

Future Runtime Context work should add live facts through the prepared runtime
layer instead of extending Foundation with mutable system state.
