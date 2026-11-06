# Controlled account fixture: architecture inspection

The pre-existing opt-in DOM adapter owns a fresh Context per synchronous call.
It caches wrappers in HostRoot handles, uses owned Golden Fish arena indices,
and publishes copy-on-write DOM changes through the canonical two-pass render
pipeline. It cannot retain listeners or deliver work in a later UI turn.

Rabbit's navigation worker owns parsing and initial layout; versioned render
snapshots reject outdated results. Live navigation does not execute scripts.
Browser FormState separately owns control values and Unicode-boundary carets.
Password inputs currently fall back to text in FormState despite a password
widget in layout. DOM attribute mutation and generic selector lookup are absent.

The narrow ownership boundary for this milestone is a controlled local session:
one Context, one DOM lease, cached wrapper roots, control state, event callbacks,
and a bounded callback queue. The owner must drop the entire session on navigation.
No native borrow may cross a JavaScript call. Roots must be released before the
Context closes. This is an explicit trusted-fixture capability, never script
execution discovered from arbitrary HTML or network responses.

Initial limitations: no Promise jobs, browser event objects, retained script
sessions, capture, IME/composition, selection editing, or native event bridge in
the existing adapter. The implementation and validation sections below track
the implemented subset; these are not claims of browser compatibility.

## Running the controlled application

Build with `bash tools/build_rappid_rabbit.sh --features js-dom-fixture` and
launch that binary with `--account-fixture`. The flag selects a compiled-in
application before Rabbit's ordinary URL/navigation path. No downloaded script
is discovered or evaluated. The UI owns the non-Send session; it never moves
into the navigation worker. Closing the fixture drops its entire session.

Host regression and snapshot commands:

```
cargo test -p sunlight-js -p golden-fish -p rappid-rabbit -p sunlight-ui --features rappid-rabbit/js-dom-fixture --target x86_64-unknown-linux-gnu
cargo run -p rappid-rabbit --example account_snapshots --features js-dom-fixture --target x86_64-unknown-linux-gnu
```

The latter paints initial, invalid, loading, and success PPM files under
`target/account-fixture/` using Rabbit's DocumentCanvas. These are synthetic
fixtures, not provider screenshots. Use invented details only. Remember-account
is an in-memory checkbox; it never persists data.

## Implemented feature matrix

| Area | Implemented subset | Deliberately unsupported |
| --- | --- | --- |
| Lookup | getElementById; querySelector for one #id, .class, or ASCII tag, tree order | Compound selectors, combinators, pseudo classes, escaping |
| Elements | Native textContent, className, get/set/remove/hasAttribute, parentElement | HTML parsing setters, DOM creation, shadow DOM |
| Controls | Live text/email/password value, checkbox checked, reflected disabled/type/name, form ancestry or explicit form ID | Selection API, radio groups, files, validation API |
| Activation | click, focus, blur, requestSubmit; associated label activation | form.reset/defaultValue, native submission/network default action |
| Events | Ordered function listeners, duplicate suppression, removal, dispatchEvent, target/currentTarget, defaultPrevented, preventDefault, stopPropagation, element ancestor bubbling | Document/window event targets, capture, options objects, once/passive/signal, handleEvent objects, stopImmediatePropagation, full DOM exceptions |
| Event creation | Explicit `Event(type, options)` factory with bubbles/cancelable flags | `new Event`: SunJS has no constructor syntax; event metadata remains ordinary JS properties |
| Keyboard | Native Unicode scalar commits; extended grapheme movement/deletion; arrows, Home/End, Tab/Shift+Tab, Enter, Escape, Space; cancellable keydown with modifier snapshots | External IME preedit UI, selection, clipboard, positive tabindex |
| Async | fixtureAuthorize(callback), one FIFO completion per explicit UI tick, boolean result | Promises, timers, networking, broker credentials |

JavaScript uses UTF-16 string indexing; private editor offsets are UTF-8
boundaries. The editor never treats bytes as JS character indices. Passwords
are replaced by one ASCII mask character per Unicode scalar before scene paint,
including caret measurement. The native keyboard protocol carries physical transitions and tagged Unicode
scalar commits as separate IPC words. The compositor forwards commits without
interpreting their payload as a shortcut or modifier transition. `Event::Key`
is committed text; `Event::KeyPress` carries physical/control transitions.
Legacy ASCII payloads remain on physical words for older TTY clients, while
`KEY_SEPARATE_TEXT` prevents duplicate text insertion in native UI clients.
`KEY_EXTENDED` preserves the physical E0 location (also exposed as
`Event::KeyPress.extended`) and `KEY_COMPOSING` prevents
preedit commands from editing or submitting the form. Invalid scalar values,
surrogates, and malformed reserved bits are discarded before creating a char.
No unchecked UTF-8 conversion is used for text commits.

The PS/2 layout is US QWERTY. To enter any printable Unicode scalar, press
Ctrl+Shift+U, release the modifiers, type up to six hexadecimal digits, and
press Enter or Space. Escape cancels; Backspace edits the private preedit.
For example, `e9` commits é, `301` commits a combining acute accent, and
`1f620` commits 😠. Preedit never reaches the DOM. This is a native scalar
composition method, not an external IME integration. Left/right Shift, Ctrl,
Alt and Super are tracked independently, so releasing one side cannot clear
the other. Modifier snapshots are refreshed on releases as well as presses.

Editing uses Unicode extended grapheme boundaries (unicode-segmentation),
including combining accents, emoji ZWJ sequences and regional-indicator pairs.
Storage remains UTF-8 and JS strings retain the engine's UTF-16 semantics.

Only text values and checkbox checked are runtime state. Attribute changes
re-read control metadata and preserve dirty text values. Programmatic value
writes do not emit input/change. User edits emit input; changed text commits
emit change before blur. Checkboxes pre-toggle for click listeners, roll back
on cancellation, and emit input then change if accepted. Enter sends a minimal
form submit event; requestSubmit has no submitter parameter or constraint
validation. Dispatch snapshots listeners per current target, skips removed
registrations, and defers newly added listeners on that target to the next
dispatch. Listener exceptions abort this subset's dispatch and reach the owner.
Detached wrappers remain valid until session teardown, as in the original adapter.

## Security and lifecycle

The persistent owner holds a Context and a copy-on-write DOM lease. Wrapper
identity maps, listener HostRoots, and queued task HostRoots participate in
tracing. No RefCell borrow crosses user callbacks. Dispatch roots its event;
listener snapshots keep callbacks pinned even when removed during dispatch.
Teardown clears runtime text, focus baselines, tasks, listeners, and wrappers,
then closes the context, breaking native-host cycles. Rust String clearing is
logical disposal, not a guarantee of forensic memory zeroization; this fixture
must never be used for real secrets.

Each session binds to document_id and the DOM lease. A generation change or
foreign DOM replacement invalidates the session, clears its roots, and rejects
completion. Navigation owners must drop the old session before publishing a
new document. There is no session attached to live navigation. Reflow snapshots
remain subject to Rabbit's existing revision checks.

The test host accepts only a callable, never an email/password payload. The
fixture clears its password before queuing authorization. Completion carries
only a boolean; success and failure are chosen by the test owner. cancel_pending
releases queued callbacks without invoking them. Closing or replacing the
session cancels all tasks. No filesystem, environment, process, cookie, token,
or network API is installed in SunJS.

Bounds: 4096 DOM nodes, 256 KiB native work per turn, 256 mutations per turn,
256 KiB cumulative DOM text/attribute allocation budget, 4096 bytes per scripted
control value, 128 listeners, dispatch nesting depth 16, and 8 pending tasks.
Context fuel and GC limits also apply. Context::host_turn shares fuel with
reentrant evaluation and provides fresh fuel only at the outer turn boundary.
One idle callback runs per UI tick; callbacks cannot synchronously wait.

Multiple writes in one turn cause one canonical style/two-pass layout rebuild,
including exceptional exits. Runtime controls are patched after that rebuild.
No password is placed in a DOM value attribute by the fixture. The application
never formats JS errors or input values into diagnostics; failure stops further
fixture work. Generic DOM dump/debug tools must not be attached to this view.
This is not a password manager and has no persistence.

## Future Account Broker and OAuth design

This proposal uses an external provider authorization endpoint and a trusted
external user-agent, as described in [RFC 8252](https://www.rfc-editor.org/rfc/rfc8252).
Rabbit's disabled live scripting does not become a provider-login engine.
A privileged Account Broker owns provider configuration and the transaction;
page JavaScript receives only a connection status and an opaque account ID.

1. Mercury, Calendar, or Files requests an account connection through a
   capability-checked broker IPC message with a provider ID and named scope
   bundle. The broker verifies the caller and presents trusted consent UI.
   Provider endpoints and registered client IDs come from an allowlisted
   configuration, never a page-supplied URL.
2. The broker generates independent cryptographic random state and verifier
   values. Use at least 256 bits for each. Encode the verifier as unpadded
   base64url (43 characters for 32 random bytes), and derive
   code_challenge = BASE64URL(SHA256(ASCII(code_verifier))). Send only S256,
   response_type=code, client_id, exact redirect_uri, requested scopes, and
   state to the provider. Keep verifier/state in a bounded, expiring broker
   transaction. These PKCE parameters follow
   [RFC 7636](https://www.rfc-editor.org/rfc/rfc7636).
3. Register an OS-owned redirect URI with an exact scheme/authority/path;
   prefer a verified claimed HTTPS redirect routed to the broker. Do not add
   a wildcard host, redirect prefix matching, or page-controlled return URL.
   Any provider-approved loopback alternative requires its own listener and
   deployment review. Intercept the registered redirect before rendering,
   subresource discovery, history, or logging. Decode query parameters once;
   reject duplicates, fragments carrying credentials, unknown transactions,
   mismatched state, provider/issuer mismatch, expired or reused transactions.
4. The broker checks state by exact constant-time comparison, binds the code
   to the initiating caller/provider/redirect and transaction nonce, and
   consumes it once. The broker exchanges it over authenticated TLS at the
   configured token endpoint using the same redirect_uri and verifier. A
   distributed native public client must not embed a client secret; a
   confidential deployment uses a separately controlled server. Neither the
   code nor verifier reaches web-page JavaScript.
5. The broker encrypts token storage with the OS secret service, schedules
   expiry using expires_in and a monotonic deadline, and serializes refresh
   per account. Store rotated refresh tokens atomically, handle revocation
   and invalid_grant by requiring reconnection, and retry only bounded,
   idempotent operations. Prefer broker-mediated API operations so clients
   receive service results rather than bearer tokens. Refresh tokens are
   explicitly prohibited in page JavaScript, Rabbit DOM, and renderer IPC.
6. Mercury requests minimum mail access, Calendar requests calendar access,
   and Files requests narrowly scoped app-folder/file access. Add scopes only
   through new trusted user consent. OIDC, if used for identity, additionally
   requires a nonce and validation of signature, issuer, audience, and expiry
   of ID tokens. An access token is not proof of identity.
7. Removal revokes provider grants when supported, cancels refresh and sync,
   destroys local token records, and invalidates caller capabilities. Offline
   revocation failure cannot preserve local access; surface a trusted status
   and permit bounded retry without retaining UI secrets. Never log codes,
   verifiers, cookies, tokens, or complete callback/authorization URLs.
8. Every reply includes a broker transaction ID and caller/session generation.
   Closing the initiating view or navigating cancels the transaction and drops
   state/verifier. Late replies are ignored, and any acquired grant is revoked
   according to policy. A completed durable account can outlive its originating
   view only after an explicit broker commit acknowledged by trusted UI.

Security review should use [RFC 9700](https://www.rfc-editor.org/rfc/rfc9700).
Before enabling Google, Dropbox, or another provider, check that provider's
current external-user-agent policy, redirect registration rules, scope review,
verification requirements, PKCE support, and token/revocation behavior. No real
provider integration is enabled by this milestone.

Recommended next milestone: implement and fuzz a broker transaction state
machine with a fake provider, redirect interception and redacted logging first.
Extend native Unicode input transport and accessibility/focus handling before
claiming interactive provider readiness. Keep all live-page scripts disabled.

## Validation record

- Engine/object-model/GC, Golden Fish, Rabbit library/document/UI bridge and
  fixture suites: 290 tests passed, including 10 sign-in integration tests,
  teardown/limit unit tests, and collection-at-every-safe-point paths.
- Node/V8 comparison: 82 selected cases and six runnable examples passed.
- Additional sunlight-ui unit/integration suites passed (284 + 2 + 4 tests).
  Its existing `horizon.rs` doctest treats an ASCII UI sketch as Rust and fails;
  eight other documentation examples are ignored. No UI doctest source changed.
- `python3 tools/check_workspace.py`: all 94 supported platform checks passed.
- Native std-overlay release builds: SunJS CLI and opt-in Rabbit fixture passed.
- Scoped rustfmt check and whitespace diff check passed.
- Strict SunJS Clippy passed. Strict Golden Fish/Rabbit checks remain blocked
  by existing pedantic/style warnings in their older sources and dependencies
  (including vendor/tl and sunlight-ipc); new binding code diagnostics were
  corrected. No crate-wide lint suppression was added.
- Initial, invalid, loading, and success snapshots were generated and visually
  inspected. Generated files stay under target/account-fixture, outside git.
The native keyboard upgrade adds driver composition/modifier tests, malformed
commit decoding tests, grapheme deletion tests, and native modifier delivery
under collection-at-every-safe-point fixture dispatch. See the current logs in
`target/qemu-keyboard-smoke/`; the preceding counts describe the original
fixture milestone.

## Live QEMU keyboard verification

Run from the repository root:

```sh
cargo test -p sunlight-kbd --lib --target x86_64-unknown-linux-gnu
cargo test -p sunlight-js -p golden-fish -p rappid-rabbit -p sunlight-ui \
  --lib --tests --features rappid-rabbit/js-dom-fixture --target x86_64-unknown-linux-gnu
python3 tools/check_workspace.py
mkdir -p target/qemu-keyboard-smoke
bash tools/test_keyboard_fixture.sh > target/qemu-keyboard-smoke/guest-gate.log 2>&1
```

The gate requires QEMU, OVMF, ImageMagick `convert`, and Tesseract. Override
`SUNLIGHT_OVMF_CODE`/`SUNLIGHT_OVMF_VARS` if firmware lives elsewhere. KVM is
used when available. It boots an isolated optical image with no writable disk,
automatically enters the test desktop, and opens Rabbit through the Run dialog.
`SUNLIGHT_ACCOUNT_FIXTURE=1` enables `js-dom-fixture` only for the embedded
Rabbit build; normal kernel images continue to omit that opt-in capability.

QMP sends real PS/2 make/break events, including Unicode scalar composition.
Email is initially focused. The fixture uses the same Unicode font and text
measurements as the browser. The gate enters invented Unicode email text and a
masked Unicode password, cycles through both checkboxes, Submit and Link in
both directions, toggles Remember with Space, and submits from Password using
Enter. It captures real guest scanout through QEMU screendump (PPM) and converts
it losslessly to PNG. OCR checks initial/typed/final views; inspect the focus
outline and checkbox visually as well. No DOM-value injection or host-rendered
snapshot is used in this gate.

Evidence is saved under `target/qemu-keyboard-smoke/`:

- `serial.log`, `qemu.log`, `guest-gate.log`, and `result.txt`.
- `qemu-fixture-initial.png` with the initial Email focus.
- `qemu-interactive-typing.png` with committed non-ASCII text.
- `qemu-tab-navigation.png` with Remember checked and focused.
- `qemu-fixture-success.png` with simulated completion.

Only invented test data may be used. The driver, kernel IRQ handler, TTY desktop
forwarder and compositor do not log typed keys or text. Passwords remain masked
and are cleared before the simulated authorization callback. Live provider
integration remains disabled.

Current native keyboard validation (2026-11-07): the live QEMU gate passed
with visible `démo@example.test`, a masked Unicode password, Remember checked
and focused, forward/reverse traversal through the controls and link, and
completion through Enter. Original guest screenshots were visually inspected.
The scripted Unicode input includes composition followed by grapheme Backspace.
External IME candidate/preedit windows, full-script shaping and universal font
coverage are outside this fixture; the transport accepts every printable
Unicode scalar regardless of the installed font's glyph coverage.

Final checks for the native keyboard upgrade: 292 SunJS/Golden Fish/Rabbit
unit and integration tests, 291 UI tests, and 3 keyboard-driver tests passed
(586 total). `tools/check_workspace.py` passed all 94 platform checks. The
final opt-in native ISO passed the QEMU gate, including exact Unicode OCR.
`validation.json` and `SHA256SUMS` record the checks and image evidence.
