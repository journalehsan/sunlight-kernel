SunlightOS libc Expansion Roadmap (FreeBASIC Compatibility Layer)
The goal of this stage is to evolve sunlight-libc from a minimal Rust runtime into a small POSIX-compatible libc shim capable of running simple C-style runtimes such as FreeBASIC.

Implementation should proceed in the following order.

SECTION 1 — POSIX ABI Bridge (posix.rs)

Create a new module:

src/posix.rs

This module exposes POSIX-compatible symbols that wrap existing SunlightOS syscalls.

Map the standard POSIX file descriptors:

0 → stdin

1 → stdout

2 → stderr

Implement the following exported functions.

read

#[no_mangle]

pub unsafe extern “C” fn read(

fd: i32,

buf: *mut u8,

count: usize

) -> isize

Behavior:

Call the kernel syscall sys_read(fd, buf, count) and return its result.

write

#[no_mangle]

pub unsafe extern “C” fn write(

fd: i32,

buf: *const u8,

count: usize

) -> isize

Behavior:

Call sys_write.

exit

#[no_mangle]

pub unsafe extern “C” fn exit(status: i32) -> !

Behavior:

Call sys_process_exit(status).

The function must never return.

SECTION 2 — Memory Functions (string.h compatibility)

FreeBASIC and many C runtimes rely heavily on the standard memory functions.

Expose the following symbols.

memcpy

#[no_mangle]

pub unsafe extern “C” fn memcpy(

dest: *mut u8,

src: *const u8,

n: usize

) -> *mut u8

Implementation should reuse the internal helper memcpy_bytes.

memset

#[no_mangle]

pub unsafe extern “C” fn memset(

dest: *mut u8,

value: i32,

n: usize

) -> *mut u8

Implementation should reuse memset_bytes.

memcmp

#[no_mangle]

pub unsafe extern “C” fn memcmp(

a: *const u8,

b: *const u8,

n: usize

) -> i32

Reuse memcmp_bytes.

strlen

Implement a classic C-style strlen:

#[no_mangle]

pub unsafe extern “C” fn strlen(s: *const u8) -> usize

Loop until the first null byte.

FreeBASIC relies heavily on strlen during string formatting.

SECTION 3 — Allocator Upgrade (alloc.rs)

FreeBASIC performs frequent dynamic allocation for strings.

The current bump allocator will fail because free() does not reclaim memory.

Short-term solution:

Increase heap size:

const HEAP_SIZE: usize = 16 * 1024 * 1024;

This prevents out-of-memory crashes during testing.

Long-term solution:

Replace the bump allocator with a simple linked-list block allocator.

Suggested design:

struct BlockHeader {

size: usize,

free: bool,

next: *mut BlockHeader

}

malloc:

find first free block large enough

free:

mark block as free

optional:

coalesce adjacent free blocks

Exports required:

#[no_mangle]

pub unsafe extern “C” fn malloc(size: usize) -> *mut u8

#[no_mangle]

pub unsafe extern “C” fn free(ptr: *mut u8)

SECTION 4 — Runtime Entry (crt0.rs)

Provide a proper C runtime entry compatible with C-style programs.

Declare:

extern “C” {

fn main(

argc: i32,

argv: *const *const u8,

envp: *const *const u8

) -> i32;

}

Implement _start.

Responsibilities:

Obtain argc and argv from the kernel.
Construct envp (may be empty for now).
Call main(argc, argv, envp).
Pass the return value to exit().
Pseudo flow:

_start

-> parse argc/argv

-> envp = null

-> let code = main(argc, argv, envp)

-> exit(code)

Validation Tests

After implementation, verify the following programs run correctly.

Test 1 — basic write

C program:

int main() {

write(1, “hello\n”, 6);

return 0;

}

Expected output:

hello

Test 2 — memory functions

Program using memcpy/memset.

Test 3 — malloc/free loop

Allocate and free strings repeatedly.

Test 4 — FreeBASIC runtime

Compile a minimal FreeBASIC program:

print “hello from freebasic”

The program should run successfully on SunlightOS.

Success Criteria

sunlight-libc exports the following POSIX symbols:

read

write

exit

memcpy

memset

memcmp

strlen

malloc

free

Programs using C-style runtimes must start through _start and call main() normally.
