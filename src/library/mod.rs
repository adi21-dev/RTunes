//! Library indexing and filesystem scanning.
//!
//! - [`scan_paths`] — synchronous recursive walk + metadata + dedupe by canonical path.
//! - [`scan_async`] — same work on a background thread with [`ScanEvent`] progress.
//! - [`scan_config_paths`] — expands `~` in config strings then calls [`scan_paths`].

pub mod scanner;

pub use scanner::{is_scanning, scan_async, scan_config_paths, scan_paths, ScanEvent};
