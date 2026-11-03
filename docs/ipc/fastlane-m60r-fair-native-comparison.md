# FastLane M6.0R fair native comparison

Status: **COMPLETE for the bounded experimental scope**. Experimental scope only. Production FastLane remains **DISABLED**. Historical M1/M4 attribution and original untraced SHM census ownership remain **UNRESOLVED**.

Durable receipts are under `.fastlane-evidence/m60r/`. The final decision, source identity, per-run metrics, rejection populations and artifact hashes are linked below. No commit, push, production enablement, scheduler change or IPC-limit change is part of this work.

## Source verification and preserved evidence

HEAD is `e5ed4849e57cb729252ba7ad0c25efcfe7fe5ae3`. `initial-source.json`, `initial.patch` and `initial-source/` preserve the inherited dirty tree, including unrelated changes. Canonical identity is SHA-256 of the sorted `sources()` map encoded with JSON `separators=(',', ':')`; this convention matches M6.0, rather than the earlier M5.4 whitespace convention.

The initial canonical map was `5c09e705fbc6c3f1d4aef27efea19bedfa5abb6d3f2622250a540e624bc78cd7`, exactly equal to the accepted M6.0 build map. All 16,938 entries in the existing M6.0 artifact index were verified without changing historical files. The nine accepted M6.0 baseline guests, additional NOT_QUALIFIED guest and four pilot attempts remain in their original groups and statuses. None was repeated.

Reuse references:

- `.fastlane-evidence/m60/results-summary.json`, `evidence-index.json` and `image-r3/build-20261103T204643.036126Z/validation-summary.json`.
- `.fastlane-evidence/m54r2/matrix-20261103T183322.818190Z/matrix.json`.
- `.fastlane-evidence/m54r2/safety-20261103T192044.990473Z/orchestration.json` and `final-readiness.json`.

`verify_reused.py` replays the retained R2 readiness predicates into a new M6.0R receipt. `source-safety-audit.json` compares 167 kernel, IPC, ring and Doorbell executable inputs against the accepted safety source; none changed. The seven M6.0R source additions/changes are the separate fair fixture, its opt-in dispatch and three host tools. The old M6.0 benchmark and parser remain intact. Full source-map changes invalidate *exact-map* reuse for the new fixture image, but do not invalidate the historical experiment or unchanged safety implementation. One corrected image is used for the new pilot and matrix. Build-only and failed-pilot images are separately preserved.

## Conventional IPC contract audit

The public `ipc/src/lib.rs::IpcMsg` reserves eight words and two capability slots. The actual x86_64 `raw_syscall_ipc` ABI transports **four u64 data words, exactly 32 bytes**, through r8/r9/r10/r12. rdi carries a separate label, rdx carries counts, rsi identifies the endpoint on entry and authenticated badge on receive, and r13/r14 carry two capability tokens. Kernel `ipc/message.rs::IPC_REG_WORDS` is four. Storage capacity is not wire capacity; words beyond the first four are not a legitimate larger-message path. No limit or syscall was changed.

For this workload the label carries operation and sequence, so framing consumes **zero of the 32 application bytes**. No capability is sent with a measured request or reply. A reply has a separate sequence label plus two u64 words containing the payload digest and exact next sequence. A protocol requiring an in-band data-word header would have correspondingly fewer usable application bytes.

`endpoint_create` creates an owned endpoint; `endpoint_bind` derives its send capability. `ipc_call_timeout` arms an absolute monotonic deadline, submits a request, resumes `WouldBlock` with yields and returns the matching terminal reply or a transport error. Re-submission resumes the pending call; it does not enqueue a second logical request. `ipc_recv_timeout` dequeues one request and installs server reply authority; `ipc_reply_result` completes that caller's pending request. A successful enqueue or Doorbell hint alone is not application completion.

Kernel `ipc/mod.rs` authenticates SEND rights and endpoint ownership with the broker. The kernel overwrites the request badge with the actual caller PID. The fixture verifies the child PID during endpoint handshake and verifies the producer badge on each consumer command and request. Reply completion uses PID, incarnation, endpoint and call generation, requires the exact live pending request and rejects a withdrawn or terminal target (`reply_identity.rs`, `deliver_reply_to_target`). Caller-supplied bytes and badges grant no identity authority. This authenticates endpoint participants; it does not cryptographically authenticate hostile shared content.

The kernel creates a fixed-value `IpcMsg` from registers, queues it by value, transfers it to the receiver and materializes reply state before returning registers to the caller. The inline consumer reconstructs a private 32-byte array from all four received words and compares every byte. These are logical copy boundaries; compiler-generated copy instructions and physical memory traffic were not counted. There is no separately allocated payload buffer in the register transport. Endpoint maps and `VecDeque` message/waiter metadata use the allocator. In particular, `reply_waiter_remove` replaces the waiter deque with a new deque: at one outstanding call, recurring waiter-buffer allocation can occur even after warm-up. Persistent endpoints do **not** establish allocation-free IPC. Allocation counts and peaks are unmeasured.

Each endpoint FIFO has capacity 64. `QueueFull` is an explicit failure; pending callers block/resume until completion or deadline. The fixture permits exactly one outstanding application request on all paths. It adds no application retries; its `retries=0` receipt refers to application retries, not the existing `WouldBlock` continuation loop. Ring Full/Empty, timeout, QueueFull, unexpected sequence or digest, and failed reply reject the guest.

There is an established **separate SHM protocol**: `shm_create`, `shm_map`/`shm_map_sized`, `shm_free`, and capability slots. For example, `notification_kv_put` already uses `KV_PUT_SHM2`, two SHM pages, word-sized lengths and two capabilities for keys/values up to 4096 bytes each; the UDP exchange protocol also transfers SHM. This is allocation/mapping/capability-transfer behavior with service-specific bounds and lifetimes, not a generic larger inline call. No new bulk protocol, fragmentation or SHM substitution is benchmarked. Conventional inline 64/256 bytes remains UNSUPPORTED.

## Frozen workload and equivalence

`workload-contract.json` specifies 16 warm-up requests and 128 measured requests per path, one producer and one consumer in distinct native processes, both restricted to CPU 0, one outstanding request, a persistent endpoint and (where applicable) data channel per configuration. Every application byte is `(sequence * 17 + offset * 3) % 251`; sequence ranges 0–143. The consumer checks the exact next sequence and all payload bytes, then replies with the digest and next count. The producer checks reply label, digest and count. Setup, warm-up, actor diagnostics, payload measurement, post-window reporting and teardown are distinct windows.

| Property | Conventional inline | Mediated FastLane | Native shared ring |
|---|---|---|---|
| Application data | Same 32 bytes | Same 32 bytes | Same 32 bytes |
| Request sequence | IPC label | IPC label and ring sequence/generation | IPC label and ring sequence/generation |
| Payload path | Four data registers and kernel queue | Authenticated word syscalls, unchanged hostile ring | Restricted mappings, unchanged hostile ring |
| Consumer copy | Four words into private 32-byte array | Bounded ring copy into private storage | Bounded ring copy into private storage |
| Completion | Checked two-word IPC reply | Ring release, then same checked IPC reply | Ring release, then same checked IPC reply |
| Notification | IPC dispatch | Authenticated Doorbell publish/observe plus IPC dispatch | Authenticated Doorbell publish/observe plus IPC dispatch |
| Outstanding requests | 1 | 1 | 1 |
| Backpressure policy | Reject errors; bounded call continuation | Reject Full/Empty/errors | Reject Full/Empty/errors |
| Actor CPU counters | Unavailable under channel selector | Selector explicitly refuses Incomplete=7 | Existing role-bound selector |

FastLane producers still initialize a private 256-byte staging array and clear all 32 words of the shared payload slot. Consumers copy the four payload words into private storage. Mediated accesses reauthenticate each word and include kernel admission/publication trace output inside timed operations. Shared-ring publication uses the existing raw memory accessor and bounded private-copy algorithm. No new unsafe block, lease semantics or authority is introduced.

The common application operation is **verified request plus acknowledgement**. Aggregate throughput is 128 acknowledged application requests divided by an independently qualified producer window. RTT covers serialization/publication, transport and dispatch, private verification/release, and checked IPC acknowledgement. Payload generation happens before each RTT start but inside the aggregate window. Local consume time starts *after* receive and excludes conventional transport; local publish/notify exists only for FastLane. Those stage metrics are not comparable send/receive syscall costs.

The table compares end-to-end application observations. The operations have different notification, copy, instrumentation and counter overheads, so **no direct transport speedup ratio**, maximum streaming rate, CPU utilization or bare-metal prediction is claimed. Fixed order is inline, mediated, shared. It is a bounded baseline, not an optimization or order-randomized study.

## Mediated CPU accounting diagnosis

M6.0's source branches start/stop accounting only when `path == 2`; no mediated actor CPU window was collected. `Scheduler::fastlane_measurement` authenticates actor identity, generation and live channel, then requires `row.native_ring[role]`. An otherwise valid mediated channel has no activated native-ring role and returns **Error::Incomplete = 7 before enabling the counters**. This is measurement-selector coverage, not evidence of defective scheduler accounting.

`historical-cpu-diagnosis.json` enumerates all **44 affected actor/configuration rows** from the retained completed pilot and ten baseline guests, including the NOT_QUALIFIED guest. For each row it preserves run/serial identity, topology, payload, actor PID/incarnation/VM, channel generation, observed timing CPU IDs, available timing samples and exact absence reason. There is no historical rejected charged-time sample to reinterpret: counters were never started. Charged time, dispatch counts and reconciliation are **unavailable**, not zero. Existing local consumer intervals cannot reconstruct a missing full actor window.

M6.0R explicitly attempts selector operations 26 and 27 once per mediated actor, outside payload timing, and requires both to return 7. `[FL-M60R-CPU]` records the authenticated channel generation, actor PID, start/stop return codes and unavailable charge/switch/reconciliation fields. Binding receipts supply incarnation and VM. `actor-producer` and `actor-consumer` records preserve full raw clock endpoints around the attempted actor measurement interval. These qualify same-CPU **wall duration only**; they cannot qualify execution time. Both endpoint actors are checked independently.

Migration, backward or cross-CPU timestamps, scheduler discontinuity, wrong actor, counter saturation, syscall boundary mismatch and failed reconciliation are not demonstrated causes of the historical mediated omission. The counters needed to test those alternatives do not exist for that path. No scheduler change or selector broadening is justified by these observations. Mediated charged time, dispatch/switch counts and reconciliation therefore remain **NOT_QUALIFIED / UNAVAILABLE with an established reason**. Inline has no FastLane channel selector and is separately labeled `no-channel-selector`.

Shared-ring kernel actor records retain charged runtime, syscall/notification/switch/wake counters, overflow flags, runtime validity, same-CPU wall reconciliation and identity. Their actor windows include control and reporting overhead outside payload measurement. The new extra outer wall intervals are distinct from the kernel's internal accounting interval. Neither is used to estimate utilization, blocked time or user/kernel splits.

## Timing qualification and execution limits

The inherited M5.4 clock uses PIT-referenced TSC scale, six calibration windows with the first discarded and median/spread from five retained windows. Each observation preserves CPU, epoch, nanoseconds, frequency, ppm, flags and spread at both endpoints. The independent host parser requires CPU 0 at both ends, matching nonzero epochs and calibration metadata, positive forward duration, PIT and stability flags, and uncertainty within 20,000 ppm. It rejects cross-CPU/nonresident, backward, stale-epoch, missing and changed-calibration samples. No coarse clock or cross-CPU subtraction replaces a rejection.

Per-pair scale uncertainty is `ceil(duration_ns * ppm / 1e6) + 1 ns`. It does not bound PIT absolute error, host scheduling, QEMU pauses, syscall/stamp jitter or overall execution noise. RTT/local percentiles use nearest-rank statistics only with at least 100 qualified observations; p99 from 128 samples is descriptive. All accepted and rejected samples remain attached to their run. Operation latency and aggregate throughput are reported separately. Host monotonic time controls run deadlines only.

One accepted 2-CPU pilot and parser/retirement checks gate expansion. Stage 2 targets three accepted independent guests at each of 2/4/12 CPUs, at most 12 attempts, 300 seconds per guest, 3600 seconds per driver invocation, minimum 12 GiB free disk, and one QEMU guest at a time. QEMU TCG uses `thread=multi`, `-cpu max`, 2 GiB RAM, no network/display, virtio RNG, and unpinned host threads. Host scheduling is uncontrolled. The existing M6.0 host metadata is reused; the matrix does not measure scaling of parallel application work because both actors remain on CPU 0.

## Safety and limits

Every accepted run must have exact payload/acknowledgement counts, authenticated binding identities, no duplicate terminal receipt, zero scoped inventory, six unique backing lifetimes, twelve role views, exactly one physical free per backing and complete request/remote-ACK/unmap/free ordering. Consumer stop/reap and producer endpoint destruction use the established APIs. Kernel cleanup receipts independently record zero owned endpoints, pending calls, reply targets, deferred targets and pending commits; final reap records preserve the IPC cleanup observation. Global SHM/page-table changes remain concurrent diagnostics, not fixture leak attribution.

The unchanged accepted R2 native remote positive witness and stale/foreign/duplicate/adversarial controls are reused. The new pilot's 18 parser fault models test missing/duplicate/premature frees, wrong TLB request, wrong actor, clock rejection, missing metadata, fabricated CPU charge, wrong selector code and outstanding-bound mismatch. These are parser controls, not newly injected native failures. Focused IPC/ring host tests and default-feature production exclusion are separate current-source checks. No duplicate full safety matrix is needed because no security-critical implementation changed.

Hostile shared memory still supplies memory safety and private scalar snapshots; coherent malicious publication and application-content authenticity remain outside the established contract. Heap peaks, physical copy counts, streaming maxima, multi-outstanding behavior, cross-CPU latency, CPU utilization, user/kernel split and blocked time remain unsupported or NOT_QUALIFIED. Historical incident classifications remain UNRESOLVED.

## Results and validation receipts



Final experimental canonical source identity: `0555baac4b8ba76c776f6d29efcb9aa405a2ced12c9e291bbdc4b8feb86996fc`.
The measured source map contains 1394 executable-input files. `final-source.json` and `m60r-only.patch` retain the exact map and the changes relative to the inherited M6.0 tree. The build/run manifests bind that same map before and after execution.

| Artifact | SHA-256 |
|---|---|
| Experimental ELF | `37600eb65a07a44938241857374daa99695d0dc5e1970e70475e701d6df74169` |
| Experimental ISO | `8d1e9b1c4c2c3d774a6731eb517064ff4f48122d623b85f1a0e154d30cfb43c4` |
| Current default-feature production ELF | `9227d7599384b05116f5a256ad03ed1b1915f6af869738741550df0d85c2e044` |
| Current default-feature production ISO | `89ebcd604b0c04fd1611b3a961373a16a371adf2d8ccb9074d3528bf984963ac` |

### Qualified three-way 32-byte results

Each row summarizes three independently accepted guests. Rates show median and full min–max range. RTT values are medians of each run's nearest-rank percentiles, not pooled percentiles. Setup and teardown are elapsed time; their CPU cost is unqualified. Per-run values, MAD, complete local-stage metrics and raw observation references are preserved in `results-summary.json` and each native manifest. All 27 configuration windows passed.

| CPUs | Path | B | Runs | Ack/s median [min–max] | MiB/s | RTT p50/p95/p99 µs | Setup/teardown ms | Max ppm | Rejected |
|---|---|---|---|---|---|---|---|---|---|
| 2 | Inline IPC | 32 | 3 | 11777.0 [11490.0–12423.4] | 0.359 | 67.5/86.1/254.2 | 4.80/6.91 | 872 | 0 |
| 2 | Mediated | 32 | 3 | 1015.8 [968.1–1027.4] | 0.031 | 873.3/1486.0/1759.9 | 13.71/8.91 | 872 | 0 |
| 2 | Shared ring | 32 | 3 | 7608.9 [7571.1–8179.9] | 0.232 | 108.0/148.7/306.4 | 9.94/9.73 | 872 | 0 |
| 4 | Inline IPC | 32 | 3 | 11184.1 [9589.0–11820.4] | 0.341 | 70.0/129.2/290.8 | 5.83/5.82 | 1363 | 0 |
| 4 | Mediated | 32 | 3 | 910.1 [905.0–948.6] | 0.028 | 993.1/1764.4/2026.5 | 16.35/14.03 | 1363 | 0 |
| 4 | Shared ring | 32 | 3 | 6917.0 [6232.5–7025.9] | 0.211 | 126.1/172.4/334.8 | 12.58/9.36 | 1363 | 0 |
| 12 | Inline IPC | 32 | 3 | 9789.1 [8841.2–11024.8] | 0.299 | 81.6/193.8/292.1 | 7.26/8.88 | 763 | 0 |
| 12 | Mediated | 32 | 3 | 869.8 [825.7–879.7] | 0.027 | 1067.7/1756.2/2119.5 | 21.02/12.25 | 763 | 0 |
| 12 | Shared ring | 32 | 3 | 6848.7 [6562.4–7147.2] | 0.209 | 132.6/200.3/353.7 | 14.87/11.28 | 763 | 0 |

The comparison completed 3,456 measured application requests, 3,888 including warm-up, with matching private-byte verification and acknowledgements. The separate accepted pilot completed another 384 measured requests. No direct speedup ratio is derived from the table.

### Preserved M6.0 64/256-byte FastLane group

These are the original accepted measurements, reproduced from the immutable M6.0 summary. They were not rerun or pooled with the common-payload group. Conventional inline IPC remains unsupported at these sizes.

| CPUs | Path | B | M6.0 accepted runs | Ack/s median [min–max] | RTT p50/p95/p99 µs |
|---|---|---|---|---|---|
| 2 | Mediated | 64 | 3 | 1013.9 [862.5–1038.4] | 899.9/1402.7/1770.4 |
| 2 | Shared ring | 64 | 3 | 7713.0 [7279.3–7770.4] | 115.3/132.8/326.2 |
| 2 | Mediated | 256 | 3 | 794.6 [774.5–920.8] | 1057.7/1583.5/4762.7 |
| 2 | Shared ring | 256 | 3 | 7237.1 [6388.3–7982.6] | 132.9/155.3/331.2 |
| 4 | Mediated | 64 | 3 | 916.2 [896.7–917.7] | 979.2/1798.3/2621.9 |
| 4 | Shared ring | 64 | 3 | 7737.2 [7470.0–7824.3] | 108.9/176.4/351.0 |
| 4 | Mediated | 256 | 3 | 856.9 [822.1–866.5] | 1098.9/1741.0/1955.4 |
| 4 | Shared ring | 256 | 3 | 6351.1 [6039.0–8082.9] | 132.0/266.1/395.9 |
| 12 | Mediated | 64 | 3 | 815.2 [781.5–831.8] | 1125.0/1850.7/2168.9 |
| 12 | Shared ring | 64 | 3 | 6495.5 [5815.5–7580.1] | 131.1/167.1/520.0 |
| 12 | Mediated | 256 | 3 | 672.8 [654.3–710.7] | 1328.6/2411.8/3222.2 |
| 12 | Shared ring | 256 | 3 | 6711.1 [6044.3–6926.6] | 128.6/157.4/484.3 |

| Qualification | M6.0 | M6.0R |
|---|---|---|
| Milestone decision | PARTIAL: no eligible inline comparison | COMPLETE for common-payload experimental scope |
| Application payload | 64/256 B, FastLane only | 32 B, all three paths |
| Accepted comparison guests | 9; 3 per topology | 9; 3 per topology, plus separate accepted pilot |
| Accepted comparison configuration windows | 36 | 27 |
| Measured application requests | 4,608 | 3,456 |
| Mediated actor CPU time | NOT_QUALIFIED; omitted by fixture | NOT_QUALIFIED; exact selector refusal demonstrated |
| Shared-ring actor CPU records | 36 | 18 in comparison; 2 more in pilot |
| Additional measurement rejection | One guest: CPU1-to-CPU0 setup pair | No timing rejections in completed comparison/pilot |
| Production FastLane | DISABLED | DISABLED |
| Historical causal attribution | UNRESOLVED | UNRESOLVED |

Payload sizes, fixture diagnostic overhead and guests differ between groups. Comparing their absolute numbers does not establish an optimization effect.

### Clock and actor accounting populations

| CPUs | Group | Qualified pairs | Rejected | Rejection rate | Max ppm | Largest per-pair scale uncertainty ns |
|---|---|---|---|---|---|---|
| 2 | baseline | 3117 | 0 | 0.0% | 872 | 110669 |
| 2 | pilot | 1039 | 0 | 0.0% | 673 | 83720 |
| 4 | baseline | 3117 | 0 | 0.0% | 1363 | 193766 |
| 12 | baseline | 3117 | 0 | 0.0% | 763 | 112139 |

There are 9,351 qualified comparison pairs and 1,039 qualified pilot pairs. Both failed pilot attempts produced no payload measurement population; their absent measurements are not counted as valid pairs. Native counter windows with unsupported selector coverage are also not counted as rejected clock pairs.

The comparison's 18 mediated actor records and the pilot's two records all have `start_code=7`, `stop_code=7`, `charged=UNAVAILABLE`, `switches=UNAVAILABLE` and `reconcile=UNAVAILABLE`. Every one has a separately qualified CPU0 wall interval and binding identity. See `cpu-accounting-diagnosis.json` for every raw pair and its scale uncertainty, and `historical-cpu-diagnosis.json` for all 44 historical affected rows. Charged runtime, dispatch/switch counts and reconciliation cannot be supplied for these actors by the existing selector.

For the existing shared-ring accounting contract, all 18 comparison actor records pass charge validity, zero discontinuity, zero migration, zero overflow/dropped counters, exact PID/incarnation/VM/generation identity, matching syscall/switch records and `charged_ns <= wall_ns` reconciliation. Their raw outer actor-wall intervals also qualify independently. The following are medians over three guests; these CPU windows include accounting controls/reporting and are distinct from payload windows.

| CPUs | Producer / consumer charged ms | Producer / consumer switch-ins | Producer / consumer syscalls |
|---|---|---|---|
| 2 | 7.564 / 8.238 | 128 / 131 | 1161 / 1165 |
| 4 | 8.661 / 9.253 | 129 / 129 | 1161 / 1165 |
| 12 | 8.470 / 9.912 | 128 / 129 | 1161 / 1165 |

No inferred utilization, wall-minus-charge blocked time or user/kernel time is published. Kernel accounting wall durations remain attached as diagnostic reconciliation inputs; they are not substituted for payload-window durations.

### Attempts, integrity and retirement

M6.0R retained **12 native attempts: 10 PASS and 2 FAIL**. PASS includes the accepted pilot and nine comparison guests. The first failure was the sandbox's refusal to bind the QMP socket before boot. The second failed inside the bounded fixture initialization: residency checked qualified stamps before clock recalibration. The correction only moves that check after calibration. A speculative stale-image hypothesis was checked and rejected: the loaded program headers matched the current probe, and panic line 441 identified `resident()`. The targeted kernel-cache rebuild performed during that investigation remains archived as an unrun build; it supplied no measurement. `pilot-image-diagnosis.json`, all three instrumented build manifests, original failed pilot serials and their original verdicts remain preserved. No aborted attempt or discarded measurement exists in this execution.

The nine comparison guests completed **54 backing lifetimes, 108 restricted role views and 54 exactly-once physical frees**; the separate pilot adds six lifetimes, twelve views and six frees. All required TLB target sets and ACK ordering passed, and scoped objects/views/frames returned to zero at every teardown census. The independent run validator also passed each actor's kernel cleanup and reap observations. Remote-positive lifetime safety and the broader adversarial matrix remain source-matched R2 evidence; no new forced malicious publication or premature-free recovery claim is made.

Each accepted run has its own receipt under `receipts/`, with native manifest and serial hashes, source/image hashes, topology, per-configuration metrics, CPU counters/refusals, retirement checks and exact kernel cleanup checks. `results-summary.json::attempts` names every original attempt and receipt, including failures. `matrix-driver-20261103T212357.310921Z.json` records the nine-attempt execution, its 370.96-second wall duration and resource bounds.

### Current-source validation and decision

- `decision.json`: COMPLETE, all experimental acceptance predicates pass.
- `focused-validation.json`: 81 IPC unit tests, 39 ring unit tests, five ring doc tests, seven M6.0R unit controls, five retained M6.0 unit controls, 18 retained-pilot parser fault models, native fixture build, rustfmt and diff checks PASS.
- `reused-safety-verification-20261103T211742.885000Z.json`: retained R2 safety predicates COMPLETE, no unmet criteria; `source-safety-audit.json` verifies unchanged security implementation.
- `production-audit.json`: current-source default-feature production build and embedded-service provenance PASS; no FastLane/measurement markers or measurement syscall symbol. Historical R2 debug/release exclusion remains retained. Production FastLane stays DISABLED.
- `m60-preservation-verification.json`: 16,938 historical artifacts match their existing index. `final-verification.json` and `evidence-index.json` bind this final report and new durable receipts.

The environmental and fixture failures did not consume comparison repetitions. No native comparison replacement, broad security sweep, unrelated subsystem test, production integration, tick-frequency change, commit or push was performed.

### Reproduction and smallest next milestone

The accepted image is `.fastlane-evidence/m60r/image-r3/build-20261103T212113.171425Z/validation-summary.json`. With the source map unchanged, independently execute a new pilot with:

```sh
python3 tools/fastlane_m60r_validate.py --section native --measurement \
  --cpus 2 --timeout 300 \
  --image-manifest .fastlane-evidence/m60r/image-r3/build-20261103T212113.171425Z/validation-summary.json \
  --evidence-root .fastlane-evidence/m60r/new-pilot
python3 .fastlane-evidence/m60r/validate_run.py <new-native>/validation-summary.json
```

A local QMP socket must be permitted by the execution environment. The bounded matrix driver resumes missing accepted topologies only:

```sh
python3 .fastlane-evidence/m60r/run_matrix.py
```

This completed matrix needs no resume; with its existing receipts the command skips all fulfilled repetitions. If a future invocation stops, preserve its failing run, diagnose it and reuse the same image only while the exact map remains matched. A clean rebuild uses the existing `--section build --measurement` driver into a fresh evidence root. Never overwrite historical directories.

The smallest M6.1 milestone is to specify and qualify **debug-only actor accounting for the existing mediated role**, with authenticated identity, raw same-CPU start/stop stamps, explicit rejection reasons, charged-time reconciliation and matched observation overhead. A tightly scoped selector/fixture change should validate that coverage on 2/4/12 CPUs before any performance optimization; equivalent inline accounting can follow under its own endpoint-authenticated contract. Do not activate native-ring authority merely to obtain counters. The current data supports measurement work, not production integration or broad optimization.
