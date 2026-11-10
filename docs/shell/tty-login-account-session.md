# TTY login account-session handoff

TTY authentication could succeed while the screen showed no working shell:

```text
[TTY]  Login success: root (uid=0, gid=0)
[SPAWN] Request from pid=4 for path=/bin/sshl0 ... caps=0x40000
[TTY]  Spawning /bin/sshl FAILED
[TTY]  Built-in shell ready
```

`tty_server::spawn_tab` still used `SPAWN_AUTHENTICATED`, which consumed the
one-use authentication grant without creating an account session. The spawn
handler's account-attachment check then refused the launch before ELF loading.
Desktop login already registered an account session through sessiond.

TTY login now sends `SESSION_CREATE_TTY` to sessiond. Only the kernel-attested
TTY service may call it. The request carries a tab number and the delivered
login proof; sessiond chooses the shell path and registers the proof with the
account broker. `SPAWN_SESSION_INITIAL` derives credentials and session quota
membership from that registration. It grants the existing UserSession profile.
The desktop manifest and graphical component startup are not involved.

Sessiond retains a bounded set of TTY registrations and checks each login
shell's PID and address-space generation. When the shell exits, dropping its
registration guard revokes session authority. Failed handoffs also revoke their
registration. TTY launch failures leave the login form visible with an error.

Run the regression with:

```sh
SUNLIGHT_TEST_SERIAL_LOG=target/tty-login-serial.log bash tools/test.sh tty-login
cargo test -p sunlight-sessiond --lib --target x86_64-unknown-linux-gnu
cargo check -p sunlight-sessiond -p sunlight-tty-server
```

The guest gate requires account-session launch, shell registration and actual
`whoami`, `id`, `sysfetch`, and `hostnamectl` execution. It does not hard-code a
shell PID. Like other injection gates, it builds an ISO with automatic keyboard
input; rebuild the normal kernel and ISO for interactive use.

Validation on 2026-11-10: the two service checks and all 27 sessiond library
tests passed. A replay of the freshly built injection ISO with two CPUs and
2 GiB RAM passed every TTY expectation, including a second shell tab and `id`
exit status zero. Its command, serial log, and verdict are retained under
`target/tty-login-replay-2g-final/`. The initial 1 GiB run launched the shell
and ran `whoami`, but subsequent external utilities encountered `NoMemory`;
`tty-login` therefore defaults to 2 GiB while respecting an explicit memory
override. The old Phase 3.9 `id` expectation included supplementary groups
that the current utility does not print; this gate checks actual UID/GID and
successful exit instead.
