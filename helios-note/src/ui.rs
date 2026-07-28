//! Ratatui UI components and layout rendering.

use crate::core::{buffer::TextBuffer, cursor::Cursor};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub struct UiState<'a> {
    pub filename: &'a str,
    pub buffer: &'a TextBuffer,
    pub cursor: &'a Cursor,
    pub show_help: bool,
    pub show_quit_confirm: bool,
    pub show_search_prompt: bool,
    pub search_input: &'a str,
    pub status_message: Option<&'a str>,
}

pub fn draw_ui(f: &mut Frame, state: UiState) {
    let size = f.size();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Title bar
            Constraint::Min(1),    // Main viewport
            Constraint::Length(1), // Status bar
            Constraint::Length(1), // Bottom shortcut bar
        ])
        .split(size);

    draw_header(f, state.filename, state.buffer, chunks[0]);
    draw_viewport(f, state.buffer, state.cursor, chunks[1]);
    draw_status_bar(
        f,
        state.buffer,
        state.cursor,
        state.status_message,
        state.show_search_prompt,
        state.search_input,
        chunks[2],
    );
    draw_shortcuts(f, chunks[3]);

    if state.show_help {
        draw_help_modal(f, size);
    }

    if state.show_quit_confirm {
        draw_quit_modal(f, size);
    }
}

fn draw_header(f: &mut Frame, filename: &str, buffer: &TextBuffer, area: Rect) {
    let modified_str = if buffer.is_modified() { " [*]" } else { "" };
    let header_text = Line::from(vec![
        Span::styled(
            " Helios Note ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} {}", filename, modified_str),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let paragraph = Paragraph::new(header_text).style(Style::default().bg(Color::DarkGray));
    f.render_widget(paragraph, area);
}

fn draw_viewport(f: &mut Frame, buffer: &TextBuffer, cursor: &Cursor, area: Rect) {
    let height = area.height as usize;
    let line_num_width = format!("{}", buffer.len().max(1)).len().max(3);

    let mut lines = Vec::with_capacity(height);

    for i in 0..height {
        let line_idx = cursor.scroll_y + i;
        if line_idx < buffer.len() {
            let line_num_str = format!("{:>width$} │ ", line_idx + 1, width = line_num_width);
            let mut spans = vec![Span::styled(
                line_num_str,
                Style::default().fg(Color::DarkGray),
            )];

            if let Some(content) = buffer.get_line(line_idx) {
                let chars: Vec<char> = content.chars().collect();
                let visible_chars: String = if cursor.scroll_x < chars.len() {
                    chars[cursor.scroll_x..].iter().collect()
                } else {
                    String::new()
                };
                spans.push(Span::styled(
                    visible_chars,
                    Style::default().fg(Color::White),
                ));
            }

            lines.push(Line::from(spans));
        } else {
            let tilde_str = format!("{:>width$} │ ~", "~", width = line_num_width);
            lines.push(Line::from(Span::styled(
                tilde_str,
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    let viewport = Paragraph::new(lines);
    f.render_widget(viewport, area);

    // Calculate visual cursor position inside area
    let visual_col = buffer.visual_col(cursor.line, cursor.col, 4);
    let screen_x =
        area.x + (line_num_width as u16) + 3 + (visual_col.saturating_sub(cursor.scroll_x) as u16);
    let screen_y = area.y + ((cursor.line.saturating_sub(cursor.scroll_y)) as u16);

    if screen_x < area.x + area.width && screen_y < area.y + area.height {
        f.set_cursor(screen_x, screen_y);
    }
}

fn draw_status_bar(
    f: &mut Frame,
    buffer: &TextBuffer,
    cursor: &Cursor,
    status_msg: Option<&str>,
    show_search: bool,
    search_input: &str,
    area: Rect,
) {
    if show_search {
        let search_line = Line::from(vec![
            Span::styled(
                " Search: ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}_", search_input),
                Style::default().fg(Color::White).bg(Color::Black),
            ),
            Span::styled(
                "  (Press Enter to confirm, Esc to cancel)",
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        let paragraph = Paragraph::new(search_line).style(Style::default().bg(Color::DarkGray));
        f.render_widget(paragraph, area);
        return;
    }

    let pos_str = format!(" Ln {}, Col {} ", cursor.line + 1, cursor.col + 1);
    let total_str = format!(" Total: {} lines ", buffer.len());
    let enc_str = " UTF-8 ";

    let msg = status_msg.unwrap_or("");

    let status_line = Line::from(vec![
        Span::styled(format!(" {} ", msg), Style::default().fg(Color::Yellow)),
        Span::styled(" ", Style::default()),
        Span::styled(total_str, Style::default().fg(Color::Black).bg(Color::Gray)),
        Span::styled(
            pos_str,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(enc_str, Style::default().fg(Color::White).bg(Color::Blue)),
    ]);

    let paragraph = Paragraph::new(status_line).style(Style::default().bg(Color::DarkGray));
    f.render_widget(paragraph, area);
}

fn draw_shortcuts(f: &mut Frame, area: Rect) {
    let shortcuts = Line::from(vec![
        Span::styled(
            " ^S",
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Save "),
        Span::styled(
            " ^F",
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Find "),
        Span::styled(
            " F3",
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Next "),
        Span::styled(
            " ^Z",
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Undo "),
        Span::styled(
            " ^Y",
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Redo "),
        Span::styled(
            " ^G",
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Help "),
        Span::styled(
            " ^Q",
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Quit "),
    ]);
    let paragraph = Paragraph::new(shortcuts).style(Style::default().bg(Color::Black));
    f.render_widget(paragraph, area);
}

fn draw_help_modal(f: &mut Frame, area: Rect) {
    let modal_area = centered_rect(60, 60, area);
    f.render_widget(Clear, modal_area);

    let help_text = vec![
        Line::from(Span::styled(
            " Helios Note Help ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Typing     : Insert characters at cursor"),
        Line::from("  Enter      : Newline"),
        Line::from("  Backspace  : Delete character before cursor / join lines"),
        Line::from("  Delete     : Delete character under cursor"),
        Line::from("  Arrow Keys : Navigate cursor"),
        Line::from("  Home / End : Line start / end"),
        Line::from("  PgUp / PgDn: Page up / down"),
        Line::from("  Ctrl+S     : Save file"),
        Line::from("  Ctrl+F     : Search text"),
        Line::from("  F3         : Find next match"),
        Line::from("  Ctrl+Z     : Undo edit"),
        Line::from("  Ctrl+Y     : Redo edit"),
        Line::from("  Ctrl+G     : Toggle Help"),
        Line::from("  Ctrl+Q / ^X: Quit editor"),
        Line::from(""),
        Line::from(Span::styled(
            " Press Esc or ^G to close help ",
            Style::default().fg(Color::Green),
        )),
    ];

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));
    let paragraph = Paragraph::new(help_text).block(block);
    f.render_widget(paragraph, modal_area);
}

fn draw_quit_modal(f: &mut Frame, area: Rect) {
    let modal_area = centered_rect(50, 30, area);
    f.render_widget(Clear, modal_area);

    let content = vec![
        Line::from(Span::styled(
            " Save Unsaved Changes? ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(" You have modified changes in this document."),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  Y / S ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Save & Quit   "),
            Span::styled(
                " D / N ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Discard   "),
            Span::styled(
                " Esc / C ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Cancel "),
        ]),
    ];

    let block = Block::default()
        .title(" Unsaved Changes ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White));
    let paragraph = Paragraph::new(content).block(block);
    f.render_widget(paragraph, modal_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
