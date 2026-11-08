# Wise Owl Action Receipt Ledger v1

## Purpose

The Action Receipt Ledger records bounded, verified facts about completed and
terminated Wise Owl action flows. It answers user-visible questions such as
“What did you just do?” and “Did it open?” from typed lifecycle events rather
than reconstructing history from conversation text.

The ledger is downstream of the planner, Action Intent boundary, policy engine,
confirmation authority, coordinator, trusted executor, and outcome observer.
It cannot authorize, confirm, execute, retry, or observe an action.

## Distinct Data Classes

| Data class | Meaning | May influence future actions? |
|---|---|---|
| Conversation history | User and assistant messages | Only through the normal bounded planner input path |
| Audit log | Redacted operational diagnostics | No |
| Action receipt | Immutable typed facts for one coordinator action flow | No |
| Learned memory | Deliberately retained knowledge or preferences | Not connected to receipts in v1 |
| Execution result | Whether the trusted dispatcher accepted or rejected a launch | No; it is one receipt fact |
| Observed outcome | Whether the target became ready, exited, timed out, or was invalidated | No; it is one receipt fact |

Receipts are facts, not memories, preferences, recommendations, or policy
precedent. Learned Memory does not consume the receipt ledger in v1.

## Receipt Construction

`wiseowl-brain/src/action_receipt.rs` defines:

- `ActionReceiptId`, deterministically bound to coordinator action,
  conversation, session, requester, and original request IDs.
- `ReceiptOpen`, the immutable redacted identity and version envelope.
- `ActionReceiptLifecycleEvent`, a typed source-identified event with strict
  sequence ordering and optional typed pipeline IDs.
- `ActionReceiptTerminalStatus`, which never treats `Unknown` as success.
- `ActionReceipt`, a sealed immutable timeline plus summaries and integrity
  digest.
- `ActionReceiptLedger`, the bounded append-only builder, retention, query,
  isolation, audit, and presentation layer.

Direct adapters derive events from `ActionDecision`, `ExecutionResult`, and
`ObservedActionOutcome`. The ledger has no raw conversation parser and no
operation that turns a receipt query into an action.

Events are rejected when the receipt, coordinator action, session, requester,
source, sequence, or terminal transition does not match. Exact duplicate
delivery is idempotent. A sealed receipt cannot be reopened or modified.

## Redaction

Receipts never store:

- Full user messages or conversation history
- Confirmation proofs, nonces, or challenge payloads
- Shell text, executable paths, or environment variables
- Raw IPC or arbitrary process payloads
- Hidden reasoning or internal policy rules

The target is represented by a bounded display key. Presentation maps known
keys to safe English and Persian labels.

## Integrity

The SHA-256 integrity digest uses the domain
`wiseowl.action-receipt.v1\0`. It binds the schema version, receipt and action
IDs, requester/session domain, operation and target kind, ordered lifecycle
events, terminal status, version generations, and bounded audit references.

This digest detects corruption. It is not a digital signature. Receipts that
fail verification are hidden from normal views and reported as
`ReceiptIntegrityFailure`.

## Persistence

`wiseowl-memorydb/src/action_receipts.rs` owns the narrow opaque persistence
namespace:

`ACTION_RECEIPTS/`

The brain ledger writes only through the `ReceiptPersistence` interface.
`ActionReceiptBlobStore` implements that interface behind the existing
`wiseowl-memorydb` durable-store boundary. This does not grant
`wiseowl-braind` arbitrary filesystem access.

Active fragments use bounded checksummed append frames. Sealing atomically
publishes a bounded checksummed receipt image before active fragments are
removed. A partial append is detected as an incomplete fragment stream; a
partial sealed receipt is never exposed. Receipts do not use the long-term
memory WAL, so receipt traffic cannot cause unbounded memory WAL growth.

## Retention and Isolation

The v1 static retention policy keeps at most:

- 16 sealed receipts per requester/session domain
- 4 non-executed denied, unsupported, or otherwise non-executed receipts per
  requester/session domain

Compile-time ledger capacities provide an additional global bound. Eviction is
oldest-first and deterministic. Active unsealed receipts are never retention
victims.

Every query includes a requester identity, active session, and maximum result
bound. Cross-user, cross-session, and guest-domain reads return no receipt.
There is no administrative cross-domain query capability in v1.

## Query and Conversation Views

Supported typed queries are:

- `Latest`
- `LatestCompleted`
- `ByReceiptId`
- `RecentLimited`
- `PendingCurrentAction`
- `LastFailure`

Unbounded enumeration is rejected. Pending views are transient and may report
awaiting clarification, awaiting confirmation, dispatching, or awaiting
outcome.

Conversation integration uses `ReceiptConversationQuestion::WhatDidYouDo` and
`ReceiptConversationQuestion::DidItOpen`. “Did it open?” returns ready only
when an observer-derived `TargetReady` fact exists. Dispatch acceptance alone
is presented as launch accepted, never target ready.

## Security Invariants

- A receipt cannot authorize a future action.
- A previous confirmation cannot be reused.
- A historically successful action does not imply future policy approval.
- Dispatch acceptance and target readiness remain distinct.
- Planner and Policy do not read from the ledger.
- Learned Memory does not read from the ledger.
- The ledger exposes no execution, confirmation, policy, retry, shell, or
  generic IPC operation.
- Audit entries contain only receipt/action/session IDs, event codes, public
  reason codes, and timestamps.

## Verification

Host tests cover typed successful and terminal flows, integrity, corruption,
replay/order rejection, bounded retention, deterministic eviction, active
receipt preservation, requester/session/guest isolation, bounded queries,
English/Persian presentation, and observer-based conversation readiness.

The feature-gated native test is:

`./tools/test.sh wiseowl-action-receipt-v1`

Expected final marker:

`[WISEOWL-RECEIPT] COMPLETE PASS`
