//! sunlight-sunsay — SunlightOS native proof-of-life binary.
//!
//! Prints a Sunlight-themed ASCII art frame with a user-provided message or a
//! built-in quote.  This is a smoke test for the full userland stack:
//!   ELF loading → _start → argv → alloc (Vec/String) → write(1,…) → exit
//!
//! Build (from workspace root):
//!   RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static" \
//!     cargo build --package sunlight-sunsay --release
//!
//! Usage (from the SunlightOS shell):
//!   sunlight-sunsay "I said this"
//!   sunlight-sunsay

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use sunlight_libc::{exit, write, STDOUT};

// ── panic handler ────────────────────────────────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = write(STDOUT, b"sunlight-sunsay: panic\n");
    exit(101);
}

// ── stdout helpers ───────────────────────────────────────────────────────────

fn print_bytes(s: &[u8]) {
    let mut written = 0;
    while written < s.len() {
        match write(STDOUT, &s[written..]) {
            Ok(n) if n > 0 => written += n,
            _ => break,
        }
    }
}

macro_rules! println {
    () => {
        print_bytes(b"\n")
    };
    ($fmt:literal) => {
        print_bytes(concat!($fmt, "\n").as_bytes())
    };
    ($fmt:literal, $($arg:tt)*) => {{
        let s: String = alloc::format!(concat!($fmt, "\n"), $($arg)*);
        print_bytes(s.as_bytes());
    }};
}

// ── ASCII art content ────────────────────────────────────────────────────────

/// Width of the message text area (right-hand column, in characters).
const WIDTH: usize = 33;

const QUOTES: &[&str] = &[
    "Build slowly. Boot proudly.",
    "Small kernels, bright userlands.",
    "Keep the kernel tiny and the ideas huge.",
    "Every syscall is a promise. Keep it honest.",
    "SunlightOS says: no magic, just clean layers.",
    "If it boots, it speaks. If it speaks, it lives.",
    "Undefined behavior fears the sunlight.",
    "Powered by tiny syscalls and suspicious optimism.",
];

// ── _start ───────────────────────────────────────────────────────────────────

/// Kernel-provided entry point.
/// rdi = argc (u64), rsi = argv (*const *const u8, NULL-terminated).
/// See sunlight_libc::crt0 for the full ABI documentation.
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let args = collect_args(argc, argv);

    let message = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        QUOTES[0].to_string()
    };

    let lines = wrap_text(&message, WIDTH);
    print_frame(&lines);

    exit(0);
}

// ── argv collection ──────────────────────────────────────────────────────────

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

// ── frame rendering ──────────────────────────────────────────────────────────

fn print_frame(lines: &[String]) {
    println!(".-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.");
    println!(".            _.,.__       .                                   .");
    println!(".           ((o\\\\o\\))     . Tip:                              .");

    print_art_line(".     .-.    `  \\\\``      .", lines.first());
    print_art_line(".  __(   )___.o\"^^\".,___  .", lines.get(1));
    print_art_line(".     ===    ~~~~~~~~     .", lines.get(2));
    print_art_line(".      ==             ldb .", lines.get(3));
    print_art_line(".       =                 .", lines.get(4));

    for line in lines.iter().skip(5) {
        print_art_line(".                         .", Some(line));
    }

    println!(".-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.-.");
}

fn print_art_line(left: &str, text: Option<&String>) {
    let text_str: &str = match text {
        Some(s) => s.as_str(),
        None => "",
    };
    // Format: "{left} {text:<WIDTH} ."
    let s: String = alloc::format!("{} {:<width$} .", left, text_str, width = WIDTH);
    print_bytes(s.as_bytes());
    print_bytes(b"\n");
}

// ── text wrapping ────────────────────────────────────────────────────────────

fn wrap_text(input: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();

    for word in input.split_whitespace() {
        if current.is_empty() {
            push_word_or_split(word, width, &mut current, &mut lines);
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = String::new();
            push_word_or_split(word, width, &mut current, &mut lines);
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

fn push_word_or_split(word: &str, width: usize, current: &mut String, lines: &mut Vec<String>) {
    if word.len() <= width {
        current.push_str(word);
        return;
    }

    // Word is wider than the column — split at character boundaries and push
    // each chunk directly to lines. current is empty at this call site.
    let mut start = 0;
    while start < word.len() {
        let mut end = (start + width).min(word.len());
        while end > start && !word.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            break;
        }
        lines.push(word[start..end].to_string());
        start = end;
    }
}
