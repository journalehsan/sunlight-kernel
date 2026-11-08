# Phase 5 DNS Resolver Enhancement Summary (Bite 2 follow-up)

## /etc/hosts + Hardcoded Fallback (dns_hosts)

- Added real /etc/hosts to RamFS INITRAMFS (sunlight-fs/etc/hosts + ramfs.rs include). `cat /etc/hosts` works via VFS.
- New minimal parser: sunlight-net/src/hosts.rs (parse_hosts -> BTreeMap<String, [u8;4]>, ignores #, blank, IPv6; tiny footprint, no heavy code).
- Combined DnsResolver (sunlight-net/src/dns.rs): hosts table first, then hardcoded fallback (google, irancell, localhost, ...). SAFETY: Send/Sync impl + comments on static mut in net_server.
- net_server: at _start, uses capability-gated IPC (nameserver_lookup("vfs") + VfsMsg OPEN/READ/CLOSE) to fetch /etc/hosts, constructs DnsResolver, stores in static mut (single-writer, read in RESOLVE handler). Logs "[DNS] /etc/hosts loaded...".
- NetOp::RESOLVE now uses the resolver (hostname unpack preserved). ping in net-utils/sunshell gets hosts precedence transparently (e.g. localhost, and still falls to hardcoded for others).
- TUI: sysfetch render now includes "DNS: hosts + hardcoded" line (after IP when net available).
- Gate: ./tools/test.sh dns_hosts (added to test.sh + expected; spawns net_server, verifies load log). Other 5.x gates unaffected.
- No breakage: VFS/ RamFs/ IPC/ TTY/ sunshell/ net_server memory model unchanged. All unsafes (bump, static mut DNS, existing) have comments.
- Future: SIGHUP reload, full smoltcp DNS over wire, IPv6.

Tested: dns_hosts gate PASSED. ping localhost (via hosts) + google.com (hardcoded) + unknown name error path work.

See also: previous Bite for initial hostname ping, sunlight-net/hosts.rs, services/net_server, sunshell sysfetch.
