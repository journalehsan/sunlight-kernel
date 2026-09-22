# Account management: ownership, protocol and verification

Audit date: 2026-09-22. The GUI is an untrusted account client.

## Existing architecture (audited before implementation)

- `sunlight-fs/src/ramfs.rs` seeds `/etc/passwd`, `/etc/group`, `/etc/shadow`. `sunlight-fs/src/passwd.rs` supplies bounded parsers. The passwd GECOS field already stores display names. Group member names are present in the file, although the old parser discards them. `etc/users.json` has no Rust consumers and is not authoritative.
- UAC (`services/sunlight-uac/src/auth.rs`, `bin/uac_service.rs`) owns Argon2id verification and development-shadow migration. Login uses its existing authentication/session-grant protocol. There was no password-change API.
- Kernel process credentials are authoritative for UID/GID. Synchronous IPC stamps the sender PID into the badge. `session_query_process` resolves live credentials and process generation; previously only sessiond could use it.
- sessiond owns the active graphical session, ID/generation, lifecycle and shell. It consumes kernel-issued login grants before spawning a shell. Its startup profile is application configuration, not account metadata.
- The kernel Capability Broker has real single-use login grants. The UAC `session::runas` prompt cache is explicitly a mock. Its legacy delegation handler echoes a supplied nonzero token and its base-capability handler has a mint TODO. Neither is accepted as account-operation authority. The existing Run-As binary was terminal-only.
- Control Panel had no account page; Vortex's Start footer used literal `User` and `U`. There was no persistent avatar field. The general TextInput supports clipboard/selection and is unsuitable for secrets. UAC already used `zeroize`.
- A material storage split existed: the VFS daemon has a private seeded RAMFS, while libc uses the kernel VFS. UAC login verification read the daemon copy. Writing passwords through libc without changing this path would leave login using the old password.

## Authoritative service and storage

UAC now owns all account queries, profile changes, password verification/change and supported group mutations against the **same kernel VFS**. The login wire protocol, seeded accounts and PHC Argon2id format remain unchanged. UAC migration also uses this store. Legacy VFS public account reads/lookups/stat use the kernel files; its shadow-open endpoint is denied.

UAC has an explicit `session-identity` endpoint capability in its service profile. It permits resolving sessiond only; ordinary authentication capability does not imply it. Backend session-control checks remain unchanged.

Kernel filesystem policy permits only the trusted UAC actor to write the exact passwd/group/shadow files and their three staging paths. The kernel derives this actor from `trusted_auth_broker`, not an executable basename, UID 0, or a GUI flag. Root GUIs and generic filesystem capabilities remain denied. Ownership changes on these paths are denied. Ordinary `/etc` remains protected.

Writes use exclusive staging and atomic rename. The existing RAMFS is volatile; these changes do **not** add reboot persistence, fsync or crash durability. A failed staging operation fails closed. A staging file left by a killed daemon requires trusted cleanup; it is not overwritten opportunistically.

## Identity and UI

`sunlight-ipc::accounts` is the shared client used by both Vortex and Preferences. UAC requests `SESSION_CURRENT_IDENTITY` from sessiond, which returns only the live Running/Degraded graphical session, never `last_closed`. The account snapshot includes session ID/generation, current UID, usernames, GECOS display names, primary GIDs, real groups and explicit plus primary membership. UID is the stable account identifier. UID 0 is the only administrator implemented by existing policy; wheel membership does not create account authority.

Snapshots contain at most 16 users and 32 groups, fitting a single 4096-byte SHM page. Invalid, duplicate, oversized or unknown-member records fail closed rather than presenting a partial administrative model. Password hashes, salts and auth-cache internals are absent. Avatars are deterministic UID-derived colors and display-name initials; no new profile database is created. An unmatched primary GID is shown as an unlisted group, preserving the existing seed data.

Preferences has a compact current-user header and a Users & Groups card in System. Existing category membership is retained; cards are slightly denser to fit the additional entry without increasing window height. Clicking the header or Start footer opens `control-panel --page users-groups`, selecting the active account. Start refreshes on open and while open; Preferences refreshes on activation, explicit Refresh, periodic polling and after mutation. Service loss clears stale identity; session replacement cancels secret entry.

The page uses Column/Fill/Flex layout, a paged list, and stacked details. Users can edit their own display name and change their password. Groups expose member inspection; administrators can create groups, add/remove membership, and delete eligible empty groups through Run-As. Reserved GIDs below 1000, primary membership, primary-group reuse and nonempty deletion are independently protected by UAC. The UI also marks protected groups. Only one Run-As request per Settings instance can be outstanding; child completion/cancellation is collected and the authoritative model refreshed.

## Authentication and authorization

Own-password change sends current and new passwords through bounded SHM to UAC. Confirmation matching is the only password comparison performed by the GUI. UAC derives the target from kernel caller credentials, validates the active session and verifies the current password with the existing Argon2 code. It then mints and consumes a typed own-password grant before publishing the replacement. Own display-name edits likewise bind to the caller/session and use an own-profile grant; they preserve username, UID/GID, home, shell and other GECOS fields.

For group mutations, Preferences launches the existing `/bin/runas` in a narrowly parsed group mode. Run-As presents the account identity, the specific group/member action and a masked field. UAC verifies the current administrator's password and obtains a real typed grant from the kernel Capability Broker. Run-As then submits that grant to UAC for the specified mutation. Passwords never return to Preferences, and the legacy mock Run-As cache is not consulted.

Grant types are `ChangeOwnPassword`, `UpdateOwnProfile`, `GroupCreate`, `GroupAddMember`, `GroupRemoveMember`, and `GroupDelete`. Each grant binds operation, target UID/GID, member UID, caller PID/process generation, session ID/generation and account revision. It expires after 500 scheduler ticks, is consumed once, and is removed on failed scope validation. The kernel table is bounded and prunes expired grants. Session changes and public account revisions are rechecked by UAC. The public-record revision is an optimistic-concurrency fingerprint, not an authentication token.

`MintAccountGrant` (151) and `ConsumeAccountGrant` (152) are restricted to trusted UAC servicing that exact synchronous IPC caller. The credentials syscall grants UAC only the same narrowly bound lookup, while preserving sessiond's existing privilege. Raw GUI badges/UIDs, nonzero tokens, mock-cache success and ordinary login grants cannot authorize mutations.

`SecretInput` is a bounded UTF-8 container without clipboard, selection, Debug, Clone or plaintext rendering. Both dialogs mask text and clear buffers on focus loss, cancel, submission and shutdown. Buffer clearing uses volatile writes/compiler fences; UAC uses zeroizing containers. Shared secret pages are cleared and unmapped. No new secret logging is added.

## IPC additions

- sessiond `0xC121`: active identity, session ID/generation and UID/state.
- UAC `0xC200`: public account/group snapshot into client SHM.
- UAC `0xC201`: change own password; session pair and bounded secret lengths.
- UAC `0xC202`: authenticate a typed group scope and issue a grant to Run-As.
- UAC `0xC203`: consume the grant and perform that group mutation.
- UAC `0xC204`: update caller's display name with expected public revision.
- Replies `0xC2FF`/`0xC2FE`: typed success/errors. Calls use at most four register words; larger bounded data travels in SHM. Service calls have deadlines.

Errors distinguish unavailable account service, denied authority, wrong password, invalid input, changed session, storage failure, rejected/expired capability, changed account revision and unavailable broker. Run-As launch/completion errors and authentication cancellation are displayed separately from success.

## Intentionally deferred

User creation/deletion, modifications to another user, resetting another password, group rename, custom avatar storage and non-root administrator policy are not exposed. There is no generic administrative operation endpoint or authority for those actions. In particular, deleting protected users is impossible through this API. Creation/deletion needs a coordinated multi-file account transaction and home/session lifecycle policy that the existing services do not supply; it must not be approximated with partial passwd/group/shadow writes.

The legacy terminal Run-As mock/delegation paths remain outside this new account authority chain. This phase does not claim to finish general command elevation or to provide a spoof-resistant compositor authentication surface.

## Verification

Host tests cover shared session-UID presentation and refresh, profile preservation and revision changes, correct/incorrect old-password verification, local confirmation rejection, secret clearing/bounds/Unicode deletion, account service loss and session replacement, group creation/membership/deletion restrictions, ordinary-user rejection, broker scope/process/session/revision/expiry/replay checks, filesystem denial for root/ordinary GUIs, Start-menu navigation, Preferences rendering at 320/500/900px, and typed launcher registration.

The implementation uses the production broker grant-validation model in host tests; this is not equivalent to live kernel syscall or GUI-interaction validation. See the task handoff for exact commands/results and the session boot-gate outcome.
