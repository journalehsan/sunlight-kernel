//! Reusable high-level media playback for SunlightOS applications.
//!
//! Applications deal in sources, playback state, time, and bounded updates.
//! Container packets, codec details, PCM queues, and audio devices remain
//! private implementation details.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod decoder;
pub mod error;
pub mod output;
pub mod player;
#[cfg(any(target_os = "none", test))]
mod state;
pub mod types;
pub mod visualization;

pub use error::{MediaError, MediaErrorKind};
pub use player::{MediaPlayer, MediaSnapshot};
pub use types::{AudioStreamInfo, MediaTime, PcmFormat, PlaybackState};
pub use visualization::{VisualizationFrame, MAX_VISUALIZATION_BINS};
