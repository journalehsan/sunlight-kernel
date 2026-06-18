//! sunlight-tls daemon — real rustls TLS endpoint for SunlightOS.
//!
//! Design B: this daemon owns both the TCP socket (via net_server) and the
//! crypto (rustls, no_std unbuffered API). Clients (e.g. `fetch`) exchange only
//! plaintext with it over IPC + shared memory:
//!
//!   TLS_CONNECT(ip, port, sni) -> session id   (opens socket, runs handshake)
//!   TLS_SEND(sid, plaintext via shm)           (encrypts + transmits)
//!   TLS_RECV(sid) -> plaintext via shm + eof   (receives + decrypts)
//!   TLS_CLOSE(sid)                             (close_notify + socket close)
//!
//! Trust anchors (root CAs) live in sunlight-kv under compact `tls.ca.*` keys.
//! On first boot the daemon self-seeds an embedded curated root (GTS Root R4)
//! into kv, then builds its RootCertStore from kv.
//!
//! Crypto provider is rustls-rustcrypto (pure Rust). RNG flows through
//! rand_service via a custom getrandom handler; time comes from the RTC/tz via
//! get_time_utc(). See the Phase-0 build recipe for the required RUSTFLAGS.

#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", no_main)]

#[cfg(feature = "sunlightos")]
extern crate alloc;

#[cfg(feature = "host")]
fn main() {
    println!("sunlight-tls: host mode (no TLS daemon loop). Use in SunlightOS image.");
}

// -----------------------------------------------------------------------------
// SunlightOS (no_std) build
// -----------------------------------------------------------------------------

#[cfg(feature = "sunlightos")]
mod sunlightos_impl {
    use alloc::format;
    use alloc::string::String;
    use alloc::sync::Arc;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::time::Duration;

    use linked_list_allocator::LockedHeap;
    use rustls::client::UnbufferedClientConnection;
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::time_provider::TimeProvider;
    use rustls::unbuffered::{ConnectionState, UnbufferedStatus};
    use rustls::{ClientConfig, RootCertStore};

    use sunlight_ipc::{
        debug_log, endpoint_create, get_time_utc, ipc_call, ipc_recv, ipc_reply_and_wait,
        monotonic_millis, nameserver_lookup, nameserver_register, shm_alloc, shm_free, shm_map,
        CapabilityToken, IpcMsg, IPC_REGISTER_WORDS,
    };

    // ── allocator ─────────────────────────────────────────────────────────────
    // rustls + RustCrypto allocate and free heavily during a handshake; the old
    // never-freeing bump allocator would leak and OOM. Use a real freeing heap.
    #[global_allocator]
    static ALLOCATOR: LockedHeap = LockedHeap::empty();

    const HEAP_SIZE: usize = 8 * 1024 * 1024; // 8 MiB
    static mut HEAP_MEM: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

    unsafe fn init_heap() {
        ALLOCATOR
            .lock()
            .init(core::ptr::addr_of_mut!(HEAP_MEM) as *mut u8, HEAP_SIZE);
    }

    // ── randomness: route getrandom -> rand_service via libc ───────────────────
    fn os_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
        let n = sunlight_libc::getrandom(buf, 0);
        if n == buf.len() as isize {
            Ok(())
        } else {
            Err(getrandom::Error::UNSUPPORTED)
        }
    }
    getrandom::register_custom_getrandom!(os_getrandom);

    // ── time: RTC/tz seconds since the unix epoch ──────────────────────────────
    #[derive(Debug)]
    struct OsTimeProvider;
    impl TimeProvider for OsTimeProvider {
        fn current_time(&self) -> Option<UnixTime> {
            Some(UnixTime::since_unix_epoch(Duration::from_secs(
                get_time_utc(),
            )))
        }
    }

    // ── output macros ──────────────────────────────────────────────────────────
    fn stdout_write(s: &str) {
        let mut data = s.as_bytes();
        while !data.is_empty() {
            match sunlight_libc::write(sunlight_libc::STDOUT, data) {
                Ok(n) if n > 0 => data = &data[n..],
                _ => break,
            }
        }
    }

    macro_rules! serial_println {
        ($($arg:tt)*) => {{
            use core::fmt::Write;
            let mut buf = heapless::String::<256>::new();
            let _ = write!(&mut buf, $($arg)*);
            debug_log(&buf);
        }};
    }

    // ── protocol labels (must match sunlight-fetch + certificatectl) ───────────
    const TLS_CONNECT: u64 = 0x5401;
    const TLS_SEND: u64 = 0x5402;
    const TLS_RECV: u64 = 0x5403;
    const TLS_CLOSE: u64 = 0x5405;
    const TLS_INSTALL: u64 = 0x5406; // certificatectl: install a CA into kv
    const TLS_LIST: u64 = 0x5407;
    const TLS_REPLY: u64 = 0x54FF;
    const TLS_ERROR: u64 = 0x54EE;

    // TLS error codes returned to clients in word[0] of a TLS_ERROR reply.
    const ERR_SESSIONS_FULL: u64 = 3;
    const ERR_CERT_EXPIRED: u64 = 5;

    // kv protocol (we are a *client* of sunlight-kv).
    const KV_REPLY: u64 = 0x4BFF;
    const KV_VALUE: u64 = 0x4B05;
    const KV_PUT_SHM: u64 = 0x4B06;
    const KV_GET_SHM: u64 = 0x4B07;

    // net_server protocol (NetOp).
    const NET_SOCKET: u64 = 1;
    const NET_CONNECT: u64 = 2;
    const NET_SEND: u64 = 6;
    const NET_RECV: u64 = 7;
    const NET_CLOSE: u64 = 8;

    const NET_CHUNK: usize = 48; // net_server caps each SEND/RECV at 48 bytes
    const SHM_PAGE: usize = 4096; // one shared page per plaintext/cert transfer
    const OUT_SCRATCH: usize = 18 * 1024; // > max TLS record (16 KiB + overhead)
    const MAX_INCOMING: usize = 256 * 1024; // guard against runaway buffering
    const MAX_SESSIONS: usize = 4;

    // Embedded curated trust anchor (GTS Root R4, self-signed, valid to 2036).
    // api.sampleapis.com chains: leaf <- GTS WE1 <- GTS Root R4.
    static EMBEDDED_GTS_R4: &[u8] = include_bytes!("roots/gts_root_r4.der");
    const TLS_CA_INDEX_KEY: &str = "tls.ca.idx";
    const TLS_CA_PREFIX: &str = "tls.ca.";
    const SEED_NAME: &str = "gtsr4";

    static mut CLIENT_CONFIG: Option<Arc<ClientConfig>> = None;

    struct Session {
        id: u64,
        conn: UnbufferedClientConnection,
        socket_id: u32,
        incoming: Vec<u8>, // received ciphertext not yet consumed by rustls
        backlog: Vec<u8>,  // decrypted plaintext not yet handed to the client
    }

    static mut SESSIONS: [Option<Session>; MAX_SESSIONS] = [const { None }; MAX_SESSIONS];

    // ── packing helpers ────────────────────────────────────────────────────────
    fn pack_str_register_words(msg: &mut IpcMsg, start_word: usize, s: &str) -> bool {
        if start_word >= IPC_REGISTER_WORDS {
            return false;
        }
        let b = s.as_bytes();
        if b.len() > (IPC_REGISTER_WORDS - start_word) * 8 {
            return false;
        }
        let mut i = 0;
        for w in start_word..IPC_REGISTER_WORDS {
            let mut word = 0u64;
            for j in 0..8 {
                if i < b.len() {
                    word |= (b[i] as u64) << (j * 8);
                    i += 1;
                }
            }
            msg.words[w] = word;
            if i >= b.len() {
                break;
            }
        }
        true
    }

    fn unpack_str(words: &[u64; 8], start_word: usize, max_len: usize) -> String {
        let mut v: Vec<u8> = Vec::new();
        let mut i = 0;
        for w in start_word..IPC_REGISTER_WORDS {
            if i >= max_len {
                break;
            }
            let word = words[w];
            for j in 0..8 {
                if i >= max_len {
                    break;
                }
                let byte = ((word >> (j * 8)) & 0xff) as u8;
                if byte == 0 {
                    break;
                }
                v.push(byte);
                i += 1;
            }
        }
        String::from_utf8(v).unwrap_or_default()
    }

    fn tls_ca_key(name: &str) -> Option<String> {
        let key = format!("{}{}", TLS_CA_PREFIX, name);
        if key.len() <= (IPC_REGISTER_WORDS - 2) * 8 {
            Some(key)
        } else {
            None
        }
    }

    fn pack_ipv4(ip: [u8; 4]) -> u64 {
        (ip[0] as u64) | ((ip[1] as u64) << 8) | ((ip[2] as u64) << 16) | ((ip[3] as u64) << 24)
    }

    // ── sunlight-kv client (shm transport) ─────────────────────────────────────
    fn kv_get_shm(key: &str) -> Option<Vec<u8>> {
        if key.len() > (IPC_REGISTER_WORDS - 2) * 8 {
            serial_println!("[SUNLIGHT-TLS] kv key too long: {}", key);
            return None;
        }
        let cap = nameserver_lookup("sunlight-kv")?;
        let mut msg = IpcMsg::with_label(KV_GET_SHM);
        if !pack_str_register_words(&mut msg, 2, key) {
            return None;
        }
        let reply = ipc_call(cap, msg);
        if reply.label != KV_VALUE {
            return None;
        }
        let n = reply.words[0] as usize;
        if n == 0 {
            return Some(Vec::new());
        }
        let tok = reply.caps[0];
        if tok == CapabilityToken::INVALID {
            return None;
        }
        let ptr = shm_map(tok).ok()?;
        let v = unsafe { core::slice::from_raw_parts(ptr, n.min(SHM_PAGE)) }.to_vec();
        let _ = shm_free(tok);
        Some(v)
    }

    fn kv_put_shm(key: &str, value: &[u8]) -> bool {
        if key.len() > (IPC_REGISTER_WORDS - 2) * 8 {
            serial_println!("[SUNLIGHT-TLS] kv key too long: {}", key);
            return false;
        }
        if value.len() > SHM_PAGE {
            return false;
        }
        let cap = match nameserver_lookup("sunlight-kv") {
            Some(c) => c,
            None => return false,
        };
        let (ptr, tok) = match shm_alloc() {
            Ok(p) => p,
            Err(_) => return false,
        };
        unsafe {
            core::ptr::copy_nonoverlapping(value.as_ptr(), ptr, value.len());
        }
        let mut msg = IpcMsg::with_label(KV_PUT_SHM)
            .word(0, value.len() as u64)
            .with_cap(0, tok);
        if !pack_str_register_words(&mut msg, 2, key) {
            let _ = shm_free(tok);
            return false;
        }
        let reply = ipc_call(cap, msg);
        let _ = shm_free(tok);
        reply.label == KV_REPLY && reply.words[0] == 0
    }

    // ── net_server client ──────────────────────────────────────────────────────
    fn net_cap() -> Option<CapabilityToken> {
        nameserver_lookup("net")
    }

    fn net_socket() -> Option<u32> {
        let cap = net_cap()?;
        let reply = ipc_call(cap, IpcMsg::with_label(NET_SOCKET));
        if reply.label != NET_SOCKET || reply.words[0] == 0 {
            return None;
        }
        Some(reply.words[0] as u32)
    }

    fn net_connect(socket_id: u32, ip_packed: u64, port: u16) -> bool {
        let cap = match net_cap() {
            Some(c) => c,
            None => return false,
        };
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(NET_CONNECT)
                .word(0, socket_id as u64)
                .word(1, ip_packed)
                .word(2, port as u64),
        );
        reply.label == NET_CONNECT && reply.words[0] != 0
    }

    fn net_send_all(socket_id: u32, data: &[u8]) -> bool {
        let cap = match net_cap() {
            Some(c) => c,
            None => return false,
        };
        let mut off = 0usize;
        while off < data.len() {
            let clen = (data.len() - off).min(NET_CHUNK);
            let mut msg = IpcMsg::with_label(NET_SEND)
                .word(0, socket_id as u64)
                .word(1, clen as u64);
            for i in 0..clen {
                let wi = 2 + i / 8;
                let bi = i % 8;
                msg.words[wi] |= (data[off + i] as u64) << (bi * 8);
            }
            let reply = ipc_call(cap, msg);
            let sent = reply.words[0] as usize;
            if sent == 0 {
                return false;
            }
            off += sent;
        }
        true
    }

    fn net_recv_chunk(socket_id: u32) -> Vec<u8> {
        let cap = match net_cap() {
            Some(c) => c,
            None => return Vec::new(),
        };
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(NET_RECV)
                .word(0, socket_id as u64)
                .word(1, NET_CHUNK as u64),
        );
        if reply.label != NET_RECV {
            return Vec::new();
        }
        let len = (reply.words[0] as usize).min(NET_CHUNK);
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let word = reply.words[2 + i / 8];
            let bi = i % 8;
            out.push(((word >> (bi * 8)) & 0xff) as u8);
        }
        out
    }

    fn net_close(socket_id: u32) {
        if let Some(cap) = net_cap() {
            let _ = ipc_call(cap, IpcMsg::with_label(NET_CLOSE).word(0, socket_id as u64));
        }
    }

    // ── trust store ─────────────────────────────────────────────────────────────
    fn seed_trust_if_empty() {
        serial_println!("[SUNLIGHT-TLS] dbg: seed get index...");
        if kv_get_shm(TLS_CA_INDEX_KEY).is_some() {
            serial_println!("[SUNLIGHT-TLS] dbg: index present, skip seed");
            return;
        }
        serial_println!(
            "[SUNLIGHT-TLS] dbg: seeding root ({} bytes)...",
            EMBEDDED_GTS_R4.len()
        );
        let Some(seed_key) = tls_ca_key(SEED_NAME) else {
            serial_println!("[SUNLIGHT-TLS] WARN: seed key too long");
            return;
        };
        if kv_put_shm(&seed_key, EMBEDDED_GTS_R4)
            && kv_put_shm(TLS_CA_INDEX_KEY, SEED_NAME.as_bytes())
        {
            serial_println!("[SUNLIGHT-TLS] seeded kv trust store with '{}'", SEED_NAME);
        } else {
            serial_println!("[SUNLIGHT-TLS] WARN: kv trust seed failed (kv down?)");
        }
    }

    fn build_root_store() -> RootCertStore {
        serial_println!("[SUNLIGHT-TLS] dbg: build_root_store start");
        let mut roots = RootCertStore::empty();
        seed_trust_if_empty();
        serial_println!("[SUNLIGHT-TLS] dbg: seed done, loading index");
        if let Some(index) = kv_get_shm(TLS_CA_INDEX_KEY) {
            if let Ok(s) = String::from_utf8(index) {
                for name in s.split(',').map(|n| n.trim()).filter(|n| !n.is_empty()) {
                    let Some(key) = tls_ca_key(name) else {
                        serial_println!("[SUNLIGHT-TLS] CA name too long {}", name);
                        continue;
                    };
                    if let Some(der) = kv_get_shm(&key) {
                        if !der.is_empty() {
                            match roots.add(CertificateDer::from(der)) {
                                Ok(()) => serial_println!("[SUNLIGHT-TLS] trust += {}", name),
                                Err(_) => serial_println!("[SUNLIGHT-TLS] bad cert {}", name),
                            }
                        }
                    }
                }
            }
        }
        if roots.is_empty() {
            // kv unavailable — fall back to the compiled-in root so HTTPS still works.
            let _ = roots.add(CertificateDer::from(EMBEDDED_GTS_R4.to_vec()));
            serial_println!("[SUNLIGHT-TLS] using embedded fallback root");
        }
        roots
    }

    fn rebuild_config() {
        let roots = build_root_store();
        let n = roots.len();
        serial_println!("[SUNLIGHT-TLS] dbg: roots loaded n={}, building config", n);
        let provider = Arc::new(rustls_rustcrypto::provider());
        serial_println!("[SUNLIGHT-TLS] dbg: provider ready");
        match ClientConfig::builder_with_details(provider, Arc::new(OsTimeProvider))
            .with_safe_default_protocol_versions()
        {
            Ok(b) => {
                let config = b.with_root_certificates(roots).with_no_client_auth();
                unsafe {
                    CLIENT_CONFIG = Some(Arc::new(config));
                }
                serial_println!("[SUNLIGHT-TLS] client config ready ({} roots)", n);
            }
            Err(_) => serial_println!("[SUNLIGHT-TLS] ERROR: failed to build client config"),
        }
    }

    // ── session table ───────────────────────────────────────────────────────────
    fn new_sid() -> u64 {
        let mut b = [0u8; 8];
        let _ = os_getrandom(&mut b);
        let v = u64::from_le_bytes(b);
        if v == 0 {
            1
        } else {
            v
        }
    }

    fn store_session(s: Session) -> Option<u64> {
        let id = s.id;
        unsafe {
            let slots = &mut *core::ptr::addr_of_mut!(SESSIONS);
            for slot in slots.iter_mut() {
                if slot.is_none() {
                    *slot = Some(s);
                    return Some(id);
                }
            }
        }
        None
    }

    fn close_session(id: u64) -> bool {
        unsafe {
            let slots = &mut *core::ptr::addr_of_mut!(SESSIONS);
            for slot in slots.iter_mut() {
                if let Some(s) = slot {
                    if s.id == id {
                        net_close(s.socket_id);
                        *slot = None;
                        return true;
                    }
                }
            }
        }
        false
    }

    // ── handshake / IO state machine (rustls unbuffered, no_std) ────────────────
    // All three loops share the same shape: process_tls_records yields a state;
    // we encode outbound handshake bytes into `outgoing` and flush on
    // TransmitTlsData, request more ciphertext on BlockedHandshake, and surface
    // app-data on ReadTraffic / readiness on WriteTraffic.

    fn drive_handshake(
        conn: &mut UnbufferedClientConnection,
        socket_id: u32,
        incoming: &mut Vec<u8>,
    ) -> Result<(), u64> {
        let mut scratch = vec![0u8; OUT_SCRATCH];
        let mut outgoing: Vec<u8> = Vec::new();
        loop {
            let mut need_read = false;
            let discard;
            {
                let UnbufferedStatus { discard: d, state } =
                    conn.process_tls_records(incoming.as_mut_slice());
                discard = d;
                match state {
                    Ok(ConnectionState::EncodeTlsData(mut st)) => match st.encode(&mut scratch) {
                        Ok(n) => outgoing.extend_from_slice(&scratch[..n]),
                        Err(_) => return Err(30),
                    },
                    Ok(ConnectionState::TransmitTlsData(st)) => {
                        if !outgoing.is_empty() {
                            if !net_send_all(socket_id, &outgoing) {
                                return Err(31);
                            }
                            outgoing.clear();
                        }
                        st.done();
                    }
                    Ok(ConnectionState::BlockedHandshake) => need_read = true,
                    Ok(ConnectionState::WriteTraffic(_)) => {
                        if discard > 0 {
                            incoming.drain(..discard.min(incoming.len()));
                        }
                        return Ok(());
                    }
                    Ok(_) => {
                        // ReadTraffic / ReadEarlyData unexpectedly early — treat as done.
                        if discard > 0 {
                            incoming.drain(..discard.min(incoming.len()));
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        serial_println!("[SUNLIGHT-TLS] handshake error: {:?}", e);
                        return Err(map_rustls_err(&e));
                    }
                }
            }
            if discard > 0 {
                incoming.drain(..discard.min(incoming.len()));
            }
            if need_read {
                let chunk = net_recv_chunk(socket_id);
                if chunk.is_empty() {
                    return Err(32); // peer closed / timeout mid-handshake
                }
                incoming.extend_from_slice(&chunk);
                if incoming.len() > MAX_INCOMING {
                    return Err(33);
                }
            }
        }
    }

    fn tls_send(
        conn: &mut UnbufferedClientConnection,
        socket_id: u32,
        incoming: &mut Vec<u8>,
        backlog: &mut Vec<u8>,
        data: &[u8],
    ) -> Result<(), u64> {
        let mut scratch = vec![0u8; OUT_SCRATCH];
        let mut outgoing: Vec<u8> = Vec::new();
        loop {
            let mut done = false;
            let mut need_read = false;
            let mut closed = false;
            let discard;
            {
                let UnbufferedStatus { discard: d, state } =
                    conn.process_tls_records(incoming.as_mut_slice());
                discard = d;
                match state {
                    Ok(ConnectionState::WriteTraffic(mut wt)) => {
                        match wt.encrypt(data, &mut scratch) {
                            Ok(n) => {
                                if !net_send_all(socket_id, &scratch[..n]) {
                                    return Err(41);
                                }
                                done = true;
                            }
                            Err(_) => return Err(42),
                        }
                    }
                    Ok(ConnectionState::ReadTraffic(mut rt)) => {
                        while let Some(rec) = rt.next_record() {
                            if let Ok(r) = rec {
                                backlog.extend_from_slice(r.payload);
                            }
                        }
                    }
                    Ok(ConnectionState::EncodeTlsData(mut st)) => match st.encode(&mut scratch) {
                        Ok(n) => outgoing.extend_from_slice(&scratch[..n]),
                        Err(_) => return Err(43),
                    },
                    Ok(ConnectionState::TransmitTlsData(st)) => {
                        if !outgoing.is_empty() {
                            if !net_send_all(socket_id, &outgoing) {
                                return Err(41);
                            }
                            outgoing.clear();
                        }
                        st.done();
                    }
                    Ok(ConnectionState::BlockedHandshake) => need_read = true,
                    Ok(ConnectionState::PeerClosed) | Ok(ConnectionState::Closed) => closed = true,
                    Ok(_) => {}
                    Err(e) => return Err(map_rustls_err(&e)),
                }
            }
            if discard > 0 {
                incoming.drain(..discard.min(incoming.len()));
            }
            if done {
                return Ok(());
            }
            if closed {
                return Err(44);
            }
            if need_read {
                let chunk = net_recv_chunk(socket_id);
                if chunk.is_empty() {
                    return Err(45);
                }
                incoming.extend_from_slice(&chunk);
            }
        }
    }

    /// Returns (plaintext, eof). Empty plaintext + eof=true means the peer closed.
    fn tls_recv(
        conn: &mut UnbufferedClientConnection,
        socket_id: u32,
        incoming: &mut Vec<u8>,
        backlog: &mut Vec<u8>,
    ) -> (Vec<u8>, bool) {
        if !backlog.is_empty() {
            let take = backlog.len().min(SHM_PAGE);
            let out: Vec<u8> = backlog.drain(..take).collect();
            return (out, false);
        }
        let mut scratch = vec![0u8; OUT_SCRATCH];
        let mut outgoing: Vec<u8> = Vec::new();
        loop {
            let mut got = false;
            let mut need_read = false;
            let mut eof = false;
            let discard;
            {
                let UnbufferedStatus { discard: d, state } =
                    conn.process_tls_records(incoming.as_mut_slice());
                discard = d;
                match state {
                    Ok(ConnectionState::ReadTraffic(mut rt)) => {
                        while let Some(rec) = rt.next_record() {
                            if let Ok(r) = rec {
                                backlog.extend_from_slice(r.payload);
                            }
                        }
                        got = true;
                    }
                    Ok(ConnectionState::WriteTraffic(_)) => need_read = true,
                    Ok(ConnectionState::EncodeTlsData(mut st)) => match st.encode(&mut scratch) {
                        Ok(n) => outgoing.extend_from_slice(&scratch[..n]),
                        Err(_) => eof = true,
                    },
                    Ok(ConnectionState::TransmitTlsData(st)) => {
                        if !outgoing.is_empty() {
                            let _ = net_send_all(socket_id, &outgoing);
                            outgoing.clear();
                        }
                        st.done();
                    }
                    Ok(ConnectionState::BlockedHandshake) => need_read = true,
                    Ok(ConnectionState::PeerClosed) | Ok(ConnectionState::Closed) => eof = true,
                    Ok(_) => {}
                    Err(_) => eof = true,
                }
            }
            if discard > 0 {
                incoming.drain(..discard.min(incoming.len()));
            }
            if got && !backlog.is_empty() {
                let take = backlog.len().min(SHM_PAGE);
                let out: Vec<u8> = backlog.drain(..take).collect();
                return (out, false);
            }
            if eof {
                return (Vec::new(), true);
            }
            if need_read {
                let chunk = net_recv_chunk(socket_id);
                if chunk.is_empty() {
                    return (Vec::new(), true);
                }
                incoming.extend_from_slice(&chunk);
                if incoming.len() > MAX_INCOMING {
                    return (Vec::new(), true);
                }
            }
        }
    }

    fn map_rustls_err(e: &rustls::Error) -> u64 {
        match e {
            rustls::Error::InvalidCertificate(rustls::CertificateError::Expired) => {
                ERR_CERT_EXPIRED
            }
            _ => 50,
        }
    }

    // ── connect entry point ─────────────────────────────────────────────────────
    fn tls_connect(ip_packed: u64, port: u16, host: &str) -> Result<u64, u64> {
        let cfg = match unsafe { (*core::ptr::addr_of!(CLIENT_CONFIG)).clone() } {
            Some(c) => c,
            None => return Err(10),
        };
        let sni: ServerName<'static> = match ServerName::try_from(host) {
            Ok(n) => n.to_owned(),
            Err(_) => return Err(11),
        };
        let mut conn = match UnbufferedClientConnection::new(cfg, sni) {
            Ok(c) => c,
            Err(_) => return Err(12),
        };
        let socket_id = match net_socket() {
            Some(s) => s,
            None => return Err(13),
        };
        if !net_connect(socket_id, ip_packed, port) {
            net_close(socket_id);
            return Err(14);
        }
        let mut incoming: Vec<u8> = Vec::new();
        if let Err(code) = drive_handshake(&mut conn, socket_id, &mut incoming) {
            net_close(socket_id);
            return Err(code);
        }
        let id = new_sid();
        let session = Session {
            id,
            conn,
            socket_id,
            incoming,
            backlog: Vec::new(),
        };
        match store_session(session) {
            Some(sid) => Ok(sid),
            None => {
                net_close(socket_id);
                Err(ERR_SESSIONS_FULL)
            }
        }
    }

    // ── IPC reply helpers for shm plaintext ────────────────────────────────────
    fn reply_with_shm(label: u64, len_word: u64, eof: u64, data: &[u8]) -> IpcMsg {
        match shm_alloc() {
            Ok((ptr, token)) => {
                unsafe {
                    core::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len().min(SHM_PAGE));
                }
                IpcMsg::with_label(label)
                    .word(0, len_word)
                    .word(1, eof)
                    .with_cap(0, token)
            }
            Err(_) => {
                let mut m = IpcMsg::with_label(TLS_ERROR);
                m.words[0] = 60;
                m
            }
        }
    }

    fn find_session_index(id: u64) -> Option<usize> {
        unsafe {
            let slots = &*core::ptr::addr_of!(SESSIONS);
            for (i, slot) in slots.iter().enumerate() {
                if let Some(s) = slot {
                    if s.id == id {
                        return Some(i);
                    }
                }
            }
        }
        None
    }

    // ── main ─────────────────────────────────────────────────────────────────
    pub fn run() -> ! {
        unsafe {
            init_heap();
        }
        debug_log("[SUNLIGHT-TLS] main() reached (no_std)\n");
        serial_println!("[SUNLIGHT-TLS] Starting sunlight-tls (real rustls)");

        let ep = endpoint_create();
        nameserver_register("sunlight-tls", ep);
        serial_println!("[SUNLIGHT-TLS] Registered as 'sunlight-tls'");

        // Build the client config (seeds + loads trust from sunlight-kv).
        rebuild_config();
        serial_println!("[SUNLIGHT-TLS] Entering IPC loop");

        let mut msg = ipc_recv(ep);
        loop {
            let mut reply = IpcMsg::with_label(TLS_REPLY);

            match msg.label {
                TLS_CONNECT => {
                    let ip = msg.words[0];
                    let port = msg.words[1] as u16;
                    let host = unpack_str(&msg.words, 2, (IPC_REGISTER_WORDS - 2) * 8);
                    let local = local_time_str();
                    serial_println!(
                        "[SUNLIGHT-TLS] connect host={} port={} local={} unix={}",
                        host,
                        port,
                        local,
                        get_time_utc()
                    );
                    match tls_connect(ip, port, &host) {
                        Ok(sid) => {
                            reply.words[0] = sid;
                            serial_println!("[SUNLIGHT-TLS] hs_OK host={} sid={:#x}", host, sid);
                        }
                        Err(code) => {
                            reply.label = TLS_ERROR;
                            reply.words[0] = code;
                            serial_println!("[SUNLIGHT-TLS] hs_FAIL host={} code={}", host, code);
                        }
                    }
                }
                TLS_SEND => {
                    let sid = msg.words[0];
                    let len = (msg.words[1] as usize).min(SHM_PAGE);
                    let tok = msg.caps[0];
                    let data = if tok != CapabilityToken::INVALID && len > 0 {
                        match shm_map(tok) {
                            Ok(ptr) => {
                                let d = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
                                let _ = shm_free(tok);
                                d
                            }
                            Err(_) => Vec::new(),
                        }
                    } else {
                        Vec::new()
                    };
                    match find_session_index(sid) {
                        Some(idx) => {
                            let res = unsafe {
                                let slots = &mut *core::ptr::addr_of_mut!(SESSIONS);
                                let s = slots[idx].as_mut().unwrap();
                                tls_send(
                                    &mut s.conn,
                                    s.socket_id,
                                    &mut s.incoming,
                                    &mut s.backlog,
                                    &data,
                                )
                            };
                            match res {
                                Ok(()) => reply.words[0] = 0,
                                Err(code) => {
                                    reply.label = TLS_ERROR;
                                    reply.words[0] = code;
                                }
                            }
                        }
                        None => {
                            reply.label = TLS_ERROR;
                            reply.words[0] = 2;
                        }
                    }
                }
                TLS_RECV => {
                    let sid = msg.words[0];
                    match find_session_index(sid) {
                        Some(idx) => {
                            let (data, eof) = unsafe {
                                let slots = &mut *core::ptr::addr_of_mut!(SESSIONS);
                                let s = slots[idx].as_mut().unwrap();
                                tls_recv(&mut s.conn, s.socket_id, &mut s.incoming, &mut s.backlog)
                            };
                            if data.is_empty() {
                                reply.words[0] = 0;
                                reply.words[1] = if eof { 1 } else { 0 };
                            } else {
                                reply = reply_with_shm(
                                    TLS_REPLY,
                                    data.len() as u64,
                                    if eof { 1 } else { 0 },
                                    &data,
                                );
                            }
                        }
                        None => {
                            reply.label = TLS_ERROR;
                            reply.words[0] = 2;
                        }
                    }
                }
                TLS_CLOSE => {
                    let sid = msg.words[0];
                    if close_session(sid) {
                        reply.words[0] = 0;
                    } else {
                        reply.label = TLS_ERROR;
                        reply.words[0] = 2;
                    }
                }
                TLS_INSTALL => {
                    // certificatectl: install a CA DER (via shm) into kv under
                    // compact tls.ca.* keys, then rebuild.
                    let name = unpack_str(&msg.words, 2, (IPC_REGISTER_WORDS - 2) * 8);
                    let len = (msg.words[0] as usize).min(SHM_PAGE);
                    let tok = msg.caps[0];
                    let mut ok = false;
                    if !name.is_empty() && tok != CapabilityToken::INVALID && len > 0 {
                        if let Ok(ptr) = shm_map(tok) {
                            let der = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
                            let _ = shm_free(tok);
                            if tls_ca_key(&name)
                                .as_deref()
                                .is_some_and(|key| kv_put_shm(key, &der))
                            {
                                append_to_index(&name);
                                rebuild_config();
                                ok = true;
                            }
                        }
                    }
                    if ok {
                        reply.words[0] = 0;
                    } else {
                        reply.label = TLS_ERROR;
                        reply.words[0] = 4;
                    }
                }
                TLS_LIST => {
                    let index = kv_get_shm(TLS_CA_INDEX_KEY).unwrap_or_default();
                    let s = String::from_utf8(index).unwrap_or_default();
                    let count = s.split(',').filter(|n| !n.trim().is_empty()).count();
                    reply.words[0] = count as u64;
                    let _ = pack_str_register_words(&mut reply, 1, &s);
                }
                _ => {
                    reply.label = TLS_ERROR;
                    reply.words[0] = 0xff;
                }
            }

            msg = ipc_reply_and_wait(ep, reply);
        }
    }

    fn append_to_index(name: &str) {
        let mut s = kv_get_shm(TLS_CA_INDEX_KEY)
            .and_then(|v| String::from_utf8(v).ok())
            .unwrap_or_default();
        if s.split(',').any(|n| n.trim() == name) {
            return;
        }
        if !s.is_empty() {
            s.push(',');
        }
        s.push_str(name);
        let _ = kv_put_shm(TLS_CA_INDEX_KEY, s.as_bytes());
    }

    fn local_time_str() -> heapless::String<32> {
        use core::fmt::Write;
        use sunlight_ipc::TzMsg;
        let mut out = heapless::String::<32>::new();
        if let Some(tz_cap) = nameserver_lookup("tz") {
            let r = ipc_call(tz_cap, IpcMsg::with_label(TzMsg::GET_LOCAL_TIME));
            if r.label == TzMsg::REPLY {
                let w0 = r.words[0];
                let year = (w0 >> 48) as u16;
                let month = ((w0 >> 40) & 0xff) as u8;
                let day = ((w0 >> 32) & 0xff) as u8;
                let hour = ((w0 >> 24) & 0xff) as u8;
                let minute = ((w0 >> 16) & 0xff) as u8;
                let second = ((w0 >> 8) & 0xff) as u8;
                let _ = write!(
                    &mut out,
                    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                    year, month, day, hour, minute, second
                );
                return out;
            }
        }
        let _ = out.push_str("tz-unavail");
        out
    }

    // silence unused warnings for helpers kept for symmetry
    #[allow(dead_code)]
    fn _unused() {
        let _ = (pack_ipv4([0; 4]), monotonic_millis, stdout_write);
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        let mut buf = heapless::String::<256>::new();
        use core::fmt::Write;
        let _ = write!(&mut buf, "[SUNLIGHT-TLS] PANIC: {}\n", info);
        debug_log(&buf);
        loop {}
    }
}

#[cfg(feature = "sunlightos")]
#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlightos_impl::run()
}
