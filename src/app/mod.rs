//! Application state and shared handles.

use std::sync::{Arc, Mutex, MutexGuard};

pub mod state;

use crate::config::RtunesConfig;
use crate::config::Theme;

/// Lock an `Arc<Mutex<T>>`, recovering from poison so the TUI keeps running after a panicked guard.
#[inline]
pub fn lock_shared<T>(m: &Arc<Mutex<T>>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Wraps [`state::AppState`] for cross-thread access in later phases.
pub fn new_shared_state(config: &RtunesConfig, theme: Theme) -> Arc<Mutex<state::AppState>> {
    Arc::new(Mutex::new(state::AppState::new(config, theme)))
}
