"""Regenerate the source-audit matrix; statuses are not runtime test results."""
import csv
from pathlib import Path
P=Path(__file__).resolve().parent
K='kernel/src/arch/x86_64/syscall.rs'
T='compat-linux/src/lib.rs:375-471'
rows=[]
def add(feature,need,h,n,e): rows.append([feature,need,h,n,e,'UNKNOWN: no new guest execution; see probes/results/results.csv'])
def shared(feature,need,fn):
    lines=Path(K).read_text().splitlines()
    line=next(i for i,s in enumerate(lines,1) if ('fn '+fn+'(') in s)
    add(feature,need,'PARTIAL: translated to native handler','PARTIAL: handler exists; C ABI wrapper not established',T+'; '+K+':'+str(line))
for a,b,c in [('open','imports/files','sys_open'),('read','I/O','sys_read'),('write','I/O','sys_write'),('lseek','buffered I/O','sys_lseek'),('fstat','I/O metadata','sys_fstat'),('rename','pyc/install','sys_rename'),('unlink','files/tempfiles','sys_unlink'),('mkdir','directories','sys_mkdir'),('fcntl','FD flags/locks','sys_fcntl'),('dup','FD duplication','sys_dup'),('dup2','subprocess stdio','sys_dup2'),('pipe','subprocess/control','sys_pipe'),('munmap','allocator','sys_munmap'),('mprotect','memory permissions','sys_mprotect'),('execve','subprocess replace image','sys_exec'),('getpid','os/random','sys_getpid'),('getppid','os','sys_getppid'),('getuid/geteuid','posix/site','sys_getuid'),('getgid/getegid','posix/site','sys_getgid'),('getcwd','startup/imports','sys_getcwd'),('chdir','posix','sys_chdir')]: shared(a,b,c)
for a,b,fn,n in [
('openat','imports/dir_fd','sys_linux_openat','UNKNOWN: no native openat interface found'),
('pread','posix positional I/O','sys_linux_pread64','UNKNOWN: C/native equivalent unverified'),
('pwrite','posix positional I/O','sys_linux_pwrite64','UNKNOWN: C/native equivalent unverified'),
('stat/lstat/newfstatat','imports metadata','sys_linux_newfstatat','PARTIAL: stat_path; no symlink type'),
('getdents64','listdir/scandir','sys_linux_getdents64','PARTIAL: readdir native ABI'),
('dup3','CLOEXEC duplication','sys_linux_dup3','UNKNOWN: use dup2 plus flags only if atomicity irrelevant'),
('pipe2','CLOEXEC/nonblock pipe','sys_linux_pipe2','PARTIAL: pipe native ABI'),
('poll/ppoll','select/asyncio','sys_linux_poll','PARTIAL: native syscall 159 is pipe polling only'),
('epoll','selectors/asyncio optimization','sys_linux_epoll_wait','UNKNOWN: no native epoll API found'),
('nanosleep/clock_nanosleep','time.sleep','sys_linux_nanosleep','PARTIAL: Rust sleep uses timed futex'),
('set_tid_address','musl pthread exit','sys_linux_set_tid_address','PARTIAL: native thread lifecycle differs'),
('arch_prctl/TLS','musl TLS','sys_arch_prctl','PARTIAL: native set_fs_base syscall 101'),
('uname','os.uname/platform','sys_linux_uname','UNKNOWN: native POSIX uname unavailable in probes')]:
    lines=Path(K).read_text().splitlines();line=next(i for i,s in enumerate(lines,1) if ('fn '+fn+'(') in s)
    add(a,b,'PARTIAL: Linux shim exists; semantics require guest probes',n,T+'; '+K+':'+str(line))
for f,need in [('statx','optional metadata fallback'),('ftruncate','truncate/tempfile/sqlite'),('fsync','durability/install/sqlite'),('select/pselect6','select module fallback'),('alarm/setitimer','signal timers'),('getrlimit/setrlimit/prlimit64','resource/stack limits')]:
    add(f,need,'MISSING: no translation arm; falls to SHIM_ENOSYS','UNKNOWN: no matching POSIX C interface established',T+'; '+K+':1146; probes/results/inventory-18.log')
add('readlink/readlinkat','venv/import path discovery','MISSING: existing paths return EINVAL; not a link resolver','MISSING in Rust std: readlink/symlink unsupported',K+':6870-6893; tools/native-std/overlay/fs/sunlightos.rs:334-339')
add('ioctl TCGETS','REPL/termios','PARTIAL: stdio fd 0..2; writes 60-byte struct, audit musl ABI size','UNKNOWN: no native POSIX termios layer',K+':8453-8474')
add('ioctl FIONREAD','buffered readiness','MISSING: no FIONREAD branch found','UNKNOWN: no native ioctl interface','command: rg -n FIONREAD kernel/src/arch/x86_64/syscall.rs (no matches); '+K+':8333-8574')
add('mmap anonymous','allocator/arenas/stacks','PARTIAL: private anonymous; demand backed for non-PROT_NONE','PARTIAL: private anonymous; eager allocation','kernel/src/process/mmap.rs:216-239; kernel/src/process/mmap.rs:287-305; kernel/src/process/mmap.rs:420-478')
add('mmap file/MAP_SHARED','mmap module/shared memory/dynamic loader','MISSING: requires anonymous/private','MISSING: backed mappings rejected','compat-linux/src/lib.rs:480-497; kernel/src/process/mmap.rs:230-233')
add('PROT_EXEC / W^X','ctypes callbacks/dynamic code; optional core','PARTIAL: RX supported; RWX rejected','PARTIAL: RX supported; RWX rejected','kernel/src/process/mmap.rs:180-213')
add('mremap','musl realloc optimization','MISSING: no translation arm','MISSING: handler returns InvalidFlags',T+'; kernel/src/process/mmap.rs:1560-1567')
add('brk','musl allocator fallback','PARTIAL: Linux handler exists','UNKNOWN: native allocator uses mmap',K+':5907; sunlight-libc/src/alloc.rs:1')
add('clock_gettime REALTIME/MONOTONIC','time/deadlines','PARTIAL: both; realtime whole seconds','PARTIAL: both; realtime whole seconds',K+':7246-7277; '+K+':11515-11546')
add('getrandom','hash seed/secrets','PARTIAL: shim exists; entropy readiness must be exercised','PARTIAL: Rust entropy runtime; C interface unverified',K+':6946; sunlight-libc/src/rand.rs:1')
add('futex','GIL/pthread locks','PARTIAL: sys_linux_futex','PARTIAL: native syscall 155',K+':1159; '+K+':1010')
add('clone/pthread','required threads/GIL','PARTIAL: thread flags accepted; process clone rejected','PARTIAL: native spawn/join; pthread C layer not found','compat-linux/src/clone.rs:19-54; '+K+':3176-3345; tools/native-std/overlay/thread/sunlightos.rs:12-35')
add('fork','subprocess/multiprocessing fork','MISSING: rejected ENOSYS','MISSING: rejected',K+':3161-3167')
add('vfork','posix_spawn/subprocess optimization','MISSING: rejected ENOSYS','UNKNOWN: no vfork API',K+':1156; '+K+':3170-3173')
add('wait4/waitpid','subprocess/reaping','MISSING: Linux wait4 explicitly rejected','PARTIAL: waitpid has different return/status contract',K+':6012-6017; '+K+':1021')
add('sigaction/sigprocmask','signal module/runtime','PARTIAL: Linux action/mask storage; no user handler entry','MISSING: native functions are stubs',K+':8065-8175; '+K+':10375-10386')
add('sigaltstack','stack overflow handling optional','PARTIAL: stores Linux state; no demonstrated handler use','UNKNOWN: no native sigaltstack API',K+':8227; '+K+':369-376')
add('kill/signal main thread','KeyboardInterrupt/termination','PARTIAL: queue/default actions; user handler not called','PARTIAL: kill queues signals; no native user sigaction',K+':369-376; '+K+':10392-10420')
add('rt_sigreturn','signal handler return','MISSING: no Linux translation','MISSING: native sigreturn stub',T+'; '+K+':10433-10435')
add('rseq','optional libc optimization','MISSING deliberately: ENOSYS fallback','UNKNOWN: not a native CPython core requirement',K+':6942-6943')
add('sysconf-like info','page size/CPU count/limits','PARTIAL: AT_PAGESZ and affinity; full sysconf UNKNOWN','PARTIAL: sysinfo; available_parallelism unsupported','kernel/src/process/spawn.rs:350-359; '+K+':6078; '+K+':11549-11557; tools/native-std/overlay/thread/sunlightos.rs:49-50')
add('socket AF_UNIX','socketpair/local IPC','PARTIAL: SOCK_STREAM only','UNKNOWN: no POSIX C socket interface',K+':7567-7584')
add('socket AF_INET/AF_INET6/DNS','network socket/ssl/pip/asyncio','MISSING: EAFNOSUPPORT','PARTIAL: service IPC networking; std::net unsupported',K+':7579-7580; docs/rappid-rabbit-native-runtime.md:53-59')
for f in ['/proc/self/*','/dev/null','/dev/urandom','/dev/tty']:
    add(f,'paths/introspection/devices; some have fallbacks','UNKNOWN: no implementation in scoped source search','UNKNOWN: no implementation in scoped source search','probes/results/inventory-14.log; verify guest open/read/write/readlink and boot mount table')
add('thread-local destructors','pthread keys/extension runtimes','UNKNOWN: pthread guest probe not run','PARTIAL: 255 usable Rust keys; five destructor passes','tools/native-std/overlay/thread_local/key/sunlightos.rs:6-51; probes/results/results.csv')
add('stack growth/size','recursion/threads','PARTIAL: initial 2 MiB fixed stack; automatic growth UNKNOWN','PARTIAL: initial/default thread stack 2 MiB; growth UNKNOWN','kernel/src/process/layout.rs:1-6; tools/native-std/overlay/thread/sunlightos.rs:7; kernel/src/process/spawn.rs:384')
add('large mmap/overcommit','allocator/large objects','PARTIAL: demand region reservation; OOM policy/load test UNKNOWN','PARTIAL: eager backing; bounded reclaim and rollback','kernel/src/process/mmap.rs:420-478; probes/mmap.c:1')
with (P/'syscall-matrix.csv').open('w') as f:
    w=csv.writer(f);w.writerow(['syscall/feature','needed by CPython','Helios status','Native status','evidence','guest verification']);w.writerows(rows)
print(len(rows),'rows')
