# Workspace targets and checks

This workspace contains Ring 0 code, freestanding native services, host tools,
and applications that use the pinned SunlightOS std overlay. Cargo's
`--workspace` flag selects one target and unifies ordinary dependency features;
it cannot assign a different target or allocator to each executable.

The repository retains `x86_64-unknown-none` as its default for existing kernel
and service build scripts. A bare `cargo check --workspace` therefore cannot
check std applications. Overriding the target to the host for every executable
also fails: several native binaries define panic handlers and use native-only
modules. Changing panic handlers or substituting host stubs to satisfy that
command would not validate the deployed programs.

Run the complete check with platform and binary feature isolation:

```sh
python3 tools/check_workspace.py
```

This checks all workspace libraries together on the host (including additive
host/native features), then checks every package's binaries separately on its
supported target. Required binary features are enabled. SVG applications and
the native brain/Rabbit executables use `tools/build_native_std.sh`, which
stages the pinned overlay without modifying rustup. The kernel check may
prepare its embedded applications, including downloaded pinned dependencies,
just as a normal kernel build does. The first run can take several minutes.

Useful individual commands:

```sh
# Combined library graph; includes every workspace member with a library.
cargo check-host-libs
# Kernel and legacy freestanding service checks retain the default target.
cargo check -p sunlight-kernel
# Native std service and CLI.
bash tools/build_native_std.sh check -p wiseowl-brain --no-default-features --features sunlightos
# Host regression suites (unit, integration, and documentation tests).
cargo test -p wiseowl-brain --all-features --target x86_64-unknown-linux-gnu
cargo test -p sunlight-js -p rappid-rabbit -p golden-fish --features rappid-rabbit/js-dom-fixture --target x86_64-unknown-linux-gnu
```

`cargo test --workspace` on the freestanding default is not a host test command.
Pass the host target and select crates with host test harnesses. Native boot
gates continue to use `tools/test.sh`; they are separate from type checking.
