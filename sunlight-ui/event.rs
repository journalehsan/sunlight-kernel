use crate::geom::Point;

/// User input event dispatched by the event loop.
///
/// Designed to carry everything the display server's SGP `EVENT_POLL`
/// reply can deliver: mouse position, keyboard state, and button clicks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// Mouse button click at window-local coordinates.
    Click { x: i32, y: i32 },

    /// Keyboard event with optional decoded ASCII byte.
    Key { keycode: u8, pressed: bool, ascii: Option<u8> },

    /// Timer tick or idle poll — no user input pending.
    Tick,
}

impl Event {
    pub fn click(x: i32, y: i32) -> Self {
        Self::Click { x, y }
    }

    pub fn key(keycode: u8, pressed: bool, ascii: Option<u8>) -> Self {
        Self::Key { keycode, pressed, ascii }
    }

    /// Return the mouse position if this is a click event.
    pub fn pos(&self) -> Option<Point> {
        match self {
            Self::Click { x, y } => Some(Point::new(*x, *y)),
            _ => None,
        }
    }
}

/// Trait for widgets that can process events.
///
/// Container types (e.g. `VBox`, `HBox`) implement this by routing
/// events to their children via `hit_test`.
pub trait HandleEvent {
    /// Process an event. Return `true` if the event was consumed.
    fn handle_event(&mut self, event: Event) -> bool;
}
