# Shell Phase 1: Pipelines and Redirection

## Audit findings

The interactive native shell is `sshl`, implemented by `sunshell/src/main.rs`.
It receives keyboard input and foreground lifecycle messages from `tty_server`;
the PTY session path uses the same shell parser and polls its foreground child.
`sunlight-tty/src/shell.rs` is a separate small no-fork shell helper, not the
interactive `sshl` implementation. The host development shell has its own
executor in `sunshell/src/exec.rs`.

Before this change, `sunshell/src/parser.rs` split words and pipes while
stripping single and double quotes, but it did not report malformed quotes,
did not understand redirections, and could silently accept unsupported shell
operators as word characters. The native `sshl` rejected every pipeline. The
host executor piped external commands but waited for the final child while
retaining only the last stage's status; it had no structured redirection model
or launch cleanup for partial pipelines. Native external commands were started
one at a time through `spawn`, which could override only child stdout.

The kernel already implements native `pipe`, `dup`, `dup2`, `close`, `open`,
`read`, `write`, `spawn`, and `waitpid`. Native pipes use a bounded ring buffer,
reader/writer reference counts, scheduler wait queues, wakeups, EOF after the
last writer closes, and a broken-pipe result after the last reader closes.
Native pipe reads and writes now park on ring readiness by default; nonblocking
descriptors still return `Again`. Native `waitpid` reports the exit code
directly; when a child is still running, the kernel blocks the caller until it
exits, and the libc wrapper retries. Spawn
inherits credentials, environment, working directory, capabilities, and the
TTY association. When a TTY association exists, the kernel installs TTY-backed
standard descriptors; explicit descriptors can override each standard slot.
The kernel does not hold the scheduler lock during userspace pipe I/O.

Helios uses its Linux syscall translation and Linux-shaped error handling. Its
`pipe2`, `dup`, `dup2`, and `wait4` behavior remains on that path. This change
extends only native spawn argument handling and does not change the Helios ABI.

## Changes

- Added a checked lexer/parser and structured simple-command AST with ordered
  redirection records. The compatibility `parse_line` entry point remains.
- Added native `spawn_with_stdio`, plus libc `dup` and `dup2` wrappers. Native
  spawn accepts optional stdin, stdout, and stderr descriptors in unused
  native syscall argument registers; existing callers still pass only stdout
  and retain their previous TTY behavior.
- Added arbitrary-length native pipelines. All stages are spawned before the
  shell waits, the parent closes all pipe ends after launching, and each child
  receives only its stage's descriptors. Foreground completion tracks and
  reaps every stage. The final stage supplies the pipeline status.
- Added a private `sshl --pipeline-builtin` execution path for supported
  built-ins. It runs each pipeline built-in in a separate child process, so
  `cd` cannot change the interactive shell's working directory.
- Added native `<`, `>`, `>>`, `2>`, and `2>&1` descriptor mapping in
  left-to-right order for external commands and pipeline built-ins. A simple
  built-in writes stdout redirection directly through the opened file handle,
  so the interactive shell's own descriptors are not changed.
- Added a host executor with streaming OS pipes, ordered descriptor mapping,
  stage launch cleanup, and final-stage status selection.
- Kept the parser and executor small and local to `sunshell`; no external shell
  implementation or dependency was added.

## Supported command grammar

Phase 1 accepts simple commands with whitespace-delimited words, single and
double quotes, backslash escaping, `|`, input redirection `<`, output
truncation `>`, output append `>>`, stderr redirection `2>`, and `2>&1`.
Redirections are applied in source order. Variable expansion remains the
shell's existing `$NAME` / `${NAME}` token expansion.

Command separators (`;`), background jobs (`&`), grouping, command
substitution, glob expansion, here-documents, and job control are not
implemented. The parser returns a syntax error for separators, background
operators, grouping, unterminated quotes, trailing escapes, empty stages, and
missing redirection targets. Expansion does not preserve quote context and is
not full POSIX expansion.

## Verification

Host tests run with:

```sh
cargo test --target x86_64-unknown-linux-gnu -p sunshell --no-default-features --features std
```

They cover two- and three-stage pipelines, a 1 MiB stream, early consumer
termination, final-stage status, missing executables, a built-in pipeline
subshell, input redirection, append/truncate, merged stderr, quoting, escaped
pipe characters, malformed syntax, and redirection order. The latest completed
run passed 7 library tests and 12 shell binary tests.

`cargo check -p sunshell` passed for the configured native target, and
`cargo check -p sunlight-kernel` passed after the native spawn descriptor
extension. The native `sshl` release build passed with the userspace linker and
x86-64-v2 flags. The `./tools/test.sh phase3.8` QEMU gate ran with its default two
CPUs and failed its expected-output checks. It reached the root login shell and
ran `whoami`, but the log showed the existing `id` user lookup limitation and
`useradd` failing to write `/etc/passwd`; the gate did not reach its Phase 3.8
marker. This gate does not exercise interactive pipeline commands.

The focused libc sentinel test passed, including the new `EPIPE` mapping. A
full `sunlight-libc` test run was started but stalled in the existing
`secret_store::tests::default_options_are_private_and_bounded` test and was
stopped; the rest of that suite is therefore unverified in this run.

## Remaining gaps and risks

- Native QEMU execution of interactive pipelines and redirections has not yet
  been directly demonstrated by an injected command sequence. The native code
  has compile coverage; the host executor has behavioral test coverage.
- Native waitpid is a yielding poll loop rather than a kernel-blocking wait.
  It does not busy-spin, but a dedicated blocking wait primitive would be a
  useful future kernel improvement.
- Native broken-pipe writes report `EPIPE` through the native errno wrapper.
  The kernel does not currently deliver SIGPIPE to native processes.
- Errors during native open/spawn are currently rendered as shell diagnostics;
  native `sshl` does not yet maintain a POSIX numeric status for every
  redirection failure.
- Simple parent built-ins route stdout to `>` or `>>` files without replacing
  the shell's descriptors. Their output is currently represented as one byte
  stream, so separate stderr routing and exact `2>&1` ordering for diagnostics
  from those built-ins are not fully modeled. External commands and pipeline
  built-ins use the full ordered descriptor map.
- The host `std::process` path and native kernel path differ in executable
  lookup and error mapping, as expected for their respective environments.
- The separate `sunlight-tty` no-fork shell does not gain pipelines or
  redirection in this phase.

## Recommended next phase

Add an automated native shell command runner to the QEMU harness, then verify
simple and multi-stage pipelines, redirection ordering, missing paths, early
consumer exit, repeated runs, and process/descriptor cleanup under the
configured SMP boot. Follow that with quote-aware parameter expansion, shell
status consistency for all failures, and a kernel blocking child wait if
foreground polling becomes measurable.
