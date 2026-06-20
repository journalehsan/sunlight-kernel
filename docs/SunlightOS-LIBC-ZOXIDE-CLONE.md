SunlightOS – Phase 2 Rust std Support and sunlight-zoxide Utility
Context
SunlightOS is a small experimental operating system with a Rust-based userspace.

A minimal libc-like compatibility crate called sunlight-libc already exists.

Phase 1 of Rust std enablement has been completed and validated with a small test program called sunlight-sunsay.

That program proved that the following stack works end-to-end:

kernel → loader → crt0 / _start → Rust main → println! → stdout → TTY

The system successfully executed a native Rust program using std.

Now we want to move to Phase 2.

Phase 2 has two goals:

Extend and stabilize sunlight-libc to support more of the Rust std runtime.
Build a real userland tool that heavily exercises filesystem, environment, and string processing.
The chosen tool is a directory jumping utility similar to zoxide, called:

Package: sunlight-zoxide

Binary: z

This tool will serve as both a useful userland command and a functional test of Rust std support.

Part 1 — Improve sunlight-libc for Phase 2
The existing sunlight-libc crate already provides:

basic syscall wrappers
errno mapping
file descriptor constants
basic file operations (open, read, write, close)
random number support via IPC
minimal compatibility layer for Rust std
Phase 2 should audit and improve the crate so that the following Rust std features work reliably.

Required Rust std capabilities
Ensure that the following APIs function correctly:

Filesystem
Rust APIs:

std::fs::read_to_string
std::fs::write
std::fs::OpenOptions
std::fs::File
std::fs::create_dir_all
Required libc/syscalls:

open
close
read
write
stat
mkdir
Confirm that:

file creation works
append mode works if available
truncate works if available
errors propagate through errno correctly
Paths
Ensure Rust path utilities work:

std::path::Path
std::path::PathBuf
joining paths
converting to strings
No special kernel work required, but verify compatibility with sunlight-libc string passing.

Environment variables
Rust APIs:

std::env::args
std::env::var
Verify:

argc/argv are passed correctly from _start
environment pointer handling works
If environment variables are not fully implemented yet, provide a minimal implementation.

Current directory
Optional but recommended:

Rust APIs:

std::env::current_dir
If possible implement support using:

getcwd
or

kernel equivalent.
If not yet supported, document the limitation.

Directory creation
Rust API:

std::fs::create_dir_all

Verify syscall:

mkdir
Ensure recursive creation behaves correctly.

Error propagation
Ensure that Rust IO errors map correctly from:

Errno
Common errors to verify:

NotFound
PermissionDenied
AlreadyExists
InvalidInput
File truncation
Used by tools that rewrite files.

Required syscall if available:

open with truncate flag
or

truncate
If not implemented, add minimal support.

Stability goals
Phase 2 libc should allow small CLI tools to reliably perform:

read configuration files
write configuration files
create directories
handle path strings
parse CLI arguments
Part 2 — Implement sunlight-zoxide
Overview
sunlight-zoxide is a simple directory history tool inspired by zoxide.

Binary name:

text
z
It remembers directories that the user visits and allows jumping to them using fuzzy matching.

This program will heavily exercise:

filesystem IO
string processing
CLI parsing
environment variables
Rust std collections
Storage
The program stores its database inside the user’s home directory.

Database path:

text
$HOME/.config/sunlight-zoxide/db.txt
If HOME is unavailable, fallback to:

text
/root/.config/sunlight-zoxide/db.txt
The program must ensure the directory exists:

text
$HOME/.config/sunlight-zoxide/
Use:

text
std::fs::create_dir_all
Database format
Use a simple line-based format.

Each line contains:

text
score<TAB>path
Example:

text
5	/root
12	/root/projects/sunlight
3	/root/projects/sunlight/kernel
Rules:

path must be absolute
score increases every time the path is visited
Commands
Add directory
text
z --add PATH
Behavior:

PATH must be absolute
increment score if already present
otherwise create new entry
This command is intended to be called automatically by the shell after a successful cd.

Output:

No output (silent success).

Resolve directory
text
z --resolve TERM...
Example:

text
z --resolve sunlight kernel
Behavior:

load database
search for paths containing all terms
match must be case-insensitive
if exactly one match exists:
Output:

text
/root/projects/sunlight/kernel
Return code:

text
0
If multiple matches exist:

Print an error to stderr and ask for more specific terms.

Return code:

text
2
If no matches exist:

Print:

text
z: no match found
Return code:

text
1
List database
text
z --list
Example output:

text
sunlight-zoxide database

score   path
-------------------------------
12      /root/projects/sunlight
7       /root/src/my-project
3       /tmp
Doctor command
text
z --doctor
Shows diagnostic information.

Example:

text
sunlight-zoxide doctor

db path: /root/.config/sunlight-zoxide/db.txt
db exists: yes
entries: 5
config directory writable: yes
Matching algorithm
Matching should be simple and deterministic.

Input terms:

text
z --resolve my project
Matching rule:

A path is a match if all terms appear in the path string.

Case-insensitive.

Example match:

text
/root/src/my-project
Scoring formula:

text
score = visit_score * 100 - path_length
The highest score wins.

If two candidates have identical scores, treat the result as ambiguous.

Integration with cd
The shell or cd utility should call:

text
z --add <path>
after a successful directory change.

Jump usage example:

text
z project
The shell should internally run:

text
z --resolve project
Capture stdout and pass the resulting path to cd.

Rust implementation requirements
The program should use only Rust std.

Expected APIs:

std::env::args
std::env::var
std::fs::read_to_string
std::fs::OpenOptions
std::fs::create_dir_all
std::io::Write
std::path::PathBuf
Vec
String
Avoid external crates for now.

Deliverables
The implementation should provide:

updates to sunlight-libc if required
new project sunlight-zoxide
binary z
build instructions
integration notes for cd
example usage scenarios
Validation
The following scenarios must work.

Add directories:

text
z --add /root/projects/sunlight
z --add /root/projects/sunlight/kernel
z --add /root/src/my-project
Resolve path:

text
z --resolve kernel
Expected output:

text
/root/projects/sunlight/kernel
List database:

text
z --list
Check diagnostics:

text
z --doctor
Goal
Phase 2 should demonstrate that SunlightOS can support real-world Rust CLI tools that depend on:

filesystem IO
configuration storage
environment variables
path manipulation
CLI argument parsing
sunlight-zoxide will serve both as a useful user tool and as a functional validation of the Rust std environment.
