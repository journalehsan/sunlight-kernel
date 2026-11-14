# SunlightOS Application Architecture

**Status:** Architectural direction; implementation in progress  
**Scope:** Native applications, Helios compatibility, sunapp bundles, Python, and execution authorization

## 1. Purpose

SunlightOS provides two complementary application paths:

1. A native path for software developed specifically for SunlightOS.
2. A compatibility path through Helios for bringing existing Linux software to SunlightOS.

The goal is to expand software availability without making the native SunlightOS application platform dependent on Linux compatibility.

## 2. Core Architectural Decisions

- Applications belonging to SunlightOS itself are distributed and managed outside the sunapp format.
- All third-party applications use the **sunapp** bundle format, including native applications.
- A sunapp may target either the native SunlightOS environment or Helios.
- Third-party status and execution environment are separate concepts: a native application is not automatically a system application.
- Execution requires valid authorization. The presence of an executable file or a bundle is not sufficient.

## 3. Native Platform and Helios

### 3.1 Native SunlightOS

The native platform is the foundation for SunlightOS's own software and for new applications that directly target its APIs and toolkit.

The planned native Python path consists of a native Python runtime and bindings to the SunlightOS toolkit. This allows developers to write new Python applications without depending on GTK, Qt, or the Helios compatibility environment.

Native third-party applications are still packaged as sunapps.

### 3.2 Helios

Helios is a user-space compatibility environment running in ring 3. Its purpose is to expand application availability by supporting Linux software, including compatible Linux binaries.

The design is conceptually comparable to a Linux compatibility facility rather than merely a graphics abstraction layer. Exact compatibility is determined by the interfaces and behavior Helios implements.

Helios may also be used for command-line software when appropriate; it is not restricted to graphical applications.

Helios supports software compatibility while the native SunlightOS platform remains a distinct development target.

### 3.3 musl and Binary Compatibility

A Helios–musl environment is a potential build and runtime target. However, supporting musl does not by itself provide compatibility with binaries built against glibc.

Two workflows must be distinguished:

- Rebuilding software for a controlled Helios–musl environment.
- Running existing Linux binaries with the loader, libraries, and operating-system behavior they expect.

The supported runtime combinations and compatibility guarantees remain to be specified. No universal Linux binary compatibility is implied by this document.

## 4. sunapp: Third-Party Application Bundles

### 4.1 Role

sunapp is the distribution format for all third-party applications, not a synonym for a Helios application.

```text
SunlightOS software
├── SunlightOS-owned applications and components
│   └── Managed outside sunapp
└── Third-party applications
    └── sunapp
        ├── Native SunlightOS application
        └── Helios application
```

### 4.2 Bundled Dependencies

A sunapp contains its application-specific runtime dependencies. For a Python application targeting Helios, these include its own Python runtime, Python packages, native extension modules, and any required GTK or Qt libraries.

The application should not rely on another application's Python installation or private libraries.

Bundled dependencies do not eliminate the host contract: applications still depend on the declared execution environment and the system services that environment exposes.

The exact manifest schema, directory layout, dependency-resolution rules, and host-provided interface allowlist are not yet finalized.

## 5. Graphics Architecture

### 5.1 Existing Applications

Existing GTK- and Qt-based Linux applications are intended to run through Helios, with their required toolkit libraries and bindings included in their sunapps.

For the planned Wayland path, the application uses its toolkit's Wayland backend and client libraries to communicate with the SunlightOS display service.

```text
Helios sunapp
┌──────────────────────────────────┐
│ Application                      │
│ Python runtime, if required      │
│ PyGObject / PySide / PyQt         │
│ GTK / Qt                         │
│ Wayland backend and client code  │
└────────────────┬─────────────────┘
                 │
       Helios-supported communication
                 │
                 ▼
       SunlightOS display service
```

The application acts as a Wayland client; it does not execute “inside” a Wayland client.

GTK and Qt are not inherently Linux-only or Wayland-only. Using their Linux builds and Wayland backends through Helios is the selected integration direction for this architecture, not a limitation of the toolkits themselves.

**Current status:** The Wayland/display integration discussed here has not yet been implemented. Transport, handle transfer, shared buffers, input, and rendering integration still require implementation and validation.

### 5.2 New Native Applications

New applications may target the SunlightOS toolkit directly. Python developers will use native toolkit bindings rather than requiring the GTK/Qt compatibility path.

This creates two complementary workflows:

- Write new software against the native SunlightOS platform.
- Package existing compatible software for execution through Helios.

## 6. Python Strategy

### 6.1 Native Python

SunlightOS will have a native Python runtime. Bindings to the SunlightOS toolkit will provide a native GUI development path.

Binding implementation details remain open, including object ownership, callback behavior, UI-thread restrictions, and event-loop integration.

### 6.2 Python in Helios Bundles

A Python application packaged for Helios carries its own compatible Python runtime and dependencies.

```text
Application code
      ↓
Bundled CPython
      ↓
Bundled Python bindings and native extensions
      ↓
Bundled toolkit and supporting libraries
      ↓
Helios execution environment
```

CPython, extension modules, and supporting libraries must agree on architecture, ABI, and runtime environment. A Linux extension module cannot be assumed to load into native SunlightOS Python merely because Helios can execute Linux programs.

This separation permits native and Helios Python environments to evolve without mixing incompatible binary packages.

## 7. Execution Authorization

Execution is capability-controlled: code without the required execution authorization does not run.

SunlightOS's own programs have execution tokens. Software installed through the package manager also receives execution authorization through that trusted installation path.

A bundle manifest cannot grant execution authority merely by declaring itself trusted or system-owned.

The detailed token format, binding to installed content, issuance rules, revocation behavior, and update semantics are outside the current specification.

### 7.1 Interpreters and Compatibility Runtimes

The authorization model must account for indirect execution through Python, Helios, launchers, and dynamically loaded code.

**Design requirement:** Authorization to execute a runtime must not automatically turn arbitrary payloads into trusted applications with that runtime's authority.

The enforcement mechanism remains to be designed. Bundle-level authorization is a possible approach; this document does not require a separate token for every script or library.

Likewise, execution authorization should not be confused with unrestricted resource access. Filesystem, display, network, and other service permissions require explicit policy.

## 8. Bundle Build Tooling

A straightforward, PKGBUILD-inspired recipe system is planned for producing sunapps.

The intended developer workflow is:

1. Declare application metadata, sources, and target environment.
2. Build or assemble the application and its dependencies.
3. Collect the required files into the bundle.
4. Generate runtime metadata and validate the result.
5. Produce a sunapp suitable for distribution and authorized installation.

The build recipe and runtime manifest serve different purposes: building may require executable scripting, while launching should consume defined application metadata.

The recipe syntax and tool names are not yet finalized.

Recommended validation includes dependency completeness, ABI consistency, accidental host-library dependencies, and redistribution-license obligations.

## 9. Filesystem and Current Development Status

Work on libc and Python porting has already been undertaken. Filesystem limitations encountered with FAT blocked further progress.

SunFS is being implemented with POSIX compatibility as a design goal. Python porting is planned to resume after the necessary filesystem support is available.

Filesystem validation should cover the semantics needed by the intended Python workflows, such as path handling, file metadata, directory traversal, renaming, permissions, and symbolic links where required.

Running the interpreter is only one milestone. Loading native Python extensions is an additional important validation step for future toolkit bindings and third-party packages.

## 10. Open Implementation Questions

The architectural direction is established, but the following details remain open:

- sunapp manifest format and runtime-version declarations.
- Supported Helios libc and loader combinations.
- Library search rules and isolation between bundles.
- Native Python distribution and versioning policy.
- Native toolkit binding design.
- Wayland transport, buffer sharing, and graphics integration.
- Execution-token lifecycle and indirect-execution enforcement.
- Application data, configuration, and cache locations.
- Build-recipe syntax and reproducibility requirements.

## 11. Summary

**Native software gets a native development platform. Existing compatible Linux software gets a Helios execution path. All third-party applications are distributed as sunapps, and execution requires valid authorization.**

This preserves a distinct SunlightOS platform while using compatibility to broaden the available software ecosystem.
