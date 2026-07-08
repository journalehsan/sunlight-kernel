//! Reusable calendar and form styling helpers.

use crate::geom::Rect;
use crate::theme::{Color, Theme};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CalendarCellState {
    Normal,
    Today,
    Selected,
    SelectedToday,
}

#[derive(Clone, Copy)]
pub struct CalendarCellStyle {
    pub fill: Option<Color>,
    pub border: Color,
    pub text: Color,
    pub marker: Color,
}

impl CalendarCellStyle {
    pub fn from_theme(theme: &Theme, state: CalendarCellState, has_events: bool) -> Self {
        let marker = if has_events {
            theme.accent
        } else {
            Color::TRANSPARENT
        };
        match state {
            CalendarCellState::Normal => Self {
                fill: None,
                border: theme.border,
                text: theme.text,
                marker,
            },
            CalendarCellState::Today => Self {
                fill: Some(theme.panel_alt),
                border: theme.accent,
                text: theme.text,
                marker,
            },
            CalendarCellState::Selected => Self {
                fill: Some(theme.accent),
                border: theme.accent_hover,
                text: theme.text_on_accent,
                marker: theme.text_on_accent,
            },
            CalendarCellState::SelectedToday => Self {
                fill: Some(theme.accent_hover),
                border: theme.text,
                text: theme.text_on_accent,
                marker: theme.text_on_accent,
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StatusTextKind {
    Normal,
    Muted,
    Error,
    Success,
}

pub fn status_text_color(theme: &Theme, kind: StatusTextKind) -> Color {
    match kind {
        StatusTextKind::Normal => theme.text,
        StatusTextKind::Muted => theme.text_muted,
        StatusTextKind::Error => theme.danger_text,
        StatusTextKind::Success => theme.ok,
    }
}

#[derive(Clone, Copy)]
pub struct FormFieldStyle {
    pub fill: Color,
    pub border: Color,
    pub label: Color,
    pub text: Color,
}

pub fn form_field_style(theme: &Theme, focused: bool) -> FormFieldStyle {
    FormFieldStyle {
        fill: if focused { theme.panel_alt } else { theme.bg },
        border: if focused { theme.accent } else { theme.border },
        label: theme.text_muted,
        text: theme.text,
    }
}

#[derive(Clone, Copy)]
pub struct EmptyStateStyle {
    pub rect: Rect,
    pub text: Color,
}

impl EmptyStateStyle {
    pub fn new(rect: Rect, theme: &Theme) -> Self {
        Self {
            rect,
            text: theme.text_muted,
        }
    }
}
