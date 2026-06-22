# Solar HTTP Server & SBSP Implementation Plan

**Date:** 2026-06-22  
**Target:** SunlightOS Ring-3 Service  
**Status:** Architectural Planning

---

## Executive Summary

Solar is a high-performance HTTP/1.1 server designed for SunlightOS's capability-based architecture. It features an embedded scripting language (SBSP - Solar Basic Server Pages) that enables dynamic content generation while maintaining the kernel's read-only security model through carefully orchestrated IPC with trusted services.

### Core Design Principles

1. **Zero-Trust File Access**: Direct VFS capabilities for reads; all writes mediated through `sunlight-sm`
2. **Memory Efficiency**: Pre-allocated SHM page pools for IPC operations
3. **Performance**: Thread-per-connection with async I/O for static content streaming
4. **Security**: Strict whitelist enforcement via capability broker and storage manager

---

## Two Critical Architectural Discoveries

### 1. The Static Whitelist Security Model

**Discovery**: `sunlight-sm` enforces a hardcoded `WHITELIST` containing:
- `/var/lib/sunlight-kv/`
- `/var/lib/sunlight/tls/`
- `/var/lib/sunlight/`

**Implication**: Traditional web server paths like `/srv/` or `/var/www/` are **not permitted**. Any attempt to read or write outside the whitelist returns `ERR_DENIED`.

**Solution**: Embrace SunlightOS's directory structure. Solar will use:
```
/var/lib/sunlight/www/          # Document root
/var/lib/sunlight/www/uploads/  # User-uploaded content
/var/lib/sunlight/www/static/   # CSS, JS, images
```

### 2. The 48-Byte Read Limit

**Discovery**: `sunlight-sm`'s `op_read` implementation packs file contents directly into `IpcMsg.words[1..8]`, limiting reads to **48 bytes maximum**. Larger files return `SmMsg::ERR_PAYLOAD_TOO_LARGE`.

**Rationale**: `sunlight-sm` is optimized for controlled metadata reads (PID files, tokens, small configs), not bulk streaming.

**Implication**: Solar **cannot** use `sunlight-sm` to serve large HTML files, images, or videos.

**Solution**: Use direct VFS capabilities (via `CapabilityBroker`) for all file reads. Reserve `sunlight-sm` exclusively for secure writes.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                     Solar HTTP Server                  │
│  ┌────────────────┐  ┌──────────────┐  ┌─────────────┐ │
│  │ HTTP/1.1 Parser│  │ Thread Pool  │  │ SBSP Engine │ │
│  │   (Chunked TE) │  │ (per-conn)   │  │  (Lexer +   │ │
│  │                │  │              │  │   Runtime)  │ │
│  └────────────────┘  └──────────────┘  └─────────────┘ │
└──────────┬──────────────────┬──────────────────┬────────┘
           │                  │                  │
           ▼                  ▼                  ▼
  ┌────────────────┐  ┌──────────────┐  ┌──────────────┐
  │ VFS Capability │  │ sunlight-sm  │  │ sunlight-kv  │
  │   (READ-only)  │  │ (WRITE-only) │  │ (Key-Value)  │
  │                │  │              │  │              │
  │ Direct syscall │  │ IPC+SHM page │  │ Unix socket  │
  │ read() on FD   │  │              │  │              │
  └────────────────┘  └──────────────┘  └──────────────┘
```

---

## Phase-by-Phase Implementation

### Phase 1: Core HTTP Server Foundation

**Objective**: Build a minimal HTTP/1.1 server capable of serving static files with zero scripting.

#### 1.1 Service Bootstrap
- **Binary**: `services/solar/src/main.rs`
- **Startup**:
  1. Acquire `www` user credentials via `sunlight-uac`
  2. Request `VfsCapability` for `/var/lib/sunlight/www/` with `READ` flags from `CapabilityBroker`
  3. Bind TCP socket on `0.0.0.0:8080`
  4. Register with nameserver as `solaris`

#### 1.2 HTTP/1.1 Parser
- **Module**: `services/solar/src/http/mod.rs`
- **Features**:
  - Request line parsing: `GET /index.html HTTP/1.1`
  - Header parsing with case-insensitive keys
  - `Content-Length` and `Transfer-Encoding: chunked` support
  - Keep-alive connection handling

#### 1.3 Static File Serving
- **Module**: `services/solar/src/file_handler.rs`
- **Flow**:
  1. Resolve URL path to filesystem path (with `..` sanitization)
  2. Use `libc::open()` with VFS capability-derived FD
  3. Stream file contents directly to TCP socket via `libc::read()` + `libc::write()`
  4. Set `Content-Type` header based on file extension
  5. Return `404 Not Found` for missing files

#### 1.4 Thread Pool
- **Module**: `services/solar/src/pool.rs`
- **Design**:
  - Pre-spawn 8 worker threads on startup
  - Each thread blocks on `accept()` (kernel load-balances connections)
  - Process HTTP request, send response, return to `accept()`

#### Validation
```bash
$ curl http://localhost:8080/index.html
<html>...</html>

$ curl http://localhost:8080/static/logo.png --output logo.png
# Binary file successfully streamed
```

---

### Phase 2: SBSP Language Specification

**Objective**: Define the Solar Basic Server Pages scripting language syntax and semantics.

#### 2.1 Language Design

**File Extension**: `.sbsp`

**Execution Model**: Server-side template engine with embedded Rust-like expressions.

**Design Philosophy**: A perfect blend of classic BASIC readability and modern web-templating convenience. Solar SBSP separates HTML from server-side logic using `{% ... %}` delimiters, with `|` providing shorthand output to the HTML stream.

---

#### 2.2 SBSP V1 Syntax Specification

##### Variables & Types
Classic `DIM` keyword with basic type support:

```basic
{% DIM i AS Integer = 0 %}
{% DIM name AS String = "SolarOS" %}
{% DIM is_active AS Boolean = True %}
```

##### Math & Logical Operators
- **Arithmetic**: `+`, `-`, `*`, `/`, `%`
- **Logical**: `&&` (AND), `||` (OR), `!` (NOT)
- **Comparison**: `==`, `!=`, `<`, `>`, `<=`, `>=`

```basic
{% DIM result AS Integer %}
{% result = (2 + 3) * 10 %}
```

##### Control Structures
Standard branching and loops with clean, parseable syntax:

```basic
{% IF result > 10 THEN %}
    <p>The result is huge!</p>
{% ELSE %}
    <p>The result is small.</p>
{% END IF %}

{% FOR i = 1 TO 5 %}
    <p>Loop iteration: {%| i %}</p>
{% NEXT %}

{% WHILE is_active %}
    {%| "Still running..." %}
{% WEND %}
```

##### Shorthand Output (`|`)
Using `|` inside tags dumps variables directly into the HTML stream:

```html
<h1>Welcome to {%| name %}</h1>
<p>The answer is {%| result %}</p>
```

##### Functions & Recursion
Full support for custom function definitions, including recursion:

```basic
{% FUNCTION fib(n AS Integer) AS Integer %}
    {% IF n <= 1 THEN %}
        {% RETURN n %}
    {% ELSE %}
        {% RETURN fib(n - 1) + fib(n - 2) %}
    {% END IF %}
{% END FUNCTION %}

<p>Fibonacci of 10 is: {%| fib(10) %}</p>
```

##### Arrays (V1 Basic Support)
Simple array syntax for iteration:

```basic
{% DIM items() AS String = ["Apple", "Banana", "Cherry"] %}

{% FOR idx = 0 TO UBOUND(items) %}
    <li>{%| items[idx] %}</li>
{% NEXT %}
```

---

#### 2.3 Token Types
```rust
pub enum SbspToken {
    Text(String),                      // Plain HTML: "Welcome, "
    Expr(String),                      // {%| user.name %}
    DimDecl { var: String, typ: String, init: Option<String> }, // {% DIM x AS Integer = 5 %}
    If(String),                        // {% IF condition THEN %}
    Else,                              // {% ELSE %}
    EndIf,                             // {% END IF %}
    For { var: String, start: String, end: String }, // {% FOR i = 1 TO 10 %}
    Next,                              // {% NEXT %}
    While(String),                     // {% WHILE condition %}
    Wend,                              // {% WEND %}
    FunctionDef { name: String, params: Vec<String>, return_type: String }, // {% FUNCTION fib(...) AS Integer %}
    Return(String),                    // {% RETURN value %}
    EndFunction,                       // {% END FUNCTION %}
    Assignment { var: String, expr: String }, // {% x = 42 %}
}
```

#### 2.4 Native Functions

Built-in functions callable from SBSP scripts:

| Function | Description | Example |
|----------|-------------|---------|
| `NATIVE_TIME()` | Returns current Unix timestamp | `{%\| NATIVE_TIME() %}` |
| `NATIVE_KV_GET(key)` | Reads from sunlight-kv | `{%\| NATIVE_KV_GET("user:42:name") %}` |
| `NATIVE_KV_PUT(key, value)` | Writes to sunlight-kv | `{%\| NATIVE_KV_PUT("session:x", "data") %}` |
| `NATIVE_FILE_WRITE(path, data)` | Secure write via sunlight-sm | `{%\| NATIVE_FILE_WRITE("/var/lib/sunlight/www/uploads/file", bytes) %}` |
| `NATIVE_HTML_ESCAPE(text)` | XSS protection | `{%\| NATIVE_HTML_ESCAPE(user_input) %}` |
| `HASH(value)` | SHA-256 hash for auth | `{%\| HASH("password") %}` |
| `UBOUND(array)` | Get upper bound of array | `{% FOR i = 0 TO UBOUND(items) %}` |

---

#### 2.5 Implementation Strategy

**Interpretation Style**: Single-pass interpreter (line-by-line execution as it reads) rather than full AST compilation. This keeps the implementation lightweight while still supporting all required features.

---

### SBSP Engine: 4-Phase Implementation Roadmap

Building Solar's scripting engine in Rust will be blazingly fast. Here's the breakdown:

#### **Phase I: The Lexer & HTML Splitter**

**Goal**: Read a `.sbsp` file and separate raw HTML from `{% %}` blocks.

**Task**: Build a Rust tokenizer that scans character-by-character:
1. Text outside `{% %}` → treat as raw HTML string
2. Text inside `{% %}` → tokenize into keywords (`DIM`, `IF`, `FOR`, etc.), identifiers, operators, literals
3. Detect `{%| expr %}` → special "echo" mode that outputs to HTML

**Key Deliverables**:
- `struct SbspLexer` with `fn next_token() -> SbspToken`
- Proper error handling for unclosed tags
- Source location tracking for debugging

#### **Phase II: The Symbol Table & Math Evaluator**

**Goal**: Make Solar understand variables and evaluate expressions.

**Task**: 
1. Create `HashMap<String, SbspValue>` to store `DIM` variables
2. Implement recursive descent parser for order of operations: `x = 2 + 3 * 4`
3. Support type coercion (String ↔ Integer, Boolean to int)

**Key Deliverables**:
- `struct SymbolTable` with type checking
- `fn evaluate_expr(expr: &str) -> Result<SbspValue>`
- Operator precedence: `()` > `*/%` > `+-` > `<>=` > `&&` > `||`

#### **Phase III: Control Flow (The Jump Logic)**

**Goal**: Implement `IF/ELSE`, `FOR`, and `WHILE` blocks.

**Task**: 
1. Parse conditional expressions into boolean results
2. If an `IF` condition is false, efficiently skip tokens until `ELSE` or `END IF`
3. For loops: track loop variable, increment, and detect `NEXT`
4. While loops: re-evaluate condition at each iteration

**Key Deliverables**:
- Block-aware parser that tracks nesting depth
- Stack-based scope management
- Zero HTML output for skipped blocks

#### **Phase IV: Functions & Scope**

**Goal**: Support custom `FUNCTION` definitions and recursion.

**Task**:
1. Parse `FUNCTION name(param1 AS Type, ...) AS ReturnType`
2. When function is called, push new environment (local HashMap) onto stack
3. Handle `RETURN` statements and stack unwinding
4. Support recursion without variable collision (e.g., fib calling itself)

**Key Deliverables**:
- `struct SbspFunction` storing params, return type, body tokens
- `fn call_function(name: &str, args: Vec<SbspValue>) -> Result<SbspValue>`
- Stack-based environment management

---

### Phase 3: SBSP Lexer & Parser

**Objective**: Implement a zero-allocation lexer that tokenizes `.sbsp` files into executable AST nodes.

#### 3.1 Lexer Implementation
- **Module**: `services/solar/src/sbsp/lexer.rs`
- **Algorithm**:
  1. Scan input string character-by-character
  2. Detect `{%` delimiter → enter SBSP mode
  3. Parse keyword (`DIM`, `IF`, `FOR`, `END`, `FUNCTION`, `RETURN`) or expression
  4. Detect `%}` delimiter → return to HTML mode
  5. Accumulate plain text as `Text` tokens

#### 3.2 Parser Rules
```rust
// services/solar/src/sbsp/parser.rs

fn parse_sbsp_tag(input: &str) -> SbspToken {
    let trimmed = input.trim();
    
    if trimmed.starts_with("DIM ") {
        parse_dim_declaration(trimmed)
    } else if trimmed.starts_with("IF ") {
        SbspToken::If(trimmed[3..].to_string())
    } else if trimmed.starts_with("FOR ") {
        parse_for_loop(trimmed)
    } else if trimmed.starts_with("FUNCTION ") {
        parse_function_def(trimmed)
    } else if trimmed == "ELSE" {
        SbspToken::Else
    } else if trimmed == "END IF" {
        SbspToken::EndIf
    } else if trimmed == "NEXT" {
        SbspToken::Next
    } else if trimmed == "END FUNCTION" {
        SbspToken::EndFunction
    } else if trimmed.starts_with("RETURN ") {
        SbspToken::Return(trimmed[7..].to_string())
    } else {
        SbspToken::Expr(trimmed.to_string())
    }
}
```

#### 3.3 Error Handling
- Unclosed `{%` tags → Return `500 Internal Server Error` with debug info
- Invalid variable names → Log and render as empty string
- Missing `{% END IF %}` or `{% END FUNCTION %}` → Syntax error with line number
- Type mismatch on assignment → Return `500 Internal Server Error` with strict type error

---

### Phase 4: SBSP Runtime & Native Functions

**Objective**: Execute tokenized SBSP scripts with access to IPC services.

#### 4.1 Execution Context
```rust
// services/solar/src/sbsp/runtime.rs

pub struct SbspContext {
    variables: HashMap<String, SbspValue>,
    kv_socket: UnixStream,              // Connection to sunlight-kv
    sm_endpoint: u64,                   // IPC endpoint for sunlight-sm
    shm_pool: ShmPagePool,              // Pre-allocated SHM pages
}

pub enum SbspValue {
    String(String),
    Number(i64),
    Bool(bool),
    List(Vec<SbspValue>),
    Object(HashMap<String, SbspValue>),
}
```

#### 4.2 Native Function: `NATIVE_FILE_WRITE`
```rust
fn native_file_write(
    ctx: &mut SbspContext,
    path: &str,
    data: &[u8],
) -> Result<(), SbspError> {
    // 1. Acquire SHM page from pool
    let shm_page = ctx.shm_pool.acquire()?;
    
    // 2. Pack payload: path_bytes + content_bytes
    let path_bytes = path.as_bytes();
    let path_len = path_bytes.len();
    let content_len = data.len();
    
    if path_len + content_len > SmMsg::PAGE_CAPACITY {
        return Err(SbspError::PayloadTooLarge);
    }
    
    unsafe {
        let shm_ptr = shm_map(shm_page.token);
        core::ptr::copy_nonoverlapping(
            path_bytes.as_ptr(),
            shm_ptr,
            path_len,
        );
        core::ptr::copy_nonoverlapping(
            data.as_ptr(),
            shm_ptr.add(path_len),
            content_len,
        );
        shm_unmap(shm_ptr);
    }
    
    // 3. Send IPC message to sunlight-sm
    let mut msg = IpcMsg::with_label(SmMsg::WRITE_FILE);
    msg.words[0] = path_len as u64;
    msg.words[1] = content_len as u64;
    msg.caps[0] = shm_page.token;
    
    let reply = ipc_call(ctx.sm_endpoint, msg)?;
    
    // 4. Return SHM page to pool
    ctx.shm_pool.release(shm_page);
    
    // 5. Check reply
    if reply.label == SmMsg::REPLY_OK {
        Ok(())
    } else {
        Err(SbspError::SmError(reply.words[0]))
    }
}
```

#### 4.3 Native Function: `NATIVE_KV_GET`
```rust
fn native_kv_get(
    ctx: &mut SbspContext,
    key: &str,
) -> Result<Option<String>, SbspError> {
    // 1. Serialize request using bincode
    let req = KvRequest::Get { key: key.to_string() };
    let serialized = bincode::serialize(&req)?;
    
    // 2. Send to sunlight-kv via Unix socket
    ctx.kv_socket.write_all(&serialized)?;
    
    // 3. Read response
    let mut response_buf = vec![0u8; 4096];
    let n = ctx.kv_socket.read(&mut response_buf)?;
    let resp: KvResponse = bincode::deserialize(&response_buf[..n])?;
    
    match resp {
        KvResponse::Value(val) => Ok(Some(val)),
        KvResponse::NotFound => Ok(None),
        KvResponse::Error(e) => Err(SbspError::KvError(e)),
    }
}
```

---

### Phase 5: SHM Page Pool Architecture

**Objective**: Eliminate per-request allocation overhead by maintaining a reusable pool of shared memory pages.

#### 5.1 Design Decision

**Question**: Should Solar pre-allocate a pool of SHM pages on startup, or allocate/free on demand per write request?

**Recommendation**: **Pre-allocated Pool**

**Rationale**:
1. **Performance**: Avoids syscall overhead (`shm_alloc`/`shm_free`) in hot path
2. **Predictability**: Bounded memory usage (e.g., 16 pages = 64 KB)
3. **Concurrency**: Lock-free pool with per-thread TLS cache
4. **IPC Semantics**: SHM pages are cheap capability tokens, not scarce resources

#### 5.2 Implementation
```rust
// services/solar/src/shm_pool.rs

pub struct ShmPagePool {
    pages: Mutex<Vec<CapabilityToken>>,
    capacity: usize,
}

impl ShmPagePool {
    pub fn new(capacity: usize) -> Self {
        let mut pages = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            let token = shm_alloc(4096);
            pages.push(token);
        }
        Self {
            pages: Mutex::new(pages),
            capacity,
        }
    }
    
    pub fn acquire(&self) -> Result<ShmPage, PoolError> {
        let mut guard = self.pages.lock();
        guard.pop().ok_or(PoolError::Exhausted)
    }
    
    pub fn release(&self, token: CapabilityToken) {
        let mut guard = self.pages.lock();
        if guard.len() < self.capacity {
            guard.push(token);
        } else {
            shm_free(token); // Overflow protection
        }
    }
}
```

#### 5.3 Startup Configuration
```rust
// services/solar/src/main.rs

fn main() -> ! {
    // ...capability acquisition...
    
    let shm_pool = ShmPagePool::new(16); // 16 pages = 64 KB
    let ctx = Arc::new(SbspContext {
        shm_pool,
        // ...
    });
    
    spawn_thread_pool(8, ctx);
}
```

---

### Phase 6: Security & Validation

#### 6.1 Path Traversal Prevention
```rust
fn sanitize_path(url_path: &str) -> Result<PathBuf, SecurityError> {
    let base = Path::new("/var/lib/sunlight/www/");
    let requested = base.join(url_path.trim_start_matches('/'));
    
    // Resolve `..` components
    let canonical = requested.canonicalize()?;
    
    // Ensure result is still under base directory
    if !canonical.starts_with(base) {
        return Err(SecurityError::PathTraversal);
    }
    
    Ok(canonical)
}
```

#### 6.2 SBSP Injection Defense
- **Auto-escaping**: All `{%| variable %}` expressions are HTML-escaped by default (HTML entities for `<`, `>`, `&`, `"`)
- **Strict typing**: Type mismatches caught at parse time, preventing coercion bugs
- **Whitelist**: Only allow `NATIVE_*` and user-defined functions; no arbitrary Rust code execution
- **Sandboxed IPC**: All file writes require explicit `sunlight-sm` IPC with whitelist enforcement

#### 6.3 Rate Limiting (Future)
- Track connections per IP address
- Enforce max 100 req/sec per client
- Return `429 Too Many Requests` when exceeded

---

## File Structure

```
services/solar/
├── Cargo.toml
├── src/
│   ├── main.rs                # Service bootstrap, capability acquisition
│   ├── http/
│   │   ├── mod.rs             # HTTP/1.1 parser
│   │   ├── request.rs         # Request struct
│   │   ├── response.rs        # Response builder
│   │   └── headers.rs         # Header utilities
│   ├── file_handler.rs        # Static file serving via VFS
│   ├── sbsp/
│   │   ├── mod.rs             # SBSP entry point
│   │   ├── lexer.rs           # Tokenizer
│   │   ├── parser.rs          # AST builder
│   │   ├── runtime.rs         # Execution engine
│   │   └── native.rs          # NATIVE_* function implementations
│   ├── shm_pool.rs            # SHM page pool
│   ├── pool.rs                # Thread pool
│   └── security.rs            # Path sanitization, escaping
└── examples/
    ├── hello.sbsp             # Simple template
    └── todo.sbsp              # KV-backed todo list
```

---

## Testing Strategy

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_sbsp_lexer_simple() {
        let input = "Hello {%| name %}!";
        let tokens = lex(input);
        assert_eq!(tokens, vec![
            Token::Text("Hello "),
            Token::Expr("name"),
            Token::Text("!"),
        ]);
    }
    
    #[test]
    fn test_path_traversal_blocked() {
        let result = sanitize_path("/../etc/passwd");
        assert!(result.is_err());
    }
}
```

### Integration Tests
```bash
# Start Solar in QEMU
$ ./tools/build.sh

# From another terminal
$ curl http://10.0.2.15:8080/test.sbsp
<html>Generated at: 1719043200</html>

$ curl -X POST http://10.0.2.15:8080/upload \
    --data-binary @avatar.png
{"status": "ok", "path": "/uploads/avatar.png"}
```

---

## Dependencies

```toml
[dependencies]
sunlight-ipc = { path = "../../ipc" }
sunlight-libc = { path = "../../sunlight-libc" }
heapless = { version = "0.7", default-features = false }
bincode = { version = "1.3", default-features = false }
```

---

## Performance Targets

| Metric | Target | Rationale |
|--------|--------|-----------|
| Static file throughput | >500 MB/s | Direct VFS read, zero-copy streaming |
| SBSP render time (1KB) | <1 ms | Lexer is single-pass, no allocations |
| Concurrent connections | 1000 | Thread-per-connection model |
| SHM pool miss rate | <1% | Pre-allocated 16 pages handles burst traffic |

---

## Future Enhancements

### Phase 7: HTTP/2 Support
- Binary framing layer
- Multiplexing multiple requests over single connection
- Server push for CSS/JS assets

### Phase 8: WebSocket Support
- Upgrade from HTTP/1.1 to WebSocket protocol
- Real-time chat application demo
- Integration with `sunlight-gcd` for event broadcasting

### Phase 9: TLS via sunlight-sm
- Read TLS certificates from `/var/lib/sunlight/tls/`
- Integrate rustls for HTTPS support
- Automatic HTTP → HTTPS redirect

---

## Open Questions

### 1. SHM Page Pool Size
**Question**: Should the pool size be configurable via command-line argument or hardcoded?

**Recommendation**: Start with hardcoded `16 pages`, add config file support in Phase 7.

### 2. Error Page Customization
**Question**: Should `404.sbsp` and `500.sbsp` be customizable templates?

**Recommendation**: Yes. Look for `/var/lib/sunlight/www/errors/{404,500}.sbsp` first, fall back to hardcoded HTML.

### 3. SBSP Compilation Cache
**Question**: Should parsed SBSP ASTs be cached in memory?

**Recommendation**: **Yes**. Use `HashMap<PathBuf, Vec<SbspToken>>` with file modification time tracking. Invalidate cache entry if file changes.

---

## Assumptions

1. **Read-Only Root**: SunlightOS kernel enforces immutable root filesystem; all writes require IPC
2. **Capability Bootstrap**: `CapabilityBroker` is running and accessible via nameserver
3. **sunlight-sm Availability**: Storage manager service is running before Solar starts
4. **sunlight-kv Daemon**: Key-value service listens on `/tmp/sunlight/kv.sock`
5. **Network Stack**: TCP/IP stack is functional and bound to loopback or Ethernet device

---

## Success Criteria

- [ ] Solar boots successfully, acquires VFS capability, binds to port 8080
- [ ] Static file serving works for HTML, CSS, JS, images, videos
- [ ] SBSP lexer parses all syntax constructs without panics
- [ ] `NATIVE_FILE_WRITE` successfully writes files via `sunlight-sm` IPC
- [ ] `NATIVE_KV_GET`/`NATIVE_KV_PUT` interact with `sunlight-kv` daemon
- [ ] Path traversal attempts return `403 Forbidden`
- [ ] SHM page pool handles 100 concurrent write requests without exhaustion
- [ ] `./tools/test.sh` includes Solar boot and basic HTTP GET validation

---

## Timeline Estimate

| Phase | Estimated Effort | Dependencies |
|-------|------------------|--------------|
| Phase 1: HTTP Server | 2-3 sessions | VFS capabilities working |
| Phase 2: SBSP Spec | 1 session | None |
| Phase 3: Lexer/Parser | 2 sessions | Phase 2 complete |
| Phase 4: Runtime | 3 sessions | sunlight-sm, sunlight-kv running |
| Phase 5: SHM Pool | 1 session | Phase 4 complete |
| Phase 6: Security | 1 session | All phases integrated |

**Total**: ~10-12 development sessions

---

## Approval & Next Steps

This plan is ready for review. Once approved, implementation will begin with **Phase 1: Core HTTP Server Foundation**.

**First concrete action**: Create `services/solar/` directory and scaffold `Cargo.toml` + `main.rs` with capability acquisition logic.
