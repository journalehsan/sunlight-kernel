# Day 22 — Desktop Input Stabilization

## Summary

- Mouse input for the desktop path now coalesces multiple pending PS/2 motion
  packets in `sunlight-mouse` before forwarding them to `sunlight-display`.
- The compositor applies a fixed-point, stateless pointer curve with:
  - sensitivity scalar
  - optional acceleration
  - gentle acceleration factor
  - hard per-batch delta cap
- Cursor motion now stops immediately when fresh hardware deltas stop. The
  display server does not synthesize inertia or decay movement.
- Alt+Tab is now stateful in `sunlight-display`:
  - trigger once on `Tab` keydown while `Alt` is held
  - allow repeat only while both keys remain pressed
  - cap repeat to `120 ms` between switches
  - stop repeat immediately on `Alt` or `Tab` keyup

## Debug Counters

- `sunlight-mouse` logs raw PS/2 packet totals plus coalesced batch totals.
- `sunlight-display` logs raw mouse batch deltas, final applied deltas,
  cursor clamps, per-batch caps, and Alt+Tab trigger counts.
- Verbose per-event logging remains behind `INPUT_DEBUG` / `MOUSE_DEBUG`.

## Day 22 Checklist

- [x] Coalesce pending mouse deltas before desktop delivery
- [x] Clamp large per-batch cursor jumps
- [x] Keep mouse acceleration stateless and moderate
- [x] Stop cursor movement immediately when input stops
- [x] Make Alt+Tab edge-triggered and rate-limited
- [x] Stop shortcut repeats on key release
