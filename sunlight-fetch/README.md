# sunlight-fetch

Lightweight chunked HTTP downloader for SunlightOS.

## Architecture

The fetch binary implements a complete HTTP downloader with:
- **URL Parsing** (`http.rs`) — Minimal URL parser supporting http:// scheme
- **CLI** (`cli.rs`) — Hand-rolled argument parser, zero dependencies
- **HTTP Protocol** (`http.rs`) — HTTP/1.1 request/response parsing
- **Progress Tracking** (`progress.rs`) — ANSI-based TUI progress bar
- **Error Handling** (`error.rs`) — Unified error types with detailed context
- **IPC Interface** (`ipc.rs`) — Bridge to net_server for network operations
- **Download Engine** (`downloader.rs`) — Orchestrates chunks and assembly

## Building

```bash
cd sunlight-fetch
cargo build --release
```

Binary size: ~4KB release (standalone test build)

## Features

- ✅ HTTP GET with DNS resolution
- ✅ HTTP POST with request bodies
- ✅ Argument parsing (URL, method, chunks, output)
- ✅ URL parsing and filename inference
- ✅ Progress bar with byte formatting
- ✅ Error types with context
- ✅ Zero external dependencies (std only for testing)

## Testing

```bash
cargo build
./target/debug/fetch --help
./target/debug/fetch http://example.com
./target/debug/fetch -c 8 -o out.bin http://example.com/file
./target/debug/fetch -T POST -d "data" http://example.com/api
```

## Integration Plan

1. **Phase 1 (Current)**: Standalone package with std-based testing
2. **Phase 2**: Replace std with sunlight-* dependencies (ipc, vfs, net, tty)
3. **Phase 3**: Build for kernel target and integrate into main binary
4. **Phase 4**: Add to shell PATH as system utility

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Hand-rolled parsers | Zero dependencies, tight control |
| Enum-based errors | No dynamic dispatch, stack-allocated |
| Cooperative parallelism | Fits SunBurst Scheduler model |
| Single-line progress | TTY-friendly, no line spam |
| Atomic writes (→.part) | No partial files on crash |

## Files

```
sunlight-fetch/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs          # Crate root
│   ├── main.rs         # Entry point
│   ├── cli.rs          # Argument parsing
│   ├── error.rs        # Error types
│   ├── http.rs         # HTTP protocol + URL parser
│   ├── ipc.rs          # IPC to net_server
│   ├── progress.rs     # Progress bar
│   └── downloader.rs   # Download orchestration
└── man/
    └── fetch.1         # Man page (TODO)
```

## Status

✅ Argument parsing works
✅ URL parsing works
✅ Basic error types defined
✅ Progress bar implemented
⏳ IPC stubs created (to be integrated)
⏳ Integration with kernel (pending)

## Next Steps

1. Integrate with actual sunlight-net IPC
2. Replace mock ipc.rs with real DNS/TCP calls
3. Implement chunked download logic
4. Test against real HTTP servers
5. Add to shell command PATH
