# Preserved development attempts

1. Baseline storage tests completed (68 block, 30 VirtIO). The wrapper then
   failed recording a path relative to SFS-5B.5 rather than SFS-5B.6. Raw
   baseline-storage.log is retained; the wrapper's OUT binding was corrected.
2. The v1 real-backend fixture delayed before backend admission. During drain,
   QEMU parked that coroutine outside its in-flight counter. The device
   callback was not terminal at drain return; the assertion aborted before
   mapping release. `backend-v1-failed.c`, `device-v1.c`, `qemu-v1-failed`
   and every raw invocation/log are preserved. No guest allocation was freed.
3. v2 moves the hold past `blk_wait_while_drained` and `bdrv_inc_in_flight`.
   Each invocation records its executable SHA256. The first run series can
   contain both binary revisions because rebuilding occurred while the
   supervisor continued; qualification must use the invocation hash, not
   directory or case name alone.
4. A documentation patch missed its expected context and made no changes;
   the corrected patch was applied separately.
5. Filesystem ENOSPC interrupted earlier kernel builds, scoped checks, hosted
   evidence packaging and one native startup. Passing test output preceding
   an interrupted wrapper is not a passing wrapper. The debug profile raw
   schedules remain under `raw/regression-20261115T143024.754199Z/`; release
   raw schedules were rescued in `raw/rescued-hosted-release/raw-evidence.tar.gz`.
   No evidence or unrelated cache was removed. After the user freed Docker
   image space, free space was approximately 143 GiB. The focused checks and
   both kernel builds were freshly rerun successfully.
6. Running a copied emulator outside its build directory failed to locate
   `bios-256k.bin`. This startup failure is retained and supplies no scenario
   coverage. Final launches use the original build path; a snapshot and
   invocation hashes bind the executable without changing firmware lookup.
7. Rebuilding the emulator during an exploratory native series caused one
   executable PermissionError while linking. Exploratory runs are separated
   by their executable and guest hashes. No emulator rebuild overlapped the
   final 36-run series or final indefinitely held negative case.
8. Final guest build `raw/guest-20261115T145335.368366Z/` succeeded after disk
   recovery. Final native series `raw/native-20261115T145612.275450Z/` contains
   30 real-backend cases and six old-worker controls; the separately bounded
   indefinitely held request is in `raw/native-20261115T145612.276073Z/`.
   Final focused checks are in `raw/focused-20261115T145612.278735Z/`.
9. A later combined audit/report documentation patch also missed its context
   and was rejected atomically. Its report and audit changes were reapplied
   separately. No executable/native source was changed by those patches.
10. Final review found an evidence-parser false negative: unanchored `ticks=`
    and `work=` matched `start-ticks=` and `start-work=`. The original parsed
    SMP/matrix results and parser are preserved under
    `raw/smp-parser-prefix-failure/`. Whitespace-delimited field parsing fixes
    this; positive and missing-final-counter negative checks were added.
    Original guest logs and native execution are unchanged. Progress during
    backend drain specifically remains NOT_RUN, rather than inferred from
    the corrected counters that bracket reset ownership and terminal state.
