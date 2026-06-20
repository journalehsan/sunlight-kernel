use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{env, fs, io};
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

        if let Event::Key(key) = event::read()? {
            let h = terminal.size()?.height as usize - 2;
            match (key.modifiers, key.code) {
                (_, KeyCode::Char('q')) | (KeyModifiers::CONTROL, KeyCode::Char('q')) => break,
                (_, KeyCode::Up)
                | (KeyModifiers::NONE, KeyCode::Char('k'))
                | (KeyModifiers::NONE, KeyCode::Char('h')) => app.up(),
                (_, KeyCode::Down)
                | (KeyModifiers::NONE, KeyCode::Char('j'))
                | (KeyModifiers::NONE, KeyCode::Char('l')) => app.down(h),
                (KeyModifiers::CONTROL, KeyCode::Char('b')) | (_, KeyCode::PageUp) => {
                    app.page_up(h)
                }
                (KeyModifiers::CONTROL, KeyCode::Char('f')) | (_, KeyCode::PageDown) => {
                    app.page_down(h)
                }
                (KeyModifiers::CONTROL, KeyCode::Char('u')) => app.page_up(h / 2),
                (KeyModifiers::CONTROL, KeyCode::Char('d')) => app.page_down(h / 2),
                (_, KeyCode::Home) | (KeyModifiers::NONE, KeyCode::Char('g')) => app.top(),
                (_, KeyCode::End) | (KeyModifiers::NONE, KeyCode::Char('G')) => app.bottom(h),
                _ => {}
            }
        }
    }
    Ok(())
}
