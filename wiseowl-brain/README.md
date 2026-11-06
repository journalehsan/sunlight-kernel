# Wise Owl brain

Wise Owl brain is a bounded userland cognitive service. It does not run in
Ring 0. Its shared library also supports freestanding clients such as the
Wise Owl graphical console.

There are three supported configurations:

| Configuration | Target and features | Runtime |
| --- | --- | --- |
| Host development | host target, default `host` feature | Unix socket daemon/CLI, serde protocol types |
| SunlightOS service | `x86_64-sunlightos`, `--no-default-features --features sunlightos` | Native std startup, TLS, System allocator, panic/exit; capability IPC |
| Freestanding library/legacy binaries | `x86_64-unknown-none`, `--no-default-features --features sunlightos` | `core`/`alloc`, explicit startup and allocator |

Build the native service with the pinned std overlay:

```sh
./tools/build_wiseowl_brain.sh
# Additional QEMU gate features are forwarded:
./tools/build_wiseowl_brain.sh --features executor-v1-test
```

The binaries are written to `target/x86_64-sunlightos/release/`. The kernel
embeds those paths; the boot/build/test scripts use this helper. The native
service keeps its capability IPC transport and bounded pipeline. It does not
use the host Unix socket transport, which native std does not implement.

Host checks and tests, including the workspace's additive feature combination:

```sh
cargo check -p wiseowl-brain --all-targets --all-features --target x86_64-unknown-linux-gnu
cargo test -p wiseowl-brain --all-features --target x86_64-unknown-linux-gnu
cargo clippy -p wiseowl-brain --all-targets --target x86_64-unknown-linux-gnu --no-deps
cargo fmt -p wiseowl-brain -- --check
```

Platform selection is based on both the target OS and the `sunlightos` feature
(the build script emits the checked internal `sunlight_native` cfg). Enabling
`sunlightos` on a host does not select native startup or issue SunlightOS
syscalls. Host protocol types explicitly import `alloc` collections, and
`BrainError` implements `core::error::Error` in every configuration. The
`sunlight-networkd` library dependency disables its default allocator feature;
std applications own allocator selection.

For all workspace libraries and binaries, use `python3 tools/check_workspace.py`.
See [workspace target guidance](../docs/workspace-checks.md).
