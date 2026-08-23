# Helios Linux x86_64 syscall coverage

Source-derived from `compat-linux` and `kernel/src/arch/x86_64/syscall.rs`.
Classification is the behaviour userspace observes, not merely "the number is
dispatched".

Legend: I = IMPLEMENTED, P = PARTIAL, S = STUB, U = UNSUPPORTED, M = MISSING.

| Nr | Name | Class | Implementation | Native primitive | Flags / notes | Probe | Note? | sbase echo? |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | read | I | native `sys_read` | VFS/TTY/pipe | EAGAIN on empty TTY | tier1 | Y | Y |
| 1 | write | I | native `sys_write` | VFS/TTY/pipe | O_APPEND honoured | tier1 | Y | Y |
| 2 | open | I | `open_resolved_path` | VFS open/create | see open-flag table | runtime | Y | |
| 3 | close | I | native `sys_close` | fd table + VFS close | | tier1 | Y | |
| 4 | stat | I | rewritten to newfstatat | VFS stat | AT_FDCWD | runtime | Y | |
| 5 | fstat | I | `sys_fstat` Linux layout | VFS fstat_handle | synthetic st_dev/st_ino; times 0 | runtime | Y | |
| 6 | lstat | P | newfstatat + AT_SYMLINK_NOFOLLOW | VFS stat | no symlinks, same as stat | | | |
| 7 | poll | P | `sys_linux_poll` | TTY/pipe ready | timeout via scheduler | | Y | |
| 8 | lseek | I | native `sys_lseek` | fd offset + fstat | ESPIPE on non-VFS | runtime | | |
| 9 | mmap | P | `sys_linux_mmap` | anonymous mmap | PRIVATE\|ANONYMOUS only | tier1 | Y | |
| 10 | mprotect | I | native | VMM | | | Y | |
| 11 | munmap | I | native | VMM | | tier1 | Y | |
| 12 | brk | I | `sys_brk` | process brk region | | tier1 | Y | |
| 13 | rt_sigaction | P | records disposition | LinuxProcessState | no handler delivery | | Y | |
| 14 | rt_sigprocmask | P | records mask | LinuxProcessState | no async delivery | | Y | |
| 16 | ioctl | P | termios / winsize | LinuxProcessState + TTY | 80x25 synthetic winsize | | Y | |
| 17 | pread64 | I | `sys_linux_pread64` | VFS read(offset) | does not move fd offset; ESPIPE | runtime | | |
| 18 | pwrite64 | I | `sys_linux_pwrite64` | VFS write(offset) | O_APPEND writes at EOF; ESPIPE | runtime | | |
| 20 | writev | I | `sys_linux_writev` | sys_write loop | | | Y | |
| 21 | access | I | faccessat AT_FDCWD | VFS stat + check_permission | F_OK/R_OK/W_OK/X_OK | runtime | | |
| 22 | pipe | I | native | kernel pipe | | | | |
| 24 | sched_yield | I | native | scheduler | | | | |
| 32 | dup | I | native fd_table.dup | fd table | clears CLOEXEC | tier1 | Y | |
| 33 | dup2 | I | native fd_table.dup2 | fd table | | | | |
| 35 | nanosleep | I | `sys_linux_nanosleep` | timer block | timespec validated | tier1 | | |
| 39 | getpid | I | native | process.pid | | | Y | |
| 41 | socket | U | ENOSYS | | out of scope | | | |
| 53 | socketpair | P | pipe self-pipe | kernel pipe | AF_UNIX SOCK_STREAM only | | Y | |
| 56 | clone | U | ENOSYS | | out of scope | | | |
| 57 | fork | U | ENOSYS | | out of scope | | | |
| 58 | vfork | U | ENOSYS | | out of scope | | | |
| 59 | execve | P | native exec of embedded ELFs | spawn/exec | no PT_INTERP | | | |
| 60 | exit | I | native | process exit + fd cleanup | | tier1 | Y | Y |
| 61 | wait4 | U | ENOSYS | | ABI mismatch with native waitpid | | | |
| 62 | kill | P | native kill | signal queue | no Linux handler frames | | | |
| 63 | uname | I | `sys_linux_uname` | Helios identity | not a Linux distro | runtime | | |
| 72 | fcntl | I | native | fd flags | F_GETFD/SETFD/GETFL/SETFL/DUPFD[_CLOEXEC] | tier1, runtime | Y | |
| 79 | getcwd | I | native | process.cwd | | | | |
| 80 | chdir | I | native | process.cwd | | | | |
| 82 | rename | I | native | VFS rename | | | | |
| 83 | mkdir | I | mkdirat AT_FDCWD | VFS mkdir | EEXIST mapped | runtime | | |
| 87 | unlink | I | native | VFS unlink | | runtime | | |
| 89 | readlink | I | readlinkat AT_FDCWD | VFS stat | EINVAL if exists (no symlinks) | runtime | | |
| 96 | gettimeofday | I | `sys_linux_gettimeofday` | RTC unix time | timezone written as zeros | runtime | | |
| 102 | getuid | I | native | process.uid | | runtime | | |
| 104 | getgid | I | native | process.gid | | runtime | | |
| 107 | geteuid | I | native getuid | process.uid | no euid split | runtime | | |
| 108 | getegid | I | native getgid | process.gid | no egid split | runtime | | |
| 110 | getppid | I | native | process.ppid | | runtime | | |
| 131 | sigaltstack | P | records stack | LinuxProcessState | no handler frames | | Y | |
| 158 | arch_prctl | I | ARCH_SET_FS | FS_BASE | | | Y | |
| 186 | gettid | I | native getpid | pid | single-threaded | | Y | |
| 200 | tkill | P | same-pid only | kill | | | | |
| 202 | futex | U | ENOSYS | | out of scope | | | |
| 213 | epoll_create | I | epoll pool | epoll | | | Y | |
| 217 | getdents64 | I | `sys_linux_getdents64` | VFS read_dir | synthetic ino; `.`/`..` added | runtime | | |
| 218 | set_tid_address | P | stores pointer | LinuxProcessState | no clear_child_tid | | Y | |
| 228 | clock_gettime | I | realtime/monotonic | RTC + monotonic_ns | other clock ids EINVAL | runtime | Y | |
| 231 | exit_group | I | process exit | | | | Y | |
| 232 | epoll_wait | P | collect_ready + timeout | epoll | pwait mask ignored | | Y | |
| 233 | epoll_ctl | I | epoll ctl | epoll | | | Y | |
| 257 | openat | I | dirfd resolve + open | VFS | AT_FDCWD, absolute, dir fd | runtime | Y | |
| 258 | mkdirat | I | dirfd resolve + mkdir | VFS mkdir | | runtime | | |
| 262 | newfstatat | I | dirfd resolve + Linux stat | VFS stat | AT_SYMLINK_NOFOLLOW no-op | runtime | Y | |
| 263 | unlinkat | P | dirfd resolve + unlink | VFS unlink | AT_REMOVEDIR ENOSYS | | | |
| 264 | renameat | I | dirfd resolve + rename | VFS rename | | | | |
| 267 | readlinkat | I | dirfd + stat | VFS stat | EINVAL if exists | runtime | | |
| 269 | faccessat | I | dirfd + permission | VFS stat + check_permission | AT_EACCESS no-op | runtime | | |
| 273 | set_robust_list | P | stores pointer | LinuxProcessState | no robust futex | | Y | |
| 281 | epoll_pwait | P | epoll_wait | epoll | sigmask ignored | | Y | |
| 291 | epoll_create1 | I | epoll_create | epoll | EPOLL_CLOEXEC | | Y | |
| 292 | dup3 | I | fd_table.dup3 | fd table | O_CLOEXEC only; same fd EINVAL | runtime | | |
| 293 | pipe2 | I | pipe + flags | kernel pipe | O_CLOEXEC / O_NONBLOCK | | | |
| 318 | getrandom | I | entropy::fill | kernel entropy | unknown flags EINVAL | tier1 | Y | |
| 334 | rseq | S | ENOSYS | | libc disables rseq | | Y | |

Unlisted numbers return `-ENOSYS`.
