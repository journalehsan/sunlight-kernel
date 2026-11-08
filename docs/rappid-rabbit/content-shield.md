# Golden Fish Content Shield — Phase 1

Content Shield provides a bounded EasyList/Adblock Plus **subset**, not full
EasyList or uBlock Origin compatibility. EasyList is the only subscription
installed and enabled by default. Startup uses pinned local data and makes no
subscription-provider request.

## Architecture and inspection

The implementation reuses these existing boundaries:

* Golden Fish owns the HTML node arena and stable `NodeId`s. Its mutation API
  preserves identities; it is not a CSS engine.
* Rabbit's `css::StyleContext`, `parse_selector` and `selector_matches` own
  computed styles and selector semantics. `layout::StyleTree` supplies them to
  the two-pass layout arena, which already prunes non-rendered subtrees.
* `DocumentRenderState` owns immutable DOM/style/scene snapshots. DocumentCanvas
  draws and hit-tests the retained scene. Progressive images patch that scene
  without rebuilding the source document.
* The navigation worker, independent image lane, DOM revision and viewport
  version already reject superseded work. Shield adds filter-revision checking
  and cancellation of subresource policy leases.
* SunJS currently runs only through the opt-in controlled DOM fixture host.
  Normal navigation does not discover/execute downloaded scripts, and there is
  no browser fetch/XHR or frame-document execution API to intercept in Phase 1.
* Fetch owns DNS, redirects, HTTP, TLS and body limits. A consumer policy hook
  runs before initial DNS and each redirect-target resolution; transport and
  certificate validation remain unchanged.
* Sunlight UI supplies Checkbox, Canvas, Label and the existing dark chrome.
  KV supplies kernel-authenticated caller identities, per-key ACLs and bounded
  shared-memory records. There was no existing Rabbit preferences page.

`golden_fish::shield` is the reusable policy core: subscription management,
settings codec, parsing/compilation, diagnostics, URL/hostname canonical views,
Public Suffix List party classification and indexed network decisions. It uses
`alloc`, has no UI/storage/network/JavaScript authority, and introduces no new
crate dependencies. Bundled rules borrow immutable source slices through `compile_static`;
updated candidates use owned strings through `compile`. Domain restrictions retain
one validated compact string per rule. Network and cosmetic indexes use sorted
packed vectors and binary searches, avoiding a heap allocation for every index
bucket on the native first-fit allocator. Other Golden Fish consumers can supply their own strict
selector validator and use its cosmetic selector output.

Rabbit's `content_shield` adapter compiles selectors using the existing CSS
parser. It rejects unsupported syntax before invoking that parser. Cosmetic
selectors are indexed by their rightmost ID, class, attribute name or tag.
The index is shared per immutable compiled-list snapshot through a bounded
eight-entry weak-key cache. Document policies retain the compiled snapshot,
applicable exceptions and site identity, rather than copying every generic
selector. Only selector candidates relevant to the DOM are compiled by the CSS
parser during evaluation; this also covers newly introduced IDs/classes after
mutations. The lexical dispatch hint does not decide selector matches.
The resulting `CosmeticPolicy` is shared through `Arc`; the style context owns
a **separate hidden-node set**, leaving source DOM and computed author styles
unchanged. Layout prunes these nodes before creating boxes, geometry or scene
objects, so their descendants occupy no space and cannot receive canvas hits.

DOM fixture mutations rebuild computed styles through the established path and
reevaluate the same policy against the current document. Settings changes
schedule a new presentation on the existing worker. Old policy completions
cannot replace it. Pointer dispatch to an obsolete presentation is suppressed
while that rebuild is pending; browser navigation shortcuts remain available.
Disabling a policy restores the same nodes without reparsing HTML.

## Request behavior

Images (including discovered CSS backgrounds), external stylesheets and CSS
imports use the policy-aware fetch entry point. Supported script/frame/media/
font/XHR types are available to consumers in the core, but Phase 1 does not
invent browser dispatchers for those currently unsupported resource loaders.
Top-level navigation and forms retain their existing behavior.

Requests are evaluated against the final initiating document URL, resource
type, enabled subscriptions and exceptions. The core indexes domain-anchored
rules by canonical hostname and other patterns by a literal four-byte token.
Repeated URL tokens are deduplicated before collecting candidate rules.
Allow rules override blocking rules across active lists. A blocked request
returns `FetchError::BlockedByPolicy`, appears as a failed network entry, and
never becomes an HTTP success or decoded image. Redirects are checked before
dispatch too. Cache ownership and failure/presentation semantics are preserved.

HTTP(S) filter views lowercase scheme/ASCII hostname, remove a terminal DNS dot,
omit default ports, supply an empty path slash, retain query strings and remove
fragments. Userinfo, malformed/encoded hosts, control characters, backslashes,
non-ASCII authorities and IPv6 are outside this adapter's canonical subset.
Unsupported URLs still pass through existing transport validation; filtering
does not authorize or relax a request. Unicode hostnames must already be ASCII
A-labels. This is not a replacement URL parser for the transport stack.

First/third-party decisions use the registrable domain from the pinned Public
Suffix List, including its private section, wildcard entries and exceptions.
Distinct IP addresses are different parties. Scheme/port do not change ABP
party classification; they still participate in normal browser origin policy.
Hostname suffix checks require a dot boundary: `ads.test.evil` and
`notads.test` cannot impersonate `ads.test`.

## Supported syntax matrix

| Syntax | Phase 1 behavior |
| --- | --- |
| `!` comments, `[Adblock Plus …]` header | Comments skipped; valid header required |
| `! Version:`, `! Checksum:` | Bounded metadata retained; SHA-256 also pinned separately |
| Basic URL patterns | Case-insensitive substring matching by default |
| `*` | Wildcard; iterative matching with a shared work budget |
| `|pattern`, `pattern|` | Start/end anchors |
| `||ads.example^` | Literal domain anchor, exact host or boundary-checked subdomain |
| `^` | Separator or end; letters/digits/underscore/hyphen/dot/percent are not separators |
| `@@pattern` | Network exception, overriding matching blocking rules |
| `$image,stylesheet,script,subdocument,media,font,xmlhttprequest,other` | Type restrictions; `~type` exclusions supported |
| `$domain=a.test\|~b.a.test` | Initiating-document inclusion/exclusion, with hostname boundaries; most-specific matching domain wins, exclusion wins ties |
| `$third-party`, `$~third-party` | PSL-based third/first-party restriction |
| `$match-case` | Case-sensitive pattern matching |
| `##selector` | Generic element hiding |
| `a.test,~b.a.test##selector` | Domain-specific element hiding and excluded domains |
| `#@#selector` | Applicable cosmetic exception cancels that exact trimmed selector |
| `@@…$elemhide`, `@@…$generichide` | Disable all / generic cosmetic rules on matching documents |
| CSS tag, universal, ID, class, compound selectors | Existing selector matcher; ASCII, bounded identifiers |
| Descendant and `>` child combinators | At most eight parts |
| Attribute presence, `=`, `~=`, `\|=`, `^=`, `$=`, `*=` | Quoted values or safe unquoted identifiers; optional `i` flag |

Unsupported options reject the **whole rule**, including exception rules.
No unsupported suffix is stripped into a broader blocking pattern. Wildcards
inside domain-anchor hostnames and wildcard domain restrictions are rejected.
Domain anchors intentionally require canonical literal hostname identities.

Regular expressions, pseudo classes (including procedural and approximated
dynamic states), pseudo elements, sibling combinators, selector lists, escaped
selectors, scriptlets, snippets, arbitrary CSS declarations, transformations,
HTML filtering, `redirect`, `removeparam`, `important`, `badfilter`, sitekeys,
`$document` and other unlisted options are not supported. Conditional/include
directives (`!#…`) reject the entire candidate list rather than activating an
uninterpreted branch. Unknown ordinary metadata comments do not execute.

## Bounds and subscriptions

* Four MiB source per candidate; 100,000 compiled rules per list; 2,048 bytes per
  source line; 512 bytes per network pattern; 256 bytes per cosmetic selector.
* Eight registered subscriptions; 48 MiB aggregate estimated compiled rule
  capacity. No more than 256 short/unindexable network patterns per list.
* Domain restrictions have at most 64 entries. Diagnostics retain eight reason
  counters and at most 32 `(line number, category)` samples, without raw rules
  or browsing URLs. Duplicate rules are counted separately and deduplicated.
* Network matching has a one-million-step budget across candidate rules.
  Exhaustion returns `WorkLimit` and allows the request, preserving possible
  unexamined exceptions; the browser counts the event.
* Cosmetic matching has a one-million-unit budget weighted by selector parts,
  document depth, attribute lookup counts and potentially scanned value bytes.
  Feature extraction allows at most 128 attributes / 64 KiB per element, 256
  feature keys per element and 256 distinct attribute names per document.
  Exhaustion leaves that document unfiltered and emits a
  bounded diagnostic, rather than applying a partial policy.
* Source size, compiled-capacity limits, owned snapshots, bounded browser worker
  lanes and existing DOM/image limits bound retained state. Requested-byte
  accounting in the benchmark checks repeated request matching for growth.

Each `Subscription` records stable ID, display name, source, activation state,
last successful update time, supported/unsupported counts, version/checksum and
an immutable compiled snapshot. `replace` validates an entire candidate before
swapping its `Arc` and advancing the revision. Errors preserve the last known
good snapshot. Settings/list changes do not mutate a snapshot held by a worker.

Live automatic updates are deferred. Existing native KV records fit one shared
page, not the two-MiB subscription; there is no established browser-owned atomic
large-list storage/update protocol. Phase 1 therefore exposes no update URL UI,
does not follow subscription redirects and sends neither credentials nor
browsing URLs to subscription providers. Replacing bundled data is a reviewed
source/build operation. This does not add a storage service or package manager.

Pinned sources, checksums, dates, licenses and attribution are in
`golden-fish/filters/`. EasyList snapshot version is `202611081212`;
the PSL snapshot is `2026-11-06_09-01-47_UTC`. The ASCII PSL derivative retains
ICANN/private/wildcard/exception rules. See `NOTICE.md` for its derivation.

## Settings and UI

The small orange/gray `S` indicator occupies the existing tab strip, preserving
toolbar height and narrow address-field space. Click it or press Ctrl+Shift+S
to open Content Shield settings. Reusable native checkboxes control global
activation, EasyList and an exception for the current canonical hostname.
The popup reports supported/unsupported counts, blocked requests and hidden
nodes. Network changes explicitly require reload; cosmetic changes are live.
User site exceptions are **exact hostname identities**, not inherited wildcard
exceptions for every subdomain. There is a 64-site cap and a four-KiB persisted
record limit.

Browser chrome loads/saves `rr.cs:<uid>` using existing authenticated KV SHM
operations and per-key ACLs. The native adapter validates the entire settings
record, bounds IPC waits, and frees mapped/allocated regions on every path.
Unavailable KV gives an explicit session-only message. Host tests do not touch
the user's home directory or settings. No settings operation, filesystem path,
subscription method or privileged handle is registered in SunJS; DOM content
cannot configure the shield. Existing KV root/service administrative authority
is unchanged.

## Validation and measurements

See [content-shield-results.md](content-shield-results.md) for the measured
host/native results and screenshot paths. Benchmark numbers describe local
fixtures and a fixed font measurer, not live browsing performance.

```sh
cargo test -p golden-fish --target x86_64-unknown-linux-gnu
cargo test -p sunlight-fetch --target x86_64-unknown-linux-gnu
cargo test -p rappid-rabbit --target x86_64-unknown-linux-gnu --features js-dom-fixture
cargo run --release -p rappid-rabbit --example shield_bench --target x86_64-unknown-linux-gnu
bash tools/build_rappid_rabbit.sh --features js-dom-fixture
SUNLIGHT_ACCOUNT_FIXTURE=1 cargo build -p sunlight-kernel
# The existing kernel build script rebuilds Rabbit: this flag preserves its
# js-dom-fixture feature. It does not select an account or change navigation.
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
  target/content-shield/probe.iso
python3 tools/network_tls_gate.py --iso target/content-shield/probe.iso \
  --uefi --tcg --cpus 2 --content-shield --evidence target/content-shield/native
```

`--content-shield-only` reruns only the browser fixture. The probe is compiled
only with `js-dom-fixture`, requires an explicit `--shield-probe` argument and
a local HTTP fixture, and never enables downloaded JavaScript. The normal
production browser requires neither probe flag nor that optional feature.
For a production image, rebuild the kernel without `SUNLIGHT_ACCOUNT_FIXTURE`
and package that kernel separately; the fixture image must not be confused with
the production image.

## File map

| Responsibility | Files |
| --- | --- |
| Reusable core and exports | `golden-fish/src/lib.rs`, `golden-fish/src/shield/{mod,network,parser,tests}.rs` |
| Pinned sources / notices | `golden-fish/filters/{easylist.txt,easylist.json,public_suffix_list.dat,public_suffix_ascii.txt,public_suffix.json,NOTICE.md}` |
| CSS policy adapter / KV codec adapter | `rappid-rabbit/src/content_shield.rs`, `rappid-rabbit/src/content_shield/settings.rs` |
| Style/layout and mutation integration | `rappid-rabbit/src/{lib,css,layout,render}.rs` |
| Browser request/lifecycle/chrome integration | `rappid-rabbit/src/{main,browser_ui,shield_ui}.rs` |
| Transport policy hook and typed failure | `sunlight-fetch/src/{backend,error}.rs` |
| Host browser / transport regressions | `rappid-rabbit/src/shield_tests.rs`, `rappid-rabbit/tests/content_shield.rs`, `sunlight-fetch/tests/filter_policy.rs` |
| Native fixture / QEMU driver | `rappid-rabbit/src/shield_probe.rs`, `rappid-rabbit/tests/fixtures/content-shield.html`, `tools/{network_tls_gate,content_shield_ui_gate}.py` |
| Benchmark | `rappid-rabbit/examples/shield_bench.rs` |
| Documentation | `docs/rappid-rabbit/content-shield.md`, `docs/rappid-rabbit/content-shield-results.md` |

Syntax reference: [Adblock Plus filter documentation](https://help.adblockplus.org/adblock-plus-help-center/how-to-write-filters).
Bundled material attribution: [The EasyList authors](https://easylist.to/),
[EasyList licensing](https://easylist.to/pages/licence.html),
[Public Suffix List](https://publicsuffix.org/list/).
