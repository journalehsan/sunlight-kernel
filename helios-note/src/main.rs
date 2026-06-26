use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{env, fs, io, io::Read};
use tui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

struct App {
    filename: String,
    lines: Vec<String>,
    scroll_y: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Char(u8),
    ArrowUp,
    ArrowDown,
    PageUp,
    PageDown,
    Home,
    End,
    Unknown,
}

impl App {
    fn open(filename: String) -> io::Result<Self> {
        let content = fs::read_to_string(&filename)?;
        let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Ok(Self {
            filename,
            lines,
            scroll_y: 0,
        })
    }

    fn up(&mut self) {
        self.scroll_y = self.scroll_y.saturating_sub(1);
    }
    fn down(&mut self, h: usize) {
        self.scroll_y = (self.scroll_y + 1).min(self.lines.len().saturating_sub(h));
    }
    fn page_up(&mut self, h: usize) {
        self.scroll_y = self.scroll_y.saturating_sub(h);
    }
    fn page_down(&mut self, h: usize) {
        self.scroll_y = (self.scroll_y + h).min(self.lines.len().saturating_sub(h));
    }
    fn top(&mut self) {
        self.scroll_y = 0;
    }
    fn bottom(&mut self, h: usize) {
        self.scroll_y = self.lines.len().saturating_sub(h);
    }
}

fn read_byte(stdin: &mut io::Stdin) -> io::Result<u8> {
    let mut buf = [0u8; 1];

    loop {
        match stdin.read(&mut buf) {
            Ok(0) => unsafe {
                libc::sched_yield();
            },
            Ok(_) => return Ok(buf[0]),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) =>
            unsafe {
                libc::sched_yield();
            },
            Err(e) => return Err(e),
        }
    }
}

fn read_key(stdin: &mut io::Stdin) -> io::Result<Key> {
    let b0 = read_byte(stdin)?;
    if b0 != b'\x1b' {
        return Ok(Key::Char(b0));
    }

    let b1 = read_byte(stdin)?;
    if b1 != b'[' {
        return Ok(Key::Unknown);
    }

    let b2 = read_byte(stdin)?;
    let key = match b2 {
        b'A' => Key::ArrowUp,
        b'B' => Key::ArrowDown,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'1' | b'4' | b'5' | b'6' | b'7' | b'8' => {
            let b3 = read_byte(stdin)?;
            if b3 != b'~' {
                Key::Unknown
            } else {
                match b2 {
                    b'1' | b'7' => Key::Home,
                    b'4' | b'8' => Key::End,
                    b'5' => Key::PageUp,
                    b'6' => Key::PageDown,
                    _ => Key::Unknown,
                }
            }
        }
        _ => Key::Unknown,
    };

    Ok(key)
}

fn main() -> io::Result<()> {
    let filename = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: viewer <file>");
        std::process::exit(1);
    });
    let mut app = App::open(filename)?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let res = run(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    res
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> io::Result<()> {
    let mut stdin = io::stdin();

    loop {
        terminal.draw(|f| {
            let size = f.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(size);

            let h = chunks[0].height as usize;
            let total = app.lines.len();
            let percent = if total == 0 {
                100
            } else {
                (app.scroll_y + 1) * 100 / total
            };

            let visible: Vec<Spans> = (0..h)
                .map(|i| {
                    let row = app.scroll_y + i;
                    if row < total {
                        Spans::from(vec![
                            Span::styled(
                                format!("{:4} │ ", row + 1),
                                Style::default().fg(Color::DarkGray),
                            ),
                            Span::raw(app.lines[row].clone()),
                        ])
                    } else {
                        Spans::from(Span::styled("   ~ ", Style::default().fg(Color::DarkGray)))
                    }
                })
                .collect();

            f.render_widget(
                Paragraph::new(visible).block(Block::default().borders(Borders::NONE)),
                chunks[0],
            );

            let status = format!(
                " {} | {}/{} {}% | q:quit  j/k:scroll  ^F/^B:page  ^D/^U:½page  g/G:top/bot",
                app.filename,
                app.scroll_y + 1,
                total,
                percent
            );
            f.render_widget(
                Paragraph::new(status).style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                chunks[1],
            );
        })?;

        let h = terminal.size()?.height as usize - 2;
        match read_key(&mut stdin)? {
            Key::Char(b'q') => break,
            Key::ArrowUp | Key::Char(b'k') | Key::Char(b'h') => app.up(),
            Key::ArrowDown | Key::Char(b'j') | Key::Char(b'l') => app.down(h),
            Key::PageUp | Key::Char(0x02) => app.page_up(h),
            Key::PageDown | Key::Char(0x06) => app.page_down(h),
            Key::Char(0x15) => app.page_up(h / 2),
            Key::Char(0x04) => app.page_down(h / 2),
            Key::Home | Key::Char(b'g') => app.top(),
            Key::End | Key::Char(b'G') => app.bottom(h),
            Key::Unknown | Key::Char(_) => {}
        }
    }
    Ok(())
}
