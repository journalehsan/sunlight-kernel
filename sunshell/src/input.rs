use std::io::{self, Read, Write};

pub enum ReadLine {
    Line(String),
    Eof,
}

/// Read one line from stdin byte-by-byte (no readline dependency).
/// Returns Eof on Ctrl+D / closed stdin.
pub fn readline(prompt: &str) -> io::Result<ReadLine> {
    let mut stdout = io::stdout();
    stdout.write_all(prompt.as_bytes())?;
    stdout.flush()?;

    let mut buf = Vec::with_capacity(128);
    let stdin = io::stdin();
    let mut byte = [0u8; 1];

    loop {
        match stdin.lock().read(&mut byte) {
            Ok(0) => {
                // EOF
                if buf.is_empty() {
                    // Print newline so the next shell prompt appears on a new line
                    let _ = stdout.write_all(b"\n");
                    return Ok(ReadLine::Eof);
                }
                // Partial line followed by EOF — treat as a line
                break;
            }
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(e) => return Err(e),
        }
    }

    Ok(ReadLine::Line(String::from_utf8_lossy(&buf).into_owned()))
}
