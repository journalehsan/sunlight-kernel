# SunlightOS — Bite: Fix TLS startup hang in KV_GET_SHM("tls/ca/index")

## Model
Use careful debugging mode. Work incrementally. Do not optimize or refactor and update file in docs/Bite_Fix_TLS_Startup.md with new summary and informations.

## Hard constraints

- Do not ask for confirmation.
- Do not refactor unrelated code.
- Do not change the IpcMsg ABI globally.
- Do not change the capability/security model.
- Do not hardcode test success.
- Do not bypass sunlight-kv.
- Do not fake TLS trust-store success.
- Add diagnostics first, reproduce second, identify root cause third, fix fourth.
- If root cause is not clear after diagnostics, stop and report logs instead of guessing.

## Starting context

Read this first:
```text
docs/TLS_RUSTLS_PROGRESS.md
Important known facts from the progress doc:

Real rustls-based sunlight-tls now builds.
End-to-end QEMU boot is blocked.
The daemon hangs during startup root-store loading.
The last observed log is:
text
[SUNLIGHT-TLS] dbg: build_root_store start
[SUNLIGHT-TLS] dbg: seed get index...
The hang happens on:
rust
kv_get_shm("tls/ca/index")
Internally this calls:
rust
ipc_call(kv_cap, KV_GET_SHM)
That call currently never returns.
Current IPC transport only transmits:
words[0..3]
caps[0..1]
words[4..7] are dropped.
"tls/ca/index" should fit within the transmitted word limit, so simple key truncation alone does not explain this first hang.
Longer names such as "tls/ca/gts-root-r4" and hostnames such as "api.sampleapis.com" may still need proper repacking later, but do not fix those until the first hang is explained.
Goal
Find and fix the exact cause of the startup hang on:

text
sunlight-tls -> sunlight-kv
KV_GET_SHM("tls/ca/index")
This is not a general TLS refactor task.

Step 1 — Add boundary diagnostics first
Before changing behavior, add serial/debug logs at the exact IPC and KV boundaries.

In sunlight-tls
Log immediately before and immediately after every KV call used during TLS trust loading.

At minimum log around:

rust
kv_get_shm("tls/ca/index")
Required logs:

text
[SUNLIGHT-TLS] kv_get_shm begin key=tls/ca/index
[SUNLIGHT-TLS] kv_get_shm sending op=KV_GET_SHM key_len=<n> ...
[SUNLIGHT-TLS] kv_get_shm returned key=tls/ca/index status=<...> len=<...>
[SUNLIGHT-TLS] kv_get_shm error key=tls/ca/index err=<...>
Also log:

which capability is used for kv
whether the key is encoded inline or through shm
exact word fields used for the request
exact cap slots used for the request
whether ipc_call is entered
whether ipc_call returns
Example:

text
[SUNLIGHT-TLS] ipc_call enter target=kv op=KV_GET_SHM words=[...] caps=[...]
[SUNLIGHT-TLS] ipc_call return status=<...> words=[...] caps=[...]
In sunlight-kv
Add logs at server receive/dispatch boundaries.

Required logs:

text
[SUNLIGHT-KV] server loop waiting
[SUNLIGHT-KV] received msg op=<...> words=[...] caps=[...]
[SUNLIGHT-KV] dispatch KV_GET_SHM
[SUNLIGHT-KV] KV_GET_SHM decoded key=<...> key_len=<...>
[SUNLIGHT-KV] KV_GET_SHM found=<true/false> value_len=<...>
[SUNLIGHT-KV] KV_GET_SHM reply begin status=<...> len=<...>
[SUNLIGHT-KV] KV_GET_SHM reply sent
Also log failure branches:

text
[SUNLIGHT-KV] KV_GET_SHM decode failed err=<...>
[SUNLIGHT-KV] KV_GET_SHM missing shm cap
[SUNLIGHT-KV] KV_GET_SHM invalid cap direction
[SUNLIGHT-KV] KV_GET_SHM reply failed err=<...>
In IPC layer if needed
If TLS logs show ipc_call enter but KV logs never show receipt, add minimal IPC logs only around this call path:

text
[IPC] call enter caller=<pid> target=<pid/cap> op=KV_GET_SHM
[IPC] enqueue/send target=<...>
[IPC] caller blocked waiting reply
[IPC] server received msg
[IPC] reply send
[IPC] caller unblocked
[IPC] call return
Do not spam all IPC traffic unless needed. Keep logs targeted to KV_GET_SHM or caller/server pids if possible.

Step 2 — Reproduce and capture serial log
Run the documented QEMU/end-to-end reproduction from docs/TLS_RUSTLS_PROGRESS.md.

If a test exists, run:

bash
./tools/test.sh phase5.x.4
Capture the full serial log from boot until the hang or success.

Do not proceed to fixing until the logs answer this question:

text
Where exactly does control stop?
One of:

sunlight-tls never enters ipc_call
sunlight-tls enters ipc_call, but IPC never delivers to sunlight-kv
sunlight-kv receives request but fails/stalls decoding it
sunlight-kv handles request but fails/stalls replying
reply is sent but sunlight-tls is not unblocked
ipc_call returns but TLS code mishandles response and stalls later
Step 3 — Identify root cause before fixing
Before changing logic, write a one-sentence root cause in the work log or commit message.

Acceptable examples:

text
Root cause: KV_GET_SHM requests from sunlight-tls pass the shm capability in the wrong cap slot, so sunlight-kv blocks/fails while trying to access the response buffer.
text
Root cause: sunlight-kv receives KV_GET_SHM and sends a reply, but the IPC reply path does not unblock the original synchronous caller for this message shape.
text
Root cause: sunlight-tls encodes KV_GET_SHM using words beyond the 4-word transmitted ABI, so sunlight-kv decodes an invalid request and never replies.
Do not write vague causes like:

text
Root cause: TLS was broken.
Root cause: KV was buggy.
Root cause: IPC issue.
Step 4 — Fix only that root cause
Fix only the root cause proven by logs.

Likely fix areas:

If request reaches KV with wrong/truncated fields
Repack KV_GET_SHM / KV_PUT_SHM control messages so all required metadata fits into:

text
words[0..3]
caps[0..1]
Do not rely on words[4..7].

If key encoding is the issue
Ensure "tls/ca/index" is encoded and decoded consistently.

For this first bug, do not over-engineer long-key handling unless required to pass this exact startup path.

If shm cap direction is wrong
Fix the capability transfer direction or cap slot usage only for the affected KV SHM operation.

Document expected cap layout, for example:

text
caps[0] = response shm / value buffer
caps[1] = optional extra cap or unused
If KV handler does not reply on miss
Ensure KV_GET_SHM("tls/ca/index") returns a valid “not found” response instead of hanging.

A missing key must be a normal result, not a deadlock.

Expected behavior:

text
found=false
len=0
status=NOT_FOUND or equivalent
reply sent
If IPC reply path fails
Fix the minimal synchronous call/reply path needed for this message.

Do not redesign IPC.

Step 5 — Verify
Run the TLS gate if available:

bash
./tools/test.sh phase5.x.4
Then run regressions:

bash
./tools/test.sh phase5.x.5
./tools/test.sh phase5.x.6
Also rerun the documented manual TLS reproduction from:

text
docs/TLS_RUSTLS_PROGRESS.md
Success criteria:

Boot no longer hangs at:
text
seed get index...
kv_get_shm("tls/ca/index") returns.
Missing key, if applicable, is handled cleanly.
Trust seeding proceeds or root store building reaches the next expected step.
No regression in sunlight-utils and sunlight-net-utils tests.
Step 6 — Update progress doc
Append a dated entry to:

text
docs/TLS_RUSTLS_PROGRESS.md
Include:

md
## YYYY-MM-DD — KV_GET_SHM startup hang investigation

### Root cause
...

### Fix
- File(s):
  - ...
- Summary:
  - ...

### Verification
- `./tools/test.sh phase5.x.4`: pass/fail/not available
- `./tools/test.sh phase5.x.5`: pass/fail/not available
- `./tools/test.sh phase5.x.6`: pass/fail/not available
- Manual TLS reproduction: pass/fail
Final report format
Return one short paragraph plus a bullet list.

Paragraph:

text
Root cause was <one precise sentence>. I fixed it by <specific code change>. Verification: <tests/manual command results>.
Bullet list:

text
Changed files:
- path/to/file.rs — brief reason
- path/to/other.rs — brief reason

Key logs:
- TLS entered ipc_call: yes/no
- KV received request: yes/no
- KV replied: yes/no
- TLS unblocked: yes/no

Verification:
- phase5.x.4: ...
- phase5.x.5: ...
- phase5.x.6: ...
- manual reproduction: ...
If not fixed, report:

text
Could not prove root cause within this session. New logs show: ...
The next narrow step is: ...
text

---

## نسخه‌ی خیلی کوتاه‌تر برای وقتی مدل context کم دارد

اگر خواستی برای مدل‌هایی که سریع خسته می‌شوند یا context کم دارند، این نسخه‌ی فشرده‌تر را بده:

```md
# SunlightOS Bite — Find KV_GET_SHM hang blocking sunlight-tls

Read `docs/TLS_RUSTLS_PROGRESS.md`.

Known hang:
```text
[SUNLIGHT-TLS] dbg: build_root_store start
[SUNLIGHT-TLS] dbg: seed get index...
Exact stuck call:

rust
kv_get_shm("tls/ca/index")
which calls:

rust
ipc_call(kv_cap, KV_GET_SHM)
Task:

Add logs before changing logic:

in sunlight-tls before/after kv_get_shm
before/after ipc_call
in sunlight-kv receive/dispatch/reply for KV_GET_SHM
in IPC only if request does not reach KV
Reproduce and determine where it stops:

TLS before ipc_call
inside ipc_call before delivery
KV receive
KV decode
KV reply
IPC unblock
TLS response handling
Write one-sentence root cause before fixing.

Fix only that root cause.

Do not refactor.
Do not change global IpcMsg ABI.
Do not hardcode test success.
Do not bypass KV.
Do not fake TLS trust-store.
Verify:

bash
./tools/test.sh phase5.x.4
./tools/test.sh phase5.x.5
./tools/test.sh phase5.x.6
Append dated entry to docs/TLS_RUSTLS_PROGRESS.md.
Final report:

root cause
changed files
key logs proving it
verification result
If root cause is unclear, stop and report the new logs. Do not guess.
