#![no_std]
#![no_main]

use sunlight_libc::{exit, getrandom, read, write_all, Errno, GRND_NONCRYPTO, STDIN, STDOUT};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_MISSES: usize = 6;
const WORD_MAX_LEN: usize = 8;

const WORDS: &[&[u8]] = &[
    b"RUST", b"CARGO", b"CRATE", b"KERNEL", b"UNIX", b"LINUX", b"BASH", b"SHELL", b"BORROW",
    b"OWNER", b"LIMINE", b"QEMU", b"BOOT", b"EXEC", b"PIPE", b"THREAD", b"SIGNAL", b"BUFFER",
    b"SCHED", b"SOCKET", b"MODULE", b"TRAIT", b"STRUCT", b"ENUM", b"MACRO", b"INODE", b"PROC",
    b"DEVICE", b"NETWORK", b"ARRAY", b"VECTOR", b"STRING", b"SLICE", b"MATCH", b"IMPL", b"FMT",
    b"PANIC", b"INPUT", b"OUTPUT", b"ERROR",
];

const ART: [&str; 7] = [
    " +---+\n |   |\n     |\n     |\n     |\n     |\n=========\n",
    " +---+\n |   |\n O   |\n     |\n     |\n     |\n=========\n",
    " +---+\n |   |\n O   |\n |   |\n     |\n     |\n=========\n",
    " +---+\n |   |\n O   |\n/|   |\n     |\n     |\n=========\n",
    " +---+\n |   |\n O   |\n/|\\  |\n     |\n     |\n=========\n",
    " +---+\n |   |\n O   |\n/|\\  |\n/    |\n     |\n=========\n",
    " +---+\n |   |\n O   |\n/|\\  |\n/ \\  |\n     |\n=========\n",
];

// ANSI escape sequences (widely supported in modern terminals)
const ANSI_RESET: &[u8] = b"\x1b[0m";
const ANSI_BOLD: &[u8] = b"\x1b[1m";
const ANSI_RED: &[u8] = b"\x1b[31m";
const ANSI_GREEN: &[u8] = b"\x1b[32m";
const ANSI_YELLOW: &[u8] = b"\x1b[33m";
const ANSI_CYAN: &[u8] = b"\x1b[36m";
const ANSI_CLEAR: &[u8] = b"\x1b[2J\x1b[H"; // clear screen + home cursor

// ---------------------------------------------------------------------------
// Panic handler (no_std requirement)
// ---------------------------------------------------------------------------
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = write_all(STDOUT, b"hangman: panic\n");
    exit(101);
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Write a slice of bytes to stdout; ignore errors (best effort).
fn puts(data: &[u8]) {
    let _ = write_all(STDOUT, data);
}

/// Pick a random word using `getrandom` as the entropy source.
fn pick_word() -> [u8; WORD_MAX_LEN] {
    let mut seed = 0u64;
    // SAFETY: `seed` is properly aligned for u64 and lives on the stack;
    // we pass a correctly sized slice of bytes.
    let seed_slice =
        unsafe { core::slice::from_raw_parts_mut(&mut seed as *mut u64 as *mut u8, 8) };
    getrandom(seed_slice, GRND_NONCRYPTO);

    let idx = (seed as usize) % WORDS.len();
    let word = WORDS[idx];
    let mut buf = [b' '; WORD_MAX_LEN];
    buf[..word.len()].copy_from_slice(word);
    buf
}

/// Length of the actual word (stripped from trailing spaces).
fn word_len(buf: &[u8; WORD_MAX_LEN]) -> usize {
    buf.iter().position(|&c| c == b' ').unwrap_or(WORD_MAX_LEN)
}

/// Slice of the actual word.
fn word_str(buf: &[u8; WORD_MAX_LEN]) -> &[u8] {
    let len = word_len(buf);
    &buf[..len]
}

/// Returns `true` if every letter of `word` has been guessed.
fn solved(word: &[u8; WORD_MAX_LEN], guessed: &[bool; 26]) -> bool {
    let wlen = word_len(word);
    word[..wlen].iter().all(|&ch| guessed[(ch - b'A') as usize])
}

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

/// Read one byte from stdin. On EOF or ^D returns `None`.
/// Non‑alphabetic input is silently ignored.
/// Alphabetic characters are uppercased and echoed.
fn read_letter() -> Option<u8> {
    let mut byte = [0u8; 1];
    loop {
        match read(STDIN, &mut byte) {
            Ok(0) => return None, // EOF
            Ok(_) => {
                let b = byte[0];
                if b == 4 {
                    return None; // ^D (EOT)
                }
                if b.is_ascii_alphabetic() {
                    let upper = b.to_ascii_uppercase();
                    puts(&[upper, b'\n']);
                    return Some(upper);
                }
            }
            Err(Errno::Again) => sunlight_libc::yield_now(),
            Err(_) => return None,
        }
    }
}

/// Ask a yes/no question. Returns `true` for 'Y', `false` for 'N' or EOF/^D.
fn ask_yes_no(prompt: &[u8]) -> bool {
    loop {
        puts(prompt);
        match read_letter() {
            Some(b'Y') => return true,
            Some(b'N') | None => return false,
            _ => puts(b"Please answer Y or N.\n"),
        }
    }
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

/// Print the intro banner (only shown once per session).
fn print_intro() {
    puts(ANSI_CLEAR);
    puts(ANSI_BOLD);
    puts(ANSI_CYAN);
    puts(b"   Sunlight Hangman\n");
    puts(b"   Unix / Rust edition\n");
    puts(ANSI_RESET);
    puts(b"\n");
}

/// Render the current game state: gallows, word, guessed letters, misses.
/// This function assumes the screen has been cleared just before calling.
fn render(word: &[u8; WORD_MAX_LEN], guessed: &[bool; 26], misses: usize) {
    // ---- Gallows (red) ----
    puts(ANSI_RED);
    puts(ART[misses].as_bytes());
    puts(ANSI_RESET);

    // ---- Word ----
    puts(b"Word: ");
    let wlen = word_len(word);
    let iter = 0..wlen;
    for i in iter {
        let ch = word[i];
        let idx = (ch - b'A') as usize;
        if guessed[idx] {
            puts(ANSI_GREEN);
            // Print the letter and a space
            let buf = [ch, b' '];
            puts(&buf);
            puts(ANSI_RESET);
        } else {
            puts(ANSI_YELLOW);
            puts(b"_ ");
            puts(ANSI_RESET);
        }
    }
    puts(b"\n\n");

    // ---- Guessed letters ----
    puts(b"Guessed: ");
    let w = word_str(word);
    let mut any_guessed = false;
    for ch in b'A'..=b'Z' {
        let idx = (ch - b'A') as usize;
        if guessed[idx] {
            any_guessed = true;
            if w.contains(&ch) {
                puts(ANSI_GREEN);
            } else {
                puts(ANSI_RED);
            }
            let buf = [ch, b' '];
            puts(&buf);
            puts(ANSI_RESET);
        }
    }
    if !any_guessed {
        puts(b"(none)");
    }
    puts(b"\n");

    // ---- Misses ----
    puts(ANSI_RED);
    // "Misses: X / MAX_MISSES"
    // We can't easily format numbers, so write a fixed string.
    // Quick helper: small array to convert digits.
    let mut buf = [b' '; 3]; // enough for "6/6"
    let mut pos = 0;
    let m = misses;
    if m >= 10 {
        buf[pos] = b'0' + (m / 10) as u8;
        pos += 1;
    }
    buf[pos] = b'0' + (m % 10) as u8;
    pos += 1;
    buf[pos] = b'/';
    pos += 1;
    buf[pos] = b'0' + MAX_MISSES as u8;
    pos += 1;
    puts(b"Misses: ");
    puts(&buf[..pos]);
    puts(b"\n");
    puts(ANSI_RESET);
    puts(b"\n");
}

// ---------------------------------------------------------------------------
// Game logic for a single round
// ---------------------------------------------------------------------------

/// Plays one game. Returns `true` if the player wants another round.
fn play_one_game() -> bool {
    let mut guessed = [false; 26];
    let mut misses = 0usize;
    let word = pick_word();

    // Clear screen and show initial frame.
    puts(ANSI_CLEAR);
    render(&word, &guessed, misses);

    loop {
        // Win?
        if solved(&word, &guessed) {
            puts(ANSI_GREEN);
            puts(b"\nYou got it! The word was: ");
            puts(word_str(&word));
            puts(b"\n");
            puts(ANSI_RESET);
            return ask_yes_no(b"Play again? (y/n): ");
        }
        // Lose?
        if misses >= MAX_MISSES {
            puts(ANSI_RED);
            puts(b"\nGame over. The word was: ");
            puts(word_str(&word));
            puts(b"\n");
            puts(ANSI_RESET);
            return ask_yes_no(b"Play again? (y/n): ");
        }

        // Prompt for a guess.
        puts(b"Guess a letter: ");
        match read_letter() {
            Some(letter) => {
                let idx = (letter - b'A') as usize;
                if guessed[idx] {
                    puts(b"Already guessed.\n");
                } else {
                    guessed[idx] = true;
                    if !word_str(&word).contains(&letter) {
                        misses += 1;
                        puts(b"Nope.\n");
                    }
                }
                // Redraw the screen after each guess.
                puts(ANSI_CLEAR);
                render(&word, &guessed, misses);
            }
            None => return false, // EOF -> exit whole program
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------
#[no_mangle]
pub extern "C" fn _start() -> ! {
    print_intro();

    // Main replay loop
    while play_one_game() {
        // continue
    }

    // Clean exit
    exit(0);
}
