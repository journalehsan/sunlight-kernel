# M5.3R2: IPC fixture qualification and membership boundary

November 9, 2026. Overall acceptance: **BLOCKED**. Production FastLane,
hostile-process zero-copy and production restricted admission remain disabled.
The 512 MiB process and 1 GiB supplied-session ceilings are unchanged.

Evidence is retained in
`.fastlane-evidence/m53r2-20261109T153723Z/REPORT.md` and its
`validation-summary.json`. Original failures, interrupted attempts, source maps,
independent comparison builds and raw guest output remain separate artifacts.
Consult those results for the final execution matrix; older M5.3R passes do not
validate this source.

## Wider IPC identity contract

The boot security fixture manually installs `IpcCallId` values. M5.3 allocates
nonzero process incarnations during process construction, including for these
unscheduled fixture records. The fixture still supplied incarnation zero.
Production reply, deferred completion and endpoint-close paths correctly reject
calls whose incarnation differs from the current caller. The fixture must use
the actual caller incarnation; weakening production identity checks would be
incorrect.

The repair changes only that fixture identity and adds two bounded debug
receipts with actor incarnation/VM and the twelve component assertions. It does
not change production IPC authority or continuation stack selection. The new
`fastlane_m53r2_assess.py` rejects missing or duplicate success markers, any
`UNEXPECTED`, failed component assertions and mismatched call identity. Its
negative controls preserve the distinction between a passing boot-method probe
and scheduled userspace lifecycle evidence. `fastlane_m53r2_validate.py` checks
source/image/embedded-service hashes before bounded replay. Controlled old-source
comparison and its limitations are recorded in the evidence report.

Run the new negative controls with:

```sh
python3 -m unittest discover -s tools -p test_fastlane_m53r2.py
```

For the strict boot IPC replay, build a source-matched mapping image using
`tools/fastlane_mapping_validate.py --section build --census-trace`, then pass
its successful manifest to `tools/fastlane_m53r2_validate.py --image-manifest`
with `--evidence-root`, `--cpus 1 2 4 12 --repetitions 2 --timeout 180`.
This runner stops at the boot security checkpoint. It does not execute the M1/M2
userspace programs; their separate mapping runner and strict assessor are still
required. The 1-CPU M1 limitation does not apply to this boot-method IPC replay.

## Authenticated session membership is incomplete

The authenticated Account `SessionRef { id, epoch }` is created using the trusted
session-service registration path, one-use Auth proof and exact recipient/service
identity. That registry tracks Account authority, not every process belonging to
a login. `SpawnRequest` does not carry an authorized Account session attachment.
The login shell, optional application and intermediary spawn-service paths
therefore cannot establish complete login membership before first execution.

Foreground's broker-incarnation/terminal-ID/generation tuple authenticates a PTY
relationship. Creating a terminal or binding its foreground owner does not
consume a login Auth proof. Separate terminals in one login can receive separate
quota keys. Existing children/workers may also precede binding. A PTY budget is
not evidence of the claimed authenticated login-wide 1 GiB budget.

Excluded from a complete login aggregate: unbound or detached processes,
general desktop and service-launched applications without an authorized handoff,
pre-binding workers, and shared services acting for multiple users. Ordinary
anonymous, file and page-table allocations are outside the SHM backing quota.
Experimental restricted backing retains its immutable authenticated channel
owner/session. No global implicit session or UID-based ownership is introduced.

The current receipt remains private and move-only. Charges stay with the original
owner/session while peers retain backing, including across originator exit/exec;
physical reclamation precedes receipt destruction. This source contract and host
controls do not prove all scheduled kill/crash/exec races or native foreign and
duplicate receipt consumption. Those acceptance requirements remain open.

## Required next milestone

Implement an operation-specific session-authority-approved launch attachment,
installed atomically before process publication and inherited by authorized
children/workers. Define request-owner-qualified allocation for shared services.
Fence new reservations and attachments when a session retires while retaining
existing charges through last unmap, remote TLB acknowledgement and PMM release.
Do not migrate old receipts when membership changes.

Then add coordinated scheduled owner/peer exit, kill, genuine fault and exec
probes, both reachable terminal orders with mapped backing, independent-session
budget probes and actual PID/incarnation reuse. Retain boot-method IPC races,
restricted userspace authorization and full physical reclamation as separate
gates. All must pass on verified final-source images before acceptance changes.
