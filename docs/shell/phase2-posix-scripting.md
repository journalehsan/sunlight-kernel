# Shell Phase 2: POSIX-oriented scripting, first vertical slice

This is a tested partial implementation, **not full POSIX compatibility** and
not completion of all Phase 2 requirements. The boundary is a shared parser and
control-flow interpreter, descriptor-based noninteractive entry points, and
selected function/variable semantics. Expansion stage D and built-in stage E
remain incomplete. Scripts using the supported subset run through `sshl`,
without delegating their language interpretation to another shell.

Semantic reference: POSIX.1-2024 Shell Command Language, especially sections
2.2, 2.5, 2.6, 2.7, 2.9, 2.10, and 2.15:
`https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html`.

## Phase 1.5 audit

The starting repository already contained the following implementations:

| Capability | Source evidence at audit | Boundary |
| --- | --- | --- |
| Quote provenance | `parser.rs`: `WordPart` and `Quote` | Single/double/unquoted/escaped regions retained |
| Ordered expansion | `expansion.rs`: parameters, IFS splitting, quote removal | No substitution, arithmetic, parameter operators, positional parameters, or pathname expansion |
| Exit status | Host error mapping; native `builtin_status`, foreground completion and `ShellEnv["?"]` | Checked parsing and expansion use status 2; native launch/wait updates status |
| Native validation | `native_tests.rs`, `shell_phase1_5.expected`, `test.sh shell-phase1.5` | Actual TTY IPC and PTY runners, 1 MiB pipelines, early close, descriptor/heap repetition, live 64 KiB output tests |
| Descriptor execution | Native `launch_pipeline`, `spawn_with_stdio`; host `execute_expanded` | All children started before wait; error cleanup kills/reaps already-started children |
| Environment | `shellenv.rs`, native `envp` access and spawn inheritance | Starting `export` modified shell-local values; no userspace spawn interface to pass those changes to children |
| Script entry | Host `-c` supported one simple command/pipeline; native startup registered a terminal shell | File entry points and compound language were absent |

The status-query helper in the Phase 1.5 runners also overwrote `$?` while
reading it; this was exposed by the independent PTY run and corrected by
restoring the observed status after the query.

The old Phase 1 document predates several of these changes. The audit did not
infer a successful native test run from the presence of the test source.
Current-run results are recorded below.

## Architecture

`parser.rs` provides one quote-aware lexer with byte offsets and ordered
redirection tokens. `script_parser.rs` implements a bounded recursive-descent
parser and structured `CommandKind` AST. It has all requested variants:
SimpleCommand, Pipeline, AndOrList, SequentialList, IfCommand, ForCommand,
WhileCommand, UntilCommand, CaseCommand, FunctionDefinition, BraceGroup,
Subshell, and NegatedPipeline. Compound nodes retain their redirection list and
source offset. Diagnostics report line and column; EOF errors also expose an
`incomplete` flag for future interactive continuation.

The legacy `parse_line_checked` entry point now adapts this same grammar to the
existing simple/pipeline AST. It rejects compound interactive input explicitly;
semicolons can no longer accidentally flatten a list into one command.

`script.rs` is an `alloc`/`no_std` interpreter shared by both platforms.
`ShellExecutionContext` contains cwd, variables and attributes, positional
parameters (`env.positional`), script name, shell PID, last status, function
bodies, borrowed stdio descriptors, and loop/call/source depths. Its
`interactive` field is reserved; shell options are not implemented. Runtime
operations are confined to process execution, descriptor I/O, directory
validation, and bounded script-file reads. Host and native runtimes do not
implement independent control-flow languages.

`host_script.rs` adapts the existing host streaming executor. `native_script.rs`
uses the existing native pipeline launcher, `spawn_with_stdio`, and `waitpid`.
Script execution happens before TTY/PTY registration and requires no terminal
input. The native adapter temporarily sets process cwd only while spawning and
waiting, then restores it. Shell-side cwd, variables, functions, and positional
parameters are cloned for subshell groups. These are isolated execution
contexts in the same interpreter process; signals and jobs are not modeled.

The existing interactive native keyboard dispatcher, foreground notifications,
terminal output draining, PTY authority, and Helios ABI remain in place. Full
interactive compound evaluation is deferred until it can use asynchronous
execution/continuations without blocking terminal draining. Both interactive
and script execution share lexical, grammar, and supported expansion semantics;
interactive entry points currently accept the simple-command/pipeline subset.

### Native descriptor repair discovered by the gate

The first live script run failed with `write fd 1: Failed`. Spawn installed a
VFS handle into the child without sharing its open description; closing the
parent descriptor closed the backend still needed by the child. This also
reset file offsets on spawn and kept independent offsets after `dup`.

`FdTable::install_from` now retains the source open description. Shared file
positions live in that description and are used by read/write/lseek/getdents.
Consuming descriptor snapshots identify whether the VFS backend needs closing.
Close, displaced dup2/dup3 handles, CLOEXEC, and process-reap cleanup release a
VFS backend only on the final reference. Pipes retain their existing endpoint
reference accounting. The independent PTY check also exposed unnumbered `/bin/sh` descendants being
reattached to TTY tab 0 instead of their invoking tab. Spawn now gives only
explicit numbered `/bin/sshlN` paths a new tab; unnumbered aliases inherit the
parent association. Script output/input therefore use the invoking PTY path.

The 64 KiB live-output check then exposed full native TTY rings returning a
successful zero-byte write, which libc treated as failure. Full rings now
report EAGAIN, allowing the existing bounded `write_all` retry/yield path and
terminal consumer to preserve the remaining bytes. Positive partial writes
retain their existing behavior.

This repairs generic process/descriptor behavior; there
is no shell-specific VFS workaround or new syscall/ABI.

## Entry points and supported grammar

The bounded source is parsed in full before execution. A later syntax error
therefore prevents earlier commands from running; incremental POSIX input
consumption is deferred.

```sh
sh script.sh
sh script.sh arg1 arg2
sh -c 'echo hello'
sh -c 'echo "$0:$#:$1"' script-name argument
```

The native `/bin/sh` alias already maps to the `sshl` ELF. `$0` is the filename
for file execution; after `-c`, the optional first operand sets `$0` and the
remaining operands set positional parameters. Without that operand `$0` is
the invoking argv name. Blank lines, comments (including a shebang line),
multiline compound commands, quoted newlines, escaped newline continuation,
and empty scripts are supported.

The kernel loader still accepts ELF personalities rather than interpreting
`#!` lines. Native scripts must be invoked explicitly with `sh`; direct
`spawn` of the shebang fixture is checked to fail in the native test.
There is no automatic ENOEXEC script fallback. `-s`, stdin-as-script, option
parsing, and terminal multiline continuation are not implemented.

Supported execution includes nested `if/elif/else/fi`, `for` with explicit
words or implicit `"$@"`, `while`, `until`, `case/esac`, semicolon/newline lists,
left-associative `&&`/`||`, negated pipelines, brace groups, subshell groups,
and function definitions with compound bodies. `case` implements literal,
unquoted `*`, and unquoted `?` patterns and `|` alternatives. Quoted pattern
characters remain literal. Bracket patterns and alternative case terminators
are rejected.

Simple pipeline stages retain ordered `<`, `>`, `>>`, `n>&m`/`n<&m` for
standard descriptors 0–2. Compound commands and function invocations can have
scoped redirections. Simple redirection-only commands are supported in script
mode. The lexer currently treats newline and semicolon separators alike in some
continuation/header positions. This permissively accepts semicolons where
POSIX permits only newlines (after logical/pipeline operators, between a
function header and its body, and before `in`). Strict separator classification
is deferred; valid multiline POSIX syntax in the supported subset is accepted.
No background jobs, here-documents, here-strings, descriptor closing,
noclobber, arbitrary descriptor numbers, aliases, or job control are provided.

Pipelines support simple stages. Function stages, scripting special built-in
stages, `test`/`[`/`read` stages, compound stages, and prefix assignments in stages fail with status 2
before any stage is launched. Native regular built-in children retain the
Phase 1 execution path. The host executor handles `cd` pipeline isolation with
a status-only child utility, rather than constructing source for another shell;
its options/diagnostics are limited. The final stage supplies pipeline status;
partial launch failure preserves the existing 126/127/failure status and
kills/reaps already-started stages.

## Functions and variable visibility

Function bodies are immutable shared `Rc` ASTs, so nested invocation and
subshell-context cloning do not copy entire bodies on each call.
Definitions persist in the context; invocation substitutes positional
parameters and restores them on normal return, `return`, `exit`, or error.
`$0` remains the script name. Nested calls are supported. Ordinary assignments
inside functions remain visible to their caller. Prefix assignments are
permanent for special built-ins and assignment-only commands; other command
prefix values are temporary. Restoring prefixes restores only those names,
so unrelated function changes remain visible. Subshell changes stay isolated.

Variables carry independent export/readonly attributes. Assigning or unsetting
readonly values is an error. Host children receive only exported variables
plus temporary command-prefix assignments through `Command::env_clear/envs`.

Native spawn currently inherits the kernel process environment and exposes no
userspace argument for a changed export map. The native script adapter rejects
an external launch with status 2 if its exported map differs from the inherited
map; it does not silently discard exports. Shell-local assignments and exported
values consumed by in-process built-ins/functions work. Changing an exported
PATH or passing prefix environment assignments to an external native command
therefore remains unsupported. The old interactive export behavior is
unchanged. An authorized native environment interface is a prerequisite for
completing stage C.

## Expansion semantics

Expansion preserves quote regions through parameter expansion, field
splitting, and quote removal. Single quotes and escaped regions suppress
parameter expansion. Double quotes preserve whitespace and empty arguments.
Only characters produced by unquoted expansion undergo IFS splitting;
literal whitespace inside a word is not retroactively split. Default IFS is
space/tab/newline, non-whitespace delimiters produce empty interior fields,
and an empty IFS disables splitting.

Supported parameters are `$NAME`, `${NAME}`, `$0` through `$9`, `$#`, `$@`,
`$*`, `$?`, and `$$` (also their braced forms). `"$@"` retains separate
parameters and empty arguments, and yields no fields when there are no
parameters. Prefix/suffix text attaches to the first/last positional fields.
`"$*"` joins with the first IFS character (space when unset, no separator when
IFS is empty). Unquoted positional values are subject to splitting.

Assignment RHS, declaration assignments to export/readonly, redirection
operands, and case words use scalar expansion without splitting or pathname
expansion. Case patterns retain their quote mask. Tilde expansion, `${name:-x}`
and other parameter operators, `${#name}`, pathname expansion, command
substitution, backticks, and arithmetic expansion remain unsupported and
produce explicit errors. Thus the requested arithmetic counter example is
not supported yet. There is no output capture, truncation, arithmetic overflow
behavior, or hidden external shell implementation to claim as completed.
Redirection or scalar uses of `"$@"` with multiple parameters are outside this
slice; the scalar adapter selects the first resulting field.

## Built-in compatibility matrix (script mode)

| Utility | Status | Limitations |
| --- | --- | --- |
| `:` | Supported special | Null command |
| `.`, `eval` | Partial special | Shared interpreter; dot requires explicit filename/path, no PATH search or operand positional scope |
| `exit`, `return` | Supported special | Numeric status modulo 256; return only in function/source |
| `export`, `readonly` | Partial special | Names and assignments; no listing/options; native external export changes rejected |
| `set`, `shift` | Partial special | `set --` and numeric shift; no shell options or variable listing |
| `unset` | Partial special | Variables only, no `-f`/`-v` or function removal |
| `exec` | Rejected | Process replacement/persistent descriptor ownership deferred |
| `times`, `trap` | Rejected | Timing and native signal integration deferred |
| `break`, `continue` | Supported | Positive optional count; nested loop transfers |
| `cd`, `pwd` | Partial regular | Cwd/context isolation; no `-L`/`-P`, CDPATH, or cd `-` |
| `echo` | Partial regular | Joins arguments and newline; no escape/option handling |
| `test`, `[` | Partial regular | 0/1-argument truth, `!` with one operand, `-n`, `-z`, `=`, `!=`, six integer comparisons; no file/compound tests |
| `true`, `false` | Supported regular | Status 0 and 1 |
| `printf` | Existing external utility | Host/native executors resolve it normally; not a new shared builtin |
| `read` | Rejected | Descriptor-backed input/IFS assignment deferred |

The special class also includes break/continue. Special-built-in prefix assignments persist. Syntax/expansion errors terminate
noninteractive execution with status 2. `cd` and `test` errors return command
status so logical/list execution can continue. External command errors return
126/127/1 and can be tested with `||`. Complete POSIX special-built-in error
rules, interactive recovery, redirection-error continuation, and function-
lookup precedence across all regular built-ins remain incomplete: some
redirection/I/O errors currently terminate the script with status 1.

## Resources and safety boundaries

| Resource | Current bound |
| --- | --- |
| Source/file/eval input | 64 KiB; oversize fails, never silently truncated |
| Lexer tokens | 8192 |
| Parser nesting | 32 compound levels |
| Function calls | 16 active calls |
| Source/eval nesting | 16 active levels |
| Function definitions | 64 per context |
| Shell variables | 256 names through checked assignment; 16 KiB value |
| Expanded word | 16384 atoms, checked before parameter-value insertion |
| Expanded arguments / for words | 256 |
| Native external argv | Existing libc: 15 entries including argv[0], 1024-byte aggregate arena |
| Native paths | Existing libc: 256-byte C-string buffer; existing shell normalization also limits components |
| Command-substitution capture | Not implemented; all substitution syntax fails |

The recursion and syntax limits are deliberate safeguards for constrained
native stacks. Loops have no arbitrary iteration ceiling. Pipelines stream
through existing pipes, without capturing or buffering their complete output.
Descriptor scopes close every descriptor they open, including partial-open
failures, and restore borrowed parent mappings. Child-launch failure reaps
started children; normal scripts wait every stage. Shared VFS lifetime stays
in the kernel.

Script reads use a bounded buffer and native `try_reserve`. Other parser,
expansion, AST/context cloning, and map allocations still use Rust's infallible
`alloc` paths. General graceful OOM handling is therefore **not complete**;
explicit bounds reduce exposure but are not proof of OOM recovery. A fallible
arena/context allocation design and tests under forced allocation failure are
needed before claiming the full resource-safety requirement. No scheduler
lock was added to the shell; the descriptor fix reuses the kernel's existing
open-description synchronization and syscall scheduling paths.

## Validation

Host commands:

```sh
cargo test -p sunshell --no-default-features --features std --target x86_64-unknown-linux-gnu
cargo build -p sunshell --bin sshl --no-default-features --features std --target x86_64-unknown-linux-gnu
python3 tools/test_shell_phase2_host.py
cargo test --manifest-path tools/tests/fd-description/Cargo.toml --target x86_64-unknown-linux-gnu --target-dir target/fd-description --offline
```

If dash is outside PATH, set `SSHL_REFERENCE_SHELL` to its executable path.
The current host did not have dash installed; validation used `/bin/dash`
extracted into `/tmp` from the already cached Ubuntu 24.04 image. No shell
binary was added to the repository or production dependencies.

Completed host results: 19 library tests and 16 binary tests passed, including
seven shared-interpreter integration tests and existing pipeline/status tests.
23 supported-subset comparisons matched dash stdout and status, followed by
script-file arguments, dot sourcing, entry-point errors, and explicit rejection
checks for substitution, arithmetic, globs, and parameter operators. Those
rejection checks are not feature support. The descriptor harness passed 13
tests, including shared spawn/dup offsets and final-close/CLOEXEC/reap ownership.
Its pipe/eventfd adapters are inert stubs; it tests open-description logic,
not real kernel I/O. Native pipeline behavior is tested independently.

Native commands:

```sh
SUNLIGHT_TEST_CPUS=2 SUNLIGHT_TEST_SERIAL_LOG=target/shell-phase2-smp2.log bash tools/test.sh shell-phase2
SUNLIGHT_TEST_CPUS=1 SUNLIGHT_TEST_SERIAL_LOG=target/shell-phase2-smp1.log bash tools/test.sh shell-phase2
SUNLIGHT_TEST_CPUS=4 SUNLIGHT_TEST_SERIAL_LOG=target/shell-phase2-smp4.log bash tools/test.sh shell-phase2
SUNLIGHT_TEST_CPUS=4 SUNLIGHT_TEST_TIMEOUT=300 SUNLIGHT_TEST_SERIAL_LOG=target/shell-phase2-smp4-retry.log bash tools/test.sh shell-phase2
SUNLIGHT_TEST_CPUS=2 SUNLIGHT_TEST_SERIAL_LOG=target/shell-phase1_5-smp2.log bash tools/test.sh shell-phase1.5
```

The existing remote-shell repetition checks measure the **driver** process
heap/descriptor allocator, not the remote interactive shell. They establish
driver cleanup, not a complete interactive-shell leak audit. The added 100
success/failure redirection-scope runs execute the shared interpreter in the
driver itself and compare its heap and descriptor reuse there. Kernel serial
logs independently show spawned children being waited and reaped.

The new gate enters the real login shell, launches its driver, writes a script
fixture, then spawns `/bin/sh fixture args` with file-backed output. It checks
exact file bytes and status 23, `-c` positional naming/status, syntax errors,
functions/return, loop control, case, subshell state, compound redirection,
streaming native pipelines/final status, and repeated processes/scopes with
parent heap/descriptor reuse. The Phase 2 gate also runs the existing Phase 1.5 PTY workload through the
authorized PTY service path, including 100 repetitions, 1 MiB streams,
early-close/failed-launch tests and 64 KiB live output, then invokes `sh -c`
from that interactive PTY. Additional checks exercise recursion rejection,
native export rejection, regular-builtin recovery, and lack of shebang loading.

Native result tracking for this implementation:

| Check | Result |
| --- | --- |
| Kernel `cargo check` | Passed |
| Native userspace release build | Passed as part of the QEMU gate |
| Core Phase 2, 2 CPUs | Passed after generic VFS descriptor repair |
| Extended Phase 2 + PTY, 2 CPUs | Passed, including 64 KiB live output and script output witness |
| Extended Phase 2 + PTY, 1 CPU | Passed |
| Core Phase 2, 4 CPUs | Passed, including recursion/export/error recovery checks |
| Extended Phase 2 + PTY, 4 CPUs | Passed with 300-second deadline; initial 180-second run timed out during final live output |
| Phase 1.5 TTY/PTY regression, 2 CPUs | Failed before workloads: user-session driver lookup of `sshl6` denied |

The extended gate now defaults to 300 seconds, matching the successful
four-CPU retry. The initial timeout log is retained separately from that
successful run; a timeout is not counted as a pass.

The separate Phase 1.5 gate currently fails while resolving its sibling
`sshl6`: the driver inherits user-session lookup restrictions, which exclude
numbered-shell names. That policy is unchanged. The TTY IPC workload was not
reached and remains unverified; the Phase 2 gate exercises PTY independently.
Initial failing native execution is documented above; later success is not
inferred from compilation. QEMU evidence is kept under ignored `target/`.
Human interactive script editing, a separate Helios runtime regression, and
SSH-session execution have not been tested.

## Remaining work and SSH dependencies

1. Finish stage C: native environment-map handoff, scripting/function/compound
   pipeline children, context transport, options, and interactive continuation
   compatible with foreground/PTY output draining.
2. Finish stage D: tokenize structured substitutions/arithmetic, implement POSIX
   parameter operators, checked integer arithmetic, pathname expansion, and
   command substitution with a bounded spool/capture policy that reports limit
   errors while draining/terminating/reaping producers. Test expansion order and
   assignment/substitution status before adding more syntax.
3. Finish stage E: read/printf built-ins, exec descriptor persistence and process
   replacement, full dot/set/export/readonly/unset modes, signal-backed traps,
   timing support, and precise special/regular built-in error rules.
4. Replace remaining infallible allocations with a documented fallible strategy;
   extend native failure injection and descriptor/process/SMP regressions.
5. For SSH, connect authenticated session identity, inherited capabilities,
   stdio descriptors, cwd/environment/locale, and the noninteractive `-c` entry.
   Keep terminal registration out of exec sessions. Interactive SSH additionally
   needs PTY authority/winsize, foreground input forwarding, signal handling,
   clean disconnect cancellation/reaping, and backpressure tests. No SSH server
   integration or signal/job-control capability is claimed by this slice.
