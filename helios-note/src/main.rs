mod app;
mod core;
mod file_ops;
mod ui;

use app::App;
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{env, io, time::Duration};
use ui::{draw_ui, UiState};

struct TerminalGuard;

impl TerminalGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let filename = args.get(1).map(|s| s.as_str()).unwrap_or("untitled.txt");

    let _guard = TerminalGuard::new()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let mut app = App::new(filename);
    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            terminal.draw(|f| {
                let area = f.size();
                let view_height = area.height.saturating_sub(3) as usize;
                let view_width = area.width.saturating_sub(6) as usize;

                app.cursor.adjust_viewport(view_width, view_height, &app.buffer, 4);

                draw_ui(
                    f,
                    UiState {
                        filename: &app.filename,
                        buffer: &app.buffer,
                        cursor: &app.cursor,
                        show_help: app.show_help,
                        show_quit_confirm: app.show_quit_confirm,
                        show_search_prompt: app.show_search_prompt,
                        search_input: &app.search_input,
                        status_message: app.status_message.as_deref(),
                    },
                );
            })?;
            needs_redraw = false;
        }

        if app.should_quit {
            break;
        }

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    let size = terminal.size()?;
                    let view_height = size.height.saturating_sub(3) as usize;
                    let view_width = size.width.saturating_sub(6) as usize;

                    app.handle_key(key, view_height, view_width);
                    needs_redraw = true;
                }
                Event::Resize(_, _) => {
                    needs_redraw = true;
                }
                _ => {}
            }
        }
    }

    Ok(())
}
