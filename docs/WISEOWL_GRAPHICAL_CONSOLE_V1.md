# Wise Owl Graphical Console v1

This document describes the architecture and design decisions for the first native graphical Wise Owl application (v1).

## Architecture

The Graphical Console (`wiseowl-console`) is a purely presentation-focused client for the Wise Owl cognitive services. It relies entirely on existing native IPC paths to communicate with `wiseowl-braind`.

It is **not** a Planner, Policy Engine, Confirmation Authority, or Executor. All action dispatch and state transition logic remains in `wiseowl-brain`.

```text
Wise Owl Graphical Console (UI Client)
          ↓
Native IPC (BrainRequestWire / BrainResponseWire)
          ↓
Conversational Action Coordinator (in wiseowl-braind)
          ↓
TrustedActionFlow
          ↓
Outcome Observer
          ↓
Action Receipt Ledger
```

## Application Registration
The app follows standard SunlightOS graphical application patterns:
* Resides in `/Applications/WiseOwl.sunapp`
* Uses `Manifest.toml` format to define ID (`org.sunlight.wiseowl`), icon, and executable (`/bin/wiseowl`).
* Discoverable in the Start menu as "Wise Owl" (English) / "جغد دانا" (Persian).
* Reuses existing single-instance startup logic where supported.

## UI-to-Brain Boundary
A new typed client boundary is established over the native IPC. The GUI sends requests (like `SubmitConversationTurn`, `CancelPendingAction`) and receives structured, bounded responses.

The UI does not access the MemoryDB directly, nor does it possess unrestricted VFS access or process execution authority. It renders `ClarificationRequired` and `ConfirmationRequired` using strictly typed candidates and levels provided by the Coordinator.

## Page Structure
1. **Conversation**: The primary surface. Bounded history (stored strictly in memory presentation-side). Renders text input, typing indicators, clarification cards, confirmation cards, and the action progress timeline.
2. **Activity**: View bounded `ActionReceipt` items from the Receipt Ledger. Displays terminal outcomes (Ready, Denied, Timeout).
3. **Health**: Read-only display of component health (braind, memorydb, planner, coordinator, etc.) via public snapshot data.
4. **Privacy**: Simple views explaining retention logic and controls for clearing the bounded conversation presentation history.

## Action Progress Semantics
The UI explicitly distinguishes between dispatch and readiness.
* **Dispatch Accepted**: `Opening Calculator...` (Action passed policy and was launched).
* **Ready**: Shown only when the Outcome Observer signals the target application is ready for interaction.

Receipts are treated as immutable facts of history, not mutable learned preferences.

## Reused Components
* **UI Toolkit**: `sunlight-ui` components (App, Window, Button, Canvas)
* **Text/Font**: `sun_font` (with support for English and Persian RTL).
* **Theme**: Integrates with the existing system accent and dark/light modes.
* **Localization**: Hardcoded layout switches or basic string matching based on the active locale, mirroring the `sunlight-welcome` approach.

## Deferred (Not in v1)
* Persistent desktop owl character
* Shell lower-right notification/popover integration
* Broad voice input / wake word
* Autonomous autonomous agent tasks

## Tests and Bounds
* Bounded history lengths (to prevent UI memory leaks).
* Feature-gated QEMU checks (`wiseowl-graphical-console-v1`) verifying layout, components, bounds, and boundary security.
* Host tests checking translation mapping and timeline states.
