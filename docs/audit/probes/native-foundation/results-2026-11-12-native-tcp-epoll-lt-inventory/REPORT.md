# Incomplete TCP epoll inventory

No implementation or acceptance claim. Initial HEAD and incoming status, toolchain, current archive and lock hashes, and command exit statuses are in inventory.json. No build was performed; the archive hash is inventory only, not fresh provenance.

Verified gap: sunlight-libc/src/socket.rs forwards epoll operations to native syscalls 171–173. Kernel process/epoll.rs owns the registrations, eight raw data bytes, ET/oneshot state, and instance pool. Its inet_socket readiness returns POLLERR; it cannot observe libc net_bridge NetOp readiness. Native records are 16 bytes with data at offset 8; Linux records are separately packed at 12 bytes. Public header currently exposes epoll_create1, not epoll_create.

Smallest architecture identified for further implementation: retain kernel registration ownership and expose a native registration snapshot/revalidation operation for libc to combine production NetOp readiness with local readiness. This has not been implemented or fully designed. Existing local ET/oneshot behavior must remain intact. TCP unsupported modes require explicit rejection. Existing collector iterates ascending fd and stops at maxevents; fairness needs work. Capacity is 64 registrations versus NetOp batch limit 32. Close detaches registrations from queues held by the descriptor table. Full lifetime, dup/fork, concurrent mutation and identity analysis remains incomplete.

PASS: read-only identification of the disconnected epoll/TCP path. All requested builds, ABI tests, functional fixtures, external and loopback acceptance on 2/4 CPUs, pressure, terminal, lifecycle, capacity, fairness, and regression execution: NOT_RUN. No FAIL, BUILD_FAIL, LINK_FAIL or TIMEOUT was newly observed. Historical cpp-runtime LINK_FAIL/runtime NOT_RUN remains historical; no fresh rerun. Prior build audit qualifications (stale-lock failure not independently rerun; version conventions not exhaustively traced) remain unchanged.

Production code, historical fixtures and reports were not edited. No commit. This inventory does not satisfy the requested implementation milestone. No verified environmental blocker was established.
