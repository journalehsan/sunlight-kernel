# Git through Helios

This is an opt-in boot image port. The image exposes upstream Git as
`/bin/git` through the standard Linux ELF launch path. It adds no SunShell
dispatch rules or new grants. It is not yet a SunApp.

Build with `SUNLIGHT_HELIOS_GIT=1 bash tools/build.sh`. To build just the
payload, run `bash tools/build_helios_git.sh`. The script needs an x86-64
musl C toolchain, GNU make, binutils, curl and tar with xz support.

| Input | Upstream archive | SHA-256 |
| --- | --- | --- |
| Git 2.56.1 | https://www.kernel.org/pub/software/scm/git/git-2.56.1.tar.xz | `f0b0bf89264b9bd2e1c640363e7529643989da76281600473c50745cd34a607f` |
| zlib 1.4 | https://zlib.net/fossils/zlib-1.4.tar.xz | `4f11c512053d7bab8d9e9b89bde3575add3e4c50a383b42ef6cb837035fc5a7c` |

No upstream source patch is applied. The binary uses Git's C implementation,
bundled hash and regex code, musl and zlib. It omits curl, OpenSSL, Expat,
gettext, iconv, PCRE2, scripting tools and the built-in fsmonitor daemon.
The output is a stripped static x86-64 ELF executable (ET_EXEC), with no
interpreter or dynamic section. The build stamps it with the same HLNX01
marker used by Btop. The tested binary was built with musl 1.2.6 and had
SHA-256 `1df52e733ac5a90d5040f053644c94a5b0d45991455c7368460aabf4a6e9bf6d`.

Btop demonstrates both terminal bundle launch and native shell launch, but
does not exercise Git's worktree or child process lifecycle. Helios copies
argv and the exported environment into the Linux image. SunShell supplies
HOME and PATH; Git supports repository-local identity and normally uses
HOME/XDG_CONFIG_HOME for user configuration. The fixture uses repository-local
configuration, no pager, and paths under /tmp on a fresh diskless boot.
Git still expects a writable worktree, exclusive lockfile creation, rename
and unlink, same-directory temporary objects and usable metadata. Templates,
hooks, pagers, editors, filters and remote helpers can require separate files
or processes. This build ships only the executable; identity lookup without
explicit configuration and global config writes are unverified.

The first image with Git visible to the VFS server stopped at startup because
Git's standard descriptor sanitizer opens /dev/null read/write. The serial
trace recorded the immutable root denial. Helios now supplies a null device
descriptor with EOF reads and discarded writes, independent of filesystem
write policy. The next boot ran `git --version` with exit 0.

`git init /tmp/git-helios` still exits 128 after the directory creation step:
Git reports that it cannot change into the resulting directory. The Linux
mkdir shim mutates the kernel VFS; a caller with the inherited stat profile
uses the VFS service for chdir. The observed lookup failure is consistent
with that split. The service mounts a separate in-memory root from the same
boot seed. Its retained cwd and stat operations traverse that live mount;
they do not consult a cache of kernel directory entries. The successful
kernel mkdir therefore has nothing to invalidate in the service. The run4
trace shows an authenticated chdir completion with ENOENT; the later EAGAIN
cases instead failed request preparation.

`Vfs::mkdir_beneath` now provides a tested creation primitive for the
namespace owner: it resolves the parent under retained anchors, checks
search and write permission, checks write policy on the resolved path, and
inserts the directory before returning success. It is not wired to production
syscalls. Routing mkdir alone would strand subsequent open/create, rename,
unlink and directory enumeration on the kernel tree. Those operations need
to share the namespace owner before the routed profile can support this
workflow. The service's volatile root cannot reconstruct dynamic entries
from its boot seed after a restart.

Lock replacement, timestamps used for
index freshness, and durability on persistent filesystems remain unverified.
Git can fall back from hard links to rename or copy in some paths; that
fallback was not reached in the guest.

Run the test with
`python3 tools/test_helios_git.py target/sunlightos.iso target/git-helios/replay`.
Add `--accel kvm` where available. The harness uses the guest keyboard,
captures command output from the TTY serial trace, and attaches no writable
disk or NIC.

| Status | Guest coverage |
| --- | --- |
| PASS | Git version and normal process exit; shell fixture file creation and overwrite under /tmp/git-helios |
| FAIL | Init, repository identity configuration, add, commit, status, log, diff, branch and checkout exited 128; local clone could not find the source repository |
| BLOCKED | HTTPS requires a Git remote helper built with curl and a guest-visible CA bundle. SSH requires a compatible client and child launch. The current Linux fork/vfork policy rejects address space duplication; thread clone and exec of the current process do not replace it. |
| NOT_RUN | Fetch, pull, push, clone history, HTTPS handshake, SSH, credential prompting/helpers and persistent storage |

Helios provides socket, pipe, wait, signal and clock shims. The local run
verified normal TTY output and termination; it did not verify child pipe
behavior, interrupt cleanup, TCP or DNS. SunlightOS has native resolver and
TLS services, with trust roots managed through its certificate tooling.
There is no demonstrated bridge from Git's Linux TLS library to that trust
store, nor a validated Linux CA path for this image. Retain certificate
verification when adding HTTPS. Git's helper protocols and native credential
storage likewise have no integration in this port.
