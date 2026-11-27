# Adaptive TLS qualification closure

**PARTIAL.** Fresh 1 GiB BIOS boots passed bundle discovery with both
eager and adaptive TLS. The historical timeout remains unexplained.
The repaired overlap fixture passed on all four requested guest
configurations.

The two historical 1 GiB attempts reached session service registration,
installed the session process's VFS root routing policy and passed the
document workload. Neither emitted the eligible-bundle checkpoint. Both
continued scheduling the TTY sampler, and the documented free-frame low
water stayed above 137,000 frames. The gate checks a kernel checkpoint
emitted when sessiond reports an eligible bundle; service registration
alone cannot satisfy it. Source inspection shows discovery enumerates
`/Applications` via kernel ReadDir, opens and parses bundle manifests,
then checks executable paths via stat. The passing traces include routed
stat calls from sessiond and the Welcome bundle discovery marker. The
failed traces do not identify which earlier catalog step failed. There
is no demonstrated production defect or evidence that the fixture
misread the marker. The comparison used retained eager and adaptive
images with adaptive TTY and docserv and identical compressed resource
modules. Timing and kernel image differences limit causal attribution.

The overlap fixture formerly held the first response for 12 seconds,
past the observed client failure, and waited for its exit from the
other tab. TTY collects foreground status for the selected tab. An
attempt to wait for the peer HTTP request also failed: the first
blocking TLS receive held the service loop until the fixture returned
HTTP 504. The fixture now delays the first response by three seconds,
selects each tab to collect its foreground exit, checks one zero exit
per command and reads back two distinct expected payloads. It requires
two live TLS sessions and a return to zero. Boot deadlines and
readiness markers are unchanged.

| Guest | Boot, interactive TTY, document | Verified overlap and readback |
| --- | --- | --- |
| 1 GiB BIOS | PASS | PASS |
| 512 MiB BIOS | PASS | PASS |
| 512 MiB UEFI | PASS | PASS |
| 768 MiB BIOS | PASS | PASS |

Each overlap run had two successful TLS handshakes and two observed
client exits. Mapped TLS heap capacity stayed at 262,144 bytes during
the workload. Peak reply samples with both sessions active held about
14.9 KB of live payload and 17.4 KB of backing. The final close left
about 4.5 KB live and 5,776 bytes of backing. The kernel recorded
reaping 1,047 user frames and nine page-table frames per client. This
proves client frame reclamation; TLS-specific physical frame deltas
after close were not sampled.

**PASS** for the existing bounded startup allocation probe: the
8 MiB reservation was rejected, the subsequent 1 MiB reservation
grew capacity to 1,314,816 bytes, and dropping it returned mapped
capacity to 262,144 bytes. Verified HTTPS succeeded afterward.
**BLOCKED** for allocation failure during an active request: the
existing probe only injects a fallible reservation at startup.
Certificate validation code was untouched. The earlier adaptive
qualification recorded both issuer and hostname rejection; the
fresh overlap matrix tested successful verified transfers.

**NOT_RUN:** sustained 512 MiB operation, arbitrary OOM recovery,
non-compressed native boot qualification, and a fresh full PTY suite.
See the [results JSON](tls-adaptive-qualification-closure-results.json)
for run paths and per-guest measurements.
