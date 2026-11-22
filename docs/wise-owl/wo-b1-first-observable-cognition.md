# WO-B1 — First observable cognition

Wise Owl’s installation identity is a validated binary root and lineage in MemoryDB’s persistent store, implemented in `wiseowl-identity/src/{root,lineage,codec}.rs` and `wiseowl-memorydb/src/{identity,activation,identity_status}.rs`. It is distinct from a person's identity. MemoryDB stores long-term records; `wiseowl-memory/src/service.rs` serves bounded working memory and `wiseowl-index/src/ingest.rs` ingests document content. Indexed text is not an identity or policy source.

The Brain’s typed, UID-scoped preferences and welcome memory use `wiseowl-brain/src/{mtm,kv_client}.rs` and sunlight-kv. The greeting pipeline already composes templates from grounded context, while the planner/coordinator handle action intents. The missing path is a user-supplied comparison that reads saved preferences and explains the outcome. The shortest path is a bounded advisory command in the existing native `wiseowl-brainctl`, using the existing KV reader and MemoryDB status adapter. It needs no new RPC, storage, provider, or action executor.

`wiseowl-memorydb/src/backup.rs` can capture typed Brain KV entries via a trusted coordinator; its package is passive. Inspecting a package does not imply that restored preferences become active. This must be evaluated separately from package validation.

## Request and decision

Use `wiseowl-brainctl observe identity` to read the validated installation fingerprint. The response explicitly says personal identity is unknown; it does not infer a person’s name from a UID, request, or document.

Use `wiseowl-brainctl observe choose OPTION OPTION [no-machine]` to compare two distinct summaries. `brief` includes neither optional section, `machine` includes only machine details, `index` includes only index status, and `full` includes both. `no-machine` rejects candidates with machine details before scoring. Each known saved setting contributes one point if the option matches it. Missing, unreadable, or malformed settings contribute no points; no runtime defaults are substituted. Equal scores produce a tie. The result is advice rendered from a fixed template. It does not produce the summary, execute actions, write state, or request a model.

The native entry point is `wiseowl-brain/src/bin_parts/wiseowl-brainctl-native-body.rs` (`cmd_observe`). It derives the preference UID from the kernel and reads `sms` and `sis` through `kv_client.rs` (`NativeKvStore`) using `mtm.rs` keys and decoders. `adapters.rs` (`WiseOwlStatusContextSource`) reads MemoryDB’s native identity status. `observable.rs` renders the verified status and ranks the options. The command does not pass untrusted text to the index, preference writer, or identity writer.

The existing trust boundary still has gaps outside this command. The legacy Brain preference setter accepts a request-supplied UID and checks only for a nonzero sender badge. The KV daemon applies per-key ACLs, but `wb1:` is classified as a general namespace; it has no UID-aware creation rule. The new command adds neither a write path nor a caller-selected UID. Full protection against other IPC clients modifying or precreating preference records is unverified. Direct reads also depend on existing key ACLs; a non-root caller may not be able to read keys created by the root Brain service.

## Validation

Host: `cargo test -p wiseowl-brain -p wiseowl-memorydb -p wiseowl-identity -p sunlight-kv --target x86_64-unknown-linux-gnu` — 321 passed. The new tests cover validated, unknown, and invalid identity; preference changes; conflicting preferences and ties in both input orders; hard constraints, including no eligible candidates; and missing, malformed, and other-user data. Existing MemoryDB tests cover the typed backup component and passive package validation. An active backup/restore round trip could not be run: there is no restore or native coordinated backup implementation to reuse (see [Phase D](../wiseowl/identity-phase-d.md)).

Native: `bash tools/build_wiseowl_brain.sh` built both native std binaries. The kernel smoke sequence is `build_wo_b1_sequence` in `kernel/src/main.rs`, selected with `SUNLIGHT_INJECT_PHASE=wiseowl-wo-b1` and the `key_inject` feature. For a fresh durable fixture, build with `SUNLIGHT_TRUST_LEGACY_STATE=1`, create a disk with `tools/state-disk.sh`, and attach it as legacy virtio-blk. Build the ISO via `tools/make_hybrid_iso.sh`. Keep the VM isolated with `-nic none`. The sequence selects the root test login, reads identity, compares summaries before and after setting preferences, applies the hard constraint, and attempts to restart sunlight-kv before a final comparison.

The initial native runs booted with the test volume mapped at `/boot`, leaving MemoryDB without durable `/state`. Actual guest stdout captured in `target/wo-b1/uefi-serial.log`:

```text
$ wiseowl-brainctl observe identity
Wise Owl installation identity unavailable. Personal identity unknown.
$ wiseowl-brainctl observe choose machine brief
Tie: machine and brief (0 matches each). Saved: machine=unknown, index=unknown.
$ wiseowl-brainctl observe choose machine brief no-machine
Choose brief; machine is ineligible. Saved: machine=unknown, index=unknown. Machine details forbidden.
```

This verified the live shell path, missing-state handling, and constraint enforcement without a NIC. It did not verify live saved preferences, identity continuity, or preference replay: the Brain endpoint was unavailable and the restart command returned an error.

The trusted fixture mounted at `/state`. MemoryDB loaded and reported ready identity, and the CLI consumed that status. The guest changed its decision after the existing preference setter updated sunlight-kv. The storage manager logged successful appends. These are write and live-read observations, not restart or reboot evidence. The restart command was refused at the kernel nameserver lookup for `sunlightd`: the `sunlightctl` caller had capability mask `0x40000` and exited with code 1. The repeated result came from the still-running KV service; the gate did not bypass that authority check.

On the final two-vCPU TCG build with no NIC, `target/wo-b1/final-serial.log` recorded these actual responses:

```text
$ wiseowl-brainctl observe identity
Validated Wise Owl installation CEB98E8D. Personal identity unknown.
$ wiseowl-brainctl observe choose machine brief
Choose machine (2 vs 1 matches). Saved: machine=show, index=hide.
$ wiseowl-brainctl observe choose machine brief no-machine
Choose brief; machine is ineligible. Saved: machine=show, index=hide. Machine details forbidden.
$ wiseowl-brainctl preferences set show-machine-summary false
$ wiseowl-brainctl observe choose machine brief
Choose brief (2 vs 1 matches). Saved: machine=hide, index=hide.
```

Status: **PARTIAL**. The native shell demonstration and live preference-dependent choice passed, as did the final host run. An actual preference replay after service restart was blocked by service-control authorization. Active backup/restore is unavailable, and the pre-existing preference write authorization gaps remain open.
