# SunlightOS Bite Task List
## Immutable filesystem write policy with UAC and scoped capability broker

Use this checklist as the execution tracker for the bite.  
Tick items only after they are actually verified. Do not tick based on assumptions.

---

## 0. Safety gates

- [ ] Confirm immutable/read-only root is intentional and must be preserved.
- [ ] Confirm `/` must not become broadly writable.
- [ ] Confirm `touch` must not fake success.
- [ ] Confirm `touch` must use the common filesystem authorization path.
- [ ] Confirm normal UAC must not override protected OS paths.
- [ ] Confirm capability broker is for trusted OS services only, not normal users.
- [ ] Confirm capability broker must not ask the user directly.
- [ ] Confirm scoped capabilities cannot become global bypass tokens.
- [ ] Confirm unrelated TLS/KV/scheduler/shell logic will not be refactored.

Definition of done:

- [ ] A short safety note is written before implementation.
- [ ] Any discovered exception is documented before code changes.

---

## 1. Repository audit

Search for filesystem write policy, immutable/rootfs behavior, UAC, capabilities, and utilities.

Suggested search terms:

```text
touch
ls
read-only
readonly
EROFS
EPERM
EACCES
mount
rootfs
tmp
home
uac
run_as
capability
cap
token
broker
fs
sunlight-fs
sunlight-utils
sunlight_uac
services
state
var/lib
```

Inspect likely areas if they exist:

```text
services/sunlight-fs/
services/sunlight_uac/
services/capability-broker/
services/
sunlight-utils/
userland/
utils/
kernel/fs/
kernel/capability/
kernel/ipc/
kernel/syscall/
```

Audit questions:

- [ ] Where is root filesystem marked read-only?
- [ ] Is read-only behavior global, per mount, or per path?
- [ ] What error does denied create/write currently return?
- [ ] Does `touch` distinguish `EROFS`, `EACCES`, and `EPERM`?
- [ ] Does `ls` fail anywhere because read-only is confused with unreadable?
- [ ] Are `/tmp` and `/home/<user>` treated specially?
- [ ] Does UAC currently participate in filesystem writes?
- [ ] Is there an existing capability broker?
- [ ] Are service identities separate from user identities?
- [ ] Are service state directories already defined?
- [ ] Are `/run`, `/dev`, or similar paths modeled as runtime paths?
- [ ] Are there TODOs/stubs around write authorization?

Audit output:

- [ ] Add an audit note to the final report.
- [ ] List the files that currently own filesystem write decisions.
- [ ] List the files that currently own user/service identity.
- [ ] List the files that currently own UAC/run-as decisions.
- [ ] List whether broker exists or must be scaffolded.

Stop condition:

- [ ] Do not implement policy until audit findings are written.

---

## 2. Current behavior reproduction

Run or create minimal checks for the current behavior before changing code.

Read/list matrix:

- [ ] `ls /` works if `/` is readable.
- [ ] `ls /tmp` works if `/tmp` exists.
- [ ] `ls /home/<current-user>` works if home exists.
- [ ] `ls /bin` works if `/bin` exists.
- [ ] `ls /etc` works if `/etc` exists.

Write/create matrix:

- [ ] `touch /x` currently observed and result recorded.
- [ ] `touch /tmp/x` currently observed and result recorded.
- [ ] `touch /home/<current-user>/x` currently observed and result recorded.
- [ ] `touch /home/<other-user>/x` currently observed and result recorded.
- [ ] `touch /bin/x` currently observed and result recorded.
- [ ] `touch /etc/x` currently observed and result recorded.
- [ ] `touch /run/x` currently observed and result recorded if `/run` exists.

Service state matrix, if possible:

- [ ] `sunlight-kv` write to own state path observed.
- [ ] `sunlight-kv` write to arbitrary root path observed.
- [ ] `sunlight-tls` write to own state/cache path observed.
- [ ] `sunlight-tls` write to `/etc` or `/services` observed.

Reproduction output:

- [ ] Add observed failures to final report.
- [ ] Identify root cause of `touch: file system is read-only`.
- [ ] Confirm whether the problem is missing allowlist, wrong mount flag, missing actor identity, or missing capability path.

---

## 3. Add targeted diagnostics

Filesystem authorization logs:

- [ ] Add log for write/create/mkdir/delete request.
- [ ] Include actor.
- [ ] Include operation.
- [ ] Include normalized path.
- [ ] Include raw path if useful for debugging.
- [ ] Add log for policy decision.
- [ ] Include `allow` or `deny`.
- [ ] Include policy reason.
- [ ] Include mapped error.

Required log shape:

```text
[SUNLIGHT-FS] request actor=<actor> op=<write|create|mkdir|delete> path=<path>
[SUNLIGHT-FS] decision actor=<actor> op=<op> path=<path> result=<allow|deny> reason=<reason> err=<err>
```

UAC logs, only if UAC participates:

- [ ] Log UAC authorization request.
- [ ] Log UAC allow/deny decision.
- [ ] Log reason.

Required log shape:

```text
[SUNLIGHT-UAC] authorize actor=<actor> action=<op> path=<path>
[SUNLIGHT-UAC] decision result=<allow|deny> reason=<reason>
```

Capability broker logs, only if broker exists or is added:

- [ ] Log broker request.
- [ ] Log broker UAC/policy decision.
- [ ] Log minted capability scope and rights.
- [ ] Log deny reason.

Required log shape:

```text
[CAP-BROKER] request actor=<actor> subject=<subject> rights=<rights> scope=<path>
[CAP-BROKER] uac decision=<allow|deny>
[CAP-BROKER] minted subject=<subject> rights=<rights> scope=<path>
[CAP-BROKER] denied reason=<reason>
```

Noise control:

- [ ] Do not spam ordinary reads.
- [ ] Do not log every `ls` entry unless debugging read permission.
- [ ] Keep logs behind existing debug/trace style if available.

---

## 4. Implement path normalization and path categories

Normalize path before any policy decision.

Bypass tests that must be denied:

- [ ] `/tmp/../etc/x`
- [ ] `/home/<self>/../../etc/x`
- [ ] `/state/sunlight-kv/../../../bin/x`
- [ ] `/run/../services/x`

Path categories to implement or document:

- [ ] `Tmp`
- [ ] `CurrentUserHome`
- [ ] `OtherUserHome`
- [ ] `RuntimePath`
- [ ] `ServiceState`
- [ ] `ProtectedImmutable`
- [ ] `ImmutableRoot`
- [ ] `Unknown`

Protected immutable paths:

- [ ] `/boot`
- [ ] `/kernel`
- [ ] `/bin`
- [ ] `/sbin`
- [ ] `/services`
- [ ] `/etc`
- [ ] `/proc`
- [ ] `/sys`

Runtime paths:

- [ ] `/run`
- [ ] Selected `/dev` subpaths only if existing design supports them.
- [ ] Any additional runtime path discovered during audit.

Definition of done:

- [ ] Policy receives normalized path.
- [ ] Policy cannot be bypassed with `..`.
- [ ] Protected path detection happens before UAC approval.

---

## 5. Implement or complete common write policy

Create or complete one common function:

```rust
can_write(actor, path, operation, optional_capability) -> Decision
```

Decision shape:

- [ ] `allowed: bool`
- [ ] `reason: PolicyReason`
- [ ] `error: Option<FsError>`

Required allow reasons:

- [ ] `AllowedTmp`
- [ ] `AllowedCurrentUserHome`
- [ ] `AllowedRuntimePathWithUac`
- [ ] `AllowedServiceState`
- [ ] `AllowedByScopedCapability`

Required deny reasons:

- [ ] `DeniedImmutableRoot`
- [ ] `DeniedProtectedPath`
- [ ] `DeniedOtherUserHome`
- [ ] `DeniedRuntimePathNeedsUac`
- [ ] `DeniedMissingCapability`
- [ ] `DeniedInvalidCapability`
- [ ] `DeniedUnknownActor`

Error mapping:

- [ ] `DeniedImmutableRoot -> EROFS`
- [ ] `DeniedProtectedPath -> EPERM` or `EROFS`, but document which one.
- [ ] `DeniedOtherUserHome -> EACCES`
- [ ] `DeniedRuntimePathNeedsUac -> EPERM`
- [ ] `DeniedMissingCapability -> EPERM`
- [ ] `DeniedInvalidCapability -> EPERM`
- [ ] `DeniedUnknownActor -> EPERM`

Common-path requirement:

- [ ] `touch` uses this path.
- [ ] File create uses this path.
- [ ] File write uses this path.
- [ ] `mkdir` uses this path if implemented.
- [ ] Delete/unlink uses this path if implemented.
- [ ] Service persistence uses this path if possible.

---

## 6. User policy

Normal user write allowlist:

- [ ] Allow `/tmp/*`.
- [ ] Allow `/home/<current-user>/*`.

Normal user write denylist:

- [ ] Deny `/*`.
- [ ] Deny `/home/<other-user>/*`.
- [ ] Deny `/state/*`.
- [ ] Deny `/var/lib/*`.
- [ ] Deny `/services/*`.
- [ ] Deny `/kernel/*`.
- [ ] Deny `/boot/*`.
- [ ] Deny `/bin/*`.
- [ ] Deny `/sbin/*`.
- [ ] Deny `/etc/*`.
- [ ] Deny `/proc/*`.
- [ ] Deny `/sys/*`.

Normal user UAC behavior:

- [ ] UAC may approve runtime path operations only if design supports it.
- [ ] UAC must not approve protected immutable paths.
- [ ] UAC must not approve arbitrary `/`.
- [ ] UAC must not approve another user's home.

Required tests:

- [ ] `user:alice create /tmp/x -> allow`
- [ ] `user:alice create /home/alice/x -> allow`
- [ ] `user:alice create /x -> deny`
- [ ] `user:alice create /home/bob/x -> deny`
- [ ] `user:alice create /bin/x -> deny`
- [ ] `user:alice create /etc/x -> deny`
- [ ] `user:alice create /run/x -> deny unless UAC-approved`

---

## 7. Service state policy

Pick existing layout if already present. Otherwise use:

```text
/state/<service-name>
```

Required service state directories:

- [ ] `/state/sunlight-kv`
- [ ] `/state/sunlight-tls`
- [ ] `/state/sunlight-uac`
- [ ] `/state/capability-broker` if broker is added.

Service write allowlist:

- [ ] `service:sunlight-kv` can write `/state/sunlight-kv/*`.
- [ ] `service:sunlight-tls` can write `/state/sunlight-tls/*`.
- [ ] `service:sunlight-uac` can write `/state/sunlight-uac/*`.
- [ ] `service:capability-broker` can write `/state/capability-broker/*`.

Service write denylist:

- [ ] Service cannot write arbitrary `/`.
- [ ] Service cannot write `/home/<user>/*` unless explicitly authorized.
- [ ] Service cannot write `/services/*`.
- [ ] Service cannot write `/bin/*`.
- [ ] Service cannot write `/etc/*`.
- [ ] Service cannot write another service's state directory.

Required tests:

- [ ] `service:sunlight-kv create /state/sunlight-kv/db -> allow`
- [ ] `service:sunlight-kv create /state/sunlight-tls/cache -> deny`
- [ ] `service:sunlight-kv create /etc/kv.conf -> deny`
- [ ] `service:sunlight-tls create /state/sunlight-tls/cache -> allow`
- [ ] `service:sunlight-tls create /services/tls/x -> deny`

---

## 8. UAC integration

Reuse existing UAC or run-as identity.

Do not create a competing UAC system.

UAC adapter tasks:

- [ ] Identify actor format used by UAC/run-as.
- [ ] Convert filesystem actor to UAC actor without losing user/service identity.
- [ ] Add filesystem action names:
  - [ ] `read`
  - [ ] `write`
  - [ ] `create`
  - [ ] `mkdir`
  - [ ] `delete`
- [ ] Add path/resource representation.
- [ ] Ensure UAC deny-by-default for unknown filesystem resources.

UAC must allow only eligible runtime operations:

- [ ] `/run/*` if existing model allows runtime writes.
- [ ] Selected `/dev/*` only if explicitly modeled.

UAC must deny protected paths:

- [ ] `/bin/*`
- [ ] `/sbin/*`
- [ ] `/etc/*`
- [ ] `/kernel/*`
- [ ] `/boot/*`
- [ ] `/services/*`
- [ ] `/proc/*`
- [ ] `/sys/*`

Definition of done:

- [ ] UAC is called only when policy category requires UAC.
- [ ] UAC is not called for normal `/tmp` and own-home writes unless existing design requires it.
- [ ] UAC cannot override protected immutable paths.
- [ ] UAC decisions are logged.

---

## 9. Capability broker

First decision:

- [ ] Search whether capability broker exists.
- [ ] If it exists, reuse it.
- [ ] If it does not exist, add minimal scaffold only.

Broker access rules:

- [ ] Normal users cannot request broker filesystem caps directly.
- [ ] Normal apps cannot request global write caps.
- [ ] Only trusted OS services can request broker filesystem caps.
- [ ] Broker does not ask the user directly.
- [ ] Broker uses UAC/policy approval internally.
- [ ] Broker mints scoped capabilities only.

Capability fields:

- [ ] `issuer`
- [ ] `subject`
- [ ] `rights`
- [ ] `scope`
- [ ] `expires_at` or documented lifetime/session behavior.

Rights:

- [ ] `read`
- [ ] `write`
- [ ] `create`
- [ ] `mkdir`
- [ ] `delete`, only if delete exists.

Capability validation:

- [ ] Subject must match actor.
- [ ] Operation must be included in rights.
- [ ] Path must be inside scope.
- [ ] Expired capability must fail if time/session exists.
- [ ] Capability for one service state dir must not work elsewhere.
- [ ] Capability must not grant global `/` write.
- [ ] Capability must not bypass protected paths broadly.

Required tests:

- [ ] Trusted service requests scoped state cap -> allowed.
- [ ] Normal user requests broker cap -> denied.
- [ ] Cap works inside scope -> allowed.
- [ ] Cap fails outside scope -> denied.
- [ ] Cap fails for different subject -> denied.
- [ ] Cap cannot write `/etc` broadly -> denied.
- [ ] Cap cannot write `/bin` broadly -> denied.

---

## 10. Utilities: `ls`, `touch`, and error display

`ls`:

- [ ] Uses read/list permission, not write permission.
- [ ] Works on readable immutable directories.
- [ ] Reports permission denied only for unreadable paths.
- [ ] Does not care that root is read-only.

`touch`:

- [ ] Uses common create/write path.
- [ ] Does not bypass policy.
- [ ] Does not fake success.
- [ ] Maps `EROFS` to read-only filesystem message.
- [ ] Maps `EACCES` to permission denied.
- [ ] Maps `EPERM` to operation not permitted or authorization required.
- [ ] Succeeds in `/tmp`.
- [ ] Succeeds in `/home/<current-user>`.
- [ ] Fails in `/`.
- [ ] Fails in `/home/<other-user>`.
- [ ] Fails in `/bin`, `/etc`, `/sbin`, `/services`.

Output expectations:

```text
touch /x
=> read-only filesystem

touch /home/other/x
=> permission denied

touch /run/x
=> operation not permitted or UAC required

touch /tmp/x
=> success
```

---

## 11. Automated tests

Add or update executable tests where the repo supports them.

Normal user tests:

- [ ] `/tmp/file` allowed.
- [ ] `/home/<self>/file` allowed.
- [ ] `/file` denied.
- [ ] `/services/file` denied.
- [ ] `/bin/file` denied.
- [ ] `/etc/file` denied.
- [ ] `/dev/file` denied unless special UAC path exists.
- [ ] `/run/file` denied unless UAC-approved.
- [ ] `/home/<other-user>/file` denied.

Service tests:

- [ ] Own `/state/<service>/file` allowed.
- [ ] Arbitrary `/file` denied.
- [ ] `/home/<user>/file` denied unless explicitly authorized.
- [ ] `/services/file` denied.
- [ ] Another service state dir denied.

Capability broker tests:

- [ ] Trusted service scoped cap allowed.
- [ ] Normal user broker request denied.
- [ ] Cap inside scope allowed.
- [ ] Cap outside scope denied.
- [ ] Cap with wrong subject denied.
- [ ] Cap to protected path broadly denied.

Path normalization tests:

- [ ] `/tmp/../etc/x` denied.
- [ ] `/home/self/../../etc/x` denied.
- [ ] `/state/svc/../../../bin/x` denied.
- [ ] `/run/../services/x` denied.

Error tests:

- [ ] Immutable root denial returns `EROFS`.
- [ ] Other-user home denial returns `EACCES`.
- [ ] Missing/invalid cap returns `EPERM`.
- [ ] Protected path denial returns documented error.

---

## 12. Manual verification

Run after implementation.

Read/list:

- [ ] `ls /` passes.
- [ ] `ls /bin` passes if readable.
- [ ] `ls /etc` passes if readable.
- [ ] `ls /tmp` passes.
- [ ] `ls /home/<current-user>` passes.

Touch/write:

- [ ] `touch /x` denied.
- [ ] `touch /tmp/x` allowed.
- [ ] `touch /home/<current-user>/x` allowed.
- [ ] `touch /home/<other-user>/x` denied.
- [ ] `touch /bin/x` denied.
- [ ] `touch /sbin/x` denied.
- [ ] `touch /etc/x` denied.
- [ ] `touch /kernel/x` denied.
- [ ] `touch /boot/x` denied.
- [ ] `touch /services/x` denied.
- [ ] `touch /run/x` UAC-gated, denied, or allowed exactly according to documented policy.

Service:

- [ ] Service writes own state dir.
- [ ] Service cannot write arbitrary root.
- [ ] Service cannot write another service state dir.
- [ ] Service cannot write protected immutable path.

Capability:

- [ ] Scoped cap allows inside path.
- [ ] Same scoped cap denies outside path.
- [ ] Cap subject mismatch denied.
- [ ] Normal user broker request denied.

---

## 13. Regression tests

Use documented commands if present.

Try only commands that exist:

```sh
./tools/test.sh phase4
./tools/test.sh phase5
./tools/test.sh sunlight-utils
./tools/test.sh sunlight-fs
./tools/test.sh sunlight-uac
cargo test
```

Regression checklist:

- [ ] List actual test commands found.
- [ ] Run closest relevant tests.
- [ ] Record pass/fail.
- [ ] If a test cannot run, record exact reason.
- [ ] Do not claim verification for tests that were not run.

---

## 14. Documentation

Create or update:

```text
docs/FILESYSTEM_SECURITY.md
```

Required sections:

- [ ] Immutable root.
- [ ] User writable paths.
- [ ] Runtime UAC-gated paths.
- [ ] Protected paths.
- [ ] Service state directories.
- [ ] UAC role.
- [ ] Capability broker role.
- [ ] Error semantics.
- [ ] Examples.

Required policy documentation:

```text
/tmp                         writable by normal users
/home/<user>                 writable by that user
/run                         UAC-gated if supported
/state/<service>             writable by owning service
/bin, /sbin, /etc, /boot     protected immutable paths
/services, /kernel           protected immutable paths
/proc, /sys                  protected virtual/system paths
```

Required error documentation:

```text
EROFS   immutable/read-only region
EACCES  actor lacks access to writable region
EPERM   operation needs elevated permission or valid capability
```

---

## 15. Final report checklist

Final paragraph must answer:

- [ ] Was root read-only behavior intentional, unintentional, or partially implemented?
- [ ] What was preserved?
- [ ] What missing path was completed?
- [ ] What authorizes writes now?
- [ ] What verification passed?

Audit findings:

- [ ] Root read-only source.
- [ ] Existing UAC/run-as integration.
- [ ] Existing broker yes/no.
- [ ] Missing piece.

Policy implemented:

- [ ] `/tmp`.
- [ ] `/home/<user>`.
- [ ] `/run` and runtime paths.
- [ ] Protected root paths.
- [ ] Service state paths.
- [ ] Capability override.

Changed files:

- [ ] Every changed file listed.
- [ ] Reason for each file listed.

Key logs:

- [ ] Filesystem policy decision log yes/no.
- [ ] UAC decision log yes/no.
- [ ] Broker mint/check log yes/no.

Verification:

- [ ] `ls /`.
- [ ] `ls /bin`.
- [ ] `touch /x`.
- [ ] `touch /tmp/x`.
- [ ] `touch /home/<user>/x`.
- [ ] `touch /home/<other-user>/x`.
- [ ] `touch /bin/x`.
- [ ] `touch /etc/x`.
- [ ] `touch /run/x`.
- [ ] Service state write.
- [ ] Scoped capability inside path.
- [ ] Scoped capability outside path.
- [ ] Regression tests.

If incomplete:

- [ ] State what could not be completed.
- [ ] State exact missing piece.
- [ ] State what was implemented safely.
- [ ] State what remains unimplemented.
- [ ] Give the next narrow bite.

---

## Final answer template for the coding agent

```text
Root filesystem read-only behavior was <intentional/unintentional/partially implemented>. I preserved immutable root and completed <specific missing path>. Writes are now authorized through <policy function/UAC/capability broker>. Verification: <summary>.
```

```text
Audit findings:
- Root read-only source: ...
- Existing UAC/run-as integration: ...
- Existing broker: yes/no
- Missing piece: ...

Policy implemented:
- /tmp: ...
- /home/<user>: ...
- /run and runtime paths: ...
- protected root paths: ...
- service state paths: ...
- capability override: ...

Changed files:
- path/to/file: reason
- path/to/file: reason

Key logs:
- FS policy decision: yes/no
- UAC decision: yes/no
- broker mint/check: yes/no

Verification:
- ls /: pass/fail
- ls /bin: pass/fail
- touch /x: denied as expected/pass/fail
- touch /tmp/x: pass/fail
- touch /home/<user>/x: pass/fail
- touch /home/<other-user>/x: denied as expected/pass/fail
- touch /bin/x: denied as expected/pass/fail
- touch /etc/x: denied as expected/pass/fail
- touch /run/x: UAC-gated/pass/fail/not implemented
- service state write: pass/fail
- scoped capability inside path: pass/fail
- scoped capability outside path: denied as expected/pass/fail
- regression tests: ...
```

If not fully implemented:

```text
Could not complete broker integration in this bite.

Audit shows the exact missing piece is:
- ...

Implemented safely:
- ...

Not implemented:
- ...

Next narrow bite:
- ...
```
