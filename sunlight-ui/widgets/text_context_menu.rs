//! Shared context-menu model and renderer for interactive text widgets.

use crate::font::VecText;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::Theme;

const MENU_WIDTH: u32 = 220;
const ITEM_HEIGHT: u32 = 24;
const SEPARATOR_HEIGHT: u32 = 9;
const MENU_PAD: i32 = 4;
const MAX_ITEMS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

impl TextCommand {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::Delete => "Delete",
            Self::SelectAll => "Select All",
        }
    }

    pub const fn shortcut(self) -> &'static str {
        match self {
            Self::Undo => "Ctrl+Z",
            Self::Redo => "Ctrl+Y",
            Self::Cut => "Ctrl+X",
            Self::Copy => "Ctrl+C",
            Self::Paste => "Ctrl+V",
            Self::Delete => "",
            Self::SelectAll => "Ctrl+A",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextWidgetKind {
    EditableSingleLine,
    EditableMultiline,
    SelectableReadOnly,
}

impl TextWidgetKind {
    pub const fn editable(self) -> bool {
        !matches!(self, Self::SelectableReadOnly)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextMenuState {
    pub kind: TextWidgetKind,
    pub has_selection: bool,
    pub has_text: bool,
    pub can_paste: bool,
    pub can_delete: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

impl TextMenuState {
    pub const fn editable(kind: TextWidgetKind, has_selection: bool, has_text: bool) -> Self {
        Self {
            kind,
            has_selection,
            has_text,
            can_paste: true,
            can_delete: has_selection,
            can_undo: false,
            can_redo: false,
        }
    }

    pub const fn selectable(has_selection: bool, has_text: bool) -> Self {
        Self {
            kind: TextWidgetKind::SelectableReadOnly,
            has_selection,
            has_text,
            can_paste: false,
            can_delete: false,
            can_undo: false,
            can_redo: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Command { command: TextCommand, enabled: bool },
    Separator,
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextContextMenu {
    pub rect: Rect,
    entries: [Entry; MAX_ITEMS],
    count: usize,
}

impl TextContextMenu {
    pub fn open_at(x: i32, y: i32, bounds: Rect, state: TextMenuState) -> Self {
        let mut entries = [Entry::Empty; MAX_ITEMS];
        let mut count = 0usize;
        let mut push = |entry| {
            if count < entries.len() {
                entries[count] = entry;
                count += 1;
            }
        };

        if state.kind.editable() && (state.can_undo || state.can_redo) {
            push(Entry::Command {
                command: TextCommand::Undo,
                enabled: state.can_undo,
            });
            push(Entry::Command {
                command: TextCommand::Redo,
                enabled: state.can_redo,
            });
            push(Entry::Separator);
        }

        if state.kind.editable() {
            push(Entry::Command {
                command: TextCommand::Cut,
                enabled: state.has_selection,
            });
            push(Entry::Command {
                command: TextCommand::Copy,
                enabled: state.has_selection,
            });
            push(Entry::Command {
                command: TextCommand::Paste,
                enabled: state.can_paste,
            });
            push(Entry::Command {
                command: TextCommand::Delete,
                enabled: state.can_delete,
            });
        } else {
            push(Entry::Command {
                command: TextCommand::Copy,
                enabled: state.has_selection,
            });
        }
        push(Entry::Separator);
        push(Entry::Command {
            command: TextCommand::SelectAll,
            enabled: state.has_text,
        });

        let content_h: u32 = entries[..count]
            .iter()
            .map(|entry| match entry {
                Entry::Separator => SEPARATOR_HEIGHT,
                Entry::Command { .. } => ITEM_HEIGHT,
                Entry::Empty => 0,
            })
            .sum();
        let height = content_h + (MENU_PAD as u32 * 2);
        let max_x = (bounds.right() - MENU_WIDTH as i32).max(bounds.x);
        let max_y = (bounds.bottom() - height as i32).max(bounds.y);
        let rect = Rect::new(
            x.clamp(bounds.x, max_x),
            y.clamp(bounds.y, max_y),
            MENU_WIDTH.min(bounds.w),
            height.min(bounds.h),
        );
        Self {
            rect,
            entries,
            count,
        }
    }

    pub fn command_at(&self, point: Point) -> Option<TextCommand> {
        if !self.rect.contains(point) {
            return None;
        }
        let mut y = self.rect.y + MENU_PAD;
        for entry in &self.entries[..self.count] {
            match *entry {
                Entry::Command { command, enabled } => {
                    let rect = Rect::new(
                        self.rect.x + MENU_PAD,
                        y,
                        self.rect.w.saturating_sub((MENU_PAD * 2) as u32),
                        ITEM_HEIGHT,
                    );
                    if enabled && rect.contains(point) {
                        return Some(command);
                    }
                    y += ITEM_HEIGHT as i32;
                }
                Entry::Separator => y += SEPARATOR_HEIGHT as i32,
                Entry::Empty => {}
            }
        }
        None
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme, font: Option<&dyn VecText>) {
        canvas.fill_rounded_rect(self.rect, 7, theme.panel);
        canvas.stroke_rounded_rect(self.rect, 7, 1, theme.border);
        let mut y = self.rect.y + MENU_PAD;
        for entry in &self.entries[..self.count] {
            match *entry {
                Entry::Command { command, enabled } => {
                    let rect = Rect::new(
                        self.rect.x + MENU_PAD,
                        y,
                        self.rect.w.saturating_sub((MENU_PAD * 2) as u32),
                        ITEM_HEIGHT,
                    );
                    let color = if enabled { theme.text } else { theme.text_dim };
                    draw_vcenter(canvas, font, command.label(), rect.x + 8, rect, color);
                    let shortcut = command.shortcut();
                    if !shortcut.is_empty() {
                        let width = measure_w(font, shortcut) as i32;
                        draw_vcenter(
                            canvas,
                            font,
                            shortcut,
                            rect.right() - width - 8,
                            rect,
                            theme.text_dim,
                        );
                    }
                    y += ITEM_HEIGHT as i32;
                }
                Entry::Separator => {
                    canvas.hbar(
                        self.rect.x + 9,
                        y + (SEPARATOR_HEIGHT as i32 / 2),
                        self.rect.w.saturating_sub(18),
                        1,
                        theme.border,
                    );
                    y += SEPARATOR_HEIGHT as i32;
                }
                Entry::Empty => {}
            }
        }
    }

    #[cfg(test)]
    fn commands(&self) -> alloc::vec::Vec<(TextCommand, bool)> {
        self.entries[..self.count]
            .iter()
            .filter_map(|entry| match *entry {
                Entry::Command { command, enabled } => Some((command, enabled)),
                _ => None,
            })
            .collect()
    }
}

fn measure_w(font: Option<&dyn VecText>, text: &str) -> u32 {
    font.map(|font| font.measure_w(text))
        .unwrap_or_else(|| Canvas::measure_text(text))
}

fn draw_vcenter(
    canvas: &mut Canvas,
    font: Option<&dyn VecText>,
    text: &str,
    x: i32,
    rect: Rect,
    color: crate::theme::Color,
) {
    if let Some(font) = font {
        font.draw_vcenter(canvas, text, x, rect.y, rect.h, color);
    } else {
        let y = rect.y + (rect.h as i32 - 10) / 2;
        canvas.draw_text(x, y, text, color);
    }
}

#[cfg(test)]
mod tests {
    use super::{TextCommand, TextContextMenu, TextMenuState, TextWidgetKind};
    use crate::geom::{Point, Rect};

    #[test]
    fn editable_menu_exposes_local_shortcuts_and_state() {
        let mut state = TextMenuState::editable(TextWidgetKind::EditableSingleLine, true, true);
        state.can_delete = true;
        let menu = TextContextMenu::open_at(20, 20, Rect::new(0, 0, 400, 400), state);
        assert_eq!(
            menu.commands(),
            alloc::vec![
                (TextCommand::Cut, true),
                (TextCommand::Copy, true),
                (TextCommand::Paste, true),
                (TextCommand::Delete, true),
                (TextCommand::SelectAll, true),
            ]
        );
        assert_eq!(TextCommand::Copy.shortcut(), "Ctrl+C");
    }

    #[test]
    fn read_only_menu_omits_mutating_commands() {
        let menu = TextContextMenu::open_at(
            390,
            390,
            Rect::new(0, 0, 400, 400),
            TextMenuState::selectable(true, true),
        );
        assert_eq!(
            menu.commands(),
            alloc::vec![(TextCommand::Copy, true), (TextCommand::SelectAll, true)]
        );
        assert!(menu.rect.right() <= 400);
        assert!(menu.rect.bottom() <= 400);
        assert!(menu.command_at(Point::new(-1, -1)).is_none());
    }
}
