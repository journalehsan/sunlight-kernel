# Sunlight Reminders

Sunlight Reminders is the native SunlightOS app for personal tasks, reminders,
and day planning.

- Display name: `Sunlight Reminders`
- Window title: `Sunlight Reminders & Tasks`
- Storage: `sunlight-kv`
- Binary: `sunlight-reminders`

This app is separate from the system/task monitor. It is for user todos,
reminders, and planning, not process or system supervision.

## Storage in sunlight-kv (stable namespaces)

- Tasks: `app.reminders.tasks/<task-id>`
- Lists: `app.reminders.lists/<list-id>` (inbox/work/personal)
- All tasks index: `app.reminders.index/all`
- By-due-date index (list + markers): `app.reminders.index.by-date/<yyyy-mm-dd>` and `.../<date>/<task-id>`
- By-reminder-date index (list + markers): `app.reminders.index.reminder-date/<yyyy-mm-dd>` and `.../<date>/<task-id>`
- Settings: `app.reminders.settings/<key>`

Task records contain: id, title, notes, list_id, status (todo/done), due_date, due_time, reminder_date, reminder_time, created_at, updated_at.

Dates are YYYY-MM-DD (empty if unset); times HH:MM (empty if unset). Validation enforces format on save.

## Calendar integration

Sunlight Calendar reads selected-day previews directly from the shared by-date and reminder-date indexes in sunlight-kv.

- Calendar **Tasks** column shows tasks whose due_date matches the selected day (todos first; completed shown dimmed).
- Calendar **Reminders** column shows entries with reminder_date (or due_date fallback when reminder_time present but no reminder_date).
- Calendar is a **preview/dashboard only**. Full editing, notifications, and deep links live in Sunlight Reminders.
- No linked Calendar event storage yet.
- Notification scheduling is still TODO.
- Date/time entry remains manual text (YYYY-MM-DD / HH:MM) with validation; no DatePicker in this change.
- App registry/launcher was not redesigned.

When a task is saved or deleted in Reminders, the date indexes (lists + per-id markers) are updated so Calendar sees fresh data on next load/refresh.

## Remaining TODO (out of scope for this sync/preview work)

- Linked calendar event storage / bidirectional links
- Notification scheduling from reminder times
- DatePicker UI (manual entry + validation only)
- App registry changes or deep linking to specific tasks from Calendar
- AI/WiseOwl integration
- Recurring tasks or advanced rules

