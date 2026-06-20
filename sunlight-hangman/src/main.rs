#![no_std]
#![no_main]

use sunlight_libc::{exit, getrandom, read, write_all, Errno, GRND_NONCRYPTO, STDIN, STDOUT};

const MAX_MISSES: usize = 6;
const WORD_MAX_LEN: usize = 8;

const WORDS: &[&[u8]] = &[
    b"RUST", b"CARGO", b"CRATE", b"KERNEL", b"UNIX", b"LINUX", b"BASH",
    b"SHELL", b"BORROW", b"OWNER", b"LIMINE", b"QEMU", b"BOOT", b"EXEC",
    b"PIPE", b"THREAD", b"SIGNAL", b"BUFFER", b"SCHED", b"SOCKET",
    b"MODULE", b"TRAIT", b"STRUCT", b"ENUM", b"MACRO", b"INODE", b"PROC",
    b"DEVICE", b"NETWORK", b"ARRAY", b"VECTOR", b"STRING", b"SLICE",
    b"MATCH", b"IMPL", b"FMT", b"PANIC", b"INPUT", b"OUTPUT", b"ERROR",
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

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = write_all(STDOUT, b"hangman: panic\n");
    exit(101);
}

fn pick_word() -> [u8; WORD_MAX_LEN] {
    let mut seed = 0u64;
    let seed_slice = unsafe {
        core::slice::from_raw_parts_mut(&mut seed as *mut u64 as *mut u8, 8)
    };
    getrandom(seed_slice, GRND_NONCRYPTO);

    let idx = (seed as usize) % WORDS.len();
    let word = WORDS[idx];
    let mut buf = [b' '; WORD_MAX_LEN];
    buf[..word.len()].copy_from_slice(word);
    buf
}

fn word_len(buf: &[u8; WORD_MAX_LEN]) -> usize {
    buf.iter().position(|&c| c == b' ').unwrap_or(WORD_MAX_LEN)
}

fn word_str(buf: &[u8; WORD_MAX_LEN]) -> &[u8] {
    let len = word_len(buf);
    &buf[..len]
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut guessed = [false; 26];
    let mut misses = 0usize;
    let word = pick_word();

    print_intro();
    loop {
        render(&word, &guessed, misses);
        if solved(&word, &guessed) {
            let _ = write_all(STDOUT, b"\nYou got it! The word was: ");
            let _ = write_all(STDOUT, word_str(&word));
            let _ = write_all(STDOUT, b"\n");
            exit(0);
        }
        if misses >= MAX_MISSES {
            let _ = write_all(STDOUT, b"\nGame over. The word was: ");
            let _ = write_all(STDOUT, word_str(&word));
            let _ = write_all(STDOUT, b"\n");
            exit(1);
        }

        let _ = write_all(STDOUT, b"Guess a letter: ");
        match read_letter() {
            Some(letter) => {
                let idx = (letter - b'A') as usize;
                if guessed[idx] {
                    let _ = write_all(STDOUT, b"Already guessed.\n");
                } else {
                    guessed[idx] = true;
                    if !word_str(&word).contains(&letter) {
                        misses += 1;
                        let _ = write_all(STDOUT, b"Nope.\n");
                    }
                }
            }
            None => exit(0),
        }
    }
}

fn print_intro() {
    let _ = write_all(
        STDOUT,
        b"Sunlight Hangman\nUnix/Rust edition.\n\n",
    );
}

fn render(word: &[u8; WORD_MAX_LEN], guessed: &[bool; 26], misses: usize) {
    let _ = write_all(STDOUT, ART[misses].as_bytes());
    let _ = write_all(STDOUT, b"Word: ");
    let wlen = word_len(word);
    for &ch in &word[..wlen] {
        let idx = (ch - b'A') as usize;
        if guessed[idx] {
            let _ = write_all(STDOUT, &[ch, b' ']);
        } else {
            let _ = write_all(STDOUT, b"_ ");
        }
    }
    let _ = write_all(STDOUT, b"\n\n");
}

fn solved(word: &[u8; WORD_MAX_LEN], guessed: &[bool; 26]) -> bool {
    let wlen = word_len(word);
    word[..wlen].iter().all(|&ch| guessed[(ch - b'A') as usize])
}

fn read_letter() -> Option<u8> {
    let mut byte = [0u8; 1];
    loop {
        match read(STDIN, &mut byte) {
            Ok(0) => return None,
            Ok(_) => {
                let b = byte[0];
                if b == 4 {
                    return None;
                }
                if b.is_ascii_alphabetic() {
                    let upper = b.to_ascii_uppercase();
                    let _ = write_all(STDOUT, &[upper, b'\n']);
                    return Some(upper);
                }
            }
            Err(Errno::Again) => sunlight_libc::yield_now(),
            Err(_) => return None,
        }
    }
}
