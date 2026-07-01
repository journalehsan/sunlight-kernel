//! sunlight-thumbd — asynchronous thumbnail daemon for SunlightOS.
//!
//! Receives thumbnail requests via IPC, decodes .simg/.tga sources, scales
//! them, and writes to the on-disk cache:
//!
//!   <home>/.cache/sunlightos/thumbs/normal/<hex>.simg  (≤128×128)
//!   <home>/.cache/sunlightos/thumbs/large/<hex>.simg   (≤256×256)
//!
//! IPC protocol (ThumbMsg):
//!   PATH_CHUNK (0x7411): words[0..3] = 32 bytes of source path (non-final).
//!   PATH_END   (0x7412): words[0..2] = ≤24 remaining bytes;
//!                         words[3] = (path_len as u32) | ((size as u32) << 32).
//!   REPLY      (0x74FF): ACK; generation happens in this call (v0 cooperative).
//!   ERROR      (0x74FE): malformed request or source missing.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

// ── Bump allocator (512 KiB) ──────────────────────────────────────────────────

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        const SIZE: usize = 512 * 1024;
        static mut HEAP: [u8; SIZE] = [0u8; SIZE];
        static mut NEXT: usize = 0;
        let aligned = (NEXT + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned + layout.size();
        if end > SIZE {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

// ── Imports ───────────────────────────────────────────────────────────────────

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, process_yield,
    EndpointId, IpcMsg, ThumbMsg,
};
use sunlight_libc::{
    self as libc, env, mkdir_recursive, open_with_flags, read_dir, rename, stat, unlink, write_all,
    DirEntry, Fd, O_CREAT, O_RDONLY, O_TRUNC, O_WRONLY,
};
use sunlight_ui::image::simg::{decode, encode_tga_type2_bgr24, scale_fit};

// ── Constants ──────────────────────────────────────────────────────────────────

const MAX_PATH: usize = 256;
const MAX_NORMAL_EDGE: u32 = 128;
const MAX_LARGE_EDGE: u32 = 256;
const CACHE_MAX_BYTES: u64 = 64 * 1024 * 1024;

// ── Fixed-capacity path buffer ─────────────────────────────────────────────────

struct PathBuf {
    bytes: [u8; MAX_PATH],
    len: usize,
}

impl PathBuf {
    const fn empty() -> Self {
        Self {
            bytes: [0u8; MAX_PATH],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn append_bytes(&mut self, src: &[u8]) {
        for &b in src {
            if b == 0 {
                break;
            }
            if self.len < MAX_PATH {
                self.bytes[self.len] = b;
                self.len += 1;
            }
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

// ── FNV-1a 64-bit hash → 16-char hex cache key ────────────────────────────────

fn fnv1a_64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn cache_key(src: &[u8], size_flag: u32) -> [u8; 16] {
    let mut buf = [0u8; MAX_PATH + 4];
    let n = src.len().min(MAX_PATH);
    buf[..n].copy_from_slice(&src[..n]);
    buf[n..n + 4].copy_from_slice(&size_flag.to_le_bytes());
    let h = fnv1a_64(&buf[..n + 4]);
    let nib = b"0123456789abcdef";
    let mut hex = [0u8; 16];
    for i in 0..8 {
        let byte = ((h >> (56 - i * 8)) & 0xFF) as u8;
        hex[i * 2] = nib[(byte >> 4) as usize];
        hex[i * 2 + 1] = nib[(byte & 0xF) as usize];
    }
    hex
}

// ── Path builders ─────────────────────────────────────────────────────────────

fn build_cache_path(
    home: &[u8],
    key: &[u8; 16],
    size_dir: &[u8],
    ext: &[u8],
    out: &mut [u8; MAX_PATH],
) -> usize {
    let mut p = 0usize;
    macro_rules! append {
        ($slice:expr) => {
            for &b in $slice {
                if p < MAX_PATH - 1 {
                    out[p] = b;
                    p += 1;
                }
            }
        };
    }
    append!(home);
    append!(b"/.cache/sunlightos/thumbs/");
    append!(size_dir);
    append!(b"/");
    append!(key.as_ref());
    append!(ext);
    out[p] = 0;
    p
}

fn cache_dir(home: &[u8], size_dir: &[u8], out: &mut [u8; MAX_PATH]) -> usize {
    let mut p = 0usize;
    macro_rules! append {
        ($slice:expr) => {
            for &b in $slice {
                if p < MAX_PATH - 1 {
                    out[p] = b;
                    p += 1;
                }
            }
        };
    }
    append!(home);
    append!(b"/.cache/sunlightos/thumbs/");
    append!(size_dir);
    out[p] = 0;
    p
}

// ── Thumbnail generation ───────────────────────────────────────────────────────

fn generate_thumb(src_path: &[u8], home: &[u8], size_flag: u32) {
    // Stat + read source file.
    let src_stat = match stat(src_path) {
        Ok(s) => s,
        Err(_) => {
            debug_log("[THUMBD] source not found\n");
            return;
        }
    };
    let file_size = src_stat.size;
    if file_size == 0 || file_size > 32 * 1024 * 1024 {
        debug_log("[THUMBD] source size out of range\n");
        return;
    }

    let fd = match open_with_flags(src_path, O_RDONLY) {
        Ok(f) => f,
        Err(_) => {
            debug_log("[THUMBD] open failed\n");
            return;
        }
    };
    let mut src_data: Vec<u8> = Vec::with_capacity(file_size as usize);
    src_data.resize(file_size as usize, 0u8);
    let n = libc::read(fd, &mut src_data).unwrap_or(0);
    let _ = libc::close(fd);
    if n < 18 {
        debug_log("[THUMBD] short read\n");
        return;
    }
    src_data.truncate(n);

    // Decode SIMG/TGA.
    let img = match decode(&src_data) {
        Ok(i) => i,
        Err(_) => {
            debug_log("[THUMBD] decode failed\n");
            return;
        }
    };
    let max_edge = if size_flag == 0 {
        MAX_NORMAL_EDGE
    } else {
        MAX_LARGE_EDGE
    };
    let thumb = scale_fit(&img, max_edge, max_edge);
    let encoded = encode_tga_type2_bgr24(&thumb);

    let size_dir: &[u8] = if size_flag == 0 { b"normal" } else { b"large" };

    // Ensure cache directory exists.
    let mut dir_buf = [0u8; MAX_PATH];
    let dlen = cache_dir(home, size_dir, &mut dir_buf);
    let _ = mkdir_recursive(&dir_buf[..dlen]);

    // Build file paths.
    let key = cache_key(src_path, size_flag);
    let mut thumb_path = [0u8; MAX_PATH];
    let tlen = build_cache_path(home, &key, size_dir, b".simg", &mut thumb_path);
    let mut tmp_path = [0u8; MAX_PATH];
    let tmplen = build_cache_path(home, &key, size_dir, b".simg.tmp", &mut tmp_path);
    let mut meta_path = [0u8; MAX_PATH];
    let mlen = build_cache_path(home, &key, size_dir, b".meta", &mut meta_path);

    // Atomic write: write to .tmp then rename.
    let fd = match open_with_flags(&tmp_path[..tmplen], O_WRONLY | O_CREAT | O_TRUNC) {
        Ok(f) => f,
        Err(_) => {
            debug_log("[THUMBD] cannot create tmp\n");
            return;
        }
    };
    if write_all(fd, &encoded).is_err() {
        let _ = libc::close(fd);
        debug_log("[THUMBD] write failed\n");
        return;
    }
    let _ = libc::close(fd);

    if rename(&tmp_path[..tmplen], &thumb_path[..tlen]).is_err() {
        debug_log("[THUMBD] rename failed\n");
        return;
    }

    // Write .meta (best-effort).
    write_meta(
        &meta_path[..mlen],
        src_path,
        file_size,
        thumb.width,
        thumb.height,
    );

    if let Ok(s) = core::str::from_utf8(&thumb_path[..tlen]) {
        debug_log("[THUMBD] cached: ");
        debug_log(s);
        debug_log("\n");
    }
}

fn write_meta(path: &[u8], src: &[u8], src_size: u64, tw: u32, th: u32) {
    let fd = match open_with_flags(path, O_WRONLY | O_CREAT | O_TRUNC) {
        Ok(f) => f,
        Err(_) => return,
    };
    let _ = write_all(fd, b"source_path=");
    let _ = write_all(fd, src);
    let _ = write_all(fd, b"\nsource_size=");
    write_u64_fd(fd, src_size);
    let _ = write_all(fd, b"\nthumb_w=");
    write_u32_fd(fd, tw);
    let _ = write_all(fd, b"\nthumb_h=");
    write_u32_fd(fd, th);
    let _ = write_all(fd, b"\ndecoder_version=1\n");
    let _ = libc::close(fd);
}

fn write_u64_fd(fd: Fd, v: u64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    let mut n = v;
    if n == 0 {
        let _ = write_all(fd, b"0");
        return;
    }
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let _ = write_all(fd, &buf[i..]);
}

fn write_u32_fd(fd: Fd, v: u32) {
    write_u64_fd(fd, v as u64);
}

// ── IPC word → byte extraction ────────────────────────────────────────────────

fn words_to_bytes(words: &[u64; 8], n_words: usize, n_bytes: usize, out: &mut [u8]) {
    let mut w = 0usize;
    'outer: for wi in 0..n_words {
        for bi in 0..8usize {
            if w >= n_bytes || w >= out.len() {
                break 'outer;
            }
            out[w] = ((words[wi] >> (bi * 8)) & 0xFF) as u8;
            w += 1;
        }
    }
}

// ── Pre-warm sample pictures ──────────────────────────────────────────────────

fn prewarm_sample_pictures(home: &[u8]) {
    const SAMPLES: &[&[u8]] = &[
        b"/usr/share/sunlightos/sample-pictures/01_solar_blossom.simg",
        b"/usr/share/sunlightos/sample-pictures/02_amber_dunes.simg",
        b"/usr/share/sunlightos/sample-pictures/03_blue_garden.simg",
        b"/usr/share/sunlightos/sample-pictures/04_lantern_jellyfish.simg",
        b"/usr/share/sunlightos/sample-pictures/05_sleepy_koala.simg",
        b"/usr/share/sunlightos/sample-pictures/06_sunrise_lighthouse.simg",
        b"/usr/share/sunlightos/sample-pictures/07_penguin_walk.simg",
        b"/usr/share/sunlightos/sample-pictures/08_orange_tulips.simg",
    ];
    for &path in SAMPLES {
        let key = cache_key(path, 0);
        let mut check = [0u8; MAX_PATH];
        let clen = build_cache_path(home, &key, b"normal", b".simg", &mut check);
        if stat(&check[..clen]).is_ok() {
            continue;
        }
        generate_thumb(path, home, 0);
        process_yield();
    }
}

// ── Cache cleanup (evict oldest when over 64 MiB) ────────────────────────────

fn cleanup_cache(home: &[u8]) {
    let mut total: u64 = 0;
    // (path_buf, len, size) for up to 64 candidate files
    let mut candidates: [([u8; MAX_PATH], usize, u64); 64] = [([0u8; MAX_PATH], 0usize, 0u64); 64];
    let mut count = 0usize;

    for size_dir in [b"normal" as &[u8], b"large"] {
        let mut dir_buf = [0u8; MAX_PATH];
        let dlen = cache_dir(home, size_dir, &mut dir_buf);
        let mut dir_entries = [DirEntry::zeroed(); 32];
        let n = match read_dir(&dir_buf[..dlen], &mut dir_entries) {
            Ok(n) => n,
            Err(_) => continue,
        };
        for entry in &dir_entries[..n] {
            let name = entry.name_bytes();
            if name.len() < 5 || &name[name.len() - 5..] != b".simg" {
                continue;
            }
            total += entry.size;
            if count < 64 {
                let mut fp = [0u8; MAX_PATH];
                let mut flen = 0usize;
                for &b in dir_buf[..dlen].iter().chain(b"/".iter()).chain(name) {
                    if flen < MAX_PATH - 1 {
                        fp[flen] = b;
                        flen += 1;
                    }
                }
                candidates[count] = (fp, flen, entry.size);
                count += 1;
            }
        }
    }

    if total <= CACHE_MAX_BYTES {
        return;
    }

    let mut freed: u64 = 0;
    for i in 0..count {
        if total.saturating_sub(freed) <= CACHE_MAX_BYTES {
            break;
        }
        let (ref fp, flen, sz) = candidates[i];
        if unlink(&fp[..flen]).is_ok() {
            freed += sz;
        }
    }
}

// ── Panic / entry ──────────────────────────────────────────────────────────────

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[THUMBD] panic\n");
    sunlight_libc::exit(1);
}

#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8, envp: *const *const u8) -> ! {
    env::init(envp);
    let home_str = env::getenv(b"HOME").unwrap_or("/root");
    let home = home_str.as_bytes();

    // Create cache directories.
    let _ = mkdir_recursive(home);
    {
        let mut d = [0u8; MAX_PATH];
        let n = cache_dir(home, b"normal", &mut d);
        let _ = mkdir_recursive(&d[..n]);
    }
    {
        let mut d = [0u8; MAX_PATH];
        let n = cache_dir(home, b"large", &mut d);
        let _ = mkdir_recursive(&d[..n]);
    }
    {
        let mut d = [0u8; MAX_PATH];
        let mut p = 0usize;
        for &b in home
            .iter()
            .chain(b"/.cache/sunlightos/thumbs/failed".iter())
        {
            if p < MAX_PATH - 1 {
                d[p] = b;
                p += 1;
            }
        }
        let _ = mkdir_recursive(&d[..p]);
    }

    let ep = endpoint_create();
    nameserver_register("thumbd", ep);
    debug_log("[THUMBD] registered as 'thumbd'\n");

    cleanup_cache(home);
    prewarm_sample_pictures(home);
    debug_log("[THUMBD] ready\n");

    let mut in_flight_pid: u64 = 0;
    let mut path_buf = PathBuf::empty();

    let mut msg = ipc_recv(ep);
    loop {
        let sender = msg.badge;
        let reply = match msg.label {
            ThumbMsg::PATH_CHUNK => {
                if sender != in_flight_pid {
                    path_buf.clear();
                    in_flight_pid = sender;
                }
                let mut chunk = [0u8; 32];
                words_to_bytes(&msg.words, 4, 32, &mut chunk);
                path_buf.append_bytes(&chunk);
                let mut r = IpcMsg::with_label(ThumbMsg::REPLY);
                r.words[0] = 0;
                r
            }
            ThumbMsg::PATH_END => {
                if sender != in_flight_pid {
                    path_buf.clear();
                    in_flight_pid = sender;
                }
                let mut tail = [0u8; 24];
                words_to_bytes(&msg.words, 3, 24, &mut tail);
                path_buf.append_bytes(&tail);

                let meta = msg.words[3];
                let path_total_len = (meta & 0xFFFF_FFFF) as usize;
                let size_flag = ((meta >> 32) & 0xFFFF_FFFF) as u32;

                if path_total_len < path_buf.len {
                    path_buf.len = path_total_len;
                }

                let mut src_copy = [0u8; MAX_PATH];
                let slen = path_buf.len.min(MAX_PATH - 1);
                src_copy[..slen].copy_from_slice(&path_buf.bytes[..slen]);

                path_buf.clear();
                in_flight_pid = 0;

                generate_thumb(&src_copy[..slen], home, size_flag);

                let mut r = IpcMsg::with_label(ThumbMsg::REPLY);
                r.words[0] = 0;
                r
            }
            _ => IpcMsg::with_label(ThumbMsg::ERROR),
        };
        msg = ipc_reply_and_wait(ep, reply);
    }
}
