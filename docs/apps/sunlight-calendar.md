# Sunlight Calendar Storage

Sunlight Calendar stores durable app data in `sunlight-kv`. The old
`~/.local/share/sunlight-calendar/events.dat` file is treated as a legacy import
source only.

## Key Namespace

- `app.calendar.events/<event-id>` stores one event record.
- `app.calendar.index/all` stores the durable event id list.
- `app.calendar.index/by-date/<yyyy-mm-dd>` stores event ids for one civil date.
- `app.calendar.settings/selected-date` stores the last selected date.
- `app.calendar.settings/view-month` stores the last visible month.
- `app.calendar.settings/file-v1-imported` marks the one-time legacy file import.

Event records are small UTF-8 field records containing title, date, start/end
time, all-day flag, notes, and created/updated timestamps. Bad or malformed
records are skipped during load instead of aborting the whole calendar.

## Tasks & Reminders

The Calendar toolbar's `Tasks & Reminders` button launches the native
`sunlight-reminders` app through `sun-exec`. That app owns personal tasks,
reminders, and daily planning; it is separate from the system task monitor.

## Migration

On first successful `sunlight-kv` startup load, Calendar imports the legacy
`events.dat` file if present, then writes the migration marker. The old file is
not deleted automatically.

## Data Placement Rule

Small persistent app data belongs in `sunlight-kv`: settings, preferences,
lightweight structured records, UI state, and recent items. Future large,
binary, attachment-like, or complex app data should move to a future
`sunlight-sm`/file-backed storage layer instead of being packed into KV.
