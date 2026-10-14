# `sunlight-top` refresh liveness

## Root cause

`sunlight-top` used a function named `read_key_nonblocking`, but it issued the
ordinary native `read(0, ...)` syscall. Native empty TTY reads now park the
calling task until input arrives. The call therefore stopped the top loop
before its 100 ms spin delay and before `Telemetry::poll()` could run. A key
press woke the task, which made the failure look like a terminal repaint
problem. The graphical terminal used the same native TTY ring after PTY
attachment, so both TTY and PTY modes shared the failure.

The output path also ignored short writes. A top frame larger than the kernel's
4 KiB syscall chunk could be truncated after the first write, leaving a partial
screen even when the application had rendered it.

## Event-loop architecture

The native poll ABI now accepts readable pipe or controlling-TTY descriptors
and a relative timeout. Readiness and waiter registration happen under the
scheduler lock. A finite wait records a scheduler deadline; TTY input wakes it
immediately, while timer expiry wakes it when the deadline passes. The wait
does not spin and does not change the process personality or capability set.

`sunlight-top` renders its initial frame immediately, then uses the monotonic
clock to wait for either fd 0 input or the next sample deadline. The default
statistics interval is **1,000 ms**. A late wake rebases the next deadline from
the current time, avoiding catch-up bursts. Keys are read only after readiness,
and `q`, Escape, Ctrl+C, cancellation, EOF, and PTY closure all use the normal
alternate-screen cleanup path.

Statistics sampling and terminal repaint are separate: timed refreshes sample
telemetry and repaint, while a key that changes visible state repaints from the
current snapshot immediately.

## TTY and PTY behavior

Native TTY input uses the tab's kernel stdin ring. Graphical Sunlight Terminal
PTY sessions attach the same foreground process to a distinct TTY tab, so the
same readiness and timeout path applies. Existing foreground ownership,
cancellation, PTY generation checks, and capability validation remain in force.

Top output now retries short writes until the frame is drained or cancellation
is reported, so frame data is not silently discarded at the 4 KiB syscall
boundary.

## Validation

Source and build validation completed with `cargo check -p sunlight-top` and
`cargo check -p sunlight-kernel`. The host `sunlight-ipc` test command cannot
run in this workspace's configured no-std target environment because rustc
cannot provide the `test` crate for that target.

On October 14, 2026, the native `top` gate was attempted at 1, 2, and 4
CPUs. The default 1/2/4 CPU runs reached login but exhausted the 1 GiB guest
memory before `sshl` could load. A bounded 4 GiB, 1 CPU run loaded `sshl`, but
the existing synthetic keyboard sequence stopped after login and did not reach
the `top` command. Consequently there is no direct QEMU evidence yet for
`[TOP] rendering`, five timed refreshes, TTY/PTY parity, or runtime CPU,
wakeup, interval, and output-byte measurements. These are validation gaps,
not performance claims.

## Remaining limitations

The current terminal renderer still emits a complete top frame for each timed
sample or visible interaction. It does not yet maintain a cell-level damage
set. Terminal resize is sampled at the refresh cadence because native SIGWINCH
delivery is not available.
