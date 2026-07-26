//! sunlight-zoxide — directory jump utility for SunlightOS.
//!
//! Tracks visited directories by score and resolves fuzzy queries.
//!
//! Commands:
//!   z --add PATH          Add/increment a directory in the database.
//!   z --resolve TERM...   Print the best-matching path.
//!   z --list              List all entries sorted by score (descending).
//!   z --doctor            Show diagnostic information.
//!
//! Database: $HOME/.config/sunlight-zoxide/db.txt
//! Format per line: "<score>\t<path>\n"

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use sunlight_libc::{
    close, env, exit, fstat, mkdir_recursive, open, open_with_flags, read, stat, write, write_all,
    Fd, MAX_ARGS, O_CREAT, O_TRUNC, O_WRONLY, STDERR, STDOUT,
};

const MAX_ARG_LEN: usize = 1024;

// ── panic handler ────────────────────────────────────────────────────────────

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    let _ = write(STDERR, b"z: internal panic\n");
    exit(101);
}

// ── stdout/stderr helpers ─────────────────────────────────────────────────────

fn print(fd: Fd, s: &[u8]) {
    let _ = write_all(fd, s);
}

fn println(fd: Fd, s: &[u8]) {
    print(fd, s);
    print(fd, b"\n");
}

fn println_str(fd: Fd, s: &str) {
    println(fd, s.as_bytes());
}

// ── entry point ───────────────────────────────────────────────────────────────

/// SysV ABI: rdi=argc, rsi=argv, rdx=envp.
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, envp: *const *const u8) -> ! {
    env::init(envp);
    let mut args = [""; MAX_ARGS];
    let count =
        unsafe { sunlight_libc::crt0::collect_utf8_args(argc, argv, &mut args, MAX_ARG_LEN) };
    let code = run(&args[..count]);
    exit(code);
}

// ── database path ─────────────────────────────────────────────────────────────

fn home_dir() -> &'static str {
    env::getenv(b"HOME").unwrap_or("/root")
}

fn db_dir(buf: &mut String) {
    buf.push_str(home_dir());
    buf.push_str("/.config/sunlight-zoxide");
}

fn db_path(buf: &mut String) {
    db_dir(buf);
    buf.push_str("/db.txt");
}

// ── file I/O helpers ──────────────────────────────────────────────────────────

fn read_file(path: &str) -> Option<String> {
    let fd = open(path.as_bytes()).ok()?;
    let mut content: Vec<u8> = Vec::new();
    if let Ok(metadata) = fstat(fd) {
        if let Ok(size) = usize::try_from(metadata.size) {
            if content.try_reserve_exact(size).is_err() {
                let _ = close(fd);
                return None;
            }
        }
    }
    let mut buf = [0u8; 4096];
    loop {
        match read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => content.extend_from_slice(&buf[..n]),
            Err(_) => {
                let _ = close(fd);
                return None;
            }
        }
    }
    let _ = close(fd);
    String::from_utf8(content).ok()
}

fn write_file(path: &str, content: &str) -> bool {
    match open_with_flags(path.as_bytes(), O_WRONLY | O_CREAT | O_TRUNC) {
        Ok(fd) => {
            let ok = write_all(fd, content.as_bytes()).is_ok();
            let _ = close(fd);
            ok
        }
        Err(_) => false,
    }
}

// ── database parsing / serialisation ─────────────────────────────────────────

#[derive(Clone)]
struct Entry {
    score: u64,
    path: String,
}

fn parse_db(text: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(tab) = line.find('\t') {
            let score_str = &line[..tab];
            let path_str = &line[tab + 1..];
            if let Some(score) = parse_u64(score_str) {
                entries.push(Entry {
                    score,
                    path: path_str.to_string(),
                });
            }
        }
    }
    entries
}

fn serialize_db(entries: &[Entry]) -> String {
    let capacity = entries.iter().fold(0usize, |total, entry| {
        total.saturating_add(21 + 1 + entry.path.len() + 1)
    });
    let mut out = String::with_capacity(capacity);
    for e in entries {
        push_u64(&mut out, e.score);
        out.push('\t');
        out.push_str(&e.path);
        out.push('\n');
    }
    out
}

fn load_db() -> Vec<Entry> {
    let mut path = String::new();
    db_path(&mut path);
    read_file(&path).map(|s| parse_db(&s)).unwrap_or_default()
}

fn save_db(entries: &[Entry]) -> bool {
    // Ensure directory exists.
    let mut dir = String::new();
    db_dir(&mut dir);
    let _ = mkdir_recursive(dir.as_bytes());

    let mut path = String::new();
    db_path(&mut path);
    let text = serialize_db(entries);
    write_file(&path, &text)
}

// ── commands ──────────────────────────────────────────────────────────────────

fn cmd_add(target: &str) -> u64 {
    if !target.starts_with('/') {
        println(STDERR, b"z: --add requires an absolute path");
        return 1;
    }
    let mut entries = load_db();
    let found = entries.iter_mut().find(|e| e.path == target);
    if let Some(e) = found {
        e.score = e.score.saturating_add(1);
    } else {
        entries.push(Entry {
            score: 1,
            path: target.to_string(),
        });
    }
    if save_db(&entries) {
        0
    } else {
        1
    }
}

fn cmd_resolve(terms: &[&str]) -> u64 {
    if terms.is_empty() {
        println(STDERR, b"z: --resolve requires at least one term");
        return 1;
    }
    let entries = load_db();

    // Collect all entries whose path contains every term (case-insensitive).
    let lower_terms: Vec<String> = terms.iter().map(|term| to_lower_string(term)).collect();
    let mut matches: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            let lower = to_lower_string(&e.path);
            lower_terms.iter().all(|term| lower.contains(term.as_str()))
        })
        .collect();

    if matches.is_empty() {
        println(STDERR, b"z: no match found");
        return 1;
    }

    // Score: visit_score * 100 - path_length
    matches.sort_by(|a, b| {
        let sa = a
            .score
            .saturating_mul(100)
            .saturating_sub(a.path.len() as u64);
        let sb = b
            .score
            .saturating_mul(100)
            .saturating_sub(b.path.len() as u64);
        sb.cmp(&sa)
    });

    if matches.len() >= 2 {
        let sa = matches[0]
            .score
            .saturating_mul(100)
            .saturating_sub(matches[0].path.len() as u64);
        let sb = matches[1]
            .score
            .saturating_mul(100)
            .saturating_sub(matches[1].path.len() as u64);
        if sa == sb {
            println(STDERR, b"z: ambiguous match - be more specific");
            print(STDERR, b"  ");
            println_str(STDERR, &matches[0].path);
            print(STDERR, b"  ");
            println_str(STDERR, &matches[1].path);
            return 2;
        }
    }

    println_str(STDOUT, &matches[0].path);
    0
}

fn cmd_list() -> u64 {
    let mut entries = load_db();
    // Sort descending by score.
    entries.sort_by(|a, b| b.score.cmp(&a.score));

    println(STDOUT, b"sunlight-zoxide database\n");
    println(STDOUT, b"score   path");
    println(STDOUT, b"-------------------------------");
    for e in &entries {
        let mut line = String::with_capacity(8 + e.path.len());
        push_u64(&mut line, e.score);
        // Pad score to 8 chars.
        while line.len() < 8 {
            line.push(' ');
        }
        line.push_str(&e.path);
        println_str(STDOUT, &line);
    }
    0
}

fn cmd_doctor() -> u64 {
    let mut path_buf = String::new();
    db_path(&mut path_buf);

    let exists = stat(path_buf.as_bytes()).is_ok();
    let entries = if exists { load_db().len() } else { 0 };

    let mut dir_buf = String::new();
    db_dir(&mut dir_buf);
    let dir_writable = stat(dir_buf.as_bytes()).is_ok();

    println(STDOUT, b"sunlight-zoxide doctor\n");

    print(STDOUT, b"db path: ");
    println_str(STDOUT, &path_buf);

    print(STDOUT, b"db exists: ");
    println(STDOUT, if exists { b"yes" } else { b"no" });

    print(STDOUT, b"entries: ");
    println_str(STDOUT, &u64_to_str(entries as u64));

    print(STDOUT, b"config directory writable: ");
    println(
        STDOUT,
        if dir_writable {
            b"yes"
        } else {
            b"no (not yet created)"
        },
    );

    0
}

// ── command dispatch ──────────────────────────────────────────────────────────

fn run(args: &[&str]) -> u64 {
    if args.len() < 2 {
        usage();
        return 1;
    }
    let cmd = args[1];
    match cmd {
        "--add" => {
            if args.len() < 3 {
                println(STDERR, b"z: --add requires a path argument");
                return 1;
            }
            cmd_add(args[2])
        }
        "--resolve" => cmd_resolve(&args[2..]),
        "--list" => cmd_list(),
        "--doctor" => cmd_doctor(),
        _ => {
            // Treat a bare argument as --resolve for shell integration convenience.
            cmd_resolve(&args[1..])
        }
    }
}

fn usage() {
    println(STDOUT, b"sunlight-zoxide - directory jump utility");
    println(STDOUT, b"");
    println(STDOUT, b"Usage:");
    println(
        STDOUT,
        b"  z --add PATH          Record a visited directory",
    );
    println(
        STDOUT,
        b"  z --resolve TERM...   Print the best matching path",
    );
    println(STDOUT, b"  z --list              List all database entries");
    println(
        STDOUT,
        b"  z --doctor            Show diagnostic information",
    );
}

// ── utilities ─────────────────────────────────────────────────────────────────

fn parse_u64(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut v = 0u64;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(v)
}

fn u64_to_str(n: u64) -> String {
    let mut out = String::with_capacity(20);
    push_u64(&mut out, n);
    out
}

fn push_u64(out: &mut String, mut n: u64) {
    let mut digits = [0u8; 20];
    let mut len = 0usize;
    if n == 0 {
        out.push('0');
        return;
    }
    while n > 0 {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
    }
    while len > 0 {
        len -= 1;
        out.push(digits[len] as char);
    }
}

fn to_lower_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        for lc in c.to_lowercase() {
            out.push(lc);
        }
    }
    out
}
