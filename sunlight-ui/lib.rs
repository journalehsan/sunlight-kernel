//! # sunlight-ui
//!
//! SunlightOS widget toolkit — framebuffer-native, zero-alloc, no_std compatible.
//!
//! ## Usage
//!
//! ```ignore
//! use sunlight_ui::{Theme, Canvas, widgets::*};
//! use sunlight_ui::geom::Rect;
//!
//! // 1. Pick (or load) a theme — one change repaints every widget.
//! let theme = Theme::sunlight_dark();
//!
//! // 2. Wrap your framebuffer.
//! let mut canvas = Canvas::new(&mut fb_pixels, stride, width, height);
//!
//! // 3. Draw widgets.
//! let btn = Button::new(Rect::new(10, 10, 80, 24), "Launch");
//! btn.draw(&mut canvas, &theme);
//! ```
//!
//! ## Theming
//!
//! All color references go through [`Theme`]. To add a new theme, construct
//! a `Theme` literal and pass it to your draw calls — no widget code changes.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod draw;
pub mod event;
pub mod font;
pub mod geom;
pub mod horizon;
pub mod image;
pub mod layout;
pub mod material;
pub mod paint;
pub mod theme;
pub mod widgets;

// Application framework (requires the "app" feature + sunlight-ipc)
#[cfg(feature = "app")]
pub mod app;

// Top-level re-exports for convenience
pub use event::Event;
pub use font::{UiGlyph, UiSymbol, VecText};
pub use geom::{Point, Rect, Size};
pub use horizon::{
    HorizonControl, HorizonControlState, HorizonLayout, HorizonMetrics, HorizonPalette,
};
pub use layout::{GridColIter, GridRow, HBox, VBox};
pub use material::{Material, MaterialKind, SurfaceRole};
pub use paint::Canvas;
pub use theme::{Color, Theme};

#[cfg(feature = "app")]
pub use app::{
    request_close, set_client_cursor, App, CursorShape, EventPollCounters, Window, WindowConfig,
    WindowDecoration,
};
