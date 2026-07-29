# Wise Owl Graphical Console: Conversation Interaction v1

## Existing GUI Audit

The graphical console uses `sunlight-ui`'s native `App` event loop and
`Window`. Conversation input reuses `sunlight_ui::widgets::TextInput`, the
same shared single-line editor infrastructure used by native search and other
application inputs. It already provides focus, mouse selection, UTF-8-safe
cursor movement, Backspace/Delete, Home/End, Ctrl+A/C/X/V, clipboard
integration, and a visible selection/caret. The console also reuses native
`Panel`, `Button`, sidebar, conversation bubble, and owl-avatar widgets.

There is no shared multiline editor with reliable modifier-aware submit
semantics in this shell. Version 1 therefore deliberately uses a single-line
input: Enter and the Send button submit through the same path. Shift+Enter is
not claimed as multiline support.

The current event stream supplies key, pointer, focus, and timer events. It
does not expose a scroll-wheel event, so v1 preserves manual history position
through Page Up/Page Down and avoids forced scrolling while the reader is
above the latest message. Native text rendering preserves Persian code points;
right-to-left metadata is selected from the first strong Persian/Arabic-range
character and assistant/user bubbles align accordingly. Full Unicode bidi
reordering and grapheme-cluster cursor movement remain toolkit limitations and
are not emulated by the console.

## Conversation Boundary

Chat text is conversation input, **not** a shell command. The console owns a
bounded presentation model only:

- maximum input: 4000 UTF-8 bytes and 2048 Unicode scalar values;
- maximum local message history: 128 messages;
- maximum rendered clarification choices: 8;
- maximum redacted UI audit records: 96;
- bounded fake test-event queue: 16 events.

Every user turn receives a request identity derived from the console instance
and submission sequence; the request also carries the conversation and session
identities. Text contents are never used as an idempotency key. A user bubble
is appended only after a typed `Accepted` response, so failed unsent text stays
in the editor and repeated keyboard/button delivery cannot duplicate it.

`wiseowl-console/src/transport.rs` defines the narrow public conversation
requests: submit turn, select clarification, submit confirmation, cancel, and
query state. The native transport calls only `wiseowl.brain.v1` through the
typed `ConsoleUi` operation; it has no planner, executor, launch-adapter,
shell, VFS, or MemoryDB dependency. The production daemon currently supports
bounded local conversation and safe state queries. Action interactions remain
unavailable until the coordinator protocol is connected, rather than creating
a GUI-only execution path. The `conversation-v1-test` feature supplies
deterministic typed fake responses for the QEMU gate and never dispatches an
application.

The GUI cannot create `ActionIntent`, `PlannerResult`, policy decisions,
confirmation grants, readiness records, execution requests, or observation
evidence. It cannot confirm an action locally: soft confirmation sends a typed
answer, while Strong and Critical cards remain disabled until an existing
trusted proof control is available. Proof material is neither held nor logged
by this UI.

## Presentation Behavior

Typed assistant text, clarification, confirmation, cancellation, progress,
ready, failure, rejection, session invalidation, and unavailable responses
update the local model. Clarification buttons carry the exact candidate ID from
the response; labels are never treated as canonical identifiers. Cards disable
immediately after submission, expire on timer ticks, and may be cancelled while
waiting.

Progress is represented as one replaceable live card. Dispatch acceptance uses
“Opening …”; it is not presented as Ready. Only a typed `ActionReady` seals an
immutable final result. Early exit, dispatch failure, and timeout use separate
typed failures. The owl animation maps input focus to Listening, responses to
Thinking, cards to Clarification/Confirmation, dispatch to Acting, readiness
wait to Observing, success to Success, errors to Warning, and endpoint loss to
Offline. Reduced motion has no extra animation beyond the existing low-rate
avatar tick.

Page navigation retains the conversation page and continues timer-driven
transport updates; switching to Activity, Health, or Privacy never cancels a
pending action. Those three pages remain placeholders. Session invalidation
disables the editor, clears pending cards and presentation-only action state,
and requires a new binding. UI history is not Learned Memory.

Offline mode disables Send and does not replay uncertain submissions. A future
service endpoint can restore connectivity and re-query typed state; it must
not auto-resubmit old user text.

## Capability and Audit Profile

The console uses normal GUI/display, clipboard, session presentation, and
nameserver access to the typed Wise Owl endpoint. It does not request or link
spawn, scheduler/process control, direct executor/launch access, policy
mutation, confirmation authority, unrestricted VFS, or MemoryDB writes.

The bounded UI audit records only event kind, redacted request identity, and
input length. It never records user/assistant text, proof material, candidate
payloads, correlation tokens, executable paths, or raw IPC.

## Tests and QEMU Gate

Host tests cover shared editor focus, English/Persian/mixed input, UTF-8-safe
Backspace, bounds, empty submission rejection, deduplicated accepted messages,
typed assistant output, clarification identity/double-submit prevention,
Strong confirmation refusal, message eviction, progress, timeout, and offline
handling.

The feature-gated gate is `wiseowl-gui-conversation-v1`. It builds
`wiseowl-console` with `conversation-v1-test` and expects:

```text
[WISEOWL-GUI-CHAT] INPUT_FOCUS PASS
[WISEOWL-GUI-CHAT] ENGLISH_INPUT PASS
[WISEOWL-GUI-CHAT] PERSIAN_INPUT PASS
[WISEOWL-GUI-CHAT] SUBMIT PASS
[WISEOWL-GUI-CHAT] USER_MESSAGE PASS
[WISEOWL-GUI-CHAT] ASSISTANT_MESSAGE PASS
[WISEOWL-GUI-CHAT] CLARIFICATION PASS
[WISEOWL-GUI-CHAT] CONFIRMATION PASS
[WISEOWL-GUI-CHAT] CANCELLATION PASS
[WISEOWL-GUI-CHAT] ACTION_PROGRESS PASS
[WISEOWL-GUI-CHAT] OUTCOME_READY PASS
[WISEOWL-GUI-CHAT] TIMEOUT PASS
[WISEOWL-GUI-CHAT] OFFLINE PASS
[WISEOWL-GUI-CHAT] BOUNDS PASS
[WISEOWL-GUI-CHAT] SECURITY_BOUNDARY PASS
[WISEOWL-GUI-CHAT] COMPLETE PASS
```
