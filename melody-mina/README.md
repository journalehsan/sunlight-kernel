# Melody Mina media integration

Melody Mina is a presentation client of `sunlight-media`. The application owns
exactly one `MelodyMediaController`, and that adapter owns exactly one
`MediaPlayer`/active worker for the lifetime of the window. Rendering reads a
copy-only view model; it never holds a backend lock or accesses decoder, PCM,
audiod, or device state.

The backend has no callback/event-queue API in this phase. Melody therefore
reads its bounded atomic snapshot at 30 Hz only while focused and playing,
10 Hz while unfocused/Loading, and 4 Hz while otherwise idle. Media position is
always copied from the backend hardware-consumption clock; the UI timer does
not advance time. Visualization is one replaceable fixed-size latest frame,
not an event queue.

The file picker returns a path to the controller. An accepted Open synchronously
publishes `Loading` and increments the backend source generation, preventing a
second path from racing the worker. Controller refreshes reject snapshots whose
generation does not match the current source. The queue panel contains only the
active path (or the empty-state row); it is not a playlist engine.

On window close, the app is dropped after the event loop. Controller drop then
drops `MediaPlayer`, which requests worker shutdown, flushes application audio,
and releases decoder/source storage. Previous, Next, Repeat, and Options remain
visible but disabled until a real playlist/settings layer owns them.
