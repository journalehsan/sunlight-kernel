# Helios ABI Translation & Library Shimming Strategy for SunlightOS

**Document status:** Architecture proposal — revised after design discussion  
**Revision:** 2.0  
**Target:** SunlightOS / SunlightX / Helios / `sunapp`  
**Initial compatibility target:** Linux x86-64 userspace ABI, musl-based executables

## 1. Executive Summary

**Helios** is SunlightOS's optional Linux binary compatibility environment. It runs supported Linux ELF applications on the SunlightX microkernel by preserving the Linux userspace ABI where required, translating Linux system calls to SunlightOS primitives and services, and optionally substituting compatible, tested native Rust library implementations.

Helios is **not** a Linux kernel, a system-wide Linux distribution, or a dependency of the SunlightOS desktop. The SunlightOS base system—including its core services and default applications—is native and Rust-based. Linux applications belong in self-contained **`sunapp` bundles**, isolated from the base system and from one another.

Each `sunapp` includes the executable, its required runtime and shared libraries, application resources, and package metadata. A compressed, preferably read-only package filesystem reduces storage usage and simplifies atomic replacement, rollback, and verification. Library shims are **optional and selected for an exact tested compatibility target**; an application may instead use its original bundled libraries.

Releases follow a distribution-style process: construct a reproducible candidate, execute functional/security/regression tests, observe the **same package artifact** in a testing channel for approximately two weeks, and promote it to the main repository and Store when the release criteria pass. An update creates a new tested `sunapp`; the Store does not need to inspect every installed application's libraries or perform system-wide dependency resolution.

**Design priorities:** ABI fidelity, capability-based isolation, predictable lifecycle semantics, independent application packaging, and measured—not assumed—performance improvements.

## 2. Architectural Invariants

1. **Native-first base:** SunlightOS boots and operates without Helios or any Linux application/library. Default system services and applications remain native Rust components.
2. **Compatibility is optional:** Disabling or removing Helios does not prevent native SunlightOS operation.
3. **Per-application ownership:** Linux binaries, loaders, libraries, and approved shims are packaged with, or immutably resolved by, their own `sunapp`. There is no requirement for a shared global Linux userspace.
4. **Original-library path always available when bundled:** A shim is an explicit, version-scoped implementation choice; it is not an automatic global replacement for a matching library name.
5. **Capabilities remain authoritative:** Linux credentials, namespaces, or a claimed UID of zero do not mint SunlightOS capabilities. All native-service access is mediated by SunlightX policy, `sun-exec`, and the applicable UAC/capability controls.
6. **FastLane is an optimization, not a correctness dependency:** Helios must work over the stable IPC path. An optimized FastLane-backed bridge is enabled only after its production security, ownership, cancellation, and retirement gates pass.
7. **Release what was tested:** Testing and stable channels must refer to the same immutable package content or content digest. Any change produces a new candidate and requires applicable revalidation.

## 3. Execution Architecture

```text
Compressed, immutable sunapp package
  |-- Linux ELF binary and resources
  |-- musl runtime / dynamic loader (if dynamically linked)
  |-- bundled original shared libraries
  |-- optional, explicitly selected ABI-compatible shims
  `-- manifest and release/test metadata
                 |
                 v
       sun-exec + Helios personality
          |           |            |
          |           |            +-- Vortex / Wayland bridge (later)
          |           +--------------- library ABI shim (optional)
          +--------------------------- Linux syscall translation
                                            |
                                   SunlightX-mediated access
                                            |
                                Native SunlightOS services
```

The Linux *process personality* identifies the executable's target ABI and governs Linux-specific syscall numbering, argument conventions, startup state, signals, resource semantics, and process lifecycle. It must be selected during ELF execution and remain correct across `fork`/`clone`/`execve` transitions, exits, and PID reuse.

Helios is deliberately split into three independently testable integration paths:

| Path | Responsibility | Requirement |
| --- | --- | --- |
| Syscall translation | Linux process, memory, file, networking, event, and threading behavior | Correctness baseline |
| Library shimming | Optional ABI-compatible entry points into Rust code or native services | Explicit opt-in per bundle and tested library version |
| Graphics bridging | Wayland/X11-facing compatibility with Vortex and relevant graphics/input protocols | Later application-enablement phase |

## 4. Linux ABI Contract and Scope

### 4.1 Target profile

The first compatibility profile is **Linux x86-64, musl-based**, with explicitly enumerated support for static and dynamic ELF executables. Other Linux ABIs—glibc, x32, i386, additional CPU architectures, and increasingly complex distribution expectations—are distinct future profiles, not implicit promises of the initial milestone.

The initial **467-syscall inventory** from the project plan is a census target to audit, not a claim of 467 implemented or universally required syscalls. Pin the source Linux UAPI revision, architecture, and table-generation rules. Maintain a generated, reviewed manifest with the exact number/name/ABI mapping and a status for each entry:

- `implemented`
- `emulated` (defined semantic equivalence)
- `partial` (documented limitations)
- `unsupported` (normally `-ENOSYS` if the call is absent)
- `policy_denied` (Linux-compatible permission error for an understood operation)

System calls must be prioritized by real application demand and semantic dependencies rather than raw table coverage.

### 4.2 Beyond system calls

Linux application compatibility also requires a process/runtime contract:

- ELF program headers, relocations, `PT_INTERP`, executable/loader entry, and startup stack;
- auxiliary vector (`auxv`), arguments, environment, TLS/FS-base setup, and stack conventions;
- signals, signal frames, `sigreturn`, process/thread identity, and lifecycle operations;
- file-descriptor inheritance, `CLOEXEC`, `fcntl`, `ioctl`, and readiness semantics;
- compatible filesystem paths and the selected `/proc`, `/dev`, and other runtime views;
- memory-map, shared-memory, atomic, futex, and scheduler-visible behavior;
- networking, sockets, polling, clocks, timers, and cancellation behavior.

A syscall returning success is not sufficient if its state transitions, flags, observable ordering, or error behavior differ from Linux.

### 4.3 Errors and fault containment

Unknown or genuinely unimplemented syscalls return Linux-compatible `-ENOSYS` (`-38` on the initial target ABI). This does **not** guarantee application-level fallback: the caller may require that feature or may not implement a fallback. Known but forbidden operations must use an appropriate permission error where Linux semantics demand one.

All user addresses, lengths, flags, handles, alignment constraints, and copy boundaries must be validated; invalid userspace pointers generally fail with `EFAULT` when specified by the syscall contract. An invalid request must not crash the kernel or corrupt shared state. The userspace application's own illegal memory accesses remain subject to normal fault/signal handling.

## 5. Incremental Roadmap

### H0 — ABI Definition and Linux Process Personality

- Freeze the initial Linux x86-64/musl target profile and generate the syscall inventory from pinned upstream sources.
- Define `sun-exec` identity, capability derivation, ELF startup, `auxv`, loader policy, signal behavior, and process inheritance.
- Document which Linux-visible `/proc`, `/dev`, and filesystem behaviors are virtualized.
- Define resource, credential, and failure boundaries before broad syscall implementation.

**Gate:** Stable ABI contract, threat model, traceability table, and a small reproducible executable-startup test set.

### H1 — Syscall Translation and Conformance Harness

- Implement the smallest coherent syscall groups needed to run real musl programs: process/memory, files, threads/futexes, signals, descriptors/events, time, and networking.
- Add explicit status and error handling for all inventoried syscall numbers; do not substitute blanket success or misleading `EINVAL` values.
- Run positive, negative, boundary, concurrency, and lifecycle tests against a pinned Linux reference as a differential oracle.
- Incorporate relevant Linux Test Project tests where feasible, with tests adapted to the scoped Helios contract.

**Gate:** A versioned compatibility matrix whose passing cases are backed by reproducible evidence, including invalid pointers, race conditions, and process retirement.

### H2 — Real Linux Application Bootstrapping

Bring up progressively demanding, *packaged* workloads:

1. Static and dynamic musl smoke binaries; BusyBox/Coreutils file and shell workflows.
2. Process, descriptor, TTY, threading, and nonblocking-I/O workloads, including Yazi where applicable.
3. Python and selected developer tools for heap pressure, filesystem behavior, threads, and process spawning.
4. Heavier compiler/runtime workloads such as `clang` and `rustc` after the underlying ABI and filesystem contracts are sufficiently mature.

Each accepted application has a documented `sunapp` package recipe, exact runtime dependencies, launch tests, and known limitations. Compilers are advanced stress targets, not prerequisites for initial compatibility.

### H3 — Optional Version-Scoped Library Shimming

- Support explicit library-selection metadata for a particular `sunapp` and ABI profile.
- Start with a bounded, well-documented library surface—such as selected `libpng` APIs—before complex TLS/crypto or broad graphics libraries.
- Compare ABI symbols, calling conventions, object ownership, callbacks, errors, and behavior against the original supported library build.
- Keep the original bundled library for a deterministic fallback **before execution** where feasible.
- Add new shim versions only with a matching compatibility profile and test evidence.

**Gate:** The exact targeted application/library pairing passes semantic and regression tests. A shim must not claim universal compatibility merely because its SONAME matches.

### H4 — Native Service Bridges and FastLane Optimization

- Allow a compatible in-process Rust shim for lightweight operations where it is simpler and faster.
- Allow a service-backed shim for large data, isolated decoding, or other workloads benefiting from native service boundaries.
- Reuse normal authenticated IPC until the relevant FastLane path is production-approved.
- Use shared-memory transfer only with explicit mapping rights, bounded leases, ownership tracking, cancellation handling, and verified retirement.
- Measure end-to-end latency, throughput, CPU, memory, and copies; do not assume zero-copy or Rust automatically outperform the original implementation.

### H5 — Vortex/Wayland Graphical Applications

- Provide the appropriate Wayland protocol bridge into Vortex, including surfaces, buffers, input, clipboard, window lifecycle, and synchronization.
- Treat X11 compatibility as a separate scope requiring its own implementation or bridge.
- Preserve sandbox boundaries for graphics buffers, device access, and privileged compositor operations.
- Package desktop programs as independently testable `sunapp` releases.

### H6 — Extended Compatibility

Expand on evidence of actual demand: broader library APIs, glibc-targeted ELF and symbol-version requirements, additional Linux facilities, and other architectures. Each remains an explicitly declared compatibility profile.

## 6. Library Shim Selection and Semantics

Shims are **package-scoped compatibility implementations**, not shared system-library upgrades. A `sunapp` author may bundle the original library, select an official Rust shim, or select a community-maintained shim that meets the same security and test policy. They may also package a fully native port instead of a Linux binary.

For each selected shim, the package should pin or otherwise immutably identify:

- architecture and libc/ELF compatibility profile;
- required SONAME and relevant ABI/symbol versions;
- original library version/build identity or supported compatibility range;
- shim implementation version and content identity;
- required dependent libraries and native-service permissions;
- conformance test profile and fallback library choice.

**Example manifest concept (illustrative, not a finalized schema):**

```toml
[package]
id = "org.example.viewer"
version = "2.4.1"
format = "sunapp"
filesystem = "compressed-readonly"

[helios]
abi = "linux-x86_64-musl"

[[helios.libraries]]
soname = "libpng16.so.16"
mode = "shim"
shim_id = "org.sunlightos.sun-image-png"
shim_version = "1.4.2"
fallback = "bundled-original"
```

The concrete loader/resolver must distinguish statically linked dependencies, which generally cannot be replaced through dynamic symbol interposition, from dynamically linked libraries. For dynamic ELF files, the relevant interpreter, `DT_NEEDED` entries, symbols, relocations, and `dlopen` behavior must be respected. Do not rely solely on `LD_PRELOAD` or on an unverified assumption that `repr(C)` proves ABI compatibility.

The original-library fallback is selected during load planning or a controlled relaunch. Replacing an initialized, stateful library after a runtime failure is not generally safe.

Rust may reduce memory-unsafety risks in a decoder, but shim FFI, pointer handling, service boundaries, and semantic changes still require hostile-input validation, fuzzing, and review. Critical libraries such as `libssl` require especially strong protocol/security conformance and should be deferred until the simpler approach is validated.

## 7. `sunapp` Packaging and Filesystem

Every Linux application is delivered as a self-contained `sunapp` with its executable, interpreter when needed, required original libraries, selected shims, configuration defaults, and resources. The image is compressed and normally mounted or exposed read-only; per-user settings, caches, and documents live in explicit writable locations controlled by sandbox policy.

Package-local dependency resolution prevents an application update from silently altering another application's ABI. The Store publishes complete, signed package artifacts and their metadata; it does **not** need to scan installed applications for library-update eligibility or maintain a system-wide Linux shared-library dependency graph.

Compression and caching decisions should be benchmarked on both constrained and capable hardware. Evaluate disk savings against startup latency, decompression CPU time, random-read behavior, memory pressure, and update size. The packaging system should support exact-content verification, atomic activation, and rollback to a previous accepted package.

## 8. Capability Security and `sun-exec`

Helios translates Linux requests into policy-controlled SunlightOS operations; it does not grant Linux processes native authority merely because their ABI permits a particular syscall.

- **Linux UID 0 is not SunlightOS root authority.** UID/GID visibility and ownership are virtualized or mapped deliberately; they never directly mint capabilities.
- Linux filesystem and device requests are checked against `sun-exec`'s granted resources and the authoritative SunlightOS capability/UAC model.
- Shim code is treated as application-supplied executable code, with no automatic privilege elevation. Native service invocations require their own authorized endpoint access.
- Trusted grants, process identity, generation/incarnation, revocation, and resource retirement must remain valid across `exec`, exit, crashes, and PID reuse.
- Package signing establishes origin and integrity, not a guarantee that a community shim is safe. Community contributions need declared privileges, review/test policy, and user-visible provenance.
- Application writable locations and host integration points are explicit; the default base-system filesystem remains protected.

## 9. Testing, Validation, and Release Policy

### 9.1 ABI conformance

Test the targeted behavior—not only whether a syscall returns or a library loads. Compare relevant outputs with a pinned Linux reference, including errors, state changes, blocking behavior, signals, file-descriptor rules, and concurrency ordering.

Use independent layers:

1. Unit tests for syscall arguments, structures, ABI translation, and mapping logic.
2. Differential and negative-path tests, including bad pointers, flags, lengths, and fault isolation.
3. Stress, fuzz, process-retirement, and multiprocess/multicore lifecycle gates.
4. Per-library ABI layout, symbol, callback, lifetime, and behavioral tests.
5. End-to-end `sunapp` workflows with realistic user operations.
6. Sandbox/capability and upgrade/rollback checks.

Report a test matrix and the limitations of its coverage. Avoid claims of **100% behavioral correctness**; the acceptance target is demonstrated compatibility against a defined versioned contract.

### 9.2 Release channels

```text
Upstream application release
   -> build a new self-contained sunapp candidate
   -> functional, ABI, security, and regression gates
   -> Testing repository / approximately 14-day observation
   -> final acceptance review
   -> promote the exact same immutable artifact to Store + Stable repository
```

For a Firefox-like application, the acceptance matrix should cover browsing, tab/window lifecycle, media, download/upload, networking/TLS, file pickers, resource usage over time, clean shutdown/relaunch, and sandbox permissions. A quiet two-week observation window supplements explicit testing; absence of bug reports alone is not proof of correctness.

If a blocking defect is found, publish a corrected candidate to Testing and re-run the applicable gates and observation period. Preserve the last accepted stable package for rollback. Promote by content identity rather than silently rebuilding or substituting untested libraries.

## 10. Developer and Community Ecosystem

Developers may distribute:

- **Native applications:** Rust-native programs using SunlightOS APIs and services directly.
- **Linux-compatible `sunapp` packages:** Unmodified supported Linux binaries with bundled original dependencies.
- **Shim-enabled `sunapp` packages:** Specific, tested ABI substitutions to Rust/native implementations.
- **Standalone shim packages or contributions:** Reusable compatibility implementations with declared version constraints, provenance, permission requirements, and test evidence. An application uses them only when its package build or resolver explicitly selects an approved compatible version.

The Store and repositories distribute versioned artifacts; they do not imply that a new shim is automatically injected into previously accepted packages. A package maintainer can incorporate an updated shim in a new candidate, test the complete package, and release it through the normal workflow.

## 11. Success Criteria

Helios is successful when:

1. The native SunlightOS base works independently of any Linux compatibility components.
2. A defined set of musl-based Linux ELF applications executes with correct observable behavior and stable process/resource lifecycle.
3. A `sunapp` installs, launches, updates, rolls back, and removes without changing shared Linux runtime state or compromising other applications.
4. Optional shims are ABI-qualified for exact targets, demonstrably secure within their scope, and measurably worthwhile where used.
5. Capability isolation holds even under malformed syscalls, hostile library inputs, concurrent teardown, and failed IPC operations.
6. Testing and stable channels preserve the identity of the validated artifact, with a practical path for regression diagnosis and rollback.

**Long-term objective:** A coherent, Rust-native SunlightOS desktop and server platform that can also run selected Linux applications seamlessly through Helios—without making Linux components part of the operating system's trusted base.

## References and Verification Baseline

- [Linux kernel syscall ABI and addition guidance](https://kernel.org/doc/html/next/process/adding-syscalls.html)
- [Linux x86-64 syscall table](https://github.com/torvalds/linux/blob/master/arch/x86/entry/syscalls/syscall_64.tbl)
- [Linux userspace ABI documentation](https://www.kernel.org/doc/html/latest/admin-guide/abi.html)
- [Linux `execve(2)` and ELF program interpreter](https://man7.org/linux/man-pages/man2/execve.2.html)
- [musl design concepts](https://wiki.musl-libc.org/design-concepts.html)
- [musl distribution guidelines](https://wiki.musl-libc.org/guidelines-for-distributions)
- [GNU dynamic linker symbol-versioning guidance](https://www.sourceware.org/glibc/manual/latest/html_node/Dynamic-Linker-Hardening.html)
- [libpng reference manual](https://github.com/pnggroup/libpng/blob/libpng18/manuals/libpng-manual.txt)
- [Linux Test Project](https://linux-test-project.readthedocs.io/en/latest/)

These references establish external ABI behavior; the versioned Helios implementation and acceptance matrix must identify the exact revisions actually tested.
