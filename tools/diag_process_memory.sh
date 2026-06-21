#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

echo "SunlightOS process-memory ownership inspection"
echo
echo "Kernel structs and routines to inspect:"
printf '  - %s\n' \
  "kernel/src/process/mod.rs: Process / ProcessState" \
  "kernel/src/process/address_space.rs: AddressSpace::map_page / reclaim_user_space" \
  "kernel/src/memory/pmm.rs: PhysicalMemoryManager owner tracking" \
  "kernel/src/memory/shared.rs: shared-page ownership and cleanup" \
  "kernel/src/ipc/mod.rs: endpoint queue cleanup" \
  "kernel/src/sched/mod.rs: finish_current_process / reap_process_resources / terminate_process_by_pid"
echo
echo "Ownership-related symbols:"
rg -n \
  "PMM_OWNER_|alloc_frame_owned|alloc_frames_owned|owner_of|owned_frame_count|reclaim_user_space|exit_cleanup_pending|terminate_process_by_pid|remove_pid_references|revoke_endpoints_owned_by" \
  kernel/src
