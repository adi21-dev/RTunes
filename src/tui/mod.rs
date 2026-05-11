//! Terminal UI: ratatui render loop, crossterm input, and theme-aware layout.

#![allow(dead_code)] // Submodules use each other; public API for future phases.

pub mod color;
pub mod events;
pub mod terminal;
pub mod ui;

#[allow(unused_imports)]
pub use terminal::TerminalGuard;
pub use ui::run as run_tui;
pub use ui::{build_snapshot, draw_frame, LibraryRowSnap, RenderScratch, RenderSnapshot};
