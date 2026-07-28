//! Application controller and state loop.

use crate::core::{buffer::TextBuffer, cursor::Cursor, search::SearchState, undo::UndoHistory};
use crate::file_ops::open_file;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct App {
    pub filename: String,
    pub buffer: TextBuffer,
    pub cursor: Cursor,
    pub undo_history: UndoHistory,
    pub search: SearchState,
    pub show_help: bool,
    pub show_quit_confirm: bool,
    pub show_search_prompt: bool,
    pub search_input: String,
    pub status_message: Option<String>,
    pub should_quit: bool,
}

impl App {
    pub fn new(path: &str) -> Self {
        match open_file(path) {
            Ok(res) => {
                let status = if res.is_new {
                    Some(format!("New file: {}", path))
                } else {
                    Some(format!("Loaded {}", path))
                };
                Self {
                    filename: res.path,
                    buffer: res.buffer,
                    cursor: Cursor::new(),
                    undo_history: UndoHistory::default(),
                    search: SearchState::new(),
                    show_help: false,
                    show_quit_confirm: false,
                    show_search_prompt: false,
                    search_input: String::new(),
                    status_message: status,
                    should_quit: false,
                }
            }
            Err(e) => Self {
                filename: path.to_string(),
                buffer: TextBuffer::new(),
                cursor: Cursor::new(),
                undo_history: UndoHistory::default(),
                search: SearchState::new(),
                show_help: false,
                show_quit_confirm: false,
                show_search_prompt: false,
                search_input: String::new(),
                status_message: Some(format!("Error loading file: {}", e)),
                should_quit: false,
            },
        }
    }

    pub fn save(&mut self) {
        match self.buffer.save_to_file_atomic(&self.filename) {
            Ok(is_atomic) => {
                let method = if is_atomic { "atomic" } else { "direct" };
                self.status_message = Some(format!("Saved ({}) {}", method, self.filename));
            }
            Err(e) => {
                self.status_message = Some(format!("Error saving file: {}", e));
            }
        }
    }

    pub fn request_quit(&mut self) {
        if self.buffer.is_modified() {
            self.show_quit_confirm = true;
        } else {
            self.should_quit = true;
        }
    }

    pub fn perform_undo(&mut self) {
        if let Some(prev) = self.undo_history.undo(self.buffer.lines.clone()) {
            self.buffer.lines = prev;
            self.cursor.clamp_col(&self.buffer);
            self.status_message = Some("Undo".to_string());
        }
    }

    pub fn perform_redo(&mut self) {
        if let Some(next) = self.undo_history.redo(self.buffer.lines.clone()) {
            self.buffer.lines = next;
            self.cursor.clamp_col(&self.buffer);
            self.status_message = Some("Redo".to_string());
        }
    }

    fn push_undo_step(&mut self) {
        self.undo_history.push_snapshot(self.buffer.lines.clone());
    }

    pub fn handle_key(&mut self, key: KeyEvent, view_height: usize, view_width: usize) {
        // Search prompt input mode
        if self.show_search_prompt {
            match key.code {
                KeyCode::Char(c) => {
                    self.search_input.push(c);
                    self.search
                        .update_query(self.search_input.clone(), &self.buffer);
                    if let Some(m) = self.search.current_match() {
                        self.cursor.line = m.line;
                        self.cursor.col = m.col;
                    }
                }
                KeyCode::Backspace => {
                    self.search_input.pop();
                    self.search
                        .update_query(self.search_input.clone(), &self.buffer);
                    if let Some(m) = self.search.current_match() {
                        self.cursor.line = m.line;
                        self.cursor.col = m.col;
                    }
                }
                KeyCode::Enter | KeyCode::Esc => {
                    self.show_search_prompt = false;
                }
                _ => {}
            }
            self.cursor
                .adjust_viewport(view_width, view_height, &self.buffer, 4);
            return;
        }

        // Quit confirmation modal
        if self.show_quit_confirm {
            match key.code {
                KeyCode::Char('y')
                | KeyCode::Char('Y')
                | KeyCode::Char('s')
                | KeyCode::Char('S') => {
                    self.save();
                    self.should_quit = true;
                }
                KeyCode::Char('d')
                | KeyCode::Char('D')
                | KeyCode::Char('n')
                | KeyCode::Char('N') => {
                    self.should_quit = true;
                }
                KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('C') => {
                    self.show_quit_confirm = false;
                    self.status_message = Some("Cancelled exit".to_string());
                }
                _ => {}
            }
            return;
        }

        // Help modal
        if self.show_help {
            if key.code == KeyCode::Esc
                || (key.code == KeyCode::Char('g') && key.modifiers.contains(KeyModifiers::CONTROL))
            {
                self.show_help = false;
            }
            return;
        }

        // Shortcuts
        if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.save();
            return;
        }

        if (key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL))
            || (key.code == KeyCode::Char('x') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            self.request_quit();
            return;
        }

        if key.code == KeyCode::Char('g') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.show_help = true;
            return;
        }

        // Undo / Redo
        if key.code == KeyCode::Char('z') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.perform_undo();
            self.cursor
                .adjust_viewport(view_width, view_height, &self.buffer, 4);
            return;
        }

        if key.code == KeyCode::Char('y') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.perform_redo();
            self.cursor
                .adjust_viewport(view_width, view_height, &self.buffer, 4);
            return;
        }

        // Search (^F and F3)
        if key.code == KeyCode::Char('f') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.show_search_prompt = true;
            self.search_input.clear();
            return;
        }

        if key.code == KeyCode::F(3) {
            if let Some(m) = self.search.next_match() {
                self.cursor.line = m.line;
                self.cursor.col = m.col;
                self.status_message = Some(format!(
                    "Match {}/{}",
                    self.search.current_idx + 1,
                    self.search.matches.len()
                ));
            } else {
                self.status_message = Some("No matches".to_string());
            }
            self.cursor
                .adjust_viewport(view_width, view_height, &self.buffer, 4);
            return;
        }

        // Editing Operations
        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
            match key.code {
                KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                    self.push_undo_step();
                    let (nl, nc) = self
                        .buffer
                        .insert_newline(self.cursor.line, self.cursor.col);
                    self.cursor.line = nl;
                    self.cursor.col = nc;
                }
                KeyCode::Char(ch) => {
                    self.push_undo_step();
                    let (nl, nc) = self
                        .buffer
                        .insert_char(self.cursor.line, self.cursor.col, ch);
                    self.cursor.line = nl;
                    self.cursor.col = nc;
                }
                KeyCode::Backspace => {
                    self.push_undo_step();
                    let (nl, nc) = self
                        .buffer
                        .delete_backspace(self.cursor.line, self.cursor.col);
                    self.cursor.line = nl;
                    self.cursor.col = nc;
                }
                KeyCode::Delete => {
                    self.push_undo_step();
                    self.buffer.delete_char(self.cursor.line, self.cursor.col);
                }
                KeyCode::Tab => {
                    self.push_undo_step();
                    let (nl, nc) = self
                        .buffer
                        .insert_char(self.cursor.line, self.cursor.col, '\t');
                    self.cursor.line = nl;
                    self.cursor.col = nc;
                }
                _ => {}
            }
        }

        // Navigation Keys
        match key.code {
            KeyCode::Up => self.cursor.move_up(&self.buffer),
            KeyCode::Down => self.cursor.move_down(&self.buffer),
            KeyCode::Left => self.cursor.move_left(&self.buffer),
            KeyCode::Right => self.cursor.move_right(&self.buffer),
            KeyCode::Home => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.cursor.move_top();
                } else {
                    self.cursor.move_home();
                }
            }
            KeyCode::End => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.cursor.move_bottom(&self.buffer);
                } else {
                    self.cursor.move_end(&self.buffer);
                }
            }
            KeyCode::PageUp => self.cursor.page_up(&self.buffer, view_height),
            KeyCode::PageDown => self.cursor.page_down(&self.buffer, view_height),
            _ => {}
        }

        self.cursor
            .adjust_viewport(view_width, view_height, &self.buffer, 4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_line_feed_creates_a_buffer_line() {
        let mut app = App::new("/helios-note-newline-regression-missing");
        app.handle_key(
            KeyEvent::new(KeyCode::Char('\n'), KeyModifiers::NONE),
            20,
            80,
        );

        assert_eq!(app.buffer.lines, vec![String::new(), String::new()]);
        assert_eq!((app.cursor.line, app.cursor.col), (1, 0));
    }
}
