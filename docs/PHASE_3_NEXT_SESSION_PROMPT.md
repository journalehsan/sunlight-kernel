# Phase 3 Next Session Prompt

Paste this into a fresh coding-agent chat from the repo root:

```text
We are in /home/ehsantor/Projects/sunlightos-kernel.

Please continue SunlightOS Phase 3.0 only. Do not start Phase 3.5 or Phase 3.6.

Read these first:
- docs/PHASE_3_ROADMAP.md
- docs/PHASE_3_0_SUMMARY.md
- tools/tests/phase3_0.expected

Current Phase 3.0 state:
- Phase 2.6 baseline was committed as:
  b309956 feat: harden IPC foundation for Phase 3
- Phase 3.0 has started but is not complete.
- sunlight-fs exists and is in the workspace.
- sunlight-fs already has tested RamFs and enum-backed Vfs support.
- Unit tests passed with:
  cargo test --target x86_64-unknown-linux-gnu --package sunlight-fs
- cargo check --workspace passed.
- services/vfs_server has not been added yet.
- tools/test.sh phase3.0 is not implemented yet.

Your next step:
1. Check git status.
2. Preserve existing uncommitted Phase 3.0 work.
3. Add services/vfs_server.
4. Register vfs_server as "vfs" through the init name server.
5. Add compact inline VFS IPC Open/Read/Close/Stat messages using the existing fixed IpcMsg ABI.
6. Serve with ipc_recv first, then ipc_reply_and_wait in the loop.
7. Add boot serial self-tests for:
   - open /etc/motd
   - read /etc/motd
   - stat /etc/sunlight/session.toml
   - missing file ENOENT
   - bad handle
8. Wire phase3.0 expectations into tools/test.sh.
9. Update docs/PHASE_3_0_SUMMARY.md checklist and commands run.
10. Verify:
    cargo test --target x86_64-unknown-linux-gnu --package sunlight-fs
    cargo check --workspace
    ./tools/test.sh phase3.0

Constraints:
- Do not change the fixed 80-byte IpcMsg ABI.
- Do not add new syscalls.
- Use existing IPC wrappers: endpoint_create, nameserver_register, ipc_recv, ipc_reply_and_wait.
- Keep serial logs deterministic.
- Add SAFETY comments for every new unsafe block.
- Keep the handoff summary updated so another session can continue cleanly.
```
