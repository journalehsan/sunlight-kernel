# SunlightOS Native Rust `std` Phase 1: Improve `sunlight-libc` and Build `sunlight-sunsay`

We are entering the **SunlightOS native Rust `std` support phase**.

The immediate goal is to improve `sunlight-libc` enough to build and run a small native Rust program using `std`, without breaking existing SunlightOS applications that currently depend on `sunlight-libc`.

This phase should produce a small, fun, visible proof-of-life application called **`sunlight-sunsay`**.

`sunlight-sunsay` is inspired by classic Unix toy programs such as `cowsay`, but it must be SunlightOS-native. It should print a fixed Sunlight-themed ASCII art frame and place a user-provided message inside the right-hand text area.

Example:

```sh
sunlight-sunsay "I said this"
```

If no message is provided, it should choose or use a built-in default quote/tip.

This application is intentionally simple and cheerful, but it is also a serious `std` smoke test. It should validate that SunlightOS can start a Rust `std` binary, pass arguments, print to the console, allocate memory, and exit cleanly.

---

## High-Level Architecture Requirements

### Keep `sunlight-libc` as the stable public compatibility layer

Do **not** replace `sunlight-libc` directly with another libc implementation.

`sunlight-libc` is the public ABI and compatibility layer for SunlightOS userland programs. Its exported symbols, syscall behavior, path policy, Sunlight-specific semantics, and compatibility guarantees must remain under SunlightOS control.

### Optional future backend support

If useful, prepare the internal structure so that another libc implementation, such as Redox OS `relibc`, can potentially be used later as an internal backend or reference implementation.

However:

- The exported ABI must remain SunlightOS-owned.
- Sunlight-specific behavior must remain implemented or mediated by `sunlight-libc`.
- Do not expose `relibc` directly as the official SunlightOS libc ABI.
- Do not make Phase 1 depend on fully integrating `relibc`.

### Kernel policy

Keep the kernel small.

Do not add broad POSIX behavior directly into the kernel unless the operation is truly primitive and belongs at kernel level.

Prefer the existing Sunlight architecture:

- syscalls for primitive kernel operations
- services / IPC for higher-level behavior
- userland compatibility logic in `sunlight-libc`

---

## Phase 1 Target

By the end of this phase, SunlightOS should be able to build and run a small native Rust `std` binary named:

```text
sunlight-sunsay
```

The binary should be able to:

1. Start through the Sunlight ELF loader.
2. Enter Rust `main` successfully.
3. Receive command-line arguments.
4. Use `std::env::args()`.
5. Allocate `Vec` and `String` values.
6. Print formatted ASCII art using `println!`.
7. Write to stdout through fd `1`.
8. Exit cleanly.

Filesystem support may be validated in a separate proof binary or an optional `--self-test` mode, but `sunlight-sunsay` itself should first remain focused on startup, argv, allocation, stdout, and clean exit.

---

## Existing Code Context

The current `sunlight-libc` appears to be a `#![no_std]` userland support crate with syscall wrappers and a small compatibility surface.

It already includes or partially includes:

- raw syscall wrapper layer
- basic file descriptor constants
- `open`
- `close`
- `read`
- `write`
- `stat`
- `mkdir`
- `pipe`
- `exec`
- `spawn`
- `waitpid`
- `exit`
- `sysinfo`
- `getrandom`
- basic Rust-side `Errno` enum mapping

It does **not** currently appear to provide all of the following pieces required for native Rust `std` support:

- crt0 / `_start`
- loader-to-userland `argc`, `argv`, `envp` handoff
- C ABI startup glue
- global `errno` storage
- POSIX-like negative return / errno convention if needed by Rust `std`
- memory functions such as `memcpy`, `memmove`, `memset`, `memcmp`
- allocator symbols such as `malloc`, `free`, `realloc`, `calloc`
- Rust global allocator backend
- `lseek`
- `fstat`
- `openat`
- `clock_gettime`
- stdout/stderr terminal mapping validation
- target JSON for `x86_64-unknown-sunlight`

Use the existing code as the base. Do not rewrite working pieces unnecessarily.

---

## Deliverable 1: Improve `sunlight-libc` Structure

Restructure or extend `sunlight-libc` so that it has clear internal modules for the following areas:

```text
sunlight-libc/
├── src/
│   ├── lib.rs
│   ├── sys.rs
│   ├── rand.rs
│   ├── crt0.rs        # or arch-specific startup module
│   ├── errno.rs
│   ├── mem.rs
│   ├── alloc.rs
│   ├── fd.rs
│   ├── time.rs
│   └── path.rs        # optional: path/policy helpers
```

The exact module layout may differ if the existing repository has a preferred structure, but the responsibilities should be clear.

Add comments around all ABI-sensitive areas, especially:

- syscall number assumptions
- `_start` ABI
- initial stack layout
- `argc` / `argv` / `envp` representation
- errno behavior
- allocator ownership and alignment guarantees
- stdout/stderr fd behavior
- path permission policy

---

## Deliverable 2: crt0 / `_start` Path

Implement or audit the program startup path.

The system must provide a stable userland entrypoint such as:

```rust
#[no_mangle]
pub unsafe extern "C" fn _start() -> ! {
    // decode loader-provided argc/argv/envp
    // call main(argc, argv, envp) or Rust runtime entry
    // exit with returned code
}
```

The exact implementation depends on the Sunlight ELF loader ABI.

Document the ABI precisely:

- Where is `argc` located?
- Where is `argv` located?
- Where is `envp` located?
- Are values passed on the stack, in registers, or through an auxiliary structure?
- Are strings NUL-terminated?
- Who owns the memory?
- Is the memory immutable?
- Are there auxiliary vector entries?
- What happens if `envp` is empty?

At minimum, support:

```text
main(argc, argv, envp)
```

where:

```c
int main(int argc, char **argv, char **envp);
```

For Rust `std`, ensure that command-line arguments can eventually be consumed by `std::env::args()` for the Sunlight target.

Do not break existing SunlightOS non-std applications.

---

## Deliverable 3: argc / argv / envp Handoff From Loader

Audit the Sunlight ELF loader and process-spawn path.

Ensure that when a program is executed with arguments:

```sh
sunlight-sunsay "I said this"
```

then the user process receives:

```text
argv[0] = "sunlight-sunsay"
argv[1] = "I said this"
argc = 2
```

If the loader currently does not pass this information, implement a minimal ABI for it.

Requirements:

- `argv` strings must be NUL-terminated.
- `argv[argc]` should be null if using conventional C layout.
- `envp` may initially be empty but should be well-formed.
- The ABI must be documented.
- Invalid or oversized argv data must be rejected cleanly.
- Existing `exec`/`spawn` behavior must remain compatible.

---

## Deliverable 4: Minimal Syscall / IPC Wrappers Needed by Rust `std`

Implement or audit the following wrappers in `sunlight-libc`.

### Required for Phase 1

- `exit`
- `read`
- `write`
- `open`
- `close`
- `fstat` or minimal equivalent
- `lseek` or minimal equivalent if required by the Rust `std` implementation
- `clock_gettime` if `SystemTime` or Rust runtime requires it

### Useful but can be staged

- `openat`
- `stat`
- `mkdir`
- `getrandom`
- `isatty`
- `poll` / `select` stubs if required later

If an operation is unsupported in Phase 1, return a clean unsupported error rather than panicking or silently succeeding.

---

## Deliverable 5: Errno Support

Implement a C-compatible errno path.

Current code has a Rust-side `Errno` enum and raw kernel return mapping. This is useful, but Rust `std` and C ABI compatibility typically need a more conventional errno mechanism.

Provide something equivalent to:

```c
int *__errno_location(void);
```

or the target-appropriate equivalent used by the Rust libc layer.

Requirements:

- Store errno in a stable location.
- Since threads are intentionally out of scope for Phase 1, a single global errno is acceptable initially.
- Add comments explaining that this is a Phase 1 non-threaded errno implementation.
- Convert syscall errors into errno values consistently.
- Avoid exposing raw kernel error encoding to normal application code.

Suggested initial errno mappings:

```text
Failed  -> EIO or EFAULT depending on context
Again   -> EAGAIN
Inval   -> EINVAL
TooBig  -> E2BIG
Unsupported -> ENOSYS
Permission denied -> EACCES or EPERM
Not found -> ENOENT
```

Use whatever error set exists in SunlightOS today, but document the mapping.

---

## Deliverable 6: Memory Functions

Implement or export the following C ABI symbols:

```c
void *memcpy(void *dst, const void *src, size_t n);
void *memmove(void *dst, const void *src, size_t n);
void *memset(void *dst, int c, size_t n);
int memcmp(const void *a, const void *b, size_t n);
```

Requirements:

- Must be `#[no_mangle]` / exported with C ABI where appropriate.
- Must handle overlapping memory correctly for `memmove`.
- Must be safe with `n = 0`.
- Add simple unit/regression tests where possible.
- Keep implementations simple and reliable.

These symbols are commonly required by compiler-generated code, Rust `compiler_builtins`, C ABI glue, or linked dependencies.

---

## Deliverable 7: Allocator Path

Implement the minimum allocator path required for Rust `std` and `Vec/String`.

Provide C ABI allocator symbols:

```c
void *malloc(size_t size);
void free(void *ptr);
void *realloc(void *ptr, size_t new_size);
void *calloc(size_t count, size_t size);
```

Also provide or connect a Rust global allocator backend if the Rust target requires it.

Requirements:

- Back allocator with the existing Sunlight heap primitive, `mmap`, `sbrk`, or equivalent service call.
- If no heap syscall exists, implement the smallest kernel/service primitive required to grow user memory, but avoid adding broad policy to the kernel.
- Ensure reasonable alignment for Rust allocations.
- `calloc` must check multiplication overflow.
- `realloc(NULL, size)` must behave like `malloc(size)`.
- `realloc(ptr, 0)` may free and return null, but document the behavior.
- `free(NULL)` must be a no-op.
- Add comments explaining Phase 1 limitations.

For Phase 1, a simple bump allocator may be acceptable only if:

- it is clearly documented as temporary,
- `free` behavior is documented,
- it does not break expected Rust `std` behavior for the proof binary.

Prefer a real minimal allocator if practical.

---

## Deliverable 8: stdout / stderr Mapping

Validate that fd `1` and fd `2` are connected to the Sunlight console/TTY service.

Requirements:

- `write(1, buf, len)` should display on the console.
- `write(2, buf, len)` should display on the console or stderr lane if supported.
- `println!` from a Rust `std` binary must work.
- Partial writes should be handled correctly or documented.
- Invalid fds should return a clean error.

`sunlight-sunsay` primarily validates this path.

---

## Deliverable 9: Minimal Time Support

If Rust `std` requires time support during startup or for `SystemTime`, implement one of:

```c
int clock_gettime(clockid_t clockid, struct timespec *tp);
```

or an equivalent target hook used by Rust `std` on SunlightOS.

For Phase 1, acceptable clocks:

- realtime from existing `sysinfo.unix_time`
- monotonic from existing uptime if available

Requirements:

- Return seconds and nanoseconds in a conventional `timespec` shape.
- If nanosecond precision is not available, set `tv_nsec = 0`.
- Document precision limitations.
- Unsupported clock IDs should return `EINVAL`.

---

## Deliverable 10: Filesystem and Path Policy

For Phase 1, ensure the following behavior is either implemented or explicitly documented:

- `/tmp` is writable by normal users.
- `/home/<user>` is writable by the owning user.
- root/system paths are rejected unless the process has the required capability.

Do not put complex POSIX policy directly into the kernel unless it is already part of the Sunlight design.

Prefer service-level or libc-level mediation where appropriate.

This is not the primary blocker for `sunlight-sunsay`, but it is required for broader `std` support and later proof binaries.

---

## Deliverable 11: Target JSON

Create:

```text
targets/x86_64-unknown-sunlight.json
```

Start conservatively.

Suggested properties:

- architecture: `x86_64`
- OS: `sunlight`
- environment: custom/native
- panic strategy: `abort`
- no dynamic linking initially
- static linking initially
- disable unsupported features
- use the correct linker / linker script for SunlightOS
- make sure the entry symbol matches the implemented crt0 path

Example skeleton, adjust for actual SunlightOS ABI:

```json
{
  "llvm-target": "x86_64-unknown-none",
  "arch": "x86_64",
  "os": "sunlight",
  "vendor": "unknown",
  "env": "",
  "target-endian": "little",
  "target-pointer-width": "64",
  "target-c-int-width": "32",
  "data-layout": "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128",
  "executables": true,
  "linker-flavor": "ld.lld",
  "linker": "rust-lld",
  "panic-strategy": "abort",
  "disable-redzone": true,
  "features": "-mmx,-sse,+soft-float",
  "relocation-model": "static",
  "code-model": "kernel",
  "dynamic-linking": false
}
```

The above is only a starting point. Validate it against the actual SunlightOS userland ABI. In particular, check whether `code-model = kernel` is appropriate for userland. If not, use the correct userland code model.

---

## Deliverable 12: Build Instructions

Document how to build the native Rust `std` proof binary.

Expected command:

```sh
cargo +nightly build \
  -Z build-std=core,alloc,std,panic_abort \
  --target targets/x86_64-unknown-sunlight.json
```

If building from a workspace, document the exact package command, for example:

```sh
cargo +nightly build \
  -Z build-std=core,alloc,std,panic_abort \
  --target targets/x86_64-unknown-sunlight.json \
  -p sunlight-sunsay
```

Also document:

- how to copy the binary into the SunlightOS image
- how to run it from the SunlightOS shell or init test path
- expected output
- known limitations

---

# `sunlight-sunsay` Application Specification

Create a new Rust binary crate:

```text
std-proof/sunlight-sunsay/
├── Cargo.toml
└── src/
    └── main.rs
```

Package name:

```text
sunlight-sunsay
```

Binary name:

```text
sunlight-sunsay
```

## Behavior

If arguments are provided, join them with spaces:

```sh
sunlight-sunsay "I said this"
```

or:

```sh
sunlight-sunsay I said this
```

Both should produce a message equivalent to:

```text
I said this
```

If no arguments are provided, print a built-in quote/tip.

For Phase 1, the default quote may be deterministic. Do not require randomness initially.

Suggested built-in quotes:

```text
Build slowly. Boot proudly.
Small kernels, bright userlands.
Keep the kernel tiny and the ideas huge.
Every syscall is a promise. Keep it honest.
SunlightOS says: no magic, just clean layers.
If it boots, it speaks. If it speaks, it lives.
Undefined behavior fears the sunlight.
Powered by tiny syscalls and suspicious optimism.
```

## Required ASCII Art

Use this ASCII art frame as the base:

```text
.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.
.            _.,.__       .                                   .
.           ((o\\o\))     . Tip:                              .
.     .-.    `  \\``      .                                   .
.  __(   )___.o"^^".,___  .                                   .
.     ===    ~~~~~~~~     .                                   .
.      ==             ldb .                                   .
.       =                 .                                   .
.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.
```

The right-hand message area should contain the wrapped text.

Example output:

```text
.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.
.            _.,.__       .                                   .
.           ((o\\o\))     . Tip:                              .
.     .-.    `  \\``      . I said this                       .
.  __(   )___.o"^^".,___  .                                   .
.     ===    ~~~~~~~~     .                                   .
.      ==             ldb .                                   .
.       =                 .                                   .
.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.
```

## Implementation Requirements

Use normal Rust `std` APIs:

- `std::env::args`
- `Vec<String>`
- `String`
- `println!`
- formatting macros

Avoid external dependencies.

Keep it simple and robust.

Suggested implementation:

```rust
use std::env;

const WIDTH: usize = 33;

const QUOTES: &[&str] = &[
    "Build slowly. Boot proudly.",
    "Small kernels, bright userlands.",
    "Keep the kernel tiny and the ideas huge.",
    "Every syscall is a promise. Keep it honest.",
    "SunlightOS says: no magic, just clean layers.",
    "If it boots, it speaks. If it speaks, it lives.",
    "Undefined behavior fears the sunlight.",
    "Powered by tiny syscalls and suspicious optimism.",
];

fn main() {
    let message = message_from_args();
    let lines = wrap_text(&message, WIDTH);
    print_frame(&lines);
}

fn message_from_args() -> String {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        // Phase 1: deterministic fallback.
        // Later this may use getrandom(), SystemTime, or Sunlight sysinfo.
        QUOTES[0].to_string()
    } else {
        args.join(" ")
    }
}

fn print_frame(lines: &[String]) {
    println!(".-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.");
    println!(".            _.,.__       .                                   .");
    println!(".           ((o\\\\o\\))     . Tip:                              .");

    print_art_line(".     .-.    `  \\\\``      .", lines.first());
    print_art_line(".  __(   )___.o\"^^\".,___  .", lines.get(1));
    print_art_line(".     ===    ~~~~~~~~     .", lines.get(2));
    print_art_line(".      ==             ldb .", lines.get(3));
    print_art_line(".       =                 .", lines.get(4));

    for line in lines.iter().skip(5) {
        print_art_line(".                         .", Some(line));
    }

    println!(".-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.");
}

fn print_art_line(left: &str, text: Option<&String>) {
    match text {
        Some(text) => println!("{} {:<width$} .", left, text, width = WIDTH),
        None => println!("{} {:<width$} .", left, "", width = WIDTH),
    }
}

fn wrap_text(input: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in input.split_whitespace() {
        if current.is_empty() {
            push_word_or_split(word, width, &mut current, &mut lines);
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = String::new();
            push_word_or_split(word, width, &mut current, &mut lines);
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

fn push_word_or_split(
    word: &str,
    width: usize,
    current: &mut String,
    lines: &mut Vec<String>,
) {
    if word.len() <= width {
        current.push_str(word);
        return;
    }

    let mut start = 0;

    while start < word.len() {
        let mut end = usize::min(start + width, word.len());

        while end > start && !word.is_char_boundary(end) {
            end -= 1;
        }

        if end == start {
            break;
        }

        lines.push(word[start..end].to_string());
        start = end;
    }
}
```

If formatting or escaping differs due to Rust string literal behavior, adjust the source while preserving the visual output.

---

# Suggested Incremental Test Plan

## Stage 0: Static output smoke test

Before enabling full argv support, optionally create a temporary hardcoded version that only prints:

```text
Hello from sunlight-sunsay!
```

inside the frame.

This validates:

- ELF loading
- `_start`
- Rust `main`
- `println!`
- `write(1, ...)`
- clean `exit`

## Stage 1: argv support

Run:

```sh
sunlight-sunsay "I said this"
```

Validate:

- `argc`
- `argv`
- `std::env::args()`
- `Vec<String>`
- `String::join`
- allocator

## Stage 2: wrapping support

Run:

```sh
sunlight-sunsay "This is a longer message that should wrap across multiple lines inside the SunlightOS ASCII art frame."
```

Validate:

- allocation under a slightly larger workload
- formatting macros
- repeated stdout writes

## Stage 3: default quote

Run:

```sh
sunlight-sunsay
```

Validate:

- no-argument path
- built-in default message

## Stage 4: optional filesystem self-test

Later, add:

```sh
sunlight-sunsay --self-test
```

This may test:

- write `/tmp/std-proof.txt`
- read it back
- print `fs: ok`

Do not make this mandatory for the first successful boot of `sunlight-sunsay`.

---

# Regression Tests

Add tests where practical.

Suggested tests:

## libc-level tests

- `memcpy` copies correctly
- `memmove` handles overlap forward and backward
- `memset` fills correctly
- `memcmp` returns expected ordering
- `calloc` zeroes memory
- `realloc` preserves old contents
- `free(NULL)` does nothing
- syscall wrappers return errors consistently
- errno is set consistently on failing operations

## startup tests

- process receives correct `argc`
- `argv[0]` exists
- `argv[1]` matches provided argument
- empty `envp` is well-formed

## `sunlight-sunsay` tests

If host-side tests are possible:

- `wrap_text` wraps short text correctly
- `wrap_text` wraps long text correctly
- long words do not panic
- empty input produces one empty line

---

# Explicit Non-Goals for Phase 1

Do **not** attempt to implement the following in this phase unless absolutely required by the minimal proof binary:

- threads
- `std::thread`
- `std::process`
- dynamic linking
- shared libraries
- `std::net`
- sockets
- full POSIX signals
- full terminal control
- fork/exec POSIX completeness
- locale
- full `stdio` implementation
- complex permissions model
- complete `relibc` integration

Keep Phase 1 small and focused.

---

# Acceptance Criteria

Phase 1 is successful when the following works on SunlightOS:

```sh
sunlight-sunsay "I said this"
```

and prints something visually equivalent to:

```text
.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.
.            _.,.__       .                                   .
.           ((o\\o\))     . Tip:                              .
.     .-.    `  \\``      . I said this                       .
.  __(   )___.o"^^".,___  .                                   .
.     ===    ~~~~~~~~     .                                   .
.      ==             ldb .                                   .
.       =                 .                                   .
.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.
```

Also:

```sh
sunlight-sunsay
```

must print the same frame with a built-in quote.

The binary must exit cleanly.

Existing SunlightOS applications that use `sunlight-libc` must continue to build and run.

---

# Notes for Implementers

- Prefer small, reviewable patches.
- Keep ABI-sensitive changes documented.
- Do not silently change syscall numbers or structure layouts.
- If a syscall enum mirrors the kernel, keep comments warning that both sides must stay synchronized.
- Make unsupported operations fail cleanly with documented errno values.
- Avoid placing compatibility policy in the kernel unless it is truly primitive.
- Use Sunlight services/IPC for higher-level behavior where appropriate.
- Treat `sunlight-sunsay` as both a toy and a serious native Rust `std` milestone.

The spirit of this phase is:

```text
Small kernel.
Clear ABI.
Growing libc.
Native Rust std.
A tiny ASCII creature proving the whole stack is alive.
```
