# Tasks Monitor UX polish

The monitor continues to consume `Telemetry` snapshots exclusively. No dependencies,
accounting changes, kernel queries, or changes to unrelated applications were added.

## Components and API

- `sunlight-ui/widgets/table.rs`: optional controlled sorting through
  `with_sorting(displayed_column_ids, Option<SortState>)`, `sort_at(x, y)`,
  `with_header_hover`, and `draw_header`. `UNSORTABLE_COLUMN` disables an individual
  header. Existing constructors and `Column` literals remain compatible; sorting
  defaults to disabled. The caller applies row ordering and retains row identity.
- `SortState::toggle` supplies ascending/descending header transitions;
  `SortState::compare` applies direction while retaining a deterministic identity
  tie-breaker. `SortKey` supports case-insensitive Unicode text, integers, floats
  (total ordering, including NaNs), raw bytes, domain ranks, and missing values.
  Application comparators may be passed directly to `SortState::compare`.
- Table draw and hit-testing clamp stale offsets consistently. Headers and cell
  text respect available column widths; incomplete rows are not selectable.
- Shared document `ScrollView` geometry now handles tracks below its minimum
  thumb size without panicking.

## Monitor behavior

All eight process columns retain their existing formatting and sort underlying
values. PID breaks process ties independently of direction. Flat mode sorts the
complete filtered list. Parent tree traversal sorts siblings and emits complete
subtrees together, with deterministic cycle/orphan recovery. App groups sort by
CPU/private/shared/mapped totals, app ID for Name/App ID, and minimum member
PID/state for those nonaggregate columns. Shared totals retain the existing mapped
row semantics and do not enter physical memory accounting.

Group expansion, selection, sort, and history remain application-owned. Refresh
preserves the top visible process identity (PID plus generation) where available;
otherwise the previous offset is clamped. Sorting and mode changes preserve a
valid offset. Wheel input is consumed within the table even at bounds. Shared
`ScrollState` supplies bounded wheel, thumb drag, track paging, and scrollbar
geometry. Home/End/Page Up/Page Down remain supported. The header stays fixed.

Details/Hide info controls one system-details state. It hides the breakdown and
accounting notes while retaining overview cards, uptime, and task count, returning
that space to the table. Enter retains the separate existing selected-process
inspection control. Narrower layouts reserve more space for wrapped statistics;
long process cells are clipped to their columns.

## Validation

- PASS: `cargo test -p sunlight-ui --lib --target x86_64-unknown-linux-gnu`
  (277 tests).
- PASS: `cargo test -p sunlight-tasks --target x86_64-unknown-linux-gnu`
  (13 tests).
- PASS: release build of `sunlight-tasks` with the repository's userspace linker
  script, static relocation, x86-64-v2, and no-redzone flags.
- PASS: `git diff --check`.
- NOT_RUN: fresh ISO build, QEMU boot gate, and interactive desktop verification
  (wheel routing across windows, pointer capture outside the window, maximized
  rendering, live Telemetry refresh, and concurrent Melody Mina). The existing
  ISO was not rebuilt and is not evidence for these changes.

Tests exercise sort transitions and disabled headers, typed comparisons and ties,
header/row event separation, stale table offsets, small scrollbar geometry,
all monitor columns and directions, aggregate groups, subtree adjacency,
selection/expansion retention during row rebuilds, viewport recovery, and wheel
and thumb input. Existing layout/scroll tests cover small and resized windows.
Host rebuild tests do not substitute for live Telemetry or rendered desktop checks.

Remaining limits: scrolling is deliberately row-based rather than animated;
when an anchor disappears the numeric offset is retained and clamped. Extremely
short windows can have no complete process row. Extremely narrow detail areas
clip remaining wrapped text at their bounds. Interactive acceptance still needs
the QEMU checks above. Existing dependency/configuration warnings remain.
