# SunlightOS Capability and UAC Elevation Audit

**Audit date:** November 3, 2026  
**Scope:** Current source in this SunlightOS checkout, including the kernel, UAC service, Vortex desktop, and dialog services.  
**Method:** Read-only source inspection. No tests or runtime validation were performed.

## Scope note

No separate SunlightX component was identified in this checkout. This audit covers the SunlightOS components present here. Recheck the scope if SunlightX refers to another repository or product.

## Findings

### 1. Capabilities and grants

The kernel tracks several forms of authority. Some capabilities use bearer-token semantics, while narrower grants can be bound to a recipient process identity and limited by expiry, rights, or one-time consumption. A token's presence or a nonzero value is not, by itself, proof that an operation is authorized.

The repository's capability authority design and remaining legacy boundaries are described in [`docs/ipc/fastlane-f11p-capability-authority.md`](ipc/fastlane-f11p-capability-authority.md).

### 2. Authenticated user-session spawn

The kernel has an authenticated session spawn path. It consumes a session grant bound to the calling process and currently restricts this route to `/bin/sshl`. This establishes a user session; it is not a general root-elevation mechanism. See [`kernel/src/arch/x86_64/syscall.rs`](../kernel/src/arch/x86_64/syscall.rs), around `handle_spawn_call` and `SPAWN_AUTHENTICATED`.

### 3. UAC-to-VFS grants

There is a narrower UAC-to-VFS authorization path. A delivered root-password proof can be exchanged for a short-lived grant to execute one exact path or a limited base executable path. The grant is bound to its recipient and consumed once by VFS. This authorizes a VFS operation; it does not cause a process to run with elevated credentials. See [`kernel/src/capability/vfs_grants.rs`](../kernel/src/capability/vfs_grants.rs) and the `AUTHORIZE_EXEC` handling in [`services/vfs_server/src/main.rs`](../services/vfs_server/src/main.rs).

### 4. Current `runas` behavior

The `runas` client prompts for a password and calls the password-verification operation. It then sends a separate `OP_RUNAS` IPC request containing caller and target UIDs, checks whether root's UAC path rules allow the target executable, and launches it with ordinary `spawn`.

The ordinary spawn API does not take an elevated identity. Its documentation says credentials and capabilities remain inherited. Therefore, this path does **not** demonstrate that the child runs as root, despite the `runas` comments and messages describing elevation. See [`services/sunlight-uac/src/bin/runas.rs`](../services/sunlight-uac/src/bin/runas.rs) and [`sunlight-libc/src/lib.rs`](../sunlight-libc/src/lib.rs).

The `OP_RUNAS` handler also passes message-supplied UIDs into the generic session model. That model is explicitly a mock: a missing session is immediately created as a prompted session, without verifying credentials itself. The successful password check occurs separately in the client. Treat the mock session response as bookkeeping, not as an authorization grant. See [`services/sunlight-uac/src/session.rs`](../services/sunlight-uac/src/session.rs) and [`services/sunlight-uac/src/bin/uac_service.rs`](../services/sunlight-uac/src/bin/uac_service.rs).

### 5. GUI authorization

The dialog host provides general-purpose application dialogs. The searched desktop and dialog code did not show a `pkexec` or polkit implementation. A normal application-created password window is not a trusted authorization surface: another application could imitate it, and a compromised requester could misrepresent the action being authorized.

The current dialogs are useful UI infrastructure, but their result must not itself mint or convey privilege. A system-owned broker must validate the request and issue authority independently of the UI response.

## Security gaps to resolve

1. **No demonstrated kernel-mediated elevated launch.** UAC approval, executable checking, and ordinary process spawn are separate operations. A successful check does not change the child process identity.
2. **Caller and target identity must be authoritative.** Privileged decisions must use kernel-authenticated caller identity, not UIDs supplied in request words.
3. **Authentication and authorization need one bound request.** The password proof, requester, requested action, target executable, and eventual grant must be connected by kernel-validated identity and lifecycle data.
4. **Executable permission is not elevation permission.** `OP_CHECK_EXEC` evaluates a path rule. It does not prove authentication or authorize a credential change.
5. **Legacy bearer paths remain a separate review area.** Do not treat the presence of newer recipient-bound grants as proof that every legacy capability or VFS path has equivalent restrictions.
6. **The GUI prompt needs a trusted presentation path.** The requesting app may initiate an authorization request, but only a system-controlled authorization flow should collect credentials and return a grant.

## Recommended design direction

1. Define UAC requests in terms of a concrete operation and target, not a requested UID. Obtain the caller identity from the kernel. Bind the request to the caller's process incarnation and VM generation, executable identity, requested action, and expiry.
2. Have the kernel consume a one-use authorization grant atomically at the privileged operation or spawn boundary. Do not make a userspace `uid=0` field, a successful password check in an unrelated call, or a path check sufficient to change credentials.
3. Prefer narrow privileged services for tasks such as protected file updates. Grant only the operation and resource required. Add general elevated process launch only after its policy, credential transition, inherited handles/capabilities, and lifecycle semantics are specified.
4. Make the system authorization broker own prompt creation and credential handling. The app can request authorization and show a non-authoritative status, but it cannot supply the trusted prompt's result or mint authority.
5. Define explicit expiry, cancellation, replay, revocation, audit-log redaction, and failure behavior. Never log passwords or raw bearer tokens.
6. Validate the actual end-to-end path separately from host/model tests: authenticated requester, trusted prompt, grant binding, kernel consumption, resulting process identity and rights, and cleanup after cancellation or exit.

## Suggested implementation order

1. Document and audit current spawn credential and capability inheritance, including legacy spawn message forms.
2. Specify a small UAC authorization request/reply protocol with kernel-authenticated requester identity and a one-use action grant.
3. Add kernel-side consumption at the operation boundary. Start with one narrow privileged operation before general root process launch.
4. Integrate a system-owned GUI authorization prompt over authenticated IPC; keep the prompt response separate from the authority token.
5. Rework `runas` to use the approved protocol and accurately report the resulting identity. Add a Run dialog entry point only after that protocol works for terminal and GUI callers.
6. Consider a `pkexec`-style command frontend as a client of the same protocol. Do not add a second policy engine or a parallel authority registry.

## Conclusion

The repository has useful security building blocks: kernel-managed capability state, authenticated user-session spawn, password verification, and recipient-bound one-use UAC-to-VFS grants. However, the current `runas` path does not connect UAC approval to an elevated kernel spawn, and the generic GUI dialog path is not a trusted authorization mechanism. The next design milestone should connect a system-owned authorization flow to kernel-enforced, narrowly scoped, one-use authority, then validate the real caller-to-operation path.
