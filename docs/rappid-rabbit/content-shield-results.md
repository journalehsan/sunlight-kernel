# Content Shield Phase 1 — validation results

Run date: 2026-11-08. See [architecture, syntax matrix, bounds, limitations and exact file map](content-shield.md).
Only EasyList is enabled by default. No live subscription request is necessary.

## Deterministic host validation

| Gate | Result |
| --- | --- |
| Golden Fish parser/filter tests, Linux host | 48 passed |
| Rabbit library, binary and integration tests, `js-dom-fixture` | 299 passed |
| Fetch library and pre-dispatch/redirect policy tests | 34 passed |
| Reusable Golden Fish core, `x86_64-unknown-none` | Check passed |
| Native Rabbit production release | Build passed; optional probe excluded |
| Native fixture-enabled Rabbit release and embedded kernel | Build passed |
| Python gate drivers / whitespace audit | Passed |

Tests cover anchors/wildcards/separators, hostname boundaries, canonical authorities,
PSL first/third party, most-specific domain restrictions, type restrictions, allow
precedence, strict selector validation, element/cosmetic exceptions, malformed and
oversized inputs, work limits, duplicates, rejected updates preserving the prior
snapshot, static/owned source lifetimes, settings encoding, exact site exceptions,
stale navigation/image/reflow results, cancellation, mutation and form behavior,
progressive scenes, absent privileged SunJS APIs and bounded feature extraction.
The fetch test listener observes no connection for blocked initial or redirect
requests. Tests use separate Cargo feature graphs for Fetch and Rabbit.

Logs: `target/content-shield/{final-golden-fish,final-rabbit,final-fetch,final-no-std,final-production-release,final-probe-build}.log`.
Existing unrelated warnings are present; these commands are not a claim that every
workspace feature combination or Clippy configuration is warning-free.

## Native QEMU validation

The gates boot a fresh diskless UEFI guest under software emulation with two CPUs,
log in through real hardware input, use local fixture servers and record serial
output / packet capture. No public browsing service participates.

The transport regression covers 132 successive TLS connections in one daemon,
large responses, post-stress connectivity, trusted/untrusted certificates,
SOCKS HTTP/HTTPS and denied destinations, explicit address resolution and hostname
validation. Its evidence is `target/content-shield/native-transport/`.

The explicit local-only browser probe exercises pre-dispatch image and stylesheet
blocking, a successfully decoded normal JPEG, preserved DOM identities, 12 SunJS
attribute mutations, reversible hiding and exact-site exceptions. Native KV
preferences are saved and read back. The UI gate enters the graphical desktop,
uses the native Run dialog, then toggles the browser checkbox and reloads the same
fixture in both modes. Server-side request counts distinguish blocked requests
from merely hidden content.

Visual gate and screenshot evidence: PENDING.

## Pinned rules and memory

The unmodified EasyList snapshot contains **73,465 supported rules** and **5,266
unsupported rules** under the documented subset. There are 23,135 supported
cosmetic rules. Reject counters are:

| Category | Count |
| --- | ---: |
| Oversized | 7 |
| Syntax | 10 |
| Unsupported option | 3,559 |
| Unsupported selector | 1,025 |
| Domain restriction | 350 |
| Regex | 1 |
| Transformation | 308 |
| Limit | 6 |

A counting host allocator records 10,185,998 bytes retained after cold manager
initialization and 12,590,803 bytes peak requested allocation. The core's owned
capacity estimate is 9,994,750 bytes; the shared cosmetic index adds 1,052,892
retained bytes after first use. Requested-byte counts exclude allocator headers,
fragmentation, stacks and executable/read-only data. Bundled source text (EasyList
2,083,376 bytes plus the ASCII PSL) is borrowed from immutable executable data.
Dynamic replacement candidates own their strings. Repeated request matching
recovers every temporary allocation in the measured loop.

## Local fixture timings

Final cold manager/list initialization: **112,991 µs**. The same release
benchmark binary and fixtures produced:

| Measurement | Disabled | Enabled |
| --- | ---: | ---: |
| Request matching p50 | 28 ns | 6,587 ns |
| Request matching p95 | 30 ns | 14,993 ns |
| Request matching maximum | 7,434 ns | 31,957 ns |
| Blocked requests / 10,000 | 0 | 2,500 |
| Style stage mean | 14,621 µs | 15,129 µs |
| Cosmetic stage mean, including policy/index setup | 2 µs | 8,160 µs |
| Cosmetic matching mean | 0 µs | 7,757 µs |
| Layout / first scene mean | 5,941 µs | 6,651 µs |
| HTML-to-first-scene pipeline mean | 20,917 µs | 30,307 µs |
| Cold first-scene pipeline | 20,966 µs | 36,334 µs |
| Hidden fixture elements | 0 | 1 |

Raw output: `target/content-shield/benchmark-final.log`. Filtering adds work on
this local fixture; these measurements do not imply a browsing speedup. The host
benchmark has no resource-ready network milestone. Native resource-ready timing
below includes fixture HTTP and JPEG decoding; no warm-cache speedup is claimed.

`shield_bench` measures 10,000 requests against four fixed URLs and 20 runs per
mode on the identical 1,005-node offline document. It uses a fixed text measurer,
includes HTML parsing in first-scene pipeline time and separately records style,
cosmetic and layout stages. No network requests run in this host benchmark.
These are local measurements on a shared machine, not a claim of improved live
browsing speed. Order/cache effects and scheduling noise affect stage comparisons.

An earlier packed native run recorded cold parse/manager setup at 1.32 seconds,
cosmetic index construction at 70 ms, initial fixture scene construction at 1.68
seconds and resources decoded/blocked at 1.73 seconds. On the 39-node native
fixture, cosmetic reevaluations took 40–70 ms under QEMU TCG. These include
emulation overhead and are not physical hardware estimates. Native scene-ready
measurements precede compositor scanout; the UI gate validates actual scanout
separately. Final native evidence is recorded below when the visual gate completes.

## Practical limits

This is the documented EasyList subset. Unsupported rules are counted and ignored
in full. Normal browser dispatch currently covers images/backgrounds and
stylesheets/imports; the core also understands resource types for future loaders.
Normal navigation still does not execute downloaded JavaScript or expose fetch/XHR.
Cosmetic work limits fail open for a document. Subscription downloading/automatic
updates are deferred because existing native settings KV is a bounded page record,
not an atomic large-list store. Network policy changes require reload; cosmetic
changes are reversible immediately. No TLS/origin/process-isolation policy changes
are introduced.

No changes were committed or pushed. Generated ELFs, ISOs, logs and screenshots
remain under `target/content-shield/` and are not repository source artifacts.
