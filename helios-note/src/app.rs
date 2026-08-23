//! Application controller and state loop.

use crate::core::{buffer::TextBuffer, cursor::Cursor, search::SearchState, undo::UndoHistory};
use crate::document::{resolve_target, Document};
use crate::file_ops::{describe_io_error, open_file};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::{Path, PathBuf};

/// Bounded single-field path prompt driving the Save As workflow.
pub struct SaveAsPrompt {
    pub input: String,
    pub cursor: usize,
    overwrite: Option<PathBuf>,
}

impl SaveAsPrompt {
    fn new(input: String) -> Self {
        let cursor = input.chars().count();
        Self {
            input,
            cursor,
            overwrite: None,
        }
    }

    pub fn pending_overwrite(&self) -> Option<&Path> {
        self.overwrite.as_deref()
    }

    pub fn split_at_cursor(&self) -> (String, String) {
        let at = self.byte_idx(self.cursor);
        (self.input[..at].to_string(), self.input[at..].to_string())
    }

    fn byte_idx(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    fn handle_edit(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char(c) => {
                let at = self.byte_idx(self.cursor);
                self.input.insert(at, c);
                self.cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let at = self.byte_idx(self.cursor - 1);
                    self.input.remove(at);
                    self.cursor -= 1;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.input.chars().count() {
                    let at = self.byte_idx(self.cursor);
                    self.input.remove(at);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.input.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.input.chars().count(),
            _ => {}
        }
    }
}

pub struct App {
    pub document: Document,
    pub save_as: Option<SaveAsPrompt>,
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
    /// New document: a display name only, with no backing filesystem path.
    pub fn untitled() -> Self {
        Self::with_document(Document::untitled(), TextBuffer::new(), None)
    }

    /// Document launched with a path argument, which becomes its backing path.
    pub fn open(path: &str) -> Self {
        match open_file(path) {
            Ok(res) => {
                let status = if res.is_new {
                    format!("New file: {}", path)
                } else {
                    format!("Loaded {}", path)
                };
                Self::with_document(Document::from_path(&res.path), res.buffer, Some(status))
            }
            Err(e) => Self::with_document(
                Document::from_path(path),
                TextBuffer::new(),
                Some(format!("Error loading file: {}", describe_io_error(&e))),
            ),
        }
    }

    fn with_document(
        document: Document,
        buffer: TextBuffer,
        status_message: Option<String>,
    ) -> Self {
        Self {
            document,
            save_as: None,
            buffer,
            cursor: Cursor::new(),
            undo_history: UndoHistory::default(),
            search: SearchState::new(),
            show_help: false,
            show_quit_confirm: false,
            show_search_prompt: false,
            search_input: String::new(),
            status_message,
            should_quit: false,
        }
    }

    /// Save writes an existing backing path; an untitled document enters Save As.
    pub fn save(&mut self) {
        if self.document.is_untitled() {
            self.begin_save_as();
            return;
        }
        if let Some(path) = self.document.backing_path().map(Path::to_path_buf) {
            self.save_to(&path, false);
        }
    }

    pub fn begin_save_as(&mut self) {
        self.save_as = Some(SaveAsPrompt::new(self.document.save_as_prefill()));
    }

    /// Shared write operation. `adopt` establishes the target as the backing
    /// path, and only ever after the write succeeded.
    fn save_to(&mut self, target: &Path, adopt: bool) -> bool {
        match self.buffer.save_to_file_atomic(&target.to_string_lossy()) {
            Ok(_) => {
                if adopt {
                    self.document.set_backing_path(target.to_path_buf());
                }
                self.status_message = Some(format!("Saved {}", target.display()));
                true
            }
            Err(e) => {
                self.status_message = Some(describe_io_error(&e));
                false
            }
        }
    }

    fn confirm_save_as(&mut self) {
        let Some(input) = self.save_as.as_ref().map(|p| p.input.clone()) else {
            return;
        };

        match resolve_target(&input, &self.document.save_as_base()) {
            Ok(target) => {
                let replaces_other = self.document.backing_path() != Some(target.as_path());
                if replaces_other && target.exists() {
                    if let Some(prompt) = self.save_as.as_mut() {
                        prompt.overwrite = Some(target);
                    }
                    return;
                }
                self.finish_save_as(target);
            }
            Err(msg) => self.status_message = Some(msg.to_string()),
        }
    }

    fn finish_save_as(&mut self, target: PathBuf) {
        if self.save_to(&target, true) {
            self.save_as = None;
        } else if let Some(prompt) = self.save_as.as_mut() {
            prompt.overwrite = None;
        }
    }

    fn handle_save_as_key(&mut self, key: KeyEvent) {
        if self.save_as.as_ref().is_some_and(|p| p.overwrite.is_some()) {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(target) = self.save_as.as_mut().and_then(|p| p.overwrite.take()) {
                        self.finish_save_as(target);
                    }
                }
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('n') | KeyCode::Char('N') => {
                    if let Some(prompt) = self.save_as.as_mut() {
                        prompt.overwrite = None;
                    }
                    self.status_message = Some("Overwrite cancelled".to_string());
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Enter => self.confirm_save_as(),
            KeyCode::Esc => {
                self.save_as = None;
                self.status_message = Some("Save As cancelled".to_string());
            }
            code => {
                if let Some(prompt) = self.save_as.as_mut() {
                    prompt.handle_edit(code);
                }
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
        // Save As prompt input mode
        if self.save_as.is_some() {
            self.handle_save_as_key(key);
            return;
        }

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
                    self.show_quit_confirm = false;
                    self.save();
                    // An untitled document opens Save As instead of quitting
                    // with unsaved content.
                    self.should_quit = !self.buffer.is_modified();
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

        if key.code == KeyCode::F(2) {
            self.begin_save_as();
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
        let mut app = App::open("/helios-note-newline-regression-missing");
        app.handle_key(
            KeyEvent::new(KeyCode::Char('\n'), KeyModifiers::NONE),
            20,
            80,
        );

        assert_eq!(app.buffer.lines, vec![String::new(), String::new()]);
        assert_eq!((app.cursor.line, app.cursor.col), (1, 0));
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE), 20, 80);
    }

    fn type_text(app: &mut App, text: &str) {
        for ch in text.chars() {
            press(app, KeyCode::Char(ch));
        }
    }

    fn ctrl_s(app: &mut App) {
        app.handle_key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            20,
            80,
        );
    }

    fn unique_path(tag: &str) -> String {
        format!("/tmp/helios_note_{}_{}.txt", tag, std::process::id())
    }

    #[test]
    fn untitled_save_opens_save_as_with_writable_default() {
        let mut app = App::untitled();
        assert!(app.document.is_untitled());

        type_text(&mut app, "Sunlight save test");
        assert!(app.buffer.is_modified());

        ctrl_s(&mut app);
        let prompt = app.save_as.as_ref().expect("Save As prompt is open");
        assert!(prompt.input.starts_with('/'));
        assert!(prompt.input.ends_with("untitled.txt"));
        assert_ne!(prompt.input, "/untitled.txt");
        assert!(app.document.backing_path().is_none());
        assert!(app.buffer.is_modified());
    }

    #[test]
    fn save_as_establishes_backing_path_and_later_saves_are_silent() {
        let target = unique_path("flow");
        let _ = std::fs::remove_file(&target);

        let mut app = App::untitled();
        type_text(&mut app, "first");
        ctrl_s(&mut app);

        let prompt = app.save_as.as_mut().expect("Save As prompt is open");
        prompt.input = target.clone();
        prompt.cursor = prompt.input.chars().count();
        press(&mut app, KeyCode::Enter);

        assert!(app.save_as.is_none());
        assert_eq!(app.document.backing_path(), Some(Path::new(&target)));
        assert_eq!(
            app.document.display_name(),
            Path::new(&target).file_name().unwrap()
        );
        assert!(!app.buffer.is_modified());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "first\n");

        type_text(&mut app, "-second");
        assert!(app.buffer.is_modified());
        ctrl_s(&mut app);

        assert!(app.save_as.is_none());
        assert!(!app.buffer.is_modified());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "first-second\n");
        let _ = std::fs::remove_file(&target);
    }

    #[test]
    fn bare_filename_is_resolved_against_the_save_as_base() {
        let dir = format!("/tmp/helios_note_base_{}", std::process::id());
        std::fs::create_dir_all(&dir).unwrap();
        let seed = format!("{}/a.txt", dir);
        std::fs::write(&seed, "seed\n").unwrap();

        let mut app = App::open(&seed);
        type_text(&mut app, "x");
        press(&mut app, KeyCode::F(2));

        let prompt = app.save_as.as_mut().expect("Save As prompt is open");
        prompt.input = String::from("b.txt");
        prompt.cursor = prompt.input.chars().count();
        press(&mut app, KeyCode::Enter);

        let expected = format!("{}/b.txt", dir);
        assert_eq!(app.document.backing_path(), Some(Path::new(&expected)));
        assert!(Path::new(&expected).exists());
        // The previous file keeps its original contents.
        assert_eq!(std::fs::read_to_string(&seed).unwrap(), "seed\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_target_requires_overwrite_confirmation() {
        let target = unique_path("overwrite");
        std::fs::write(&target, "old\n").unwrap();

        let mut app = App::untitled();
        type_text(&mut app, "new");
        ctrl_s(&mut app);
        let prompt = app.save_as.as_mut().expect("Save As prompt is open");
        prompt.input = target.clone();
        prompt.cursor = prompt.input.chars().count();
        press(&mut app, KeyCode::Enter);

        assert!(app
            .save_as
            .as_ref()
            .and_then(SaveAsPrompt::pending_overwrite)
            .is_some());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old\n");

        press(&mut app, KeyCode::Char('n'));
        assert!(app.document.is_untitled());
        assert!(app.buffer.is_modified());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old\n");

        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('y'));
        assert!(app.save_as.is_none());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new\n");
        assert!(!app.buffer.is_modified());
        let _ = std::fs::remove_file(&target);
    }

    #[test]
    fn cancelled_save_as_keeps_document_untouched() {
        let mut app = App::untitled();
        type_text(&mut app, "draft");
        ctrl_s(&mut app);
        press(&mut app, KeyCode::Esc);

        assert!(app.save_as.is_none());
        assert!(app.document.is_untitled());
        assert!(app.buffer.is_modified());
        assert_eq!(app.buffer.lines, vec![String::from("draft")]);
    }

    #[test]
    fn failed_save_as_preserves_contents_and_dirty_state() {
        let mut app = App::untitled();
        type_text(&mut app, "policy");
        ctrl_s(&mut app);

        let prompt = app.save_as.as_mut().expect("Save As prompt is open");
        prompt.input = String::from("/helios-note-missing-dir/denied.txt");
        prompt.cursor = prompt.input.chars().count();
        press(&mut app, KeyCode::Enter);

        assert!(app.save_as.is_some());
        assert!(app.document.is_untitled());
        assert!(app.buffer.is_modified());
        assert_eq!(app.buffer.lines, vec![String::from("policy")]);
        let msg = app.status_message.as_deref().unwrap_or_default();
        assert!(!msg.is_empty());
        assert!(!Path::new("/helios-note-missing-dir/denied.txt").exists());
    }
}
