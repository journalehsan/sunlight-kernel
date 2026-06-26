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
    close, create, env, exit, mkdir_recursive, open, read, stat, write, write_all, Fd, STDERR,
    STDOUT,
};

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
    let args = collect_args(argc, argv);
    let code = run(&args);
    exit(code);
}

// ── argument collection ───────────────────────────────────────────────────────

fn collect_args(argc: u64, argv: *const *const u8) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if argv.is_null() {
        return args;
    }
    for i in 0..(argc as usize) {
        let ptr = unsafe { *argv.add(i) };
        if ptr.is_null() {
            break;
        }
        let mut len = 0usize;
        while unsafe { *ptr.add(len) } != 0 {
            len += 1;
        }
        let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
        let s = core::str::from_utf8(bytes).unwrap_or("").to_string();
        args.push(s);
    }
    args
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
    match create(path.as_bytes()) {
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
    let mut out = String::new();
    for e in entries {
        out.push_str(&u64_to_str(e.score));
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
    let mut matches: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            let lower = to_lower_string(&e.path);
            terms
                .iter()
                .all(|t| lower.contains(&to_lower_string(t) as &str))
        })
        .collect();

    if matches.is_empty() {
        println(STDERR, b"z: no match found");
        return 1;
    }

    // Score: visit_score * 100 - path_length
    matches.sort_by(|a, b| {
        let sa = a.score * 100 - a.path.len() as u64;
        let sb = b.score * 100 - b.path.len() as u64;
        sb.cmp(&sa)
    });

    if matches.len() >= 2 {
        let sa = matches[0].score * 100 - matches[0].path.len() as u64;
        let sb = matches[1].score * 100 - matches[1].path.len() as u64;
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
        let mut line = u64_to_str(e.score);
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

fn run(args: &[String]) -> u64 {
    if args.len() < 2 {
        usage();
        return 1;
    }
    let cmd = &args[1];
    match cmd.as_str() {
        "--add" => {
            if args.len() < 3 {
                println(STDERR, b"z: --add requires a path argument");
                return 1;
            }
            cmd_add(&args[2])
        }
        "--resolve" => {
            let terms: Vec<&str> = args[2..].iter().map(|s| s.as_str()).collect();
            cmd_resolve(&terms)
        }
        "--list" => cmd_list(),
        "--doctor" => cmd_doctor(),
        _ => {
            // Treat a bare argument as --resolve for shell integration convenience.
            let terms: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();
            cmd_resolve(&terms)
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

fn u64_to_str(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut digits: Vec<u8> = Vec::new();
    while n > 0 {
        digits.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    digits.reverse();
    String::from_utf8(digits).unwrap_or_else(|_| "0".to_string())
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
