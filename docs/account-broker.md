# Account broker: native integration and provider interoperability

## Online Accounts — Live Media contract (2026-11-07)

SunlightOS runs from a Live ISO. Account data lives in the active session's
VFS/ramfs. This milestone requires consistency across **KV and broker process
restarts in the same boot**, including acknowledged writes and disconnects.
It does not promise persistence across reboot, power loss, or ISO replacement.
There is no physical installer or disk-fsync gate for this milestone.

### Storage and security

`sunlight-kv` reserves `account-broker:*` for the kernel-stamped account broker.
Namespace checks precede ACL overrides, including root and broad-prefix scans.
The broker scopes records and its index to `account-broker:uid:<uid>:*`, using the
UID returned for the live control-panel IPC badge by the broker-only caller-role
syscall. Request words cannot supply UID, role, or generation.

Broker records use `/var/lib/sunlight-kv/broker.store`, separate from the shared
TLS/general `/var/lib/sunlight/kv.store`. KV appends each broker record or deletion
tombstone to ramfs **before** updating its live map and acknowledging the request.
On restart KV replays both logs, discards any broker-prefixed entries from the
shared log, and restores broker records only from the private log. Disconnect
checks ownership through the caller UID's index, deletes the account, verifies
its absence, then removes and verifies the index entry. Tombstones remove live
entries; this is not physical secure erasure of historical log bytes.

The kernel permits opening the private backing file only for the trusted embedded
KV image. A matching process name or root UID is insufficient. Filesystem policy
also denies non-KV mutation of that exact path; the compatibility VFS service
refuses to open it. The broker itself cannot open the raw log. Storage manager
has explicit authority only for the shared log and creation of its two parent
state directories, and cannot open the private log. The shared log is created
and hardened to mode `0600` before append so non-root file readers cannot bypass other
KV namespaces' ACLs. Public TLS anchors remain readable through KV as before.
The panel gains no filesystem write authority and does not use KV directly.

The 48-byte failure was in storage-manager READ_FILE's inline reply, not the
24-byte record header. KV now uses caller-owned SHM reads with an explicit offset
and bounded chunks. APPEND_FILE appends chunks instead of reading and replacing
the entire log. A failed read is never interpreted as an empty log for append.
A missing initial log enables creation on first append instead of permanently
selecting volatile-only mode. KV values up to 4096 bytes use their own SHM2 page;
key, ACL, header and log size do not reduce that value budget. Recovery validates
lengths before allocation. Legacy TLS opcodes and public certificate ACLs remain
in place. The generic log retains its existing queued persistence behavior; the
broker log has the stricter acknowledgement described above.

### Control Panel workflow

Open **System Preferences → Online Accounts**, or launch:

```sh
control-panel --page online-accounts
```

The page shows Microsoft branding, identifier and Connected/Active or Expired
status, with Disconnect on each row and pagination for longer lists. Add Account
opens the Microsoft provider picker. A normal image explicitly displays
**Provider not configured**; no live Microsoft registration is supplied here.
A `native-mock` image displays **Mock test mode**, and the Microsoft option starts
broker StartAuthorization. Rappid Rabbit opens the trusted local HTTPS fixture;
click **Authorize Microsoft mock account** to follow the native loopback callback.
The panel polls broker flow status and refreshes metadata after completion.
Cancellation, state mismatch, timeout, provider rejection and network/callback
failure have distinct messages. The panel can cancel a pending flow. Disconnect
waits for broker deletion before refreshing the list.

Only a kernel-verified embedded control panel may use Status, StartAuthorization,
ListAccounts, Disconnect, FlowStatus, Cancel, or the RequestAccess eligibility
check. Flow polling/cancellation additionally match the initiating PID, process
generation and UID. The wire snapshot contains only opaque ID, identifier,
provider and status: no access token, refresh token, authorization code, or
verifier. The panel has no token parser, token cache, token logger or token UI.
Rabbit children do not inherit the broker's kernel-stamped service identity.
RequestAccess remains an eligibility result for the panel, never bearer material;
production third-party access remains denied by DenyAccess.

Shell-driven `--mock` flows remain transient and preserve all ten existing cases.
Only panel-initiated successful flows commit to live-session KV. The native
provider-test feature remains opt-in and requires an installed real Client ID;
no live-provider success is claimed by the local mock gate.

### Verification and evidence

```sh
cargo test -p sunlight-kv -p sunlight-ipc -p sunlight-account-broker -p sunlight-fs --target x86_64-unknown-linux-gnu
cargo test -p sunlight-account-broker --features host-runtime --target x86_64-unknown-linux-gnu
cargo test -p sunlight-control-panel --features native-svg --target x86_64-unknown-linux-gnu
python3 tools/check_workspace.py
SUNLIGHT_ACCOUNT_BROKER_FEATURES=native-mock SUNLIGHT_TEST_SERIAL_LOG=target/phase-ab-mock-boot.log ./tools/test.sh phase3.0
mkdir -p target/phase-ab-ui
cp target/sunlightos.iso target/phase-ab-ui/native-mock.iso
# Refresh this address before running the optional external HTTPS check.
SUNLIGHT_ARCH_IP=$(getent ahostsv4 archlinux.org | awk 'NR == 1 { print $1 }')
python3 tools/network_tls_gate.py --iso target/phase-ab-ui/native-mock.iso --account-broker-probe --account-broker-flow --account-broker-storage --online-accounts-ui --arch-resolve "$SUNLIGHT_ARCH_IP" --cpus 2 --timeout 240 --evidence target/phase-ab-native
SUNLIGHT_ACCOUNT_BROKER_FEATURES=native-service SUNLIGHT_TEST_SERIAL_LOG=target/phase-ab-production-boot.log ./tools/test.sh phase3.0
```

The network gate starts an ephemeral CA and HTTPS mock on host port 8443, installs
its public certificate through the unchanged TLS/KV path, runs the ten existing
native mock outcomes and 1 KiB/4 KiB storage probes across two KV restarts, and
checks private backing-file access denial and shared-log mode `0600`. Its UI
driver uses QEMU PS/2 keyboard and mouse input and screenshots/OCR of the native scanout, with no DOM injection
or fake account injection. It clicks the settings card, provider picker, browser
Authorize link and Disconnect, and requires broker deletion confirmation plus
the rendered empty state. `convert`, `tesseract`, and Python Pillow are host
test prerequisites.

Evidence paths (generated under ignored `target/`):

* `target/phase-ab-host-tests.log`, `phase-ab-host-runtime.log`,
  `phase-ab-panel-svg-tests.log`, `phase-ab-workspace.log`.
* `target/phase-ab-mock-boot.log`, `phase-ab-mock-boot-gate.log`.
* `target/phase-ab-native/serial.log`, `network.pcap`, `qemu.log`.
* `target/phase-ab-native/02-online-accounts-empty.png`.
* `target/phase-ab-native/04-provider-picker.png`.
* `target/phase-ab-native/05-browser-authorization.png`.
* `target/phase-ab-native/06-online-accounts-connected.png`.
* `target/phase-ab-native/08-online-accounts-disconnected.png`.

For a focused UI rerun after network/storage verification:

```sh
python3 tools/network_tls_gate.py --account-broker-flow --online-accounts-ui --ui-only --cpus 2 --timeout 240 --evidence target/phase-ab-ui
```

Its log is `target/phase-ab-ui-gate.log`; focused scanout captures live in
`target/phase-ab-ui/`. The final combined gate and primary screenshots are under
`target/phase-ab-native/`, with command output in `target/phase-ab-network-gate.log`.
The saved mock ISO includes the final shared-log hardening. `target/sunlightos.iso`
is restored to the normal production image, with mock authorization disabled.

Executed results: **all passed**. Host checks include 221 broker/KV/IPC/filesystem
tests, 24 broker host-runtime tests, 17 panel tests, all 95 platform-aware workspace
checks, and explicit native-mock/native-provider-test compilation. Both normal and
mock Phase 3.0 boot gates passed. The final combined two-CPU QEMU gate passed all
ten mock outcomes, 1 KiB/4 KiB writes and recovery across two KV restarts, deletion
recovery, backing-file denial, shared-log `0600`, and interactive panel → Rabbit
Authorize → rendered Connected/Active → Disconnect → rendered empty state.
Storage-manager readback recovered a 2862-byte shared log. TLS/KV fixture trust,
unknown-issuer and wrong-hostname rejection, post-timeout verified requests, SOCKS
hostname handling and native verified **archlinux.org HTTP 200** all passed.
Issued codes, verifiers and mock access/refresh tokens were checked absent from
serial output. No live Microsoft account sign-in is claimed.

A credential-free machine-readable result and screenshot/image hashes are in
`target/phase-ab-evidence-summary.json`. Packet captures and fixture credentials
stay local under target. No generated evidence was committed.

For a manual production desktop (no disk image required):

```sh
qemu-system-x86_64 -cdrom target/sunlightos.iso -m 4096M -smp 2 -enable-kvm -cpu host -netdev user,id=net0 -device virtio-net-pci,netdev=net0,disable-modern=on -device virtio-rng-pci,disable-modern=on -serial file:target/online-accounts-manual.log
```

Choose Desktop at login, open System Preferences, then Online Accounts. The
normal image shows Provider not configured. The automated mock command above
also runs the local provider and installs its ephemeral CA; booting the mock ISO
alone does not start that host fixture.

## Historical Phase A/provider notes

The following sections describe the earlier integration and its original
production gating. Where they describe disabled storage or unavailable panel
commands, the Live Media contract and current behavior above supersede them.

### Build and launch

The normal build/test scripts and kernel embedded-binary builder package
`/sbin/sunlight-account-broker`. sunlightd starts it with:

```ini
[Unit]
After=sunlight-tls.service rand_service.service
Requires=sunlight-tls.service rand_service.service
[Service]
Type=simple
ExecStart=/sbin/sunlight-account-broker
```

The unit requests network, secure-random, logging and UserSession capabilities.
UserSession is inherited by the child Rabbit process, which needs the normal
window/session interfaces. This is **not** proof of a broker process sandbox or
exclusive KV access. No KV/VFS/File Service policy has been relaxed. The broker
issues no filesystem writes. Its packaged RAMFS executable stub only makes the
embedded executable visible to shell command resolution.

Build the production-gated daemon explicitly:

```sh
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' cargo build -p sunlight-account-broker --features native-service --release
```

In a guest, sunlightd launches the service automatically. Inspect/probe it with:

```sh
sunlightctl start sunlight-account-broker # already started automatically on boot
sunlightctl status sunlight-account-broker
/sbin/sunlight-account-broker --probe
/sbin/sunlight-account-broker --loopback-probe
```

The nameserver endpoint is `sunlight-account-broker`. The kernel's existing
executable-name registration check is used; no registration bypass was added.
UserSession clients may resolve this new endpoint, which confers no token grant.
A pure IPC policy test checks that Network alone and a zero mask cannot resolve it.

### Callback transport and browser integration

Initial inspection found that native bind rejected port zero and no native
loopback route or trusted browser callback handoff was available. Native loopback
and ephemeral ports were therefore implemented, as requested after inspection.
The executed QEMU socket probe passed: bind `127.0.0.1:0`, obtain the assigned
port, listen, connect, accept, verify the peer is `127.0.0.1`, and transfer bytes.
This is native net_server IPC, not a host-only listener.

`TcpManager` selects unused ephemeral ports and exposes generation/owner-checked
GET_SOCKNAME/GET_PEERNAME operations (21/22). The interface includes 127.0.0.1;
TCP uses an explicit loopback source for loopback connects. ProxyNetDevice
queues local IPv4/ARP frames internally with a 64-frame bound, never sending
loopback frames through the NIC. Physical packets claiming loopback addresses
are rejected. IPv6 loopback and POSIX socket API exposure are not implemented.

The broker binds/listens **before** generating the authorization URL or spawning
Rabbit. It launches `/bin/rappid-rabbit --account-broker <flow-number>` using the
native spawn interface. The ordinary shared launcher limits each argv element
to 96 bytes, so the authorization URL travels over bounded SHM IPC instead.
Only the exact kernel-stamped PID launched for that session can claim the URL,
and that claim is one-use. The broker retains its URL SHM mapping until Rabbit
acknowledges copying/releasing the page (0xAB08), or flow cleanup releases it.
This handshake follows the native mapping lifetime, verified after an initial
QEMU run exposed that freeing the sender mapping before receiver map invalidates
an otherwise delivered page token. No verifier, callback code or token is in argv/URL
handoff; only the public authorization URL (state and S256 challenge) is sent.
Native entropy uses rand_service via libc getrandom.

Callback HTTP reads are bounded to 8192 bytes and a two-second read deadline.
The native socket peer must be loopback. The method, exact Host/port, callback
path, expected session, state and exact redirect target are checked. The core
rejects duplicate/ambiguous fields, fragments and conflicting code/error replies.
Sessions are consumed before exchange or storage, including provider errors.
A wrong-state callback cancels the expected native session. Listener/session
cleanup occurs on timeout, cancellation, browser failure and completion.

The callback sends a no-store 303 to about:blank without echoing credentials.
The fetch backend consumes this terminal response for the reserved loopback
callback path and produces a static credential-free document with a clean URL.
Callback redirects/connection URLs are suppressed before RequestEvent telemetry;
callback URLs and codes never enter page JavaScript or the final response headers.
A host socket regression test checks events, body, final URL and headers for an
actual callback-code sentinel. Rabbit uses its normal native navigation/render
path; there is no page JavaScript callback bridge.

### IPC and bounds

Register-only operations use 0xAB00 Status, 0xAB01 Start, 0xAB02 Complete,
0xAB03 RequestAccess and 0xAB04 Disconnect, with 0xABFF replies. Production
Start/Status report unavailable installed provider configuration (word 0 = 2).
There is no real provider registration in this checkout. Complete and Disconnect
are denied (1); apps cannot inject authorization codes. RequestAccess calls the
existing DenyAccess core policy with the kernel badge and returns denied (1),
including for UID/PID-zero claims. Unknown operations return unsupported (3).

`native-mock` is an explicit test image feature, disabled by default. It enables
0xAB10 to start only the installed local fixture at https://10.0.2.2:8443, with
mail.read and a public test client ID. Arbitrary token URLs are not accepted.
0xAB05 claims the URL for the launched Rabbit PID; 0xAB06 queries status and
0xAB07 cancels, both restricted to the starting caller's kernel badge. Status
is 0 pending, 1 complete, 2 wrong state, 3 timeout, 4 cancelled, 5 provider
failure, 6 network/callback failure. Handles are not caller grants.

There are at most eight native flows. The existing core retains its eight-session,
120-second session bound and sixteen-account bound. The mock callback timeout
case uses four seconds. Core additions provide explicit cancellation, idle expiry,
and completion bound to an expected transport session. A failed host opener now
also consumes its session. Mock tokens use bounded MemoryKv and are zeroized on
flow destruction; production always uses UnavailableDurableKv, which issues no
KV IPC or filesystem writes and fails every durable operation.

### Native token exchange

The existing VerifiedTls adapter still performs one HTTPS POST with no redirects
and a 16 KiB token-response bound, using sunlight-fetch's low-level API. A native
process-local exchange guard supplies a 25-second whole-exchange budget, a
five-second connect budget and eight-second send/read budgets. Bounded TLS IPC
operations 0x5418/0x5412/0x5413 carry the remaining budget to the TLS daemon.
The daemon bounds network waits/handshake processing and closes expired sessions;
the client closes acquired handles on all return paths. Cleanup IPC has a separate
one-second bound, and the network stack retains its existing five-second TCP close
grace. Scheduling/cleanup can extend return time beyond the nominal exchange
budget. Bounded broker requests also cap HTTP headers at 16 KiB.
Existing TLS clients retain their previous operation behavior.

Cancellation while waiting for a callback closes the listener and consumes the
session immediately. Token exchange is synchronous: a cancellation request during
exchange is serviced after the bounded exchange returns, not a hard interrupt of
a running TLS operation. This limitation must be addressed before an interactive
production consent flow promises immediate cancellation. Logs report only failure
categories, not provider error text, codes, verifiers, forms or token bodies.

### Reproduction and evidence

Run from the repository root:

```sh
cargo test -p sunlight-account-broker --target x86_64-unknown-linux-gnu
cargo test -p sunlight-account-broker --features host-runtime --target x86_64-unknown-linux-gnu
cargo test -p sunlight-kv --target x86_64-unknown-linux-gnu
cargo test -p sunlight-net --target x86_64-unknown-linux-gnu
cargo test -p sunlight-ipc --target x86_64-unknown-linux-gnu account_broker_lookup_is_a_user_session_operation
cargo test -p sunlight-fetch --features host-linux --target x86_64-unknown-linux-gnu callback_is_consumed_before_events_and_page_state
python3 tools/check_workspace.py
SUNLIGHT_ACCOUNT_BROKER_FEATURES=native-mock SUNLIGHT_TEST_SERIAL_LOG=target/account-broker-native-boot.log ./tools/test.sh phase3.0
python3 tools/network_tls_gate.py --account-broker-probe --account-broker-flow --cpus 2 --timeout 180 --evidence target/account-broker-native-flow
```

The last command starts a local HTTPS provider on host port 8443, installs its
fresh CA through the existing certificatectl/TLS/KV path, and drives guest shell
commands through QMP. It then launches the broker's native mock cases:

```sh
/sbin/sunlight-account-broker --mock 8443 0 # success
/sbin/sunlight-account-broker --mock 8443 1 # wrong state
/sbin/sunlight-account-broker --mock 8443 2 # replay
/sbin/sunlight-account-broker --mock 8443 3 # callback timeout
/sbin/sunlight-account-broker --mock 8443 4 # cancelled
/sbin/sunlight-account-broker --mock 8443 5 # provider denial
/sbin/sunlight-account-broker --mock 8443 6 # token endpoint error
/sbin/sunlight-account-broker --mock 8443 7 # native token-read deadline
/sbin/sunlight-account-broker --mock 8443 8 # refused native token connection
/sbin/sunlight-account-broker --mock 8443 9 # whole-exchange deadline despite read progress
```

The provider checks client ID, redirect URI, scopes, S256 against the POSTed
verifier, grant type and one-use authorization codes. It never logs requests.
Replay includes a duplicate callback over native loopback HTTP and verifies
that the broker still has exactly one volatile write. The fixture asserts the
expected number of actual token POSTs, not just serial success strings.

Historical results for the preceding native-mock integration (the new milestone
rerun is recorded below):

* 11 broker core tests, 16 host-runtime tests, 20 KV tests and 27 net tests pass.
  The focused IPC lookup test and callback-rendering regression pass.
* All 95 platform-aware workspace checks pass; native-mock and native TLS
  feature checks also pass.
* Phase 3.0 boots pass for both the native-mock image and the normal production-
  gated image. The final normal image has the mock fixture compiled out, and
  its serial output confirms the native broker starts with durable storage disabled.
* All ten native Rabbit/mock cases pass, including unauthorized URL claim,
  wrong state, HTTP replay, callback timeout, cancellation, provider denial,
  token HTTP failure, read timeout, refused connection and whole-exchange timeout.
  The local provider verified five actual one-use token POSTs and PKCE verifiers.
  Captured serial output contains none of the issued codes, verifiers or mock tokens.
* The packet capture contains zero loopback frames on the physical NIC. Client
  close began at 8.007 seconds for the stalled read and 24.985 seconds for the
  continuously progressing response (measured from each token connection's SYN).
  Subsequent TCP drain/grace packets are not counted as exchange time.
* The integrated native TLS/KV gate passes: two seeded roots become three after
  the fixture CA install; verified requests still succeed after broker timeouts.
  UnknownIssuer and NotValidForNameContext failures are observed for the negative
  cases. No new KV permission error appears. No trust-store ACL/namespace change
  was made.

Evidence is retained locally, not committed:

* `target/account-broker-native-flow/serial.log`, `network.pcap`, `qemu.log`, and
  `evidence-summary.json`; console result: `target/account-broker-native-flow-gate.log`.
* `target/account-broker-native-boot.log` / `target/account-broker-boot-gate.log`.
* `target/account-broker-production-boot.log` /
  `target/account-broker-production-boot-gate.log`.
* `target/account-broker-{core,host,kv,workspace}.log`, `account-net-tests.log`,
  `account-broker-ipc-test.log`, and `account-callback-test.log`.

Representative executed native markers (no credentials):

```text
[RABBIT] native broker authorization URL received
[ACCOUNT-BROKER] native mock code exchanged; completion; volatile writes=1
[ACCOUNT-BROKER] callback rejected: state
[ACCOUNT-BROKER] native HTTP callback replay rejected; volatile writes=1
[ACCOUNT-BROKER] callback timeout; listener and session released
[ACCOUNT-BROKER] cancelled; listener and session released
[ACCOUNT-BROKER] provider failure
[ACCOUNT-BROKER] native timeout
```

To restore/build the normal image after testing:

```sh
SUNLIGHT_TEST_SERIAL_LOG=target/account-broker-production-boot.log ./tools/test.sh phase3.0
```

The end-to-end result is native mock authorization only. It is not Google,
Dropbox or another live provider sign-in.

### Trust store and remaining work

TLS still seeds gtsr4/isrgx1, reads tls.ca.idx and tls.ca.* via the same KV SHM
operations, and builds verified client configurations. Account integration neither
selects durable token keys nor changes those ACLs or certificate verification.
KV account namespaces distinguish UID, not applications of one UID; Vault's
existing AUTH_BROKER_SERVICE identity is sunlight-uac, not this daemon. Exclusive
broker secret access and authenticated application grants remain unproved.

The preceding native-mock integration did not run a fresh external TLS exchange. The previous
core/host milestone recorded an external archlinux.org handshake; that historical
evidence is not a new broker/provider exchange. Local native mock authorization,
local verified TLS and live provider interoperability are separate claims.

Remaining production work includes installed provider registrations, actual
provider interoperability, subject identity lookup, consent/app grants, exclusive
KV capabilities, durable token storage, refresh/revocation, and immediate
in-flight exchange cancellation. Downloaded files, attachments and sync data
remain File Service work. No production RequestAccess grant is enabled.


### Changed source areas

* `sunlight-account-broker`: native executable, listener/flow coordinators,
  feature-gated local provider setup, core session cancellation/expiry/binding,
  host opener cleanup, native transport deadlines and tests.
* `sunlight-net`, `services/net_server`, and `ipc`: bounded loopback frames,
  ephemeral ports, endpoint queries and broker endpoint lookup policy/test.
* `rappid-rabbit`, `sunlight-fetch`, and `sunlight-tls`: native URL claim/ACK,
  pre-render callback handling and bounded native exchange operations.
* `kernel`, `services/sunlightd`, `sunlight-fs/src/ramfs.rs`, `tools/build.sh`,
  `tools/test.sh`: embedded executable resolution, service unit and packaging.
* `tools/network_tls_gate.py`: local PKCE provider, native flow cases and
  secret-leak assertions. Existing broker/KV/TLS/VFS/File Service policies remain
  in place; only the new broker's own lookup endpoint was added.


### First real-provider milestone (2026-11-07)

Selected provider: **Microsoft identity platform, personal Microsoft accounts**.
This is an interoperability test feature, not a production account service.
Policy/registration documentation checked before implementation:

* [Authorization code flow](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow):
  desktop public clients support PKCE, S256, and code exchange without a client secret.
* [Redirect rules](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url):
  HTTP 127.0.0.1 registration requires manifest editing; unlike localhost, its port
  must match. The installed redirect is therefore exactly
  `http://127.0.0.1:53682/oauth/callback`. Bind failure aborts; no alternate
  host, port, path, wildcard, scheme or externally delivered callback is accepted.
* [Browser guidance](https://learn.microsoft.com/en-us/entra/msal/dotnet/acquiring-tokens/using-web-browsers):
  Microsoft supports system browsers and some embedded views, recommends the
  system browser, and serves HTML/JavaScript for interactive authentication.
  Rabbit is launched as a separate native browser process using its existing
  renderer/navigation, not an app-hosted webview. This policy review does not
  establish that Rabbit can run Microsoft's current sign-in UI. No User-Agent
  spoofing or provider-specific browser-policy bypass was added.
* [Registration](https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app)
  requires access to an Entra tenant and permission to create an application.

#### Manual registration and local input

1. In the Entra admin center, App registrations → New registration, create a
   development-only app for **Personal Microsoft accounts only**. Record its
   Application (client) ID. Use a dedicated personal test account for consent.
2. Configure a **Mobile and desktop applications/public native** redirect (not
   Web or SPA). In the manifest register the exact HTTP 127.0.0.1 URI above as
   a native/public-client reply URL. Microsoft documents `replyUrlsWithType`
   with a `Native` type; current manifest editors may expose the equivalent
   `publicClient.redirectUris` property. Verify the saved native redirect.
3. Keep only delegated Microsoft Graph `User.Read` for this test; remove extra
   permissions. Do not create a client secret/certificate, enable implicit
   grants, or request `offline_access`. No refresh or background access is needed.
4. Supply only the public application UUID locally using
   `SUNLIGHT_MICROSOFT_CLIENT_ID`. This compile-time variable is tracked by Rust's
   `option_env!`; missing, nil or malformed UUIDs fail closed before binding or
   navigation. Passwords/MFA stay in the interactive browser, never build flags,
   chat, logs or documentation. Do not use another application's published ID.

`src/installed_provider.rs` fixes the authorization/token URLs to
`https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize` and
`https://login.microsoftonline.com/consumers/oauth2/v2.0/token`, the scope to
`User.Read`, and the redirect above. Application IPC supplies none of these.
Microsoft's optional callback `session_state` metadata is accepted only for
this installed provider, ignored as identity evidence, and duplicate fields
remain rejected. Subject identity lookup is not implemented.

Build a registered test image locally (substitute your public UUID):

```sh
export SUNLIGHT_MICROSOFT_CLIENT_ID='<your-application-client-UUID>'
SUNLIGHT_ACCOUNT_BROKER_FEATURES=native-provider-test ./tools/test.sh phase3.0
```

In the guest's interactive desktop shell, run:

```sh
/sbin/sunlight-account-broker --probe
/sbin/sunlight-account-broker --real-provider
```

The opt-in test operation is 0xAB11. It takes no app-supplied provider parameters.
It shares the bounded native listener, PID-bound Rabbit URL handoff, callback
sanitization and verified HTTPS transport with the separate mock flow. Its
120-second consent bound may require retrying after a slow login. Ordinary
production Start/Status remain gated and RequestAccess remains DenyAccess.
The test keeps token responses solely in MemoryKv, then deletes/zeroizes them
immediately on successful exchange before reporting completion. Flow destruction
also zeroizes volatile values. This is an explicit transient test backend;
UnavailableDurableKv is unchanged and no KV/file fallback is attempted.
No durable refresh-token storage is enabled even if a provider returns one.

Only these static markers may be used to report live completion:

```text
[ACCOUNT-BROKER] listener ready before native browser launch
[RABBIT] native broker authorization URL received
[ACCOUNT-BROKER] real provider loopback callback received; verified HTTPS exchange pending
[ACCOUNT-BROKER] real provider verified HTTPS code exchange complete; transient tokens discarded; access denied
```

These are expected markers, **not observed live evidence**. Do not capture the
callback address bar, authorization codes, token bodies, request forms or verifier.
Use serial markers instead of screenshots of credential-bearing browser state.

#### Current attempt and regression evidence

No registered client ID exists in the local environment and no test account was
supplied. Native `--real-provider` preflight is exercised with the registration
unset. Expected/observed outcome is exit 3 and this non-secret blocker:

```text
[ACCOUNT-BROKER] real provider blocked: local Microsoft client ID missing or invalid
```

Thus no real-provider browser launch, consent, callback or token exchange can be
claimed. Registration and an interactive test account are the first exact blockers;
Rabbit's Microsoft sign-in compatibility and native reachability of the Microsoft
endpoints remain untested, rather than asserted failures. No mock is substituted
for this attempt.

Commands for this milestone's separate regression gates:

```sh
cargo test -p sunlight-account-broker --target x86_64-unknown-linux-gnu
cargo test -p sunlight-account-broker --features host-runtime --target x86_64-unknown-linux-gnu
cargo test -p sunlight-kv --target x86_64-unknown-linux-gnu
cargo test -p sunlight-net --target x86_64-unknown-linux-gnu
cargo test -p sunlight-ipc --target x86_64-unknown-linux-gnu account_broker_lookup_is_a_user_session_operation
cargo test -p sunlight-fetch --features host-linux --target x86_64-unknown-linux-gnu callback_is_consumed_before_events_and_page_state
python3 tools/check_workspace.py
# No local registration: test the fail-closed preflight and the separate mock gate.
unset SUNLIGHT_MICROSOFT_CLIENT_ID
SUNLIGHT_ACCOUNT_BROKER_FEATURES=native-mock,native-provider-test SUNLIGHT_TEST_SERIAL_LOG=target/account-provider-native-boot.log ./tools/test.sh phase3.0
cp target/sunlightos.iso target/account-provider-test.iso
python3 tools/network_tls_gate.py --iso target/account-provider-test.iso --account-broker-probe --account-broker-flow --account-provider-preflight --arch-resolve 209.126.35.79 --cpus 2 --timeout 180 --evidence target/account-provider-native-flow
# Restore production image after all fixture runs.
SUNLIGHT_TEST_SERIAL_LOG=target/account-provider-production-boot.log ./tools/test.sh phase3.0
python3 tools/network_tls_gate.py --account-broker-probe --cpus 2 --timeout 180 --evidence target/account-provider-production-tls
```

The Arch IPv4 above was freshly resolved on this run; resolve it again for a
future run. External verified HTTPS is an independent transport check, never
provider consent evidence. Host test logs are `target/account-provider-{core,host,
kv,net,ipc,callback,workspace}.log`; boot and gate output are kept under `target/`.

#### Production capability limitation

The production service explicitly requests network, secure-random, logging and
UserSession. Inspection of `matches_user_session_service` shows UserSession
allows **both sunlight-kv and vfs lookup**. This capability is inherited by Rabbit
and is broader than a no-write profile. Startup now performs a read-only lookup
audit and reports the blocked profile; it performs no test writes. The absence
of explicit kv-store/vfs bits does not prove write denial. No KV ACL, namespace,
TLS root policy or service capability has been relaxed or redesigned. A verified
production no-write capability profile remains an outstanding blocker requiring
separation of browser capabilities or narrower kernel/service policy. The broker's
own durable operations continue to fail closed, but that is not process isolation.


Executed host results for this milestone: 1 installed-configuration test plus
12 core flow tests pass; with host-runtime, 1 configuration test plus 17 flow/
host tests pass. KV: 20; net: 27; focused IPC: 1; callback-redaction regression:
1; all pass. The provider-only native feature check and all 95 workspace checks
pass. Both fixture and restored-production Phase 3.0 boot gates pass.
The restored production broker ELF contains neither mock completion nor provider
test completion/preflight markers, consistent with both features being compiled
out. Production serial evidence includes:

```text
[ACCOUNT-BROKER] no-write capability profile BLOCKED: KV or VFS endpoint resolvable
[ACCOUNT-BROKER] native service ready; production provider unconfigured; durable storage disabled
```

The restored **production** native TCP/TLS↔KV gate passes (2 CPUs). The fresh
fixture CA is installed through certificatectl and the existing TLS/KV SHM path;
verified HTTPS succeeds, while the untrusted issuer and wrong-hostname requests
fail as expected. Evidence: `target/account-provider-production-tls/serial.log`,
`network.pcap`, `qemu.log`, and `target/account-provider-production-tls-gate.log`.

The combined **fixture** native gate also passes (2 CPUs): all ten mock modes,
URL-claim restrictions, callback replay, TLS read/whole-exchange deadlines and
subsequent verified requests pass. The gate's provider asserts five actual
one-use token POSTs with correct PKCE and checks serial output against all issued
mock codes/verifiers/tokens; those secret-leak assertions pass. This is still
local mock evidence. Native real-provider preflight exits **3** with the
missing/invalid local Microsoft client-ID marker above. No Microsoft browser
launch, consent, callback or token POST occurred.

A **fresh external TLS exchange was performed after these changes**: native
`fetch --resolve archlinux.org:443:209.126.35.79 -o /tmp/arch-direct.html https://archlinux.org`
exits 0 and reports `[SUNLIGHT-TLS] hs_OK host=archlinux.org`. It uses existing
certificate verification; it is independent of Microsoft OAuth. Representative
observed, credential-free gate output:

```text
PASS (exit 3): /sbin/sunlight-account-broker --real-provider
PASS (exit 0): fetch --resolve archlinux.org:443:209.126.35.79 -o /tmp/arch-direct.html https://archlinux.org
Native Rabbit/local mock authorization gate passed (not real-provider sign-in)
Real-provider preflight BLOCKED: no local Microsoft registration; no live authorization attempted
TCP/TLS gate passed on 2 CPUs
```

Evidence is local under `target/account-provider-native-flow/{serial.log,
network.pcap,qemu.log,evidence-summary.json}` and
`target/account-provider-native-flow-gate.log`. The default ISO has been restored
to production; the copied test ISO is `target/account-provider-test.iso`.
Remaining blockers: developer-owned Microsoft registration/client ID, interactive
personal test-account consent, verified Rabbit compatibility with Microsoft's
current sign-in UI, native Microsoft endpoint TLS/reachability, and a proved
production no-write capability profile. Durable storage, application grants,
refresh/revocation and production RequestAccess remain deliberately unavailable.
No commit or push was performed.
