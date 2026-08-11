use crate::geom::Point;

/// User input event dispatched by the event loop.
///
/// Designed to carry everything the display server's SGP `EVENT_POLL`
/// reply can deliver: mouse position, keyboard state, and button clicks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// Mouse button released (click complete) at window-local coordinates.
    Click {
        x: i32,
        y: i32,
    },

    /// Mouse button pressed down at window-local coordinates.
    /// `button`: 0 = left, 1 = right, 2 = middle.
    MouseDown {
        x: i32,
        y: i32,
        button: u8,
    },

    /// Mouse button released at window-local coordinates.
    /// `button`: 0 = left, 1 = right, 2 = middle.
    MouseUp {
        x: i32,
        y: i32,
        button: u8,
    },

    /// Pointer moved (no button change). Coordinates are window-local.
    MouseMove {
        x: i32,
        y: i32,
    },

    /// The compositor changed keyboard/input focus for this window.
    FocusChanged {
        focused: bool,
    },

    /// The native pointer entered or left this window's owned client input
    /// region. Captured drags remain owned until their final release.
    PointerOwnership {
        owned: bool,
        captured: bool,
    },

    /// Decoded character input.
    Key(char),

    /// Raw key transition for non-text controls.
    KeyPress {
        keycode: u8,
        pressed: bool,
        shift: bool,
        ctrl: bool,
        alt: bool,
        super_key: bool,
    },

    /// Mouse wheel scrolled at window-local coordinates.
    /// `delta` is the vertical scroll delta: positive = scroll down, negative = scroll up.
    /// For high-resolution touchpads the delta may be fractional steps using a fixed-point
    /// representation where 120 units = one conventional wheel detent (Windows WHEEL_DELTA).
    MouseWheel {
        x: i32,
        y: i32,
        delta: i16,
    },

    // Timer tick or idle poll — no user input pending.
    Tick,
}

impl Event {
    pub fn click(x: i32, y: i32) -> Self {
        Self::Click { x, y }
    }

    pub fn mouse_down(x: i32, y: i32, button: u8) -> Self {
        Self::MouseDown { x, y, button }
    }

    pub fn mouse_up(x: i32, y: i32, button: u8) -> Self {
        Self::MouseUp { x, y, button }
    }

    pub fn mouse_move(x: i32, y: i32) -> Self {
        Self::MouseMove { x, y }
    }

    pub fn key(ch: char) -> Self {
        Self::Key(ch)
    }

    pub fn key_press(
        keycode: u8,
        pressed: bool,
        shift: bool,
        ctrl: bool,
        alt: bool,
        super_key: bool,
    ) -> Self {
        Self::KeyPress {
            keycode,
            pressed,
            shift,
            ctrl,
            alt,
            super_key,
        }
    }

    pub fn mouse_wheel(x: i32, y: i32, delta: i16) -> Self {
        Self::MouseWheel { x, y, delta }
    }

    /// Return the mouse position if this is a pointer event.
    pub fn pos(&self) -> Option<Point> {
        match self {
            Self::Click { x, y }
            | Self::MouseDown { x, y, .. }
            | Self::MouseUp { x, y, .. }
            | Self::MouseMove { x, y }
            | Self::MouseWheel { x, y, .. } => Some(Point::new(*x, *y)),
            _ => None,
        }
    }
}

/// Window-system events delivered once at the application host boundary.
/// Widgets continue receiving ordinary [`Event`] values and react to layout
/// constraints rather than compositor protocol details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEvent {
    /// A replacement drawable client surface is active. Dimensions exclude
    /// borders, titlebars, and other compositor-owned decorations.
    Resized { width: u32, height: u32 },
}

/// Trait for widgets that can process events.
///
/// Container types (e.g. `VBox`, `HBox`) implement this by routing
/// events to their children via `hit_test`.
pub trait HandleEvent {
    /// Process an event. Return `true` if the event was consumed.
    fn handle_event(&mut self, event: Event) -> bool;
}
