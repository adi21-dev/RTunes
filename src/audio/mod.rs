//! Audio playback: rodio backend, PCM tap, and headless test backend.

#![allow(unused_imports)] // Public API for Phase 5+ wiring; not all symbols used in-tree yet.
#![allow(dead_code)] // Audio stack is exercised by tests and Phase 5 TUI wiring.

pub mod backend;
pub mod player;
pub mod tap_source;

pub use backend::{AudioBackend, SilentBackend};
pub use player::{AudioPlayer, RodioBackend};
pub use tap_source::{SampleRing, StereoSample, TapSource};
