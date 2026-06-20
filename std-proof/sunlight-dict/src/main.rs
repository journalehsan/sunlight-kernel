//! sunlight-dict — dictionary lookup utility for SunlightOS.
//!
//! Reads from one of 26 letter-indexed TSV files embedded at compile time.
//! TSV format per line:  word<TAB>wordtype<TAB>definition
//!
//! Usage:
//!   dict WORD
//!   dict "multi word phrase"
//!
//! Exit codes: 0 found, 1 not found, 2 error.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use sunlight_libc::{env, exit, write, write_all, Fd, STDERR, STDOUT};

// ── Dictionary data (compile-time embedded) ───────────────────────────────────

static DICT_A: &[u8] = include_bytes!("../../../docs/dicts/dictionary-a.tsv");
static DICT_B: &[u8] = include_bytes!("../../../docs/dicts/dictionary-b.tsv");
static DICT_C: &[u8] = include_bytes!("../../../docs/dicts/dictionary-c.tsv");
static DICT_D: &[u8] = include_bytes!("../../../docs/dicts/dictionary-d.tsv");
static DICT_E: &[u8] = include_bytes!("../../../docs/dicts/dictionary-e.tsv");
static DICT_F: &[u8] = include_bytes!("../../../docs/dicts/dictionary-f.tsv");
static DICT_G: &[u8] = include_bytes!("../../../docs/dicts/dictionary-g.tsv");
static DICT_H: &[u8] = include_bytes!("../../../docs/dicts/dictionary-h.tsv");
static DICT_I: &[u8] = include_bytes!("../../../docs/dicts/dictionary-i.tsv");
static DICT_J: &[u8] = include_bytes!("../../../docs/dicts/dictionary-j.tsv");
static DICT_K: &[u8] = include_bytes!("../../../docs/dicts/dictionary-k.tsv");
static DICT_L: &[u8] = include_bytes!("../../../docs/dicts/dictionary-l.tsv");
static DICT_M: &[u8] = include_bytes!("../../../docs/dicts/dictionary-m.tsv");
static DICT_N: &[u8] = include_bytes!("../../../docs/dicts/dictionary-n.tsv");
static DICT_O: &[u8] = include_bytes!("../../../docs/dicts/dictionary-o.tsv");
static DICT_P: &[u8] = include_bytes!("../../../docs/dicts/dictionary-p.tsv");
static DICT_Q: &[u8] = include_bytes!("../../../docs/dicts/dictionary-q.tsv");
static DICT_R: &[u8] = include_bytes!("../../../docs/dicts/dictionary-r.tsv");
static DICT_S: &[u8] = include_bytes!("../../../docs/dicts/dictionary-s.tsv");
static DICT_T: &[u8] = include_bytes!("../../../docs/dicts/dictionary-t.tsv");
static DICT_U: &[u8] = include_bytes!("../../../docs/dicts/dictionary-u.tsv");
static DICT_V: &[u8] = include_bytes!("../../../docs/dicts/dictionary-v.tsv");
static DICT_W: &[u8] = include_bytes!("../../../docs/dicts/dictionary-w.tsv");
static DICT_X: &[u8] = include_bytes!("../../../docs/dicts/dictionary-x.tsv");
static DICT_Y: &[u8] = include_bytes!("../../../docs/dicts/dictionary-y.tsv");
static DICT_Z: &[u8] = include_bytes!("../../../docs/dicts/dictionary-z.tsv");
static DICT_OTHER: &[u8] = include_bytes!("../../../docs/dicts/dictionary-other.tsv");

// ── Panic handler ─────────────────────────────────────────────────────────────

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    let _ = write(STDERR, b"dict: internal panic\n");
    exit(101);
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, envp: *const *const u8) -> ! {
    env::init(envp);
    let args = collect_args(argc, argv);
    let code = run(&args);
    exit(code);
}

// ── Argument collection ───────────────────────────────────────────────────────

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

// ── Command dispatch ──────────────────────────────────────────────────────────

fn run(args: &[String]) -> u64 {
    if args.len() < 2 {
        print(STDOUT, b"Usage: dict WORD\n");
        print(STDOUT, b"       dict --prefix PREFIX\n");
        print(STDOUT, b"       dict --random\n");
        return 1;
    }

    match args[1].as_str() {
        "--prefix" => {
            if args.len() < 3 {
                print(STDERR, b"dict: --prefix requires a word\n");
                return 2;
            }
            let prefix = to_lowercase(&join_args(&args[2..]));
            cmd_prefix(&prefix)
        }
        "--random" => cmd_random(),
        _ => {
            let query = to_lowercase(&join_args(&args[1..]));
            cmd_lookup(&query)
        }
    }
}

// ── Commands ──────────────────────────────────────────────────────────────────

fn cmd_lookup(query: &str) -> u64 {
    let first = query.bytes().next().unwrap_or(0);
    let dict = select_dict(first);
    if search_exact(dict, query.as_bytes()) { 0 } else { 1 }
}

fn cmd_prefix(prefix: &str) -> u64 {
    let first = prefix.bytes().next().unwrap_or(0);
    let dict = select_dict(first);
    if search_prefix(dict, prefix.as_bytes()) { 0 } else { 1 }
}

fn cmd_random() -> u64 {
    // Use a simple pseudo-random walk across DICT_A using the entry count.
    let count = count_lines(DICT_A);
    if count == 0 {
        print(STDERR, b"dict: no entries found\n");
        return 1;
    }
    // Simple deterministic "random" via time-based seed (uptime from sysinfo).
    let seed = sunlight_libc::sysinfo()
        .map(|i| i.unix_time ^ i.uptime_secs)
        .unwrap_or(42);
    let target = (seed as usize) % count;
    print_nth_entry(DICT_A, target);
    0
}

// ── Dictionary selection ──────────────────────────────────────────────────────

fn select_dict(first_byte: u8) -> &'static [u8] {
    match first_byte.to_ascii_lowercase() {
        b'a' => DICT_A,
        b'b' => DICT_B,
        b'c' => DICT_C,
        b'd' => DICT_D,
        b'e' => DICT_E,
        b'f' => DICT_F,
        b'g' => DICT_G,
        b'h' => DICT_H,
        b'i' => DICT_I,
        b'j' => DICT_J,
        b'k' => DICT_K,
        b'l' => DICT_L,
        b'm' => DICT_M,
        b'n' => DICT_N,
        b'o' => DICT_O,
        b'p' => DICT_P,
        b'q' => DICT_Q,
        b'r' => DICT_R,
        b's' => DICT_S,
        b't' => DICT_T,
        b'u' => DICT_U,
        b'v' => DICT_V,
        b'w' => DICT_W,
        b'x' => DICT_X,
        b'y' => DICT_Y,
        b'z' => DICT_Z,
        _ => DICT_OTHER,
    }
}

// ── Search routines ───────────────────────────────────────────────────────────

/// Exact match: entry word must equal `query` (case-insensitive).
/// Returns true if at least one match was printed.
fn search_exact(dict: &[u8], query: &[u8]) -> bool {
    let mut found = false;
    for_each_line(dict, |line| {
        let Some(tab1) = find_byte(line, b'\t') else {
            return;
        };
        let entry_word = trim_cr(&line[..tab1]);
        if !bytes_eq_lower(entry_word, query) {
            return;
        }
        print_entry(entry_word, &line[tab1 + 1..]);
        found = true;
    });
    if !found {
        print(STDOUT, b"word not found\n");
    }
    found
}

/// Prefix match: entry word must start with `prefix` (case-insensitive).
/// Returns true if at least one match was printed.
fn search_prefix(dict: &[u8], prefix: &[u8]) -> bool {
    let mut found = false;
    for_each_line(dict, |line| {
        let Some(tab1) = find_byte(line, b'\t') else {
            return;
        };
        let entry_word = trim_cr(&line[..tab1]);
        if !bytes_starts_with_lower(entry_word, prefix) {
            return;
        }
        print_entry(entry_word, &line[tab1 + 1..]);
        found = true;
    });
    if !found {
        print(STDOUT, b"word not found\n");
    }
    found
}

// ── Output helpers ────────────────────────────────────────────────────────────

/// Print one formatted entry: "word (type): definition" or "word: definition".
fn print_entry(entry_word: &[u8], after_word: &[u8]) {
    // after_word = "type\tdefinition" or "\tdefinition"
    let tab2 = find_byte(after_word, b'\t');
    let (word_type, definition) = match tab2 {
        Some(pos) => (&after_word[..pos], &after_word[pos + 1..]),
        None => (&after_word[..0], after_word),
    };

    let definition = trim_cr(definition);

    print(STDOUT, entry_word);
    if !word_type.is_empty() {
        print(STDOUT, b" (");
        print(STDOUT, word_type);
        print(STDOUT, b")");
    }
    print(STDOUT, b": ");
    print(STDOUT, definition);
    print(STDOUT, b"\n");
}

fn print_nth_entry(dict: &[u8], target: usize) {
    let mut idx = 0usize;
    for_each_line(dict, |line| {
        if idx == target {
            let Some(tab1) = find_byte(line, b'\t') else {
                idx += 1;
                return;
            };
            let entry_word = trim_cr(&line[..tab1]);
            print_entry(entry_word, &line[tab1 + 1..]);
        }
        idx += 1;
    });
}

fn count_lines(dict: &[u8]) -> usize {
    let mut n = 0usize;
    for_each_line(dict, |_| n += 1);
    n
}

fn print(fd: Fd, s: &[u8]) {
    let _ = write_all(fd, s);
}

// ── Byte utilities ────────────────────────────────────────────────────────────

fn for_each_line(data: &[u8], mut f: impl FnMut(&[u8])) {
    let mut start = 0;
    for i in 0..data.len() {
        if data[i] == b'\n' {
            let line = &data[start..i];
            if !line.is_empty() && line != b"\r" {
                f(line);
            }
            start = i + 1;
        }
    }
    // trailing bytes with no newline
    if start < data.len() {
        let line = &data[start..];
        if !line.is_empty() && line != b"\r" {
            f(line);
        }
    }
}

fn find_byte(haystack: &[u8], needle: u8) -> Option<usize> {
    haystack.iter().position(|&b| b == needle)
}

fn trim_cr(b: &[u8]) -> &[u8] {
    if b.last() == Some(&b'\r') { &b[..b.len() - 1] } else { b }
}

/// Case-insensitive ASCII equality (a lowercased, b already lowercase).
fn bytes_eq_lower(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(&x, &y)| x.to_ascii_lowercase() == y)
}

/// Case-insensitive prefix check (entry word vs already-lowercase prefix).
fn bytes_starts_with_lower(entry: &[u8], prefix: &[u8]) -> bool {
    if entry.len() < prefix.len() {
        return false;
    }
    entry[..prefix.len()]
        .iter()
        .zip(prefix.iter())
        .all(|(&a, &b)| a.to_ascii_lowercase() == b)
}

// ── String helpers (alloc) ────────────────────────────────────────────────────

fn to_lowercase(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        for lc in c.to_lowercase() {
            out.push(lc);
        }
    }
    out
}

fn join_args(args: &[String]) -> String {
    let mut out = String::new();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(arg.as_str());
    }
    out
}
