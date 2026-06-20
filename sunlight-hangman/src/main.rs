#![no_std]
#![no_main]

use sunlight_libc::{exit, read, write_all, Errno, STDIN, STDOUT};

const WORD: &[u8] = b"SUNLIGHT";
const MAX_MISSES: usize = 6;

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

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut guessed = [false; 26];
    let mut misses = 0usize;

    print_intro();
    loop {
        render(&guessed, misses);
        if solved(&guessed) {
            let _ = write_all(STDOUT, b"\nYou saved the sunbeam. Word: SUNLIGHT\n");
            exit(0);
        }
        if misses >= MAX_MISSES {
            let _ = write_all(STDOUT, b"\nGame over. The word was SUNLIGHT.\n");
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
                    if !WORD.contains(&letter) {
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
        b"Sunlight Hangman\nASCII-art libc smoke test for interactive input.\n\n",
    );
}

fn render(guessed: &[bool; 26], misses: usize) {
    let _ = write_all(STDOUT, ART[misses].as_bytes());
    let _ = write_all(STDOUT, b"Word: ");
    for &ch in WORD {
        let idx = (ch - b'A') as usize;
        if guessed[idx] {
            let _ = write_all(STDOUT, &[ch, b' ']);
        } else {
            let _ = write_all(STDOUT, b"_ ");
        }
    }
    let _ = write_all(STDOUT, b"\n\n");
}

fn solved(guessed: &[bool; 26]) -> bool {
    WORD.iter().all(|&ch| guessed[(ch - b'A') as usize])
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
