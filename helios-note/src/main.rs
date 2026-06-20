use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;

struct Terminal {
    original: libc::termios,
}

impl Terminal {
    fn enter_raw_mode() -> io::Result<Self> {
        let fd = io::stdin().as_raw_fd();

        let mut original = unsafe {
            let mut termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut termios) == -1 {
                return Err(io::Error::last_os_error());
            }
            termios
        };

        let mut raw = original;

        /*
            Raw mode:
            - disable echo
            - disable canonical input
            - disable Ctrl-C/Ctrl-Z signal handling
            - disable CRLF translation
            - read byte-by-byte
        */

        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::ISIG | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL | libc::BRKINT | libc::INPCK | libc::ISTRIP);
        raw.c_oflag &= !(libc::OPOST);
        raw.c_cflag |= libc::CS8;

        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;

        if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &raw) } == -1 {
            return Err(io::Error::last_os_error());
        }

        let mut out = io::stdout();
        write!(out, "\x1b[?1049h")?; // alternate screen
        write!(out, "\x1b[?25l")?; // hide cursor
        out.flush()?;

        Ok(Self { original })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let fd = io::stdin().as_raw_fd();

        unsafe {
            libc::tcsetattr(fd, libc::TCSAFLUSH, &self.original);
        }

        let mut out = io::stdout();
        let _ = write!(out, "\x1b[?25h"); // show cursor
        let _ = write!(out, "\x1b[?1049l"); // leave alternate screen
        let _ = out.flush();
    }
}

#[derive(Debug, Clone, Copy)]
enum Key {
    Char(u8),
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    PageUp,
    PageDown,
    Home,
    End,
    Escape,
    Unknown,
}

fn read_key() -> io::Result<Key> {
    let mut stdin = io::stdin();
    let mut buf = [0u8; 1];

    stdin.read_exact(&mut buf)?;

    if buf[0] != b'\x1b' {
        return Ok(Key::Char(buf[0]));
    }

    let mut seq = [0u8; 3];

    // Try reading escape sequence bytes.
    // In raw blocking mode this can block if user presses bare Escape.
    // For this toy viewer it is acceptable, but later we can use VTIME.
    if stdin.read(&mut seq[0..1])? == 0 {
        return Ok(Key::Escape);
    }

    if seq[0] == b'[' {
        if stdin.read(&mut seq[1..2])? == 0 {
            return Ok(Key::Escape);
        }

        match seq[1] {
            b'A' => return Ok(Key::ArrowUp),
            b'B' => return Ok(Key::ArrowDown),
            b'C' => return Ok(Key::ArrowRight),
            b'D' => return Ok(Key::ArrowLeft),
            b'H' => return Ok(Key::Home),
            b'F' => return Ok(Key::End),

            b'1' | b'4' | b'5' | b'6' | b'7' | b'8' => {
                if stdin.read(&mut seq[2..3])? == 0 {
                    return Ok(Key::Unknown);
                }

                if seq[2] == b'~' {
                    return Ok(match seq[1] {
                        b'1' | b'7' => Key::Home,
                        b'4' | b'8' => Key::End,
                        b'5' => Key::PageUp,
                        b'6' => Key::PageDown,
                        _ => Key::Unknown,
                    });
                }
            }

            _ => {}
        }
    }

    Ok(Key::Unknown)
}

fn terminal_size() -> io::Result<(usize, usize)> {
    let fd = io::stdout().as_raw_fd();

    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };

    let rc = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) };

    if rc == -1 || ws.ws_col == 0 || ws.ws_row == 0 {
        // fallback
        Ok((80, 24))
    } else {
        Ok((ws.ws_col as usize, ws.ws_row as usize))
    }
}

struct App {
    filename: String,
    lines: Vec<String>,
    scroll_y: usize,
}

impl App {
    fn open(filename: String) -> io::Result<Self> {
        let content = fs::read_to_string(&filename)?;

        let mut lines: Vec<String> = content.lines().map(|line| line.to_string()).collect();

        if lines.is_empty() {
            lines.push(String::new());
        }

        Ok(Self {
            filename,
            lines,
            scroll_y: 0,
        })
    }

    fn scroll_up(&mut self) {
        self.scroll_y = self.scroll_y.saturating_sub(1);
    }

    fn scroll_down(&mut self, viewport_height: usize) {
        let max_scroll = self.lines.len().saturating_sub(viewport_height);
        self.scroll_y = (self.scroll_y + 1).min(max_scroll);
    }

    fn page_up(&mut self, viewport_height: usize) {
        self.scroll_y = self.scroll_y.saturating_sub(viewport_height);
    }

    fn page_down(&mut self, viewport_height: usize) {
        let max_scroll = self.lines.len().saturating_sub(viewport_height);
        self.scroll_y = (self.scroll_y + viewport_height).min(max_scroll);
    }

    fn render(&self) -> io::Result<()> {
        let (cols, rows) = terminal_size()?;
        let content_rows = rows.saturating_sub(1);

        let mut out = io::stdout();

        write!(out, "\x1b[H")?; // cursor home
        write!(out, "\x1b[2J")?; // clear screen

        for screen_row in 0..content_rows {
            let file_row = self.scroll_y + screen_row;

            write!(out, "\x1b[{};1H", screen_row + 1)?;

            if file_row < self.lines.len() {
                let line = &self.lines[file_row];

                let visible = truncate_to_columns(line, cols);
                write!(out, "{}", visible)?;
            } else {
                write!(out, "~")?;
            }

            write!(out, "\x1b[K")?; // clear to end of line
        }

        self.render_status_bar(cols, rows)?;

        out.flush()
    }

    fn render_status_bar(&self, cols: usize, rows: usize) -> io::Result<()> {
        let mut out = io::stdout();

        let status = format!(
            " {} | {} lines | scroll {} | q: quit ",
            self.filename,
            self.lines.len(),
            self.scroll_y
        );

        let mut bar = truncate_to_columns(&status, cols);

        while bar.len() < cols {
            bar.push(' ');
        }

        write!(out, "\x1b[{};1H", rows)?;
        write!(out, "\x1b[7m")?; // reverse video
        write!(out, "{}", bar)?;
        write!(out, "\x1b[0m")?;

        Ok(())
    }
}

fn truncate_to_columns(s: &str, cols: usize) -> String {
    /*
        Simple ASCII-ish truncation.
        For UTF-8 Persian/emoji/wide chars this is not display-width correct.
        Later we can use unicode-width crate, but for minimal deps we avoid it.
    */
    let mut result = String::new();

    for ch in s.chars() {
        if result.len() + ch.len_utf8() > cols {
            break;
        }

        result.push(ch);
    }

    result
}

fn main() -> io::Result<()> {
    let filename = match env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("usage: helios-note <file>");
            std::process::exit(1);
        }
    };

    let _terminal = Terminal::enter_raw_mode()?;

    let mut app = App::open(filename)?;
    app.render()?;

    loop {
        let (_, rows) = terminal_size()?;
        let content_rows = rows.saturating_sub(1);

        fn read_key() -> io::Result<Key> {
            let mut stdin = io::stdin();
            let mut buf = [0u8; 1];

            // 1. Robust polling loop for the first byte
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => {
                        // TTY returned 0 bytes (no data ready). Sleep briefly to prevent a CPU spin-loop.
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Ok(_) => break, // Key pressed! (This satisfies the compiler for 1, 2, 3...)
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => return Err(e),
                }
            }

            if buf[0] != b'\x1b' {
                return Ok(Key::Char(buf[0]));
            }

            let mut seq = [0u8; 3];

            // 2. Safe non-blocking reads for escape sequence bytes
            match stdin.read(&mut seq[0..1]) {
                Ok(0) | Err(_) => return Ok(Key::Escape),
                Ok(1) => {}
                _ => return Ok(Key::Escape),
            }

            if seq[0] == b'[' {
                match stdin.read(&mut seq[1..2]) {
                    Ok(0) | Err(_) => return Ok(Key::Escape),
                    Ok(1) => {}
                    _ => return Ok(Key::Escape),
                }

                match seq[1] {
                    b'A' => return Ok(Key::ArrowUp),
                    b'B' => return Ok(Key::ArrowDown),
                    b'C' => return Ok(Key::ArrowRight),
                    b'D' => return Ok(Key::ArrowLeft),
                    b'H' => return Ok(Key::Home),
                    b'F' => return Ok(Key::End),

                    b'1' | b'4' | b'5' | b'6' | b'7' | b'8' => {
                        match stdin.read(&mut seq[2..3]) {
                            Ok(0) | Err(_) => return Ok(Key::Unknown),
                            Ok(1) => {}
                            _ => return Ok(Key::Unknown),
                        }

                        if seq[2] == b'~' {
                            return Ok(match seq[1] {
                                b'1' | b'7' => Key::Home,
                                b'4' | b'8' => Key::End,
                                b'5' => Key::PageUp,
                                b'6' => Key::PageDown,
                                _ => Key::Unknown,
                            });
                        }
                    }
                    _ => {}
                }
            }

            Ok(Key::Unknown)
        }

        app.render()?;
    }

    Ok(())
}
